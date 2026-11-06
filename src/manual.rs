//! The game's manual: a PDF (EmulationStation's scraped `manual`), rendered a
//! page at a time by poppler's `pdftoppm` in a thread of its own.
//!
//! Using poppler's command line tools rather than linking a PDF library keeps
//! the overlay small and its licence simple; every distribution ships them
//! (`poppler-utils`). A page takes some tens of milliseconds, so the pages
//! next to the one shown are rendered ahead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use tiny_skia::Pixmap;

/// The page renderer.
const PDFTOPPM: &str = "pdftoppm";
/// The page counter.
const PDFINFO: &str = "pdfinfo";
/// Rendered pages kept around (the current one and its neighbours).
const CACHE_PAGES: usize = 6;
/// The screen part a whole page may take: the rest is for the page number.
const PAGE_WIDTH: f32 = 0.96;
const PAGE_HEIGHT: f32 = 0.9;
/// The width of a zoomed page, relative to the screen...
const ZOOM_WIDTH: f32 = 0.9;
/// ...but never more than this many times its whole page size (a big page,
/// rendered for a 4K screen, would take hundreds of megabytes).
const MAX_ZOOM: f32 = 2.5;

/// How a page is fitted to the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fit {
    /// The whole page.
    Page,
    /// Zoomed to (almost) the screen's width, scrolled up and down.
    Width,
}

impl Fit {
    pub const fn toggled(self) -> Self {
        match self {
            Self::Page => Self::Width,
            Self::Width => Self::Page,
        }
    }
}

/// One page rendered at a size, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageRequest {
    pub page: u32,
    pub width: u32,
    pub height: u32,
}

/// A rendered page, or why it could not be.
pub type Rendered = (PageRequest, Result<Pixmap, String>);

/// The manual of the running game, and its renderer.
pub struct Manual {
    path: PathBuf,
    /// Each page's size in points, as shown (rotation applied).
    sizes: Vec<(f32, f32)>,
    cache: HashMap<PageRequest, Pixmap>,
    /// Requests sent and not answered yet.
    pending: Vec<PageRequest>,
    requests: Option<mpsc::Sender<PageRequest>>,
}

impl Manual {
    /// The manual, when the file is there and poppler's tools can read it.
    pub fn open(path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        let output = Command::new(PDFINFO)
            .args(["-f", "1", "-l", "100000"])
            .arg(path)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .inspect_err(|err| log::warn!("no {PDFINFO} for the manual: {err}"))
            .ok()?;
        let sizes = page_sizes(&String::from_utf8_lossy(&output.stdout));
        if sizes.is_empty() {
            log::warn!("cannot read the manual {}", path.display());
            return None;
        }
        Some(Self {
            path: path.to_owned(),
            sizes,
            cache: HashMap::new(),
            pending: Vec::new(),
            requests: None,
        })
    }

    pub fn pages(&self) -> u32 {
        u32::try_from(self.sizes.len()).unwrap_or(u32::MAX)
    }

    /// Start the renderer; what it renders goes to `deliver`.
    pub fn start(&mut self, deliver: impl Fn(Rendered) + Send + 'static) {
        let (sender, receiver) = mpsc::channel::<PageRequest>();
        let path = self.path.clone();
        let spawned = thread::Builder::new().name("manual".into()).spawn(move || {
            for request in receiver {
                deliver((request, render(&path, request)));
            }
        });
        match spawned {
            Ok(_) => self.requests = Some(sender),
            Err(err) => log::error!("cannot start the manual renderer: {err}"),
        }
    }

    /// The size a page (1-based) is rendered at, for a fit on a screen.
    pub fn request(&self, page: u32, fit: Fit, screen: (u32, u32)) -> Option<PageRequest> {
        let &(width, height) = self
            .sizes
            .get(usize::try_from(page).ok()?.checked_sub(1)?)?;
        let (screen_width, screen_height) = (screen.0 as f32, screen.1 as f32);
        let whole = (screen_width * PAGE_WIDTH / width).min(screen_height * PAGE_HEIGHT / height);
        let scale = match fit {
            Fit::Page => whole,
            Fit::Width => (screen_width * ZOOM_WIDTH / width).min(whole * MAX_ZOOM),
        };
        Some(PageRequest {
            page,
            width: ((width * scale).round() as u32).max(1),
            height: ((height * scale).round() as u32).max(1),
        })
    }

