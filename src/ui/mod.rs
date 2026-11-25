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

#[cfg(test)]
mod tests {
    use super::*;

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
