//! The game's manual: a PDF (EmulationStation's scraped `manual`), rendered a
//! page at a time by poppler's `pdftoppm` in a thread of its own, as ES's
//! own viewer does.
//!
//! Using poppler's command line tools rather than linking a PDF library keeps
//! the overlay small and its licence simple; every distribution ships them
//! (`poppler-utils`). Pages come as raw PPM (no PNG to compress and decode)
//! and are turned at once into the compositor's pixel format, ready to be
//! copied into a buffer. The page on screen always goes first: the pages
//! rendered ahead wait, and requests nobody needs any more are dropped.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;

/// The page renderer.
const PDFTOPPM: &str = "pdftoppm";
/// The page counter.
const PDFINFO: &str = "pdfinfo";
/// How much memory the rendered pages may take.
const CACHE_BYTES: usize = 128 * 1024 * 1024;

/// One page rendered at a size, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageRequest {
    pub page: u32,
    pub width: u32,
    pub height: u32,
}

/// A rendered page: opaque pixels in `wl_shm`'s ARGB8888 (B, G, R, A in
/// memory), rows of `width * 4` bytes.
pub struct PageImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A rendered page, or why it could not be.
pub type Rendered = (PageRequest, Result<PageImage, String>);

/// What the renderer has to do.
#[derive(Default)]
struct Queue {
    /// The page on screen: first.
    wanted: Option<PageRequest>,
    /// The pages rendered ahead, nearest first.
    ahead: VecDeque<PageRequest>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
}

impl Shared {
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The manual of the running game, and its renderer.
pub struct Manual {
    path: PathBuf,
    /// Each page's size in points, as shown (crop box, rotation applied).
    sizes: Vec<(f32, f32)>,
    cache: HashMap<PageRequest, PageImage>,
    /// Queued or being rendered: not to be asked again.
    requested: HashSet<PageRequest>,
    shared: Option<Arc<Shared>>,
}

impl Manual {
    /// The manual, when the file is there and poppler's tools can read it.
    pub fn open(path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        let output = Command::new(PDFINFO)
            .args(["-box", "-f", "1", "-l", "100000"])
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
            requested: HashSet::new(),
            shared: None,
        })
    }

    pub fn pages(&self) -> u32 {
        u32::try_from(self.sizes.len()).unwrap_or(u32::MAX)
    }

    /// A page's size in points (1-based; the first page's if out of range).
    pub fn page_size(&self, page: u32) -> (f32, f32) {
        let index = usize::try_from(page.saturating_sub(1)).unwrap_or(0);
        self.sizes
            .get(index)
            .or(self.sizes.first())
            .copied()
            .unwrap_or((612.0, 792.0))
    }

    /// Start the renderer; what it renders goes to `deliver`.
    pub fn start(&mut self, deliver: impl Fn(Rendered) + Send + 'static) {
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        let path = self.path.clone();
        let spawned = thread::Builder::new().name("manual".into()).spawn(move || {
            while let Some(request) = next_request(&worker) {
                deliver((request, render(&path, request)));
            }
        });
        match spawned {
            Ok(_) => self.shared = Some(shared),
            Err(err) => log::error!("cannot start the manual renderer: {err}"),
        }
    }

    pub fn get(&self, request: &PageRequest) -> Option<&PageImage> {
        self.cache.get(request)
    }

    /// The sharpest rendering of a page there is, to show (scaled) until
    /// the one wanted is ready.
    pub fn best(&self, page: u32) -> Option<(PageRequest, &PageImage)> {
        self.cache
            .iter()
            .filter(|(request, _)| request.page == page)
            .max_by_key(|(request, _)| request.width)
            .map(|(request, image)| (*request, image))
    }

    /// Render the page on screen next (instead of the one asked for before,
    /// if that has not started yet).
    pub fn want(&mut self, request: PageRequest) {
        if self.cache.contains_key(&request) || self.requested.contains(&request) {
            return;
        }
        let Some(shared) = &self.shared else {
            return;
        };
        let replaced = shared.queue().wanted.replace(request);
        if let Some(replaced) = replaced {
            self.requested.remove(&replaced);
        }
        self.requested.insert(request);
        shared.work.notify_one();
    }

    /// Render these pages when there is nothing more urgent (the ones asked
    /// for before and not started are forgotten).
    pub fn ahead(&mut self, requests: &[PageRequest]) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut queue = shared.queue();
        for dropped in queue.ahead.drain(..) {
            self.requested.remove(&dropped);
        }
        for request in requests {
            if !self.cache.contains_key(request) && self.requested.insert(*request) {
                queue.ahead.push_back(*request);
            }
        }
        drop(queue);
        shared.work.notify_one();
    }

    /// Keep a page the renderer has finished.
    pub fn store(&mut self, rendered: Rendered) {
        let (request, result) = rendered;
        self.requested.remove(&request);
        match result {
            Ok(image) => {
                let size = image.pixels.len();
                let mut used: usize = self.cache.values().map(|image| image.pixels.len()).sum();
                // forget the pages farthest from this one, the biggest first
                while used + size > CACHE_BYTES {
                    let far = self
                        .cache
                        .keys()
                        .max_by_key(|other| (other.page.abs_diff(request.page), other.width))
                        .copied();
                    let Some(freed) = far.and_then(|far| self.cache.remove(&far)) else {
                        break;
                    };
                    used -= freed.pixels.len();
                }
                self.cache.insert(request, image);
            }
            Err(err) => log::warn!("manual page {}: {err}", request.page),
        }
    }
}

