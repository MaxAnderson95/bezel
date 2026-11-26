//! The Retina recipe: ask Windows for the desktop in device pixels and tell it the scale, so
//! frames draw one remote pixel per screen pixel.

use ironrdp_displaycontrol::pdu::{
    DeviceScaleFactor, DisplayControlMonitorLayout, MonitorLayoutEntry, MonitorOrientation,
};

use crate::View;

/// What a View asks of the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Desktop {
    pub width: u16,
    pub height: u16,
    /// Percent, sent as the desktop scale factor.
    pub desktop_scale: u32,
    /// Percent, one of the three device scale factors RDP allows.
    pub device_scale: u32,
}

// MS-RDPEDISP 2.2.2.2.1: 200 to 8192 pixels each way, and an even width.
const MIN_SIDE: f32 = 200.0;
const MAX_SIDE: f32 = 8192.0;

impl Desktop {
    pub fn for_view(view: View) -> Self {
        let scale = if view.scale.is_finite() && view.scale > 0.0 {
            view.scale
        } else {
            1.0
        };
        let side = |points: f32| {
            let pixels = (points * scale).round();
            if pixels.is_finite() {
                pixels.clamp(MIN_SIDE, MAX_SIDE) as u16
            } else {
                MIN_SIDE as u16
            }
        };
        let desktop_scale = ((scale * 100.0).round() as u32).clamp(100, 500);
        let device_scale = [100, 140, 180]
            .into_iter()
            .min_by_key(|d: &u32| d.abs_diff(desktop_scale))
            .unwrap_or(100);
        Self {
            width: side(view.width) & !1,
            height: side(view.height),
            desktop_scale,
            device_scale,
        }
    }
}

impl Desktop {
    pub fn same_scale(&self, other: &Desktop) -> bool {
        (self.desktop_scale, self.device_scale) == (other.desktop_scale, other.device_scale)
    }

    /// A Display Control layout carrying both scale factors, so a live resize keeps the device
    /// scale the connection started with instead of IronRDP deriving another from the desktop
    /// scale.
    pub fn layout(&self) -> Result<DisplayControlMonitorLayout, impl std::fmt::Display + std::fmt::Debug + use<>> {
        let orientation = if self.height > self.width {
            MonitorOrientation::Portrait
        } else {
            MonitorOrientation::Landscape
        };
        let device_scale =
            DeviceScaleFactor::from_value(self.device_scale).unwrap_or(DeviceScaleFactor::Scale100Percent);
        MonitorLayoutEntry::new_primary(self.width.into(), self.height.into())
            .and_then(|entry| {
                entry
                    .with_orientation(orientation)
                    .with_desktop_scale_factor(self.desktop_scale)
            })
            .and_then(|entry| DisplayControlMonitorLayout::new(&[entry.with_device_scale_factor(device_scale)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop(width: f32, height: f32, scale: f32) -> Desktop {
        Desktop::for_view(View { width, height, scale })
    }

    #[test]
    fn retina_asks_for_device_pixels_at_double_scale() {
        assert_eq!(
            desktop(1200.0, 800.0, 2.0),
            Desktop {
                width: 2400,
                height: 1600,
                desktop_scale: 200,
                device_scale: 180
            }
        );
    }

    #[test]
    fn a_standard_display_asks_for_points() {
        assert_eq!(
            desktop(1200.0, 800.0, 1.0),
            Desktop {
                width: 1200,
                height: 800,
                desktop_scale: 100,
                device_scale: 100
            }
        );
    }

    #[test]
    fn the_device_scale_is_the_nearest_of_three() {
        assert_eq!(desktop(800.0, 600.0, 1.25).device_scale, 140);
        assert_eq!(desktop(800.0, 600.0, 1.5).device_scale, 140);
        assert_eq!(desktop(800.0, 600.0, 1.75).device_scale, 180);
        assert_eq!(
            desktop(800.0, 600.0, 3.0),
            Desktop {
                width: 2400,
                height: 1800,
                desktop_scale: 300,
                device_scale: 180
            }
        );
    }

    #[test]
    fn width_is_even_and_sides_are_clamped() {
        assert_eq!(desktop(1001.0, 701.0, 1.0).width, 1000);
        assert_eq!(desktop(1001.0, 701.0, 1.0).height, 701);
        assert_eq!(
            (desktop(50.0, 50.0, 2.0).width, desktop(50.0, 50.0, 2.0).height),
            (200, 200)
        );
        assert_eq!(
            (desktop(5000.0, 5000.0, 2.0).width, desktop(5000.0, 5000.0, 2.0).height),
            (8192, 8192)
        );
    }

    #[test]
    fn the_layout_carries_both_scale_factors() {
        let layout = desktop(800.0, 600.0, 1.25).layout().unwrap();
        let monitor = &layout.monitors()[0];
        assert_eq!(monitor.dimensions(), (1000, 750));
        assert_eq!(monitor.desktop_scale_factor(), Some(125));
        assert_eq!(monitor.device_scale_factor(), Some(DeviceScaleFactor::Scale140Percent));
    }

    #[test]
    fn a_nonsense_scale_falls_back_to_one() {
        assert_eq!(desktop(800.0, 600.0, 0.0), desktop(800.0, 600.0, 1.0));
        assert_eq!(desktop(800.0, 600.0, f32::NAN), desktop(800.0, 600.0, 1.0));
    }
}
