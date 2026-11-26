//! Turns physical key events into Bezel shortcuts or the PC scancodes to send, applying the
//! ⌘ key setting. Keys are identified by their scancode set 1 code, which names the physical
//! key; Windows applies its own keyboard layout to it.

/// A PC scancode (set 1). Extended keys carry the `0xE0` prefix in the high byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scancode(u16);

impl Scancode {
    pub const fn new(code: u16) -> Self {
        Self(code)
    }

    pub const fn code(self) -> u16 {
        self.0
    }

    pub const ESCAPE: Self = Self(0x01);
    pub const DIGIT_1: Self = Self(0x02);
    pub const DIGIT_9: Self = Self(0x0A);
    pub const Q: Self = Self(0x10);
    pub const W: Self = Self(0x11);
    pub const T: Self = Self(0x14);
    pub const BRACKET_LEFT: Self = Self(0x1A);
    pub const BRACKET_RIGHT: Self = Self(0x1B);
    pub const S: Self = Self(0x1F);
    pub const F: Self = Self(0x21);
    pub const K: Self = Self(0x25);
    pub const COMMA: Self = Self(0x33);
    pub const DELETE: Self = Self(0xE053);
    pub const CAPS_LOCK: Self = Self(0x3A);

    pub const SHIFT_LEFT: Self = Self(0x2A);
    pub const SHIFT_RIGHT: Self = Self(0x36);
    pub const CONTROL_LEFT: Self = Self(0x1D);
    pub const CONTROL_RIGHT: Self = Self(0xE01D);
    pub const ALT_LEFT: Self = Self(0x38);
    pub const ALT_RIGHT: Self = Self(0xE038);
    pub const WINDOWS_LEFT: Self = Self(0xE05B);
    pub const WINDOWS_RIGHT: Self = Self(0xE05C);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOp {
    Press(Scancode),
    Release(Scancode),
}

/// The ⌘ key setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandKey {
    /// ⌘ plus an unreserved key is Ctrl plus that key; ⌘ alone is the Windows key.
    #[default]
    MacShortcuts,
    /// ⌘ is the Windows key in every combination.
    WindowsKey,
}

/// Keys Bezel keeps for itself; everything else reaches the Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    /// ⌘K and ⌘T
    Palette,
    /// ⌘W
    Close,
    /// ⌘Q
    Quit,
    /// ⌘,
    Settings,
    /// ⇧⌘[
    PreviousTab,
    /// ⇧⌘]
    NextTab,
    /// ⌘1 to ⌘9
    Tab(u8),
    /// ⌃⌘F
    FullScreen,
    /// ⌃⌘S
    Sidebar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Scancodes to send, possibly none.
    Send(Vec<KeyOp>),
    /// A Bezel shortcut: nothing reaches the Session, its release included.
    Shortcut(Shortcut),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Shift,
    Control,
    Alt,
    Command,
}

fn modifier(key: Scancode) -> Option<Modifier> {
    match key {
        Scancode::SHIFT_LEFT | Scancode::SHIFT_RIGHT => Some(Modifier::Shift),
        Scancode::CONTROL_LEFT | Scancode::CONTROL_RIGHT => Some(Modifier::Control),
        Scancode::ALT_LEFT | Scancode::ALT_RIGHT => Some(Modifier::Alt),
        Scancode::WINDOWS_LEFT | Scancode::WINDOWS_RIGHT => Some(Modifier::Command),
        _ => None,
    }
}

/// One Session's keyboard: it remembers what is held so ⌘ chords and releases come out right.
#[derive(Debug, Default)]
pub struct Keyboard {
    setting: CommandKey,
    /// Physical keys held, as pressed.
    held: Vec<Scancode>,
    /// Keys whose press became a Bezel shortcut, so their release is swallowed too.
    swallowed: Vec<Scancode>,
    command: Option<CommandChord>,
}

#[derive(Debug, Default)]
struct CommandChord {
    /// Another key went down while ⌘ was held, so ⌘ is not a lone Windows key tap.
    used: bool,
    /// Ctrl was pressed on the Session's behalf and is released with ⌘.
    sent_control: bool,
    /// Keys forwarded during the chord. macOS drops the key-up of a key released while ⌘ is
    /// held, so these are released with ⌘ if they are still down.
    forwarded: Vec<Scancode>,
}

