//! The remote desktop as RGBA the app draws. Dirty regions are converted into a back buffer;
//! publishing copies it into a recycled buffer handed to `Pixels`, which uploads only on the
//! next frame of a window showing it.

use rune_image::Pixels;

pub(crate) struct Framebuffer {
    pixels: Pixels,
    back: Vec<u8>,
    spare: Vec<u8>,
    width: u16,
    height: u16,
    dirty: bool,
}

/// Where a changed region starts and how wide it is, in desktop pixels; its rows follow from
/// the pixels given with it. `right` is inclusive, as IronRDP reports it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Region {
    pub left: u16,
    pub top: u16,
    pub right: u16,
}

impl Framebuffer {
    pub fn new(pixels: Pixels) -> Self {
        Self {
            pixels,
            back: Vec::new(),
            spare: Vec::new(),
            width: 0,
            height: 0,
            dirty: false,
        }
    }

    /// Applies tightly packed `0x00RRGGBB` pixels for `region` of a `width` by `height`
    /// desktop. Returns true when the desktop changed size.
    pub fn apply(&mut self, width: u16, height: u16, region: Region, packed: &[u32]) -> bool {
        let resized = (width, height) != (self.width, self.height);
        if resized {
            self.width = width;
            self.height = height;
            self.back.clear();
            self.back.resize(usize::from(width) * usize::from(height) * 4, 0);
        }
        blit(&mut self.back, width, region, packed);
        self.dirty = true;
        resized
    }

    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn publish(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut next = std::mem::take(&mut self.spare);
        next.clear();
        next.extend_from_slice(&self.back);
        self.spare = self.pixels.replace(self.width.into(), self.height.into(), next);
    }
}

fn blit(back: &mut [u8], width: u16, region: Region, packed: &[u32]) {
    let row_pixels = usize::from(region.right - region.left) + 1;
    for (row, source) in packed.chunks_exact(row_pixels).enumerate() {
        let start = ((usize::from(region.top) + row) * usize::from(width) + usize::from(region.left)) * 4;
        let target = &mut back[start..start + row_pixels * 4];
        for (rgba, pixel) in target.as_chunks_mut::<4>().0.iter_mut().zip(source) {
            let [b, g, r, _] = pixel.to_le_bytes();
            *rgba = [r, g, b, 255];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_land_in_place_as_opaque_rgba() {
        let pixels = Pixels::new();
        let mut fb = Framebuffer::new(pixels.clone());
        assert!(fb.apply(
            3,
            2,
            Region {
                left: 0,
                top: 0,
                right: 2
            },
            &[0; 6]
        ));
        assert!(!fb.apply(
            3,
            2,
            Region {
                left: 1,
                top: 1,
                right: 2
            },
            &[0x112233, 0x445566]
        ));
        fb.publish();
        let (w, h, rgba) = pixels.to_rgba().unwrap();
        assert_eq!((w, h), (3, 2));
        assert_eq!(&rgba[12..16], &[0, 0, 0, 255]);
        assert_eq!(&rgba[16..24], &[0x11, 0x22, 0x33, 255, 0x44, 0x55, 0x66, 255]);
    }

    #[test]
    fn publishing_twice_keeps_every_update() {
        let pixels = Pixels::new();
        let mut fb = Framebuffer::new(pixels.clone());
        fb.apply(
            2,
            1,
            Region {
                left: 0,
                top: 0,
                right: 0,
            },
            &[0xFF0000],
        );
        fb.publish();
        fb.apply(
            2,
            1,
            Region {
                left: 1,
                top: 0,
                right: 1,
            },
            &[0x00FF00],
        );
        fb.publish();
        assert_eq!(pixels.to_rgba().unwrap().2, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn a_new_size_starts_from_a_cleared_desktop() {
        let pixels = Pixels::new();
        let mut fb = Framebuffer::new(pixels.clone());
        fb.apply(
            1,
            1,
            Region {
                left: 0,
                top: 0,
                right: 0,
            },
            &[0xFFFFFF],
        );
        assert!(fb.apply(
            2,
            1,
            Region {
                left: 1,
                top: 0,
                right: 1
            },
            &[0x0000FF]
        ));
        fb.publish();
        assert_eq!(pixels.to_rgba().unwrap().2, vec![0, 0, 0, 0, 0, 0, 255, 255]);
    }
}
