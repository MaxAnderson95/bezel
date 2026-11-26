//! Physical keys as Rune names them, to PC scancodes (set 1). Both name a key by where it sits,
//! not what it types, so this one table serves every keyboard layout.

use bezel_core::keys::{Held, KeyOp, KeyOutcome, Keyboard, Scancode};
use rune::{Mods, PhysicalInput, PhysicalKeyCode as K};

/// What a physical event over the desktop means for the Session.
#[derive(Debug, PartialEq, Eq)]
pub enum Translated {
    /// Modifiers held from before the Session had the keyboard, pressed for it first, and what
    /// the key itself makes.
    Key(Vec<KeyOp>, KeyOutcome),
    /// A modifier change that let go of modifiers held from before: send these, and leave the
    /// event to Rune.
    LetGo(Vec<KeyOp>),
    /// The window lost the keyboard; everything held is released.
    FocusLost,
    /// Not the Session's: a key with no PC position, or modifier bookkeeping Rune also sends.
    Pass,
}

pub fn translate(keyboard: &mut Keyboard, event: &PhysicalInput) -> Translated {
    match event {
        PhysicalInput::Key { key, pressed, mods, .. } => match scancode(*key) {
            Some(scancode) => {
                let held = keyboard.hold(held(*mods), scancode);
                Translated::Key(held, keyboard.key(scancode, *pressed))
            }
            None => Translated::Pass,
        },
        PhysicalInput::Modifiers(mods) => match keyboard.let_go(held(*mods)) {
            ops if ops.is_empty() => Translated::Pass,
            ops => Translated::LetGo(ops),
        },
        PhysicalInput::FocusLost => {
            keyboard.release_all();
            Translated::FocusLost
        }
    }
}

/// ⌘ is the logo key on every platform; Rune's `cmd` is Ctrl off macOS.
fn held(mods: Mods) -> Held {
    Held {
        shift: mods.shift,
        control: mods.ctrl,
        alt: mods.alt,
        command: mods.logo,
    }
}

pub fn scancode(key: K) -> Option<Scancode> {
    let code = match key {
        K::Escape => 0x01,
        K::Digit1 => 0x02,
        K::Digit2 => 0x03,
        K::Digit3 => 0x04,
        K::Digit4 => 0x05,
        K::Digit5 => 0x06,
        K::Digit6 => 0x07,
        K::Digit7 => 0x08,
        K::Digit8 => 0x09,
        K::Digit9 => 0x0A,
        K::Digit0 => 0x0B,
        K::Minus => 0x0C,
        K::Equal => 0x0D,
        K::Backspace => 0x0E,
        K::Tab => 0x0F,
        K::KeyQ => 0x10,
        K::KeyW => 0x11,
        K::KeyE => 0x12,
        K::KeyR => 0x13,
        K::KeyT => 0x14,
        K::KeyY => 0x15,
        K::KeyU => 0x16,
        K::KeyI => 0x17,
        K::KeyO => 0x18,
        K::KeyP => 0x19,
        K::BracketLeft => 0x1A,
        K::BracketRight => 0x1B,
        K::Enter => 0x1C,
        K::ControlLeft => 0x1D,
        K::KeyA => 0x1E,
        K::KeyS => 0x1F,
        K::KeyD => 0x20,
        K::KeyF => 0x21,
        K::KeyG => 0x22,
        K::KeyH => 0x23,
        K::KeyJ => 0x24,
        K::KeyK => 0x25,
        K::KeyL => 0x26,
        K::Semicolon => 0x27,
        K::Quote => 0x28,
        K::Backquote => 0x29,
        K::ShiftLeft => 0x2A,
        K::Backslash => 0x2B,
        K::KeyZ => 0x2C,
        K::KeyX => 0x2D,
        K::KeyC => 0x2E,
        K::KeyV => 0x2F,
        K::KeyB => 0x30,
        K::KeyN => 0x31,
        K::KeyM => 0x32,
        K::Comma => 0x33,
        K::Period => 0x34,
        K::Slash => 0x35,
        K::ShiftRight => 0x36,
        K::NumpadMultiply => 0x37,
        K::AltLeft => 0x38,
        K::Space => 0x39,
        K::CapsLock => 0x3A,
        K::F1 => 0x3B,
        K::F2 => 0x3C,
        K::F3 => 0x3D,
        K::F4 => 0x3E,
        K::F5 => 0x3F,
        K::F6 => 0x40,
        K::F7 => 0x41,
        K::F8 => 0x42,
        K::F9 => 0x43,
        K::F10 => 0x44,
        // A Mac keypad's Clear key sits where a PC keypad has Num Lock.
        K::NumLock | K::NumpadClear => 0x45,
        K::ScrollLock => 0x46,
        K::Numpad7 => 0x47,
        K::Numpad8 => 0x48,
        K::Numpad9 => 0x49,
        K::NumpadSubtract => 0x4A,
        K::Numpad4 => 0x4B,
        K::Numpad5 => 0x4C,
        K::Numpad6 => 0x4D,
        K::NumpadAdd => 0x4E,
        K::Numpad1 => 0x4F,
        K::Numpad2 => 0x50,
        K::Numpad3 => 0x51,
        K::Numpad0 => 0x52,
        K::NumpadDecimal => 0x53,
        K::IntlBackslash => 0x56,
        K::F11 => 0x57,
        K::F12 => 0x58,
        K::NumpadEqual => 0x59,
        K::F13 => 0x64,
        K::F14 => 0x65,
        K::F15 => 0x66,
        K::F16 => 0x67,
        K::F17 => 0x68,
        K::F18 => 0x69,
        K::F19 => 0x6A,
        K::F20 => 0x6B,
        K::F21 => 0x6C,
        K::F22 => 0x6D,
        K::F23 => 0x6E,
        K::KanaMode => 0x70,
        K::IntlRo => 0x73,
        K::F24 => 0x76,
        K::Convert => 0x79,
        K::NonConvert => 0x7B,
        K::IntlYen => 0x7D,
        K::NumpadComma => 0x7E,
        K::NumpadEnter => 0xE01C,
        K::ControlRight => 0xE01D,
        K::AudioVolumeMute => 0xE020,
        K::AudioVolumeDown => 0xE02E,
        K::AudioVolumeUp => 0xE030,
        K::NumpadDivide => 0xE035,
        K::PrintScreen => 0xE037,
        K::AltRight => 0xE038,
        K::Home => 0xE047,
        K::ArrowUp => 0xE048,
        K::PageUp => 0xE049,
        K::ArrowLeft => 0xE04B,
        K::ArrowRight => 0xE04D,
        K::End => 0xE04F,
        K::ArrowDown => 0xE050,
        K::PageDown => 0xE051,
        // A Mac keyboard's Help key sits where a PC keyboard has Insert.
        K::Insert | K::Help => 0xE052,
        K::Delete => 0xE053,
        K::SuperLeft => 0xE05B,
        K::SuperRight => 0xE05C,
        K::ContextMenu => 0xE05D,
        _ => return None,
    };
    Some(Scancode::new(code))
}

