//! Animated GIF/WebP (avatars, emoji, stickers) as a list of frames.
//! Only the Full tier animates; the others show the first frame.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ImageFormat};

use crate::decode::Image;

/// Huge animations aren't worth the memory.
const MAX_FRAMES: usize = 120;

pub struct Frame {
    pub image: Image,
    pub delay_ms: u32,
}

/// The frames of an animated image, scaled to fit `max_size`. `None` if it
/// isn't animated (or isn't readable), so callers can fall back to [`decode`].
///
/// [`decode`]: crate::decode
pub fn decode_animation(bytes: &[u8], max_size: u32) -> Option<Vec<Frame>> {
    let frames = match image::guess_format(bytes).ok()? {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames(),
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.into_frames()
        }
        _ => return None,
    };

    let mut out = Vec::new();
    for frame in frames.take(MAX_FRAMES) {
        let frame = frame.ok()?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        // Browsers treat near-zero delays as 100 ms; so do we.
        let delay_ms = numerator / denominator.max(1);
        let mut image = DynamicImage::ImageRgba8(frame.into_buffer());
        if image.width() > max_size || image.height() > max_size {
            image = image.resize(max_size, max_size, FilterType::Triangle);
        }
        let rgba = image.to_rgba8();
        out.push(Frame {
            image: Image {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            },
            delay_ms: if delay_ms < 20 { 100 } else { delay_ms },
        });
    }
    (out.len() > 1).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{Delay, RgbaImage};

    #[test]
    fn decodes_gif_frames_with_delays() {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            for shade in [0u8, 255] {
                let frame = image::Frame::from_parts(
                    RgbaImage::from_pixel(8, 8, image::Rgba([shade, shade, shade, 255])),
                    0,
                    0,
                    Delay::from_numer_denom_ms(50, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        let frames = decode_animation(&bytes, 4).expect("animated");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].delay_ms, 50);
        assert_eq!((frames[0].image.width, frames[0].image.height), (4, 4));
    }

    #[test]
    fn still_images_are_not_animations() {
        assert!(decode_animation(&crate::decode::tests::png(8, 8), 8).is_none());
    }
}
