//! Drawing: the side menu, the clock and their text.

pub mod clock;
pub mod icons;
pub mod panel;
pub mod states;
pub mod text;

/// Copy a tiny-skia pixmap (premultiplied RGBA) into a `wl_shm` ARGB8888 buffer
/// (premultiplied, little-endian: B, G, R, A in memory).
pub fn to_argb8888(pixmap: &tiny_skia::Pixmap, canvas: &mut [u8]) {
    let (sources, _) = pixmap.data().as_chunks::<4>();
    let (targets, _) = canvas.as_chunks_mut::<4>();
    for (&[red, green, blue, alpha], target) in sources.iter().zip(targets) {
        *target = [blue, green, red, alpha];
    }
}