impl Keyboard {
    pub fn new(setting: CommandKey) -> Self {
        Self {
            setting,
            ..Self::default()
        }
    }

    pub fn key(&mut self, key: Scancode, pressed: bool) -> KeyOutcome {
        if key == Scancode::CAPS_LOCK {
            // A Mac reports Caps Lock turning on as a press and off as a release; Windows
            // toggles on each press, so every change is a whole press.
            return KeyOutcome::Send(vec![KeyOp::Press(key), KeyOp::Release(key)]);
        }
        if pressed {
            self.press(key)
        } else {
            KeyOutcome::Send(self.release(key))
        }
    }

    /// Releases everything held, for when the Session loses the keyboard.
    pub fn release_all(&mut self) -> Vec<KeyOp> {
        let mut ops: Vec<KeyOp> = self
            .held
            .drain(..)
            .filter(|k| {
                !self.swallowed.contains(k)
                    && !(self.setting == CommandKey::MacShortcuts && modifier(*k) == Some(Modifier::Command))
            })
            .map(KeyOp::Release)
            .collect();
        if self.command.take().is_some_and(|c| c.sent_control) {
            ops.push(KeyOp::Release(Scancode::CONTROL_LEFT));
        }
        self.swallowed.clear();
        ops
    }

    fn is_held(&self, m: Modifier) -> bool {
        self.held.iter().any(|k| modifier(*k) == Some(m))
    }

    fn press(&mut self, key: Scancode) -> KeyOutcome {
        let repeat = self.held.contains(&key);
        if !repeat {
            self.held.push(key);
        }
        if self.swallowed.contains(&key) {
            return KeyOutcome::Send(Vec::new());
        }
        let Some(mut chord) = self.command.take().filter(|_| self.setting == CommandKey::MacShortcuts) else {
            if self.setting == CommandKey::MacShortcuts && modifier(key) == Some(Modifier::Command) {
                self.command = Some(CommandChord::default());
                return KeyOutcome::Send(Vec::new());
            }
            return KeyOutcome::Send(vec![KeyOp::Press(key)]);
        };
        let outcome = self.chord_press(&mut chord, key, repeat);
        self.command = Some(chord);
        outcome
    }

    fn chord_press(&mut self, chord: &mut CommandChord, key: Scancode, repeat: bool) -> KeyOutcome {
        match modifier(key) {
            Some(Modifier::Command) => KeyOutcome::Send(Vec::new()),
            Some(_) => KeyOutcome::Send(vec![KeyOp::Press(key)]),
            None => {
                chord.used = true;
                if !repeat && let Some(shortcut) = self.reserved(key) {
                    self.swallowed.push(key);
                    return KeyOutcome::Shortcut(shortcut);
                }
                let mut ops = Vec::new();
                if !chord.sent_control && !self.is_held(Modifier::Control) {
                    chord.sent_control = true;
                    ops.push(KeyOp::Press(Scancode::CONTROL_LEFT));
                }
                if !chord.forwarded.contains(&key) {
                    chord.forwarded.push(key);
                }
                ops.push(KeyOp::Press(key));
                KeyOutcome::Send(ops)
            }
        }
    }

    fn release(&mut self, key: Scancode) -> Vec<KeyOp> {
        let Some(at) = self.held.iter().position(|k| *k == key) else {
            return Vec::new();
        };
        self.held.remove(at);
        if let Some(at) = self.swallowed.iter().position(|k| *k == key) {
            self.swallowed.remove(at);
            return Vec::new();
        }
        if self.setting == CommandKey::WindowsKey || modifier(key) != Some(Modifier::Command) {
            if let Some(chord) = &mut self.command {
                chord.forwarded.retain(|k| *k != key);
            }
            return vec![KeyOp::Release(key)];
        }
        if self.is_held(Modifier::Command) {
            // The other ⌘ is still down; the chord goes on.
            return Vec::new();
        }
        let Some(chord) = self.command.take() else {
            return Vec::new();
        };
        if !chord.used {
            return vec![
                KeyOp::Press(Scancode::WINDOWS_LEFT),
                KeyOp::Release(Scancode::WINDOWS_LEFT),
            ];
        }
        let mut ops = Vec::new();
        for key in chord.forwarded {
            if let Some(at) = self.held.iter().position(|k| *k == key) {
                self.held.remove(at);
                ops.push(KeyOp::Release(key));
            }
        }
        if chord.sent_control {
            ops.push(KeyOp::Release(Scancode::CONTROL_LEFT));
        }
        ops
    }