    /// The page, if rendered; else it is asked for (once).
    pub fn page(&mut self, request: PageRequest) -> Option<&Pixmap> {
        if !self.cache.contains_key(&request) {
            self.ask(request);
            return None;
        }
        self.cache.get(&request)
    }

    /// Render a page ahead of time.
    pub fn prefetch(&mut self, request: PageRequest) {
        if !self.cache.contains_key(&request) {
            self.ask(request);
        }
    }

    fn ask(&mut self, request: PageRequest) {
        if self.pending.contains(&request) {
            return;
        }
        if let Some(requests) = &self.requests
            && requests.send(request).is_ok()
        {
            self.pending.push(request);
        }
    }

    /// Keep a page the renderer has finished.
    pub fn store(&mut self, rendered: Rendered) {
        let (request, result) = rendered;
        self.pending.retain(|pending| *pending != request);
        match result {
            Ok(pixmap) => {
                if self.cache.len() >= CACHE_PAGES {
                    // forget the page farthest from this one
                    let far = self
                        .cache
                        .keys()
                        .max_by_key(|other| other.page.abs_diff(request.page))
                        .copied();
                    if let Some(far) = far {
                        self.cache.remove(&far);
                    }
                }
                self.cache.insert(request, pixmap);
            }
            Err(err) => log::warn!("manual page {}: {err}", request.page),
        }
    }
}

/// The pages' sizes in what `pdfinfo -f 1 -l N` prints:
/// `Page    1 size: 612 x 792 pts (letter)` then `Page    1 rot:  90`.
fn page_sizes(info: &str) -> Vec<(f32, f32)> {
    let mut sizes = Vec::new();
    for line in info.lines() {
        let Some(rest) = line.strip_prefix("Page") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(_number), Some(field)) = (words.next(), words.next()) else {
            continue;
        };
        match field {
            "size:" => {
                let width = words.next().and_then(|word| word.parse().ok());
                let height = words.nth(1).and_then(|word| word.parse().ok());
                if let (Some(width), Some(height)) = (width, height) {
                    sizes.push((width, height));
                }
            }
            "rot:" => {
                let quarter = words
                    .next()
                    .and_then(|word| word.parse::<i32>().ok())
                    .is_some_and(|rotation| rotation.rem_euclid(180) == 90);
                if quarter && let Some((width, height)) = sizes.last_mut() {
                    std::mem::swap(width, height);
                }
            }
            _ => {}
        }
    }
    sizes
}

/// Render one page with pdftoppm (a PNG on its standard output).
fn render(path: &Path, request: PageRequest) -> Result<Pixmap, String> {
    let page = request.page.to_string();
    let output = Command::new(PDFTOPPM)
        .args(["-f", &page, "-l", &page, "-singlefile", "-png"])
        .args(["-scale-to-x", &request.width.to_string()])
        .args(["-scale-to-y", &request.height.to_string()])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("{PDFTOPPM}: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Pixmap::decode_png(&output.stdout).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_manual_is_none() {
        assert!(Manual::open(Path::new("/nonexistent/manual.pdf")).is_none());
    }

    #[test]
    fn reads_page_sizes_and_rotation() {
        let sizes = page_sizes(
            "Pages:           2\n\
             Page    1 size:  612 x 792 pts (letter)\n\
             Page    1 rot:   0\n\
             Page    2 size:  595.276 x 841.89 pts (A4)\n\
             Page    2 rot:   90\n\
             File size:       1234 bytes\n",
        );
        assert_eq!(sizes, [(612.0, 792.0), (841.89, 595.276)]);
    }

    #[test]
    fn fits_pages_to_the_screen() {
        let manual = Manual {
            path: PathBuf::new(),
            sizes: vec![(600.0, 800.0), (1600.0, 600.0)],
            cache: HashMap::new(),
            pending: Vec::new(),
            requests: None,
        };
        let screen = (1920, 1080);
        let tall = manual.request(1, Fit::Page, screen).unwrap();
        assert_eq!((tall.width, tall.height), (729, 972));
        // a wide page is limited by the width
        let wide = manual.request(2, Fit::Page, screen).unwrap();
        assert_eq!(wide.width, 1843);
        let zoomed = manual.request(1, Fit::Width, screen).unwrap();
        assert_eq!(zoomed.width, 1728);
        assert!(manual.request(3, Fit::Page, screen).is_none());
        assert!(manual.request(0, Fit::Page, screen).is_none());
    }
}
