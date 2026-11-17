//! A scraped picture (PNG or JPEG, what scrapers save), read at the size it
//! is shown: a large screenshot is not kept whole for a corner of the menu.

use std::path::Path;

use tiny_skia::{FilterQuality, IntSize, Pixmap, PixmapPaint, Transform};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

/// The picture at `path`, shrunk (never enlarged) to fit `width` x `height`.
pub fn load(path: &Path, width: f32, height: f32) -> Option<Pixmap> {
    let data = std::fs::read(path)
        .inspect_err(|err| log::warn!("cannot read {}: {err}", path.display()))
        .ok()?;
    let image = decode(&data);
    if image.is_none() {
        log::warn!("{}: neither a PNG nor a JPEG", path.display());
    }
    fit(image?, width, height)
}

fn decode(data: &[u8]) -> Option<Pixmap> {
    if data.starts_with(b"\x89PNG") {
        return Pixmap::decode_png(data).ok();
    }
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    // opaque: RGBA is already premultiplied
    let pixels = decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;
    Pixmap::from_vec(
        pixels,
        IntSize::from_wh(u32::try_from(width).ok()?, u32::try_from(height).ok()?)?,
    )
}

fn fit(image: Pixmap, width: f32, height: f32) -> Option<Pixmap> {
    let scale = (width / image.width() as f32)
        .min(height / image.height() as f32)
        .min(1.0);
    if scale >= 1.0 {
        return Some(image);
    }
    let size = |side: u32| ((side as f32 * scale).round() as u32).max(1);
    let mut fitted = Pixmap::new(size(image.width()), size(image.height()))?;
    let paint = PixmapPaint {
        quality: FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    fitted.draw_pixmap(
        0,
        0,
        image.as_ref(),
        &paint,
        Transform::from_scale(scale, scale),
        None,
    );
    Some(fitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinks_but_never_enlarges() {
        let image = Pixmap::new(400, 200).unwrap();
        let fitted = fit(image.clone(), 100.0, 100.0).unwrap();
        assert_eq!((fitted.width(), fitted.height()), (100, 50));
        let kept = fit(image, 1000.0, 1000.0).unwrap();
        assert_eq!((kept.width(), kept.height()), (400, 200));
    }

    #[test]
    fn knows_pngs_and_jpegs() {
        let png = Pixmap::new(3, 2).unwrap().encode_png().unwrap();
        assert_eq!(decode(&png).map(|image| image.width()), Some(3));
        assert!(decode(b"GIF89a").is_none());
    }
}