    /// The Bezel shortcut `key` makes with ⌘ and the other modifiers held now, if any.
    fn reserved(&self, key: Scancode) -> Option<Shortcut> {
        let (shift, control, alt) = (
            self.is_held(Modifier::Shift),
            self.is_held(Modifier::Control),
            self.is_held(Modifier::Alt),
        );
        match (shift, control, alt) {
            (false, false, false) => match key {
                Scancode::K | Scancode::T => Some(Shortcut::Palette),
                Scancode::W => Some(Shortcut::Close),
                Scancode::Q => Some(Shortcut::Quit),
                Scancode::COMMA => Some(Shortcut::Settings),
                k if (Scancode::DIGIT_1.0..=Scancode::DIGIT_9.0).contains(&k.0) => {
                    Some(Shortcut::Tab((k.0 - Scancode::DIGIT_1.0 + 1) as u8))
                }
                _ => None,
            },
            (true, false, false) => match key {
                Scancode::BRACKET_LEFT => Some(Shortcut::PreviousTab),
                Scancode::BRACKET_RIGHT => Some(Shortcut::NextTab),
                _ => None,
            },
            (false, true, false) => match key {
                Scancode::F => Some(Shortcut::FullScreen),
                Scancode::S => Some(Shortcut::Sidebar),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyOp::{Press, Release};

    const C: Scancode = Scancode::new(0x2E);
    const V: Scancode = Scancode::new(0x2F);
    const A: Scancode = Scancode::new(0x1E);
    const CMD: Scancode = Scancode::WINDOWS_LEFT;
    const CTRL: Scancode = Scancode::CONTROL_LEFT;
    const WIN: Scancode = Scancode::WINDOWS_LEFT;

    fn send(ops: &[KeyOp]) -> KeyOutcome {
        KeyOutcome::Send(ops.to_vec())
    }

    #[test]
    fn plain_keys_pass_through() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(k.key(A, true), send(&[Press(A)]));
        assert_eq!(k.key(A, true), send(&[Press(A)]), "repeats are sent again");
        assert_eq!(k.key(A, false), send(&[Release(A)]));
    }

    #[test]
    fn command_plus_a_key_is_control_plus_that_key() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(k.key(CMD, true), send(&[]));
        assert_eq!(k.key(C, true), send(&[Press(CTRL), Press(C)]));
        assert_eq!(k.key(C, false), send(&[Release(C)]));
        assert_eq!(k.key(V, true), send(&[Press(V)]), "control stays down across the chord");
        assert_eq!(k.key(V, false), send(&[Release(V)]));
        assert_eq!(k.key(CMD, false), send(&[Release(CTRL)]));
    }

    #[test]
    fn command_alone_is_the_windows_key() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(k.key(CMD, true), send(&[]));
        assert_eq!(k.key(CMD, false), send(&[Press(WIN), Release(WIN)]));
    }

