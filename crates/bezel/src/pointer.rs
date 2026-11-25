//! The remote pointer, drawn by macOS as the cursor over the desktop so it keeps up with the
//! mouse. Rune takes cursor pictures as `'static` PNG bytes, so each distinct shape is encoded
//! once and kept for the life of the process; the cache is capped so a server cycling through
//! shapes cannot grow it without bound.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Mutex, PoisonError};

use bezel_rdp::{Pointer, PointerShape};
use rune::{Cursor, CursorImage};

const MAX_SHAPES: usize = 256;
/// Rune's largest cursor side, in pixels.
const MAX_SIDE: f32 = 256.0;

static SHAPES: Mutex<Option<HashMap<u64, &'static [u8]>>> = Mutex::new(None);

/// `scale` is the display's backing scale: the remote pointer arrives in desktop pixels, which
/// Bezel draws one to one with screen pixels.
pub fn cursor(pointer: &Pointer, scale: f32) -> Cursor {
    match pointer {
        Pointer::Default => Cursor::Default,
        Pointer::Hidden => Cursor::Hidden,
        Pointer::Shape(shape) => image(shape, scale).map_or(Cursor::Default, Cursor::Image),
    }
}

fn image(shape: &PointerShape, scale: f32) -> Option<CursorImage> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let longest = f32::from(shape.width.max(shape.height)).max(1.0);
    // Rune draws cursor PNGs at 2x; resample so one desktop pixel covers one screen pixel.
    let factor = (2.0 / scale).min(MAX_SIDE / longest);
    let key = {
        let mut hasher = DefaultHasher::new();
        (shape, factor.to_bits()).hash(&mut hasher);
        hasher.finish()
    };
    let mut shapes = SHAPES.lock().unwrap_or_else(PoisonError::into_inner);
    let shapes = shapes.get_or_insert_with(HashMap::new);
    let bytes = match shapes.get(&key) {
        Some(bytes) => *bytes,
        None if shapes.len() >= MAX_SHAPES => return None,
        None => {
            let png = encode(shape, factor)?;
            let bytes: &'static [u8] = Box::leak(png.into_boxed_slice());
            shapes.insert(key, bytes);
            bytes
        }
    };
    let hot = |px: u16| f32::from(px) * factor / 2.0;
    Some(CursorImage::png(bytes, [hot(shape.hotspot.0), hot(shape.hotspot.1)]))
}

fn encode(shape: &PointerShape, factor: f32) -> Option<Vec<u8>> {
    let (width, height) = (usize::from(shape.width), usize::from(shape.height));
    if width == 0 || height == 0 {
        return None;
    }
    let side = |px: usize| ((px as f32 * factor).round() as usize).max(1);
    let (out_width, out_height) = (side(width), side(height));
    let mut rgba = Vec::with_capacity(out_width * out_height * 4);
    for y in 0..out_height {
        let source_row = (y * height / out_height) * width;
        for x in 0..out_width {
            let at = (source_row + x * width / out_width) * 4;
            rgba.extend_from_slice(&shape.rgba[at..at + 4]);
        }
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, out_width as u32, out_height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn shape(width: u16, height: u16) -> PointerShape {
        PointerShape {
            width,
            height,
            hotspot: (2, 4),
            rgba: vec![255; usize::from(width) * usize::from(height) * 4],
        }
    }

    fn png_size(bytes: &[u8]) -> (u32, u32) {
        let info = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        (info.info().width, info.info().height)
    }

    #[test]
    fn a_retina_pointer_is_not_resampled() {
        let png = encode(&shape(32, 32), 2.0 / 2.0).unwrap();
        assert_eq!(png_size(&png), (32, 32));
    }

    #[test]
    fn a_standard_display_doubles_it_for_rune_s_2x_pictures() {
        let png = encode(&shape(32, 16), 2.0 / 1.0).unwrap();
        assert_eq!(png_size(&png), (64, 32));
    }

    #[test]
    fn shapes_become_image_cursors() {
        assert!(matches!(
            cursor(&Pointer::Shape(Arc::new(shape(8, 8))), 2.0),
            Cursor::Image(_)
        ));
        assert_eq!(cursor(&Pointer::Hidden, 2.0), Cursor::Hidden);
        assert_eq!(cursor(&Pointer::Shape(Arc::new(shape(0, 0))), 2.0), Cursor::Default);
    }
}
