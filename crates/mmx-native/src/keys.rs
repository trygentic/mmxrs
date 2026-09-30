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
    (KeyCode::KeyA, BTN_L),
    (KeyCode::KeyS, BTN_R),
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
    (KeyCode::Escape, Hotkey::Quit),
    (KeyCode::KeyP, Hotkey::Pause),
    // P is P2 Start in co-op, so pause also lives on F6.
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

/// What a key does. The P2 table is live only in co-op, and there it wins
/// over P1 and hotkeys: A/S (P1 L/R) and P (pause) collide with WASD/P2 Start.
// ponytail: collisions resolved by priority, not by a rebinding UI.
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
        assert_eq!(map_key(KeyCode::KeyA, false), Some(KeyAction::P1(BTN_L)));
    }

    #[test]
    fn pad_buttons_cover_all() {
        assert_eq!(PAD_BUTTONS.iter().fold(0, |a, (_, b)| a | b), ALL12);
    }

    #[test]
    fn stick() {
        assert_eq!(stick_bits(0.0, 0.0), 0);
        assert_eq!(stick_bits(-1.0, 1.0), BTN_LEFT | BTN_UP);
        assert_eq!(stick_bits(0.9, -0.9), BTN_RIGHT | BTN_DOWN);
    }
}
