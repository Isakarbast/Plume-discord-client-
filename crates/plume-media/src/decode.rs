use image::imageops::FilterType;

use crate::MediaError;

/// Decoded pixels, 4 bytes (RGBA) per pixel, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn byte_size(&self) -> usize {
        self.rgba.len()
    }
}

/// Decodes any supported format and shrinks it to fit `max_size`, keeping
/// the aspect ratio. Never scales up. Decoding at display size is the main
/// trick for keeping image memory small.
pub fn decode(bytes: &[u8], max_size: u32) -> Result<Image, MediaError> {
    let mut image = image::load_from_memory(bytes)?;
    if image.width() > max_size || image.height() > max_size {
        image = image.resize(max_size, max_size, FilterType::Triangle);
    }
    let rgba = image.to_rgba8();
    Ok(Image {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;

    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, image::Rgba([200, 100, 50, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn shrinks_keeping_aspect_ratio() {
        let image = decode(&png(400, 200), 100).unwrap();
        assert_eq!((image.width, image.height), (100, 50));
        assert_eq!(image.rgba.len(), 100 * 50 * 4);
        assert_eq!(&image.rgba[..4], &[200, 100, 50, 255]);
    }

    #[test]
    fn never_scales_up() {
        let image = decode(&png(20, 10), 100).unwrap();
        assert_eq!((image.width, image.height), (20, 10));
    }

    #[test]
    fn garbage_is_an_error_not_a_crash() {
        assert!(decode(b"definitely not an image", 64).is_err());
        assert!(decode(&[], 64).is_err());
    }
}
