//! Which OAM slots draw each player, for [`Game::set_display_offset`] (the
//! netplay smoothing's render-time sprite offset, docs/netplay.md "Rollback
//! smoothing"). Observe traps only: nothing here changes the simulation.
//!
//! P1: the OAM build `$80:D544` emits the player's armour parts
//! (`JSR $80:D640` at `$80:D55A`) then the player (`JSR $80:D671` at
//! `$80:D55D`, returning to `$80:D560`) through the cursor `$E4`, so the
//! slots P1 took are `$E4` at `$D55A` up to `$E4` at `$D560` (a Zero P1 goes
//! through the same cursor, zero.rs `emit_p1`). P2: its display slice
//! (coop.rs `p2_display`) records its own range, HUD excluded.

use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::Game;

/// `JSR $80:D640` (armour parts), then `JSR $80:D671` (the player).
pub const HOOK_P1_OAM: u32 = 0x80D55A;
/// The instruction after `JSR $80:D671`.
pub const HOOK_P1_OAM_DONE: u32 = 0x80D560;
const OAM_CURSOR: usize = 0x00E4;
/// Y of a hidden OAM slot (the ROM's hide loop, coop.rs alignment padding).
const HIDDEN_Y: u8 = 0xE0;

pub fn register(game: &mut Game) {
    game.traps.register(HOOK_P1_OAM, "display_p1_oam", TrapReturn::Observe, p1_start);
    game.traps.register(HOOK_P1_OAM_DONE, "display_p1_oam_done", TrapReturn::Observe, p1_done);
}

/// Slots `a..b` as a mask (bit n = slot n, clamped to 128).
pub fn slots(a: usize, b: usize) -> u128 {
    (a.min(128)..b.min(128)).fold(0, |m, s| m | 1 << s)
}

/// `mask` minus the slots hidden in the OAM buffer (`$0700`).
pub fn visible(wram: &[u8; 0x20000], mask: u128) -> u128 {
    (0..128).filter(|&s| mask >> s & 1 != 0 && wram[ix(ram::OAM_BUF) + s * 4 + 1] != HIDDEN_Y).fold(0, |m, s| m | 1 << s)
}

/// Build in progress: P1 starts at the cursor (to slot 127 until
/// [`p1_done`]: the emitter's overflow exit skips it); P2 has no slots until
/// its display slice runs.
fn p1_start(game: &mut Game) {
    if game.isolated_depth > 0 {
        return;
    }
    let c = usize::from(game.wram()[OAM_CURSOR] & 0x7F);
    game.oam_slots = [slots(c, 128), 0];
}

fn p1_done(game: &mut Game) {
    if game.isolated_depth > 0 {
        return;
    }
    let end = usize::from(game.wram()[OAM_CURSOR] & 0x7F);
    let start = game.oam_slots[0].trailing_zeros() as usize;
    game.oam_slots[0] = slots(start, end);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netgame::{net_checksum, screen_pos, session_game};
    use crate::{Pads, Timing, BTN_LEFT, BTN_RIGHT};

    #[test]
    fn display_slots_mask() {
        assert_eq!(slots(0, 0), 0);
        assert_eq!(slots(2, 5), 0b11100);
        assert_eq!(slots(126, 200), 3 << 126);
    }

    /// Real ROM, hw-spiky with P2 in play: the PPU's OAM after step F is the
    /// buffer built in step F-1 (so the picture of step F shows frame F-1's
    /// positions, and the offset for it is set before the step from the
    /// previous build's slots). An offset on one player moves only pixels
    /// near that player and changes no state.
    #[test]
    fn display_offset_moves_one_player() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM not set; skipping");
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else { return eprintln!("{st} missing; skipping") };
        let fresh = || session_game(&rom, mmx_net::COOP_TWO_PLAYERS, Timing::Auto).unwrap();
        let mut a = fresh();
        a.load_state(&state).unwrap();
        for f in 0..120 {
            a.step(Pads { p1: if f < 60 { BTN_RIGHT } else { 0 }, p2: BTN_LEFT * u16::from(f > 90) });
        }
        assert!(!a.coop.p2_hidden, "P2 not in play");
        let built = a.wram()[ix(ram::OAM_BUF)..][..512].to_vec();
        a.step(Pads::default());
        assert_eq!(&a.oam()[..512], &built[..], "step F shows the OAM built in step F-1");
        // past any i-frame blink (a blinking player emits nothing every other frame)
        for _ in 0..200 {
            if a.oam_slots.iter().all(|&m| m != 0) {
                break;
            }
            a.step(Pads::default());
        }
        assert!(a.oam_slots.iter().all(|&m| m != 0), "{:x?}", a.oam_slots);
        assert_eq!(a.oam_slots[0] & a.oam_slots[1], 0);
        let copy = |g: &Game| {
            let mut b = fresh();
            b.load_state(&g.save_state()).unwrap();
            b.coop = g.coop.clone();
            b.oam_slots = g.oam_slots;
            b
        };
        for player in 0..2 {
            let (mut b, mut c) = (copy(&a), copy(&a));
            let pos = screen_pos(&b, player).unwrap();
            b.set_display_offset(player, 5, -7);
            b.step(Pads::default());
            c.step(Pads::default());
            assert_eq!(net_checksum(&b), net_checksum(&c));
            assert_eq!(b.oam(), c.oam());
            let (pb, w, h) = b.frame();
            let pc = c.frame().0;
            let diff: Vec<(usize, usize)> = (0..w * h).filter(|&i| pb[i] != pc[i]).map(|i| (i % w, i / w)).collect();
            assert!(!diff.is_empty(), "player {player}: the offset changed nothing");
            for &(x, y) in &diff {
                let (dx, dy) = (x as i32 - pos.0, y as i32 - pos.1);
                assert!(dx.abs() < 48 && dy.abs() < 56, "player {player}: pixel ({x},{y}) far from {pos:?}");
            }
            eprintln!("player {player} at {pos:?}: {} pixels changed", diff.len());
        }
    }
}
