//! Keyboard and gamepad tables. Physical key codes, so layouts don't matter.

use mmx_core::*;
use winit::keyboard::KeyCode;

pub const P1_KEYS: &[(KeyCode, u16)] = &[
    (KeyCode::ArrowUp, BTN_UP),
    (KeyCode::ArrowDown, BTN_DOWN),
    (KeyCode::ArrowLeft, BTN_LEFT),
    (KeyCode::ArrowRight, BTN_RIGHT),
    (KeyCode::KeyZ, BTN_B),
    (KeyCode::KeyX, BTN_Y),
    (KeyCode::KeyC, BTN_A),
    (KeyCode::KeyV, BTN_X),
    (KeyCode::KeyQ, BTN_L),
    (KeyCode::KeyE, BTN_R),
    (KeyCode::Enter, BTN_START),
    (KeyCode::ShiftLeft, BTN_SELECT),
    (KeyCode::ShiftRight, BTN_SELECT),
];

pub const P2_KEYS: &[(KeyCode, u16)] = &[
    (KeyCode::KeyW, BTN_UP),
    (KeyCode::KeyS, BTN_DOWN),
    (KeyCode::KeyA, BTN_LEFT),
    (KeyCode::KeyD, BTN_RIGHT),
    (KeyCode::KeyG, BTN_B),
    (KeyCode::KeyF, BTN_Y),
    (KeyCode::KeyH, BTN_A),
    (KeyCode::KeyT, BTN_X),
    (KeyCode::KeyR, BTN_L),
    (KeyCode::KeyY, BTN_R),
    (KeyCode::KeyP, BTN_START),
    (KeyCode::KeyO, BTN_SELECT),
];

/// Third local player: IJKL plus the keys around them.
pub const P3_KEYS: &[(KeyCode, u16)] = &[
    (KeyCode::KeyI, BTN_UP),
    (KeyCode::KeyK, BTN_DOWN),
    (KeyCode::KeyJ, BTN_LEFT),
    (KeyCode::KeyL, BTN_RIGHT),
    (KeyCode::KeyN, BTN_B),
    (KeyCode::KeyM, BTN_Y),
    (KeyCode::Comma, BTN_A),
    (KeyCode::KeyB, BTN_X),
    (KeyCode::KeyU, BTN_L),
    (KeyCode::Semicolon, BTN_R),
    (KeyCode::BracketRight, BTN_START),
    (KeyCode::BracketLeft, BTN_SELECT),
];

/// Fourth local player: the numeric keypad.
pub const P4_KEYS: &[(KeyCode, u16)] = &[
    (KeyCode::Numpad8, BTN_UP),
    (KeyCode::Numpad5, BTN_DOWN),
    (KeyCode::Numpad4, BTN_LEFT),
    (KeyCode::Numpad6, BTN_RIGHT),
    (KeyCode::Numpad1, BTN_B),
    (KeyCode::Numpad2, BTN_Y),
    (KeyCode::Numpad3, BTN_A),
    (KeyCode::Numpad0, BTN_X),
    (KeyCode::Numpad7, BTN_L),
    (KeyCode::Numpad9, BTN_R),
    (KeyCode::NumpadEnter, BTN_START),
    (KeyCode::NumpadDecimal, BTN_SELECT),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    FastForward,
    SaveState,
    LoadState,
    Slot(u8),
    Fullscreen,
    Quit,
    Pause,
    FrameStep,
    /// F10: next widescreen setting (presets, then auto).
    Widescreen,
    /// F2: fps and speed readout in the picture's corner.
    ShowFps,
}

pub const HOTKEYS: &[(KeyCode, Hotkey)] = &[
    (KeyCode::Tab, Hotkey::FastForward),
    (KeyCode::F5, Hotkey::SaveState),
    (KeyCode::F7, Hotkey::LoadState),
    (KeyCode::Digit1, Hotkey::Slot(1)),
    (KeyCode::Digit2, Hotkey::Slot(2)),
    (KeyCode::Digit3, Hotkey::Slot(3)),
    (KeyCode::Digit4, Hotkey::Slot(4)),
    (KeyCode::Digit5, Hotkey::Slot(5)),
    (KeyCode::Digit6, Hotkey::Slot(6)),
    (KeyCode::Digit7, Hotkey::Slot(7)),
    (KeyCode::Digit8, Hotkey::Slot(8)),
    (KeyCode::Digit9, Hotkey::Slot(9)),
    (KeyCode::F11, Hotkey::Fullscreen),
    (KeyCode::F10, Hotkey::Widescreen),
    (KeyCode::F2, Hotkey::ShowFps),
    (KeyCode::Escape, Hotkey::Quit),
    (KeyCode::F6, Hotkey::Pause),
    (KeyCode::Period, Hotkey::FrameStep),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    P1(u16),
    P2(u16),
    P3(u16),
    P4(u16),
    Hot(Hotkey),
}

