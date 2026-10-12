//! Which local player each gamepad drives.
//!
//! In co-op a pad joins on its first press and takes the first open player
//! slot. A slot the keyboard is already driving is not open: with the
//! keyboard on P1 and P2, the first two pads become P3 and P4 instead of
//! doubling up on P1 and P2 (which left P3 and P4 with no input at all). When
//! the keyboard starts driving a slot a pad already holds, that pad moves to
//! the next open slot so nobody shares a character.

/// Start and Select alone don't count as the keyboard playing a slot: someone
/// on a pad may still press Enter to start the game.
const PLAY_BITS: u16 = !(mmx_core::BTN_START | mmx_core::BTN_SELECT);

#[derive(Debug, Clone)]
pub struct PadSlots<Id> {
    /// The pad driving each local player.
    pub pads: [Option<Id>; 4],
    /// Keyboard halves that have played (P1/P2 tables, then P3/P4).
    kb_used: [bool; 4],
    players: usize,
    coop: bool,
}

impl<Id: Copy + Eq> PadSlots<Id> {
    pub fn new(players: usize, coop: bool) -> Self {
        Self { pads: [None; 4], kb_used: [false; 4], players: players.clamp(1, 4), coop }
    }

    fn open(&self, slot: usize) -> bool {
        self.pads[slot].is_none() && !self.kb_used[slot]
    }

    /// A pad was pressed. Single player keeps the last-pressed pad on P1.
    /// Returns the slot it was newly given, if any.
    pub fn claim(&mut self, id: Id) -> Option<usize> {
        if !self.coop {
            let fresh = self.pads[0] != Some(id);
            self.pads[0] = Some(id);
            return fresh.then_some(0);
        }
        if self.pads.contains(&Some(id)) {
            return None;
        }
        // An open slot first; with every slot taken by keyboard or pad, share
        // a keyboard slot rather than leave the pad dead.
        let slot = (0..self.players)
            .find(|&s| self.open(s))
            .or_else(|| (0..self.players).find(|&s| self.pads[s].is_none()))?;
        self.pads[slot] = Some(id);
        Some(slot)
    }

    pub fn forget(&mut self, id: Id) {
        for slot in &mut self.pads {
            if *slot == Some(id) {
                *slot = None;
            }
        }
    }

    /// A key of keyboard half `slot` was pressed with these pad `bits`.
    /// Returns `(from, to)` when a pad had to move out of the way.
    pub fn keyboard(&mut self, slot: usize, bits: u16) -> Option<(usize, usize)> {
        if !self.coop || slot >= self.players || bits & PLAY_BITS == 0 || self.kb_used[slot] {
            return None;
        }
        self.kb_used[slot] = true;
        let id = self.pads[slot]?;
        let to = (0..self.players).find(|&s| self.open(s))?;
        self.pads[slot] = None;
        self.pads[to] = Some(id);
        Some((slot, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmx_core::{BTN_B, BTN_LEFT, BTN_START};

    #[test]
    fn four_pads_fill_four_players_in_press_order() {
        let mut s = PadSlots::new(4, true);
        assert_eq!(s.claim('c'), Some(0));
        assert_eq!(s.claim('a'), Some(1));
        assert_eq!(s.claim('c'), None, "a pad is claimed once");
        assert_eq!(s.claim('d'), Some(2));
        assert_eq!(s.claim('b'), Some(3));
        assert_eq!(s.claim('e'), None, "no fifth player");
        assert_eq!(s.pads, [Some('c'), Some('a'), Some('d'), Some('b')]);
    }

    #[test]
    fn pads_skip_slots_the_keyboard_plays() {
        let mut s = PadSlots::new(4, true);
        assert_eq!(s.keyboard(0, BTN_LEFT), None);
        assert_eq!(s.keyboard(1, BTN_B), None);
        assert_eq!(s.claim('a'), Some(2));
        assert_eq!(s.claim('b'), Some(3));
        assert_eq!(s.pads, [None, None, Some('a'), Some('b')]);
    }

    #[test]
    fn keyboard_joining_late_moves_the_pad_along() {
        let mut s = PadSlots::new(3, true);
        assert_eq!(s.claim('a'), Some(0));
        assert_eq!(s.claim('b'), Some(1));
        assert_eq!(s.keyboard(0, BTN_B), Some((0, 2)));
        assert_eq!(s.pads, [None, Some('b'), Some('a'), None]);
        assert_eq!(s.keyboard(0, BTN_B), None, "only the first key moves a pad");
        // Nowhere to go: the pad stays and shares with the keyboard.
        assert_eq!(s.keyboard(1, BTN_B), None);
        assert_eq!(s.pads, [None, Some('b'), Some('a'), None]);
    }

    #[test]
    fn start_on_the_keyboard_does_not_take_a_slot() {
        let mut s = PadSlots::new(4, true);
        assert_eq!(s.claim('a'), Some(0));
        assert_eq!(s.keyboard(0, BTN_START), None);
        assert_eq!(s.claim('b'), Some(1));
        assert_eq!(s.pads[..2], [Some('a'), Some('b')]);
    }

    #[test]
    fn full_slots_share_with_the_keyboard() {
        let mut s = PadSlots::new(2, true);
        s.keyboard(0, BTN_B);
        s.keyboard(1, BTN_B);
        assert_eq!(s.claim('a'), Some(0));
        assert_eq!(s.claim('b'), Some(1));
    }

    #[test]
    fn disconnect_frees_the_slot() {
        let mut s = PadSlots::new(4, true);
        s.claim('a');
        s.claim('b');
        s.forget('a');
        assert_eq!(s.claim('c'), Some(0));
    }

    #[test]
    fn single_player_follows_the_last_pad() {
        let mut s = PadSlots::new(1, false);
        assert_eq!(s.claim('a'), Some(0));
        assert_eq!(s.claim('b'), Some(0));
        assert_eq!(s.claim('b'), None);
        assert_eq!(s.keyboard(0, BTN_B), None);
        assert_eq!(s.pads, [Some('b'), None, None, None]);
    }
}