impl Drop for Manual {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.queue().closed = true;
            shared.work.notify_one();
        }
    }
}

/// The renderer's next page: the one on screen, else the next one ahead;
/// None once the manual is closed.
fn next_request(shared: &Shared) -> Option<PageRequest> {
    let mut queue = shared.queue();
    loop {
        if queue.closed {
            return None;
        }
        if let Some(request) = queue.wanted.take().or_else(|| queue.ahead.pop_front()) {
            return Some(request);
        }
        queue = shared
            .work
            .wait(queue)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

/// The pages' sizes in what `pdfinfo -box -f 1 -l N` prints: per page,
/// `Page    1 size: 612 x 792 pts`, `Page    1 rot:  90` and its boxes,
/// `Page    1 CropBox:     0.00     0.00   612.00   792.00`.
fn page_sizes(info: &str) -> Vec<(f32, f32)> {
    let mut sizes: Vec<(f32, f32)> = Vec::new();
    let mut rotated: Vec<bool> = Vec::new();
    for line in info.lines() {
        let Some(rest) = line.strip_prefix("Page") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(number), Some(field)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(number) = number.parse::<usize>() else {
            continue;
        };
        let numbers: Vec<f32> = words.filter_map(|word| word.parse().ok()).collect();
        if number > sizes.len() {
            sizes.resize(number, (0.0, 0.0));
            rotated.resize(number, false);
        }
        let index = number - 1;
        match (field, numbers.as_slice()) {
            // the crop box is what pdftoppm -cropbox renders
            ("CropBox:", &[left, bottom, right, top]) => {
                sizes[index] = ((right - left).abs(), (top - bottom).abs());
            }
            ("size:", &[width, height, ..]) if sizes[index] == (0.0, 0.0) => {
                sizes[index] = (width, height);
            }
            ("rot:", &[rotation]) => rotated[index] = (rotation as i32).rem_euclid(180) == 90,
            _ => {}
        }
    }
    sizes
        .into_iter()
        .zip(rotated)
        .filter(|((width, height), _)| *width > 0.0 && *height > 0.0)
        .map(|((width, height), rotated)| {
            if rotated {
                (height, width)
            } else {
                (width, height)
            }
        })
        .collect()
}

/// Render one page with pdftoppm (raw PPM on its standard output).
fn render(path: &Path, request: PageRequest) -> Result<PageImage, String> {
    let page = request.page.to_string();
    let output = Command::new(PDFTOPPM)
        .args(["-cropbox", "-f", &page, "-l", &page, "-singlefile"])
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
    from_ppm(&output.stdout).ok_or_else(|| format!("{PDFTOPPM} gave no image"))
}

/// A binary PPM (`P6`, 8 bits) as ARGB8888.
fn from_ppm(data: &[u8]) -> Option<PageImage> {
    // the header: P6, width, height, maximum, each after white space (and
    // comments), then a single white space byte before the pixels
    let mut fields = Vec::with_capacity(4);
    let mut position = 0;
    while fields.len() < 4 {
        while data.get(position)?.is_ascii_whitespace() {
            position += 1;
        }
        if data[position] == b'#' {
            while *data.get(position)? != b'\n' {
                position += 1;
            }
            continue;
        }
        let start = position;
        while !data.get(position)?.is_ascii_whitespace() {
            position += 1;
        }
        fields.push(std::str::from_utf8(&data[start..position]).ok()?);
    }
    let pixels = data.get(position + 1..)?;
    let (width, height): (u32, u32) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
    if fields[0] != "P6" || fields[3] != "255" {
        return None;
    }
    let count = usize::try_from(width).ok()? * usize::try_from(height).ok()?;
    let (rgb, _) = pixels.get(..count * 3)?.as_chunks::<3>();
    let mut argb = Vec::with_capacity(count * 4);
    for &[red, green, blue] in rgb {
        argb.extend_from_slice(&[blue, green, red, 0xFF]);
    }
    Some(PageImage {
        width,
        height,
        pixels: argb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_manual_is_none() {
        assert!(Manual::open(Path::new("/nonexistent/manual.pdf")).is_none());
    }

    #[test]
    fn reads_page_sizes_from_the_crop_box() {
        let sizes = page_sizes(
            "Pages:           3\n\
             Page    1 size:  612 x 792 pts (letter)\n\
             Page    1 rot:   0\n\
             Page    1 MediaBox:     0.00     0.00   612.00   792.00\n\
             Page    1 CropBox:     10.00    20.00   510.00   720.00\n\
             Page    2 size:  595.276 x 841.89 pts (A4)\n\
             Page    2 rot:   90\n\
             Page    3 size:  100 x 200 pts\n\
             Page    3 rot:   -90\n\
             File size:       1234 bytes\n",
        );
        assert_eq!(sizes, [(500.0, 700.0), (841.89, 595.276), (200.0, 100.0)]);
    }

    #[test]
    fn decodes_ppm() {
        let mut data = b"P6\n# poppler\n2 1\n255\n".to_vec();
        data.extend_from_slice(&[255, 0, 0, 1, 2, 3]);
        let image = from_ppm(&data).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, [0, 0, 255, 255, 3, 2, 1, 255]);
        assert!(from_ppm(b"P6\n2 1\n255\n\x00").is_none());
        assert!(from_ppm(b"P5\n1 1\n255\n\x00").is_none());
    }
}