fn find<T: Copy>(t: &[(KeyCode, T)], k: KeyCode) -> Option<T> {
    t.iter().find(|(c, _)| *c == k).map(|(_, v)| *v)
}

/// What a key does with `players` local players: the P2..P4 tables are live
/// only when that player is. The tables are disjoint (pinned by a test), so
/// priority never matters.
// ponytail: no rebinding UI.
pub fn map_key(k: KeyCode, players: usize) -> Option<KeyAction> {
    let extra: [(&[(KeyCode, u16)], fn(u16) -> KeyAction); 3] =
        [(P2_KEYS, KeyAction::P2), (P3_KEYS, KeyAction::P3), (P4_KEYS, KeyAction::P4)];
    for (table, action) in extra.into_iter().take(players.saturating_sub(1)) {
        if let Some(b) = find(table, k) {
            return Some(action(b));
        }
    }
    find(P1_KEYS, k)
        .map(KeyAction::P1)
        .or_else(|| find(HOTKEYS, k).map(KeyAction::Hot))
}

/// Pad button -> SNES bit tables, the defaults below or a `--pad-map` override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadMap {
    pub gilrs: Vec<(gilrs::Button, u16)>,
    /// GameController buttons in `gc_pad::snapshot` order, derived from `gilrs`.
    pub gc: [(&'static str, u16); 10],
}
impl Default for PadMap {
    fn default() -> Self {
        Self { gilrs: PAD_BUTTONS.to_vec(), gc: GC_BUTTONS }
    }
}
/// gilrs button names accepted by `--pad-map`, and the GameController button
/// (if any) at the same physical position.
const PAD_NAMES: &[(&str, gilrs::Button, Option<&str>)] = &[
    ("South", gilrs::Button::South, Some("buttonA")),
    ("East", gilrs::Button::East, Some("buttonB")),
    ("West", gilrs::Button::West, Some("buttonX")),
    ("North", gilrs::Button::North, Some("buttonY")),
    ("LeftTrigger", gilrs::Button::LeftTrigger, Some("leftShoulder")),
    ("RightTrigger", gilrs::Button::RightTrigger, Some("rightShoulder")),
    ("LeftTrigger2", gilrs::Button::LeftTrigger2, Some("leftTrigger")),
    ("RightTrigger2", gilrs::Button::RightTrigger2, Some("rightTrigger")),
    ("Start", gilrs::Button::Start, Some("buttonMenu")),
    ("Select", gilrs::Button::Select, Some("buttonOptions")),
    ("DPadUp", gilrs::Button::DPadUp, None),
    ("DPadDown", gilrs::Button::DPadDown, None),
    ("DPadLeft", gilrs::Button::DPadLeft, None),
    ("DPadRight", gilrs::Button::DPadRight, None),
    ("LeftThumb", gilrs::Button::LeftThumb, None),
    ("RightThumb", gilrs::Button::RightThumb, None),
    ("C", gilrs::Button::C, None),
    ("Z", gilrs::Button::Z, None),
    ("Mode", gilrs::Button::Mode, None),
];
const BIT_NAMES: [(&str, u16); 12] = [
    ("A", BTN_A),
    ("B", BTN_B),
    ("X", BTN_X),
    ("Y", BTN_Y),
    ("L", BTN_L),
    ("R", BTN_R),
    ("Start", BTN_START),
    ("Select", BTN_SELECT),
    ("Up", BTN_UP),
    ("Down", BTN_DOWN),
    ("Left", BTN_LEFT),
    ("Right", BTN_RIGHT),
];
impl PadMap {
    /// `--pad-map "South=A,East=B,LeftTrigger2=none"`: each listed pad button
    /// gets that SNES button (`none` unbinds it); unlisted buttons keep the
    /// default. Names are gilrs button names and SNES button names, case-insensitive.
    pub fn parse(spec: Option<&str>) -> Result<Self, String> {
        let mut m = Self::default();
        let Some(spec) = spec else { return Ok(m) };
        for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (pad, btn) = pair
                .split_once('=')
                .ok_or_else(|| format!("bad --pad-map entry {pair:?}: want PAD=BUTTON"))?;
            let (_, button, gc) = PAD_NAMES
                .iter()
                .find(|(n, _, _)| n.eq_ignore_ascii_case(pad.trim()))
                .ok_or_else(|| format!("unknown pad button {pad:?} in --pad-map"))?;
            let bit = if btn.trim().eq_ignore_ascii_case("none") {
                0
            } else {
                BIT_NAMES
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(btn.trim()))
                    .map(|(_, b)| *b)
                    .ok_or_else(|| format!("unknown SNES button {btn:?} in --pad-map"))?
            };
            m.gilrs.retain(|(b, _)| b != button);
            if bit != 0 {
                m.gilrs.push((*button, bit));
            }
            if let Some(gc) = gc {
                for entry in m.gc.iter_mut().filter(|(n, _)| n == gc) {
                    entry.1 = bit;
                }
            }
        }
        Ok(m)
    }
}
/// Standard-mapping gamepad buttons -> SNES bits.
pub const PAD_BUTTONS: &[(gilrs::Button, u16)] = &[
    (gilrs::Button::South, BTN_B),
    (gilrs::Button::West, BTN_Y),
    (gilrs::Button::East, BTN_A),
    (gilrs::Button::North, BTN_X),
    (gilrs::Button::LeftTrigger, BTN_L),
    (gilrs::Button::RightTrigger, BTN_R),
    (gilrs::Button::LeftTrigger2, BTN_L),
    (gilrs::Button::RightTrigger2, BTN_R),
    (gilrs::Button::Start, BTN_START),
    (gilrs::Button::Select, BTN_SELECT),
    (gilrs::Button::DPadUp, BTN_UP),
    (gilrs::Button::DPadDown, BTN_DOWN),
    (gilrs::Button::DPadLeft, BTN_LEFT),
    (gilrs::Button::DPadRight, BTN_RIGHT),
];

