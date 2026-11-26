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

/// The ⌘ key setting, written `mac-shortcuts` or `windows-key` in `bezel.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
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

/// Each modifier, whether the snapshot holds it, and the key that stands in for it.
fn wanted(held: Held) -> [(Modifier, bool, Scancode); 4] {
    [
        (Modifier::Shift, held.shift, Scancode::SHIFT_LEFT),
        (Modifier::Control, held.control, Scancode::CONTROL_LEFT),
        (Modifier::Alt, held.alt, Scancode::ALT_LEFT),
        (Modifier::Command, held.command, Scancode::WINDOWS_LEFT),
    ]
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
    /// Modifiers pressed because the platform reported them held, not because this keyboard
    /// saw them go down (see `hold`). Each is the left key standing in for whichever side is
    /// held, so it is released when the platform reports its modifier let go.
    inferred: Vec<Scancode>,
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

/// The modifiers the platform reports held as a key event arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub command: bool,
}

impl Keyboard {
    pub fn new(setting: CommandKey) -> Self {
        Self {
            setting,
            ..Self::default()
        }
    }

    /// Brings the keyboard up to the modifiers the platform reports held as the event for `key`
    /// arrives, and returns what that sends. A modifier held down before this keyboard saw it,
    /// such as ⌘ still held from the ⌘2 that switched to this Session's Tab, is pressed (as its
    /// left key) so the next key makes the chord the person is holding, and one pressed that way
    /// that is no longer held is let go (see `let_go`). An inferred ⌘ never becomes a lone
    /// Windows key tap: it was held for something else.
    ///
    /// The event's own modifier is left alone: a modifier key's event carries the snapshot from
    /// before it (winit on macOS reports the key, then the modifier change), so the key event
    /// and the `let_go` for the snapshot after it settle that modifier.
    pub fn hold(&mut self, held: Held, key: Scancode) -> Vec<KeyOp> {
        let mut ops = Vec::new();
        for (m, down, stand_in) in wanted(held) {
            if modifier(key) == Some(m) {
                continue;
            }
            if !down {
                ops.extend(self.let_go_of(m));
            } else if !self.is_held(m) {
                if let KeyOutcome::Send(sent) = self.press(stand_in) {
                    ops.extend(sent);
                }
                self.inferred.push(stand_in);
                if let Some(chord) = self.command.as_mut().filter(|_| m == Modifier::Command) {
                    chord.used = true;
                }
            }
        }
        ops
    }

    /// A new modifier snapshot from the platform: every modifier pressed on its word (`hold`)
    /// that it no longer reports held is released, whichever side's key-up let it go. Never
    /// presses anything; a key seen going down is released by its own key-up.
    pub fn let_go(&mut self, held: Held) -> Vec<KeyOp> {
        wanted(held)
            .into_iter()
            .filter(|(_, down, _)| !down)
            .flat_map(|(m, ..)| self.let_go_of(m))
            .collect()
    }

    fn let_go_of(&mut self, m: Modifier) -> Vec<KeyOp> {
        let gone: Vec<Scancode> = self
            .inferred
            .iter()
            .copied()
            .filter(|k| modifier(*k) == Some(m))
            .collect();
        self.inferred.retain(|k| modifier(*k) != Some(m));
        gone.into_iter().flat_map(|k| self.release(k)).collect()
    }

