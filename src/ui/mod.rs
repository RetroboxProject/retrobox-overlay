//! Drawing: the side menu, the clock, the manual, the on-screen keyboard and their text.

pub mod audio;
pub mod clock;
pub mod help;
pub mod icons;
pub mod keyboard;
pub mod layout;
pub mod osk;
pub mod panel;
pub mod picture;
pub mod power;
pub mod states;
pub mod task;
pub mod text;
pub mod toast;
pub mod viewer;

/// Copy a tiny-skia pixmap (premultiplied RGBA) into a `wl_shm` ARGB8888 buffer
/// (premultiplied, little-endian: B, G, R, A in memory).
/// Red and blue swap places, as whole words (which the compiler turns into
/// vector instructions: a menu's worth of pixels, every redraw).
pub fn to_argb8888(pixmap: &tiny_skia::Pixmap, canvas: &mut [u8]) {
    let (sources, _) = pixmap.data().as_chunks::<4>();
    let (targets, _) = canvas.as_chunks_mut::<4>();
    let count = sources.len().min(targets.len());
    for (source, target) in sources[..count].iter().zip(&mut targets[..count]) {
        let rgba = u32::from_le_bytes(*source);
        let bgra = (rgba & 0xff00_ff00) | ((rgba & 0xff) << 16) | ((rgba >> 16) & 0xff);
        *target = bgra.to_le_bytes();
    }
}

/// Draw `image` as it is (not scaled) with its top left corner at `at`,
/// over what `pixmap` has there (source over, both premultiplied): what
/// tiny-skia's `draw_pixmap` does without a transform, in a plain loop
/// instead of its general pipeline, which samples every pixel as if it
/// could be scaled. Several times cheaper, and it adds up at every frame:
/// the icons, the controls' keyboard, the game's page while it scrolls.
pub fn blit(pixmap: &mut tiny_skia::Pixmap, image: &tiny_skia::Pixmap, at: (i32, i32)) {
    let (width, height) = (i64::from(pixmap.width()), i64::from(pixmap.height()));
    let (image_width, image_height) = (i64::from(image.width()), i64::from(image.height()));
    let (x, y) = (i64::from(at.0), i64::from(at.1));
    let (left, right) = (x.max(0), (x + image_width).min(width));
    let (top, bottom) = (y.max(0), (y + image_height).min(height));
    if left >= right || top >= bottom {
        return;
    }
    let span = (right - left) as usize * 4;
    let source = image.data();
    let target = pixmap.data_mut();
    for row in top..bottom {
        let from = ((row - y) * image_width + left - x) as usize * 4;
        let to = (row * width + left) as usize * 4;
        let (sources, _) = source[from..from + span].as_chunks::<4>();
        let (targets, _) = target[to..to + span].as_chunks_mut::<4>();
        for (source, target) in sources.iter().zip(targets) {
            match source[3] {
                0 => {}
                255 => *target = *source,
                alpha => {
                    let rest = 255 - u32::from(alpha);
                    for (channel, over) in target.iter_mut().zip(source) {
                        *channel = over.saturating_add(div255(u32::from(*channel) * rest));
                    }
                }
            }
        }
    }
}

/// `value / 255`, rounded, for `value` up to 255 × 255.
const fn div255(value: u32) -> u8 {
    let value = value + 128;
    ((value + (value >> 8)) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blits_like_tiny_skia() {
        // a translucent image, partly outside, over a translucent background
        let mut image = tiny_skia::Pixmap::new(5, 4).unwrap();
        image.fill(tiny_skia::Color::from_rgba8(200, 40, 90, 160));
        let paint = tiny_skia::Paint::default();
        let rect = tiny_skia::Rect::from_xywh(1.0, 1.0, 2.0, 2.0).unwrap();
        image.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
        let mut ours = tiny_skia::Pixmap::new(8, 6).unwrap();
        ours.fill(tiny_skia::Color::from_rgba8(10, 120, 250, 100));
        let mut theirs = ours.clone();
        for at in [(-2, -1), (5, 4), (1, 1)] {
            blit(&mut ours, &image, at);
            theirs.draw_pixmap(
                at.0,
                at.1,
                image.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
        }
        for (a, b) in ours.data().iter().zip(theirs.data()) {
            assert!(a.abs_diff(*b) <= 1, "{a} != {b}");
        }
    }

    #[test]
    fn swaps_red_and_blue() {
        let mut pixmap = tiny_skia::Pixmap::new(2, 1).unwrap();
        pixmap
            .data_mut()
            .copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0xa0, 0xb0, 0xc0, 0xff]);
        let mut canvas = [0u8; 8];
        to_argb8888(&pixmap, &mut canvas);
        assert_eq!(canvas, [0x33, 0x22, 0x11, 0x44, 0xc0, 0xb0, 0xa0, 0xff]);
    }
}