/// macOS GameController extended-gamepad buttons -> SNES bits, in the order
/// `gc_pad::snapshot` reads them. GC names are positional on every pad:
/// `buttonA` is the bottom face button (Xbox A, PlayStation Cross, Switch B),
/// `buttonB` right, `buttonX` left, `buttonY` top. D-pad and left stick are
/// read separately.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GC_BUTTONS: [(&str, u16); 10] = [
    ("buttonA", BTN_B),
    ("buttonB", BTN_A),
    ("buttonX", BTN_Y),
    ("buttonY", BTN_X),
    ("leftShoulder", BTN_L),
    ("rightShoulder", BTN_R),
    ("leftTrigger", BTN_L),
    ("rightTrigger", BTN_R),
    ("buttonMenu", BTN_START),
    ("buttonOptions", BTN_SELECT),
];

/// `"B Y Up"`: SNES pad bits by name, for the `--pads` diagnostic.
pub fn bit_names(bits: u16) -> String {
    const NAMES: [(u16, &str); 12] = [
        (BTN_B, "B"),
        (BTN_Y, "Y"),
        (BTN_SELECT, "Select"),
        (BTN_START, "Start"),
        (BTN_UP, "Up"),
        (BTN_DOWN, "Down"),
        (BTN_LEFT, "Left"),
        (BTN_RIGHT, "Right"),
        (BTN_A, "A"),
        (BTN_X, "X"),
        (BTN_L, "L"),
        (BTN_R, "R"),
    ];
    let v: Vec<&str> = NAMES.iter().filter(|(b, _)| bits & b != 0).map(|(_, n)| *n).collect();
    if v.is_empty() {
        "-".into()
    } else {
        v.join(" ")
    }
}

/// Every keyboard table, P1 first.
#[cfg(test)]
const PLAYER_KEYS: [&[(KeyCode, u16)]; 4] = [P1_KEYS, P2_KEYS, P3_KEYS, P4_KEYS];

/// gilrs axes that move a player: the left stick and a hat-style d-pad.
pub const MOVE_AXES: [gilrs::Axis; 4] =
    [gilrs::Axis::LeftStickX, gilrs::Axis::LeftStickY, gilrs::Axis::DPadX, gilrs::Axis::DPadY];

