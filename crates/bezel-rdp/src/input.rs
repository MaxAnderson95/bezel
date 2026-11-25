//! Turns Bezel's input into IronRDP operations.

use bezel_core::keys::{KeyOp, Scancode};
use ironrdp_input::{Operation, WheelRotations};

use crate::MouseButton;

/// RDP counts 120 wheel units per notch and Rune reports a macOS notch as 40 points.
const UNITS_PER_POINT: f32 = 3.0;
const MAX_UNITS: f32 = 255.0;

/// Wheel and trackpad scrolling, carrying the fraction of a unit left over between events so
/// slow trackpad movement still scrolls.
#[derive(Debug, Default)]
pub(crate) struct Wheel {
    x: f32,
    y: f32,
}

impl Wheel {
    pub fn rotations(&mut self, dx: f32, dy: f32) -> Vec<Operation> {
        let mut ops = Vec::new();
        // Windows scrolls right for a positive horizontal rotation, which moves content left.
        for (rest, delta, is_vertical) in [(&mut self.y, dy, true), (&mut self.x, -dx, false)] {
            if !delta.is_finite() {
                continue;
            }
            *rest += delta * UNITS_PER_POINT;
            let mut whole = rest.trunc();
            *rest -= whole;
            while whole != 0.0 {
                let units = whole.clamp(-MAX_UNITS, MAX_UNITS);
                whole -= units;
                ops.push(Operation::WheelRotations(WheelRotations {
                    is_vertical,
                    rotation_units: units as i16,
                }));
            }
        }
        ops
    }
}

pub(crate) fn key(op: KeyOp) -> Operation {
    let scancode = |s: Scancode| ironrdp_input::Scancode::from_u16(s.code());
    match op {
        KeyOp::Press(s) => Operation::KeyPressed(scancode(s)),
        KeyOp::Release(s) => Operation::KeyReleased(scancode(s)),
    }
}

pub(crate) fn button(button: MouseButton) -> ironrdp_input::MouseButton {
    match button {
        MouseButton::Left => ironrdp_input::MouseButton::Left,
        MouseButton::Right => ironrdp_input::MouseButton::Right,
        MouseButton::Middle => ironrdp_input::MouseButton::Middle,
        MouseButton::Back => ironrdp_input::MouseButton::X1,
        MouseButton::Forward => ironrdp_input::MouseButton::X2,
    }
}

pub(crate) const CTRL_ALT_DEL: [KeyOp; 6] = [
    KeyOp::Press(Scancode::CONTROL_LEFT),
    KeyOp::Press(Scancode::ALT_LEFT),
    KeyOp::Press(Scancode::DELETE),
    KeyOp::Release(Scancode::DELETE),
    KeyOp::Release(Scancode::ALT_LEFT),
    KeyOp::Release(Scancode::CONTROL_LEFT),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn units(ops: &[Operation]) -> Vec<(bool, i16)> {
        ops.iter()
            .map(|op| match op {
                Operation::WheelRotations(r) => (r.is_vertical, r.rotation_units),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_mouse_wheel_notch_is_120_units() {
        assert_eq!(units(&Wheel::default().rotations(0.0, 40.0)), vec![(true, 120)]);
        assert_eq!(units(&Wheel::default().rotations(0.0, -40.0)), vec![(true, -120)]);
    }

    #[test]
    fn fractions_carry_over_between_events() {
        let mut wheel = Wheel::default();
        assert_eq!(units(&wheel.rotations(0.0, 0.2)), vec![]);
        assert_eq!(units(&wheel.rotations(0.0, 0.2)), vec![(true, 1)]);
    }

    #[test]
    fn large_deltas_split_into_events_the_wire_can_carry() {
        assert_eq!(
            units(&Wheel::default().rotations(0.0, 200.0)),
            vec![(true, 255), (true, 255), (true, 90)]
        );
    }

    #[test]
    fn horizontal_scrolling_is_inverted_for_windows() {
        assert_eq!(units(&Wheel::default().rotations(40.0, 0.0)), vec![(false, -120)]);
    }

    #[test]
    fn non_finite_deltas_are_ignored() {
        assert!(Wheel::default().rotations(f32::NAN, f32::INFINITY).is_empty());
    }
}