#[cfg(test)]
mod tests {
    use bezel_core::keys::{CommandKey, KeyOp};
    use rune::Mods;

    use super::*;

    fn key(key: K, pressed: bool, mods: Mods) -> PhysicalInput {
        PhysicalInput::Key {
            key,
            mods,
            pressed,
            repeat: false,
            text: None,
            consumed_mods: Mods::default(),
        }
    }

    /// The order winit delivers ⌘C on macOS: the ⌘ key (still with the modifiers from before
    /// it) and the modifier change it causes, then C pressed and released with ⌘ held, then ⌘
    /// released and its modifier change.
    #[test]
    fn command_c_from_macos_reaches_windows_as_control_c() {
        let command = Mods {
            cmd: true,
            logo: true,
            ..Mods::default()
        };
        let events = [
            key(K::SuperLeft, true, Mods::default()),
            PhysicalInput::Modifiers(command),
            key(K::KeyC, true, command),
            key(K::KeyC, false, command),
            key(K::SuperLeft, false, command),
            PhysicalInput::Modifiers(Mods::default()),
        ];
        let mut keyboard = Keyboard::new(CommandKey::MacShortcuts);
        let sent: Vec<KeyOp> = events
            .iter()
            .flat_map(|e| match translate(&mut keyboard, e) {
                Translated::Key(held, KeyOutcome::Send(ops)) => held.into_iter().chain(ops).collect(),
                Translated::Pass => Vec::new(),
                other => panic!("{e:?} became {other:?}"),
            })
            .collect();
        let (ctrl, c) = (Scancode::CONTROL_LEFT, Scancode::new(0x2E));
        assert_eq!(
            sent,
            vec![
                KeyOp::Press(ctrl),
                KeyOp::Press(c),
                KeyOp::Release(c),
                KeyOp::Release(ctrl)
            ]
        );
    }

    /// Right ⇧ held from the ⇧⌘] that switched Tabs, A typed, then right ⇧ let go, as winit on
    /// macOS delivers it: the key-up still with the snapshot from before it, then the modifier
    /// change. The left ⇧ pressed for the held one goes up with that change, before any other
    /// key or click reaches Windows, and the change is still Rune's to handle.
    #[test]
    fn an_inherited_shift_goes_up_with_the_modifier_change_after_its_key_up() {
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        let (a, c) = (Scancode::new(0x1E), Scancode::new(0x2E));
        let mut keyboard = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(
            translate(&mut keyboard, &key(K::KeyA, true, shift)),
            Translated::Key(
                vec![KeyOp::Press(Scancode::SHIFT_LEFT)],
                KeyOutcome::Send(vec![KeyOp::Press(a)])
            )
        );
        assert_eq!(
            translate(&mut keyboard, &key(K::KeyA, false, shift)),
            Translated::Key(vec![], KeyOutcome::Send(vec![KeyOp::Release(a)]))
        );
        assert_eq!(
            translate(&mut keyboard, &key(K::ShiftRight, false, shift)),
            Translated::Key(vec![], KeyOutcome::Send(vec![]))
        );
        assert_eq!(
            translate(&mut keyboard, &PhysicalInput::Modifiers(Mods::default())),
            Translated::LetGo(vec![KeyOp::Release(Scancode::SHIFT_LEFT)])
        );
        assert_eq!(
            translate(&mut keyboard, &key(K::KeyC, true, Mods::default())),
            Translated::Key(vec![], KeyOutcome::Send(vec![KeyOp::Press(c)]))
        );
        assert_eq!(
            translate(&mut keyboard, &PhysicalInput::Modifiers(Mods::default())),
            Translated::Pass,
            "a change that lets go of nothing inferred is not Bezel's"
        );
    }

    #[test]
    fn letters_and_modifiers_map_to_their_pc_positions() {
        assert_eq!(scancode(K::KeyA), Some(Scancode::new(0x1E)));
        assert_eq!(scancode(K::ControlRight), Some(Scancode::CONTROL_RIGHT));
        assert_eq!(scancode(K::SuperLeft), Some(Scancode::WINDOWS_LEFT));
        assert_eq!(scancode(K::Delete), Some(Scancode::DELETE));
    }

    #[test]
    fn keys_without_a_pc_position_are_not_sent() {
        assert_eq!(scancode(K::Fn), None);
    }
}