/// Left stick -> d-pad bits (gilrs Y is up-positive).
pub fn stick_bits(x: f32, y: f32) -> u16 {
    const DZ: f32 = 0.5;
    let mut b = 0;
    if x < -DZ {
        b |= BTN_LEFT;
    }
    if x > DZ {
        b |= BTN_RIGHT;
    }
    if y > DZ {
        b |= BTN_UP;
    }
    if y < -DZ {
        b |= BTN_DOWN;
    }
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn pad_map_overrides_only_listed_buttons() {
        use super::*;
        let m = PadMap::parse(Some("south=A, East=b, LeftTrigger2=none")).unwrap();
        let bit = |b: gilrs::Button| m.gilrs.iter().find(|(x, _)| *x == b).map(|(_, v)| *v);
        assert_eq!(bit(gilrs::Button::South), Some(BTN_A));
        assert_eq!(bit(gilrs::Button::East), Some(BTN_B));
        assert_eq!(bit(gilrs::Button::LeftTrigger2), None);
        assert_eq!(bit(gilrs::Button::North), Some(BTN_X), "unlisted keeps default");
        let gc = |n: &str| m.gc.iter().find(|(x, _)| *x == n).map(|(_, v)| *v).unwrap();
        assert_eq!((gc("buttonA"), gc("buttonB"), gc("leftTrigger")), (BTN_A, BTN_B, 0));
        assert_eq!(PadMap::parse(None).unwrap(), PadMap::default());
        assert!(PadMap::parse(Some("Souths=A")).is_err());
        assert!(PadMap::parse(Some("South=Jump")).is_err());
        assert!(PadMap::parse(Some("South")).is_err());
    }
    use super::*;
    use std::collections::HashSet;

    const ALL12: u16 = 0xFFF0;

    fn no_dupes(t: &[(KeyCode, u16)]) -> u16 {
        let mut keys = HashSet::new();
        let mut bits = 0;
        for &(k, b) in t {
            assert!(keys.insert(k), "{k:?} bound twice");
            assert_eq!(b.count_ones(), 1, "{k:?} must set exactly one bit");
            bits |= b;
        }
        bits
    }

    #[test]
    fn player_tables_cover_all_buttons_once() {
        for t in PLAYER_KEYS {
            assert_eq!(no_dupes(t), ALL12);
        }
        // each button has exactly one key per player (Shift L/R both Select on P1)
        for t in PLAYER_KEYS {
            for bit in 4..16 {
                let n = t
                    .iter()
                    .filter(|(k, b)| *b == 1 << bit && *k != KeyCode::ShiftRight)
                    .count();
                assert_eq!(n, 1, "bit {bit}");
            }
        }
    }

    #[test]
    fn hotkeys_unique() {
        let mut keys = HashSet::new();
        for (k, _) in HOTKEYS {
            assert!(keys.insert(*k), "{k:?} hotkey twice");
        }
    }

    #[test]
    fn every_bound_key_recognised() {
        for &(k, b) in P1_KEYS {
            assert_eq!(map_key(k, 1), Some(KeyAction::P1(b)));
        }
        let actions: [fn(u16) -> KeyAction; 3] = [KeyAction::P2, KeyAction::P3, KeyAction::P4];
        for (i, (table, action)) in PLAYER_KEYS[1..].iter().zip(actions).enumerate() {
            let player = i + 2;
            for &(k, b) in *table {
                assert_eq!(map_key(k, player), Some(action(b)), "{k:?}");
                assert_eq!(map_key(k, 4), Some(action(b)), "{k:?}");
                assert_eq!(map_key(k, player - 1), None, "P{player} key {k:?} live with fewer players");
            }
        }
        for &(k, h) in HOTKEYS {
            for players in 1..=4 {
                assert_eq!(map_key(k, players), Some(KeyAction::Hot(h)));
            }
        }
        assert_eq!(map_key(KeyCode::KeyA, 2), Some(KeyAction::P2(BTN_LEFT)));
        assert_eq!(map_key(KeyCode::KeyA, 1), None);
        assert_eq!(map_key(KeyCode::KeyJ, 3), Some(KeyAction::P3(BTN_LEFT)));
        assert_eq!(map_key(KeyCode::Numpad4, 4), Some(KeyAction::P4(BTN_LEFT)));
        // no table shares a key with another or with a hotkey
        let mut seen = HashSet::new();
        for t in PLAYER_KEYS {
            for &(k, _) in t {
                assert!(seen.insert(k), "{k:?} in two tables");
            }
        }
        for (k, _) in HOTKEYS {
            assert!(!seen.contains(k), "{k:?} is a hotkey and a player key");
        }
    }

    #[test]
    fn pad_buttons_cover_all() {
        assert_eq!(PAD_BUTTONS.iter().fold(0, |a, (_, b)| a | b), ALL12);
    }

    #[test]
    fn gc_matches_gilrs_positions() {
        // everything but the d-pad (read separately on GC)
        let gc = GC_BUTTONS.iter().fold(0, |a, (_, b)| a | b);
        assert_eq!(gc, ALL12 & !(BTN_UP | BTN_DOWN | BTN_LEFT | BTN_RIGHT));
        let pad = |b| PAD_BUTTONS.iter().find(|(x, _)| *x == b).unwrap().1;
        let gcb = |n| GC_BUTTONS.iter().find(|(x, _)| *x == n).unwrap().1;
        assert_eq!(gcb("buttonA"), pad(gilrs::Button::South));
        assert_eq!(gcb("buttonB"), pad(gilrs::Button::East));
        assert_eq!(gcb("buttonX"), pad(gilrs::Button::West));
        assert_eq!(gcb("buttonY"), pad(gilrs::Button::North));
        assert_eq!(bit_names(0), "-");
        assert_eq!(bit_names(BTN_B | BTN_UP), "B Up");
    }

    #[test]
    fn stick() {
        assert_eq!(stick_bits(0.0, 0.0), 0);
        assert_eq!(stick_bits(-1.0, 1.0), BTN_LEFT | BTN_UP);
        assert_eq!(stick_bits(0.9, -0.9), BTN_RIGHT | BTN_DOWN);
    }
}