    #[test]
    fn the_right_command_key_works_the_same() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        k.key(Scancode::WINDOWS_RIGHT, true);
        assert_eq!(k.key(Scancode::WINDOWS_RIGHT, false), send(&[Press(WIN), Release(WIN)]));
    }

    #[test]
    fn a_key_whose_release_macos_dropped_is_released_with_command() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        k.key(CMD, true);
        k.key(C, true);
        assert_eq!(k.key(CMD, false), send(&[Release(C), Release(CTRL)]));
        assert_eq!(k.key(C, false), send(&[]), "a late release is not sent twice");
    }

    #[test]
    fn a_held_control_is_not_pressed_again_or_released_by_command() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(k.key(CTRL, true), send(&[Press(CTRL)]));
        k.key(CMD, true);
        assert_eq!(k.key(C, true), send(&[Press(C)]));
        k.key(C, false);
        assert_eq!(k.key(CMD, false), send(&[]));
        assert_eq!(k.key(CTRL, false), send(&[Release(CTRL)]));
    }

    #[test]
    fn shift_inside_a_chord_passes_through() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        k.key(CMD, true);
        assert_eq!(k.key(Scancode::SHIFT_LEFT, true), send(&[Press(Scancode::SHIFT_LEFT)]));
        assert_eq!(
            k.key(Scancode::new(0x2C), true),
            send(&[Press(CTRL), Press(Scancode::new(0x2C))])
        );
    }

    #[test]
    fn reserved_chords_become_shortcuts_and_send_nothing() {
        let cases = [
            (vec![], Scancode::K, Shortcut::Palette),
            (vec![], Scancode::T, Shortcut::Palette),
            (vec![], Scancode::W, Shortcut::Close),
            (vec![], Scancode::Q, Shortcut::Quit),
            (vec![], Scancode::COMMA, Shortcut::Settings),
            (vec![], Scancode::DIGIT_1, Shortcut::Tab(1)),
            (vec![], Scancode::DIGIT_9, Shortcut::Tab(9)),
            (
                vec![Scancode::SHIFT_LEFT],
                Scancode::BRACKET_LEFT,
                Shortcut::PreviousTab,
            ),
            (vec![Scancode::SHIFT_RIGHT], Scancode::BRACKET_RIGHT, Shortcut::NextTab),
            (vec![Scancode::CONTROL_LEFT], Scancode::F, Shortcut::FullScreen),
            (vec![Scancode::CONTROL_LEFT], Scancode::S, Shortcut::Sidebar),
        ];
        for (mods, key, shortcut) in cases {
            let mut k = Keyboard::new(CommandKey::MacShortcuts);
            for m in &mods {
                k.key(*m, true);
            }
            k.key(CMD, true);
            assert_eq!(k.key(key, true), KeyOutcome::Shortcut(shortcut), "{mods:?} {key:?}");
            assert_eq!(k.key(key, false), send(&[]), "the release is swallowed too");
            assert_eq!(k.key(CMD, false), send(&[]), "no stray Ctrl or Windows key");
        }
    }

    #[test]
    fn reserved_keys_with_other_modifiers_reach_the_session() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        k.key(Scancode::SHIFT_LEFT, true);
        k.key(CMD, true);
        assert_eq!(k.key(Scancode::K, true), send(&[Press(CTRL), Press(Scancode::K)]));
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        k.key(CMD, true);
        assert_eq!(
            k.key(Scancode::BRACKET_LEFT, true),
            send(&[Press(CTRL), Press(Scancode::BRACKET_LEFT)])
        );
    }

    #[test]
    fn command_as_the_windows_key_forwards_it_in_every_combination() {
        let mut k = Keyboard::new(CommandKey::WindowsKey);
        assert_eq!(k.key(CMD, true), send(&[Press(WIN)]));
        assert_eq!(k.key(Scancode::new(0x13), true), send(&[Press(Scancode::new(0x13))]));
        assert_eq!(k.key(Scancode::new(0x13), false), send(&[Release(Scancode::new(0x13))]));
        assert_eq!(k.key(CMD, false), send(&[Release(WIN)]));
    }

    #[test]
    fn caps_lock_toggles_on_every_change() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        let toggle = send(&[Press(Scancode::CAPS_LOCK), Release(Scancode::CAPS_LOCK)]);
        assert_eq!(k.key(Scancode::CAPS_LOCK, true), toggle);
        assert_eq!(k.key(Scancode::CAPS_LOCK, false), toggle);
    }

    #[test]
    fn releasing_everything_lets_go_of_held_keys_and_the_sent_control() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        k.key(Scancode::SHIFT_LEFT, true);
        k.key(CMD, true);
        k.key(C, true);
        let mut ops = k.release_all();
        ops.sort_by_key(|op| format!("{op:?}"));
        let mut expected = vec![Release(Scancode::SHIFT_LEFT), Release(C), Release(CTRL)];
        expected.sort_by_key(|op| format!("{op:?}"));
        assert_eq!(ops, expected);
        assert_eq!(k.key(CMD, false), send(&[]), "nothing is left to release");
    }

    #[test]
    fn a_release_without_a_press_sends_nothing() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(k.key(A, false), send(&[]));
    }
}
