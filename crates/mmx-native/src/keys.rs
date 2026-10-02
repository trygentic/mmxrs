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
    (KeyCode::Escape, Hotkey::Quit),
    (KeyCode::F6, Hotkey::Pause),
    (KeyCode::Period, Hotkey::FrameStep),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    P1(u16),
    P2(u16),
    Hot(Hotkey),
}

fn find<T: Copy>(t: &[(KeyCode, T)], k: KeyCode) -> Option<T> {
    t.iter().find(|(c, _)| *c == k).map(|(_, v)| *v)
}

/// What a key does. The P2 table is live only in co-op. The three tables are
/// disjoint (pinned by a test), so priority never matters.
// ponytail: no rebinding UI.
pub fn map_key(k: KeyCode, coop: bool) -> Option<KeyAction> {
    if coop {
        if let Some(b) = find(P2_KEYS, k) {
            return Some(KeyAction::P2(b));
        }
    }
    find(P1_KEYS, k)
        .map(KeyAction::P1)
        .or_else(|| find(HOTKEYS, k).map(KeyAction::Hot))
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
        assert_eq!(no_dupes(P1_KEYS), ALL12);
        assert_eq!(no_dupes(P2_KEYS), ALL12);
        // each button has exactly one key per player (Shift L/R both Select on P1)
        for t in [P1_KEYS, P2_KEYS] {
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
            assert_eq!(map_key(k, false), Some(KeyAction::P1(b)));
        }
        for &(k, b) in P2_KEYS {
            assert_eq!(map_key(k, true), Some(KeyAction::P2(b)));
            if find(P1_KEYS, k).is_none() && find(HOTKEYS, k).is_none() {
                assert_eq!(
                    map_key(k, false),
                    None,
                    "P2 key {k:?} leaks into single player"
                );
            }
        }
        for &(k, h) in HOTKEYS {
            if find(P1_KEYS, k).is_none() {
                assert_eq!(map_key(k, false), Some(KeyAction::Hot(h)));
            }
        }
        // no hotkey shadowed by P1, and pause stays reachable in co-op
        assert!(HOTKEYS.iter().all(|(k, _)| find(P1_KEYS, *k).is_none()));
        assert_eq!(
            map_key(KeyCode::F6, true),
            Some(KeyAction::Hot(Hotkey::Pause))
        );
        assert_eq!(map_key(KeyCode::KeyA, true), Some(KeyAction::P2(BTN_LEFT)));
        assert_eq!(map_key(KeyCode::KeyA, false), None);
        // the three tables never share a key
        for &(k, _) in P2_KEYS {
            assert!(find(P1_KEYS, k).is_none() && find(HOTKEYS, k).is_none(), "{k:?}");
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