    pub fn key(&mut self, key: Scancode, pressed: bool) -> KeyOutcome {
        if key == Scancode::CAPS_LOCK {
            // A Mac reports Caps Lock turning on as a press and off as a release; Windows
            // toggles on each press, so every change is a whole press.
            return KeyOutcome::Send(vec![KeyOp::Press(key), KeyOp::Release(key)]);
        }
        if pressed {
            // Seen going down now, the key is the person's own and is released by its key-up.
            self.inferred.retain(|k| *k != key);
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
        self.inferred.clear();
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
        self.inferred.retain(|k| *k != key);
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

    #[test]
    fn a_command_held_from_before_still_makes_a_shortcut() {
        let command = Held {
            command: true,
            ..Held::default()
        };
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert!(
            k.hold(command, Scancode::new(0x04)).is_empty(),
            "⌘ alone sends nothing yet"
        );
        assert_eq!(k.key(Scancode::new(0x04), true), KeyOutcome::Shortcut(Shortcut::Tab(3)));
        let both = Held {
            shift: true,
            command: true,
            ..Held::default()
        };
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        assert_eq!(k.hold(both, Scancode::BRACKET_RIGHT), vec![Press(Scancode::SHIFT_LEFT)]);
        assert_eq!(
            k.key(Scancode::BRACKET_RIGHT, true),
            KeyOutcome::Shortcut(Shortcut::NextTab)
        );
        assert!(k.hold(both, Scancode::BRACKET_RIGHT).is_empty(), "held once is enough");
        // The event's own modifier is its own business.
        let mut k = Keyboard::new(CommandKey::WindowsKey);
        assert!(k.hold(command, WIN).is_empty());
        assert_eq!(k.key(WIN, true), send(&[Press(WIN)]));
    }

    /// The keys a Session's Windows sees down, from the ops sent to it.
    /// The keys a Session's Windows sees down, from the ops sent to it, and the modifier
    /// snapshot the Mac last reported.
    #[derive(Debug)]
    struct Remote(Vec<Scancode>, Held);

    impl Remote {
        /// Taking the keyboard while the person holds `held`.
        fn holding(held: Held) -> Self {
            Self(Vec::new(), held)
        }

        fn apply(&mut self, ops: &[KeyOp]) {
            for op in ops {
                match *op {
                    Press(k) => {
                        if !self.0.contains(&k) {
                            self.0.push(k)
                        }
                    }
                    Release(k) => self.0.retain(|held| *held != k),
                }
            }
        }

        fn shifted(&self) -> bool {
            self.0.contains(&Scancode::SHIFT_LEFT) || self.0.contains(&Scancode::SHIFT_RIGHT)
        }
    }

    /// One key event in the order winit on macOS delivers it and `keymap::translate` feeds it,
    /// with `after` the modifiers held once it has happened. Any key: `hold` with the snapshot
    /// Rune has, then the key. A modifier key's event still carries the snapshot from before it,
    /// and the modifier change follows it (`let_go`). Returns what was sent.
    fn event(k: &mut Keyboard, remote: &mut Remote, key: Scancode, pressed: bool, after: Held) -> Vec<KeyOp> {
        let modifier_key = modifier(key).is_some();
        let mut ops = k.hold(if modifier_key { remote.1 } else { after }, key);
        match k.key(key, pressed) {
            KeyOutcome::Send(sent) => ops.extend(sent),
            KeyOutcome::Shortcut(s) => panic!("{key:?} became {s:?}"),
        }
        if modifier_key && after != remote.1 {
            ops.extend(k.let_go(after));
        }
        remote.1 = after;
        remote.apply(&ops);
        ops
    }

    fn held(shift: bool, command: bool) -> Held {
        Held {
            shift,
            command,
            ..Held::default()
        }
    }

    /// ⇧ and ⌘ held down before this keyboard saw them (the ⇧⌘] that switched Tabs), then let go
    /// in either order, with or without a key typed while both were down, for each side of each
    /// and each ⌘ setting, every event in winit's order: no Windows key tap, nothing left down in
    /// Windows once the last modifier change arrives, and the next key typed plain.
    #[test]
    fn inherited_modifiers_are_let_go_whichever_side_and_order() {
        const SHIFTS: [Scancode; 2] = [Scancode::SHIFT_LEFT, Scancode::SHIFT_RIGHT];
        const COMMANDS: [Scancode; 2] = [Scancode::WINDOWS_LEFT, Scancode::WINDOWS_RIGHT];
        for setting in [CommandKey::MacShortcuts, CommandKey::WindowsKey] {
            for shift in SHIFTS {
                for command in COMMANDS {
                    for shift_first in [true, false] {
                        for typed in [false, true] {
                            let case =
                                format!("{setting:?} {shift:?} {command:?} shift_first={shift_first} typed={typed}");
                            let mut k = Keyboard::new(setting);
                            let mut remote = Remote::holding(held(true, true));
                            let mut sent = Vec::new();
                            if typed {
                                sent.extend(event(&mut k, &mut remote, A, true, held(true, true)));
                                sent.extend(event(&mut k, &mut remote, A, false, held(true, true)));
                            }
                            let (first, then, after_first) = match shift_first {
                                true => (shift, command, held(false, true)),
                                false => (command, shift, held(true, false)),
                            };
                            sent.extend(event(&mut k, &mut remote, first, false, after_first));
                            sent.extend(event(&mut k, &mut remote, then, false, held(false, false)));
                            if setting == CommandKey::MacShortcuts {
                                assert!(!sent.contains(&Press(WIN)), "{case}: a lone Windows key tap");
                            }
                            assert_eq!(remote.0, vec![], "{case}: left down in Windows");
                            assert_eq!(
                                event(&mut k, &mut remote, C, true, held(false, false)),
                                vec![Press(C)],
                                "{case}: the next key is plain"
                            );
                            assert_eq!(k.release_all(), vec![Release(C)], "{case}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_inherited_right_shift_lets_go_when_released() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        let mut remote = Remote::holding(held(true, false));
        event(&mut k, &mut remote, A, true, held(true, false));
        assert!(remote.shifted(), "A is typed shifted");
        event(&mut k, &mut remote, A, false, held(true, false));
        // The key-up still says ⇧ is held; the modifier change after it lets go.
        assert_eq!(k.hold(held(true, false), Scancode::SHIFT_RIGHT), vec![]);
        assert_eq!(k.key(Scancode::SHIFT_RIGHT, false), send(&[]));
        assert_eq!(k.let_go(held(false, false)), vec![Release(Scancode::SHIFT_LEFT)]);
        remote.apply(&[Release(Scancode::SHIFT_LEFT)]);
        remote.1 = held(false, false);
        event(&mut k, &mut remote, C, true, held(false, false));
        assert!(!remote.shifted(), "C is typed plain");
    }

    #[test]
    fn a_shift_held_on_both_sides_stays_down_until_both_are_let_go() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        let mut remote = Remote::holding(held(true, false));
        // Right ⇧ held from before; left ⇧ pressed here, then let go while right is still down.
        event(&mut k, &mut remote, A, true, held(true, false));
        event(&mut k, &mut remote, A, false, held(true, false));
        event(&mut k, &mut remote, Scancode::SHIFT_LEFT, true, held(true, false));
        event(&mut k, &mut remote, Scancode::SHIFT_LEFT, false, held(true, false));
        event(&mut k, &mut remote, C, true, held(true, false));
        assert!(remote.shifted(), "right ⇧ is still down");
        event(&mut k, &mut remote, C, false, held(true, false));
        event(&mut k, &mut remote, Scancode::SHIFT_RIGHT, false, held(false, false));
        assert_eq!(remote.0, vec![]);
        // Both seen going down here: each key-up is its own.
        event(&mut k, &mut remote, Scancode::SHIFT_LEFT, true, held(true, false));
        event(&mut k, &mut remote, Scancode::SHIFT_RIGHT, true, held(true, false));
        event(&mut k, &mut remote, Scancode::SHIFT_LEFT, false, held(true, false));
        assert!(remote.shifted());
        event(&mut k, &mut remote, Scancode::SHIFT_RIGHT, false, held(false, false));
        assert_eq!(remote.0, vec![]);
    }

    #[test]
    fn an_inherited_command_never_taps_the_windows_key_but_one_pressed_here_does() {
        let mut k = Keyboard::new(CommandKey::MacShortcuts);
        let mut remote = Remote::holding(held(false, true));
        // ⌘ held from before, let go with nothing typed.
        let mut sent = event(&mut k, &mut remote, Scancode::SHIFT_LEFT, true, held(true, true));
        sent.extend(event(
            &mut k,
            &mut remote,
            Scancode::SHIFT_LEFT,
            false,
            held(false, true),
        ));
        sent.extend(event(
            &mut k,
            &mut remote,
            Scancode::WINDOWS_RIGHT,
            false,
            held(false, false),
        ));
        assert!(!sent.contains(&Press(WIN)));
        // ⌘ pressed and released here alone is still the Windows key.
        let sent: Vec<KeyOp> = [true, false]
            .into_iter()
            .flat_map(|down| event(&mut k, &mut remote, WIN, down, held(false, down)))
            .collect();
        assert_eq!(sent, vec![Press(WIN), Release(WIN)]);
    }

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
