//! The game's RetroAchievements, earned and still to earn, for the
//! achievements page.
//!
//! They come from the server the emulators talk to (`dorequest.php`, as
//! rcheevos calls it), with the account the launcher passes in session.json
//! (`achievements`: the user and their token, or their password) and the
//! game's id EmulationStation found (game.xml's `cheevosId`, else its
//! `cheevosHash`):
//!
//! * `login2` swaps a password for a token (when there is no token);
//! * `gameid` finds the game from its hash (when there is no id);
//! * `patch` lists the game's achievements (only the official ones count);
//! * `unlocks` lists those the user earned, in softcore and in hardcore.
//!
//! `curl` does the requests (no TLS in this program), the account going
//! through its standard input, never its command line. The badges (one
//! picture per achievement, its own for an earned one and a grey `_lock`
//! one otherwise) are cached in Retrobox's cache directory: they never
//! change. Everything runs on a thread of its own; the menu shows the list
//! as soon as it is known, then the badges.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;

const SERVER: &str = "https://retroachievements.org/dorequest.php";
const BADGES: &str = "https://media.retroachievements.org/Badge";
/// The server wants to know who asks.
const USER_AGENT: &str = concat!("retrobox-overlay/", env!("CARGO_PKG_VERSION"));
/// What an official achievement's `Flags` are (5: unofficial).
const OFFICIAL: u64 = 3;

/// The RetroAchievements account (session.json's `achievements`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Account {
    pub username: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub password: String,
}

/// The game, as RetroAchievements knows it.
#[derive(Debug, Clone, Default)]
pub struct GameId {
    /// 0: unknown.
    pub id: u64,
    /// The hash RetroAchievements identifies it by.
    pub hash: String,
}

impl GameId {
    pub fn known(&self) -> bool {
        self.id > 0 || !self.hash.is_empty()
    }
}

/// Whether, and how, an achievement was earned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Earned {
    Hardcore,
    Softcore,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Achievement {
    pub title: String,
    pub description: String,
    pub points: u64,
    pub earned: Earned,
    /// The badge's name on the server.
    badge: String,
}

impl Achievement {
    /// The badge's file: its own picture once earned, the grey one before.
    fn badge_file(&self) -> String {
        match self.earned {
            Earned::No => format!("{}_lock.png", self.badge),
            Earned::Hardcore | Earned::Softcore => format!("{}.png", self.badge),
        }
    }

    /// Where its badge is cached.
    pub fn badge_path(&self, dir: &Path) -> PathBuf {
        dir.join(self.badge_file())
    }
}

