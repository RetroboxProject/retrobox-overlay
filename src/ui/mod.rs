//! Drawing: the side menu, the clock and their text.

pub mod clock;
pub mod panel;
pub mod text;

/// Copy a tiny-skia pixmap (premultiplied RGBA) into a `wl_shm` ARGB8888 buffer
/// (premultiplied, little-endian: B, G, R, A in memory).
pub fn to_argb8888(pixmap: &tiny_skia::Pixmap, canvas: &mut [u8]) {
    for (source, target) in pixmap
        .data()
        .chunks_exact(4)
        .zip(canvas.chunks_exact_mut(4))
    {
        target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
    }
}
