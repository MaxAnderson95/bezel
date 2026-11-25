//! Turns Bezel's input into IronRDP operations.

use std::collections::VecDeque;

use bezel_core::keys::{KeyOp, Scancode};
use ironrdp_input::{Operation, WheelRotations};
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::input::mouse::PointerFlags;
use smallvec::SmallVec;

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

pub(crate) type Batch = SmallVec<[FastPathInputEvent; 2]>;

/// Input waiting for room in IronRDP's bounded queue. Every key and button change is kept and
/// delivered in order, since dropping a release would leave it held in Windows; consecutive
/// pointer moves collapse to the latest, which is all the server needs.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    queue: VecDeque<Batch>,
}

impl Outbox {
    pub fn push(&mut self, batch: Batch) {
        if batch.is_empty() {
            return;
        }
        match self.queue.back_mut() {
            Some(last) if is_motion(last) && is_motion(&batch) => *last = batch,
            _ => self.queue.push_back(batch),
        }
    }

    /// Offers batches in order until `send` hands one back for lack of room.
    pub fn flush(&mut self, mut send: impl FnMut(Batch) -> Result<(), Batch>) {
        while let Some(batch) = self.queue.pop_front() {
            if let Err(batch) = send(batch) {
                self.queue.push_front(batch);
                return;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

fn is_motion(batch: &Batch) -> bool {
    batch
        .iter()
        .all(|e| matches!(e, FastPathInputEvent::MouseEvent(m) if m.flags == PointerFlags::MOVE))
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

    fn batch(ops: Vec<Operation>, database: &mut ironrdp_input::Database) -> Batch {
        database.apply(ops)
    }

    fn mouse_move(x: u16) -> Operation {
        Operation::MouseMove(ironrdp_input::MousePosition { x, y: 0 })
    }

    #[test]
    fn a_full_queue_keeps_every_key_change_in_order() {
        let mut database = ironrdp_input::Database::new();
        let mut outbox = Outbox::default();
        let ctrl = key(KeyOp::Press(Scancode::CONTROL_LEFT));
        outbox.push(batch(vec![ctrl], &mut database));
        outbox.push(batch(vec![key(KeyOp::Release(Scancode::CONTROL_LEFT))], &mut database));
        outbox.flush(Err);
        assert!(!outbox.is_empty(), "nothing is dropped while the queue is full");
        let mut sent = Vec::new();
        outbox.flush(|b| {
            sent.push(b);
            Ok(())
        });
        assert_eq!(sent.len(), 2);
        assert!(matches!(sent[0][0], FastPathInputEvent::KeyboardEvent(flags, 0x1D) if flags.is_empty()));
        assert!(matches!(sent[1][0], FastPathInputEvent::KeyboardEvent(flags, 0x1D) if !flags.is_empty()));
        assert!(outbox.is_empty());
    }

    #[test]
    fn waiting_pointer_moves_collapse_to_the_latest() {
        let mut database = ironrdp_input::Database::new();
        let mut outbox = Outbox::default();
        outbox.push(batch(vec![mouse_move(1)], &mut database));
        outbox.push(batch(vec![mouse_move(2)], &mut database));
        outbox.push(batch(
            vec![Operation::MouseButtonPressed(ironrdp_input::MouseButton::Left)],
            &mut database,
        ));
        outbox.push(batch(vec![mouse_move(3)], &mut database));
        let mut sent = Vec::new();
        outbox.flush(|b| {
            sent.push(b);
            Ok(())
        });
        let moves: Vec<_> = sent
            .iter()
            .flatten()
            .map(|e| match e {
                FastPathInputEvent::MouseEvent(m) => (m.flags == PointerFlags::MOVE, m.x_position),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(moves, vec![(true, 2), (false, 2), (true, 3)]);
    }

    #[test]
    fn non_finite_deltas_are_ignored() {
        assert!(Wheel::default().rotations(f32::NAN, f32::INFINITY).is_empty());
    }
}