/// The game's achievements: earned ones first, each group in the game's
/// order.
pub fn fetch(account: &Account, game: &GameId) -> Result<Vec<Achievement>> {
    let token = if account.token.is_empty() {
        let login = request(&[
            ("r", "login2"),
            ("u", &account.username),
            ("p", &account.password),
        ])?;
        text(&login, "Token").context("no token from the login")?
    } else {
        account.token.clone()
    };
    let id = if game.id > 0 {
        game.id
    } else {
        let found = request(&[("r", "gameid"), ("m", &game.hash)])?;
        match found.get("GameID").and_then(Value::as_u64) {
            Some(id) if id > 0 => id,
            _ => bail!("RetroAchievements does not know this game"),
        }
    };
    let id = id.to_string();
    let user = &account.username;
    let ask = |what: &str, extra: &[(&str, &str)]| {
        let mut fields = vec![("r", what), ("u", user), ("t", &token), ("g", &id)];
        fields.extend_from_slice(extra);
        request(&fields)
    };
    let hash: &[(&str, &str)] = if game.hash.is_empty() {
        &[]
    } else {
        &[("m", &game.hash)]
    };
    let patch = ask("patch", hash)?;
    let unlocked = |hardcore: &str| -> Result<HashSet<u64>> {
        let unlocks = ask("unlocks", &[("h", hardcore)])?;
        Ok(unlocks
            .get("UserUnlocks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .collect())
    };
    let hardcore = unlocked("1")?;
    let softcore = unlocked("0")?;
    let mut list = achievements(&patch, &hardcore, &softcore);
    // earned first (sort_by_key is stable: the game's order stays)
    list.sort_by_key(|achievement| achievement.earned == Earned::No);
    Ok(list)
}

/// The official achievements of a `patch` answer.
fn achievements(
    patch: &Value,
    hardcore: &HashSet<u64>,
    softcore: &HashSet<u64>,
) -> Vec<Achievement> {
    patch
        .pointer("/PatchData/Achievements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("Flags").and_then(Value::as_u64) == Some(OFFICIAL))
        .filter_map(|entry| {
            let id = entry.get("ID")?.as_u64()?;
            let earned = if hardcore.contains(&id) {
                Earned::Hardcore
            } else if softcore.contains(&id) {
                Earned::Softcore
            } else {
                Earned::No
            };
            Some(Achievement {
                title: text(entry, "Title")?,
                description: text(entry, "Description").unwrap_or_default(),
                points: entry.get("Points").and_then(Value::as_u64).unwrap_or(0),
                earned,
                badge: text(entry, "BadgeName").filter(|name| safe_name(name))?,
            })
        })
        .collect()
}

/// Fetch the badges `list` shows that are not cached yet, into `dir`
/// (several at a time).
pub fn fetch_badges(list: &[Achievement], dir: &Path) -> Result<()> {
    let mut config = String::new();
    let mut wanted = HashSet::new();
    for achievement in list {
        let file = achievement.badge_file();
        if !wanted.insert(file.clone()) || dir.join(&file).is_file() {
            continue;
        }
        // curl's config syntax: quoted, with backslash escapes (none here)
        let _ = writeln!(config, "url = \"{BADGES}/{file}\"");
        let _ = writeln!(config, "output = \"{}\"", dir.join(&file).display());
    }
    if config.is_empty() {
        return Ok(());
    }
    curl(
        &[
            "--fail",
            "--parallel",
            "--parallel-max",
            "8",
            "--create-dirs",
            "--remove-on-error",
            "--max-time",
            "60",
            "--config",
            "-",
        ],
        &config,
    )
    .map(drop)
}

/// A request to the server; the answer when it says it succeeded (an
/// error comes with an HTTP error status, and its reason in the answer).
fn request(fields: &[(&str, &str)]) -> Result<Value> {
    let body = fields
        .iter()
        .map(|(name, value)| format!("{name}={}", form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let answer = curl(&["--max-time", "20", "--data-binary", "@-", SERVER], &body)?;
    let answer: Value = serde_json::from_slice(&answer).context("RetroAchievements' answer")?;
    if answer.get("Success").and_then(Value::as_bool) == Some(true) {
        Ok(answer)
    } else {
        let error = text(&answer, "Error").unwrap_or_else(|| "unknown error".to_owned());
        Err(anyhow!("RetroAchievements: {error}"))
    }
}

/// Run curl, `input` on its standard input; its output.
fn curl(args: &[&str], input: &str) -> Result<Vec<u8>> {
    let mut child = Command::new("curl")
        .args(["--silent", "--show-error", "--user-agent", USER_AGENT])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot run curl")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).context("curl")?;
    }
    let output = child.wait_with_output().context("curl")?;
    if !output.status.success() {
        bail!("curl: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

/// `application/x-www-form-urlencoded`'s encoding.
fn form_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn text(value: &Value, name: &str) -> Option<String> {
    value.get(name)?.as_str().map(str::to_owned)
}

/// A badge's name is a file name here: nothing but letters, digits, `_`
/// and `-`.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_forms() {
        assert_eq!(form_encode("a b&c=é"), "a%20b%26c%3D%C3%A9");
        assert_eq!(form_encode("Tok-en_1.~"), "Tok-en_1.~");
    }

    #[test]
    fn reads_the_official_achievements() {
        let patch: Value = serde_json::from_str(
            r#"{"Success": true, "PatchData": {"ID": 1, "Title": "Game", "Achievements": [
                {"ID": 10, "Title": "First", "Description": "Do it", "Points": 5, "Flags": 3, "BadgeName": "123"},
                {"ID": 11, "Title": "Second", "Description": "", "Points": 10, "Flags": 3, "BadgeName": "124"},
                {"ID": 12, "Title": "Draft", "Description": "", "Points": 1, "Flags": 5, "BadgeName": "125"},
                {"ID": 13, "Title": "Bad badge", "Description": "", "Points": 1, "Flags": 3, "BadgeName": "../x"}
            ]}}"#,
        )
        .unwrap();
        let list = achievements(&patch, &HashSet::from([11]), &HashSet::from([10, 11]));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].earned, Earned::Softcore);
        assert_eq!(list[1].earned, Earned::Hardcore);
        assert_eq!(list[0].badge_file(), "123.png");
        assert_eq!(
            Achievement {
                earned: Earned::No,
                ..list[0].clone()
            }
            .badge_file(),
            "123_lock.png"
        );
    }
}
