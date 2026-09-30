//! P2's HP bar and the down / revive rules (docs/coop.md "P2 HP bar and
//! revive").
//!
//! X's bar is seven static 16x16 sprites in OAM slots 0..6 (`$0700..$071B`):
//! the init state of `$80:D7A3` writes X = 8, attr `$34` once, and the redraw
//! (`$80:D801`, only when `$0BCF` bit 7 is set) rewrites Y and tile of each
//! slot from HP `$0BCF` / max HP `$1F9A`. The OAM build never touches slots
//! 0..15 (it starts its cursor at `$E4 = $10`). So P2's bar is that redraw
//! run isolated on P2's block, its seven entries copied into slots past P2's
//! sprites at another X, and slots 0..6 put back.

use crate::coop::{self, OFF_HP, OFF_POS_X, OFF_POS_Y};
use crate::coop_stages;
use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::Game;

/// `$80:9A94 LDA $1F30`, where both paths after the display slice hook
/// ([`coop::HOOK_DISPLAY`], `$80:9A86`) meet: every gameplay frame, after
/// P2's sprites are in OAM. Nothing after it touches OAM or `$E3`.
pub const HOOK_HUD: crate::hooks::HookId = crate::hooks::HookId::Hud;
/// HP bar redraw, state 2 of `$80:D7A3`'s jump table (listing `$80:D842`,
/// drift `-$41`): `$E4/$E5 = $10/$04`, then if `$0BCF` bit 7: clamp HP to
/// `$1F9A`, fill slots from Y `$40` upwards (tile `$80` full, `$82` empty,
/// `$84` cap), clear bit 7. `RTS`. Needs M=1, X=0.
const BAR_REDRAW: crate::hooks::HookId = crate::hooks::HookId::BarRedraw;
/// `$1F11`: HP bar state (0 init, 2 shown, 4 hide, 6 hidden).
const BAR_STATE: u32 = 0x7E1F11;
const BAR_SHOWN: u8 = 2;
/// `$1F0E`: boss object; nonzero = boss bar in slots 16..23 at X `$E8`.
const BOSS_OBJ: u32 = 0x7E1F0E;
/// Slots of X's bar (icon + up to 6 pieces).
const BAR_SLOTS: usize = 7;
/// OAM slots P2's HUD takes past P2's sprites: the bar and the down marker.
pub const HUD_SLOTS: usize = BAR_SLOTS + 1;
/// P2's bar: the right edge, like X's on the left (8 px in); left of the
/// boss bar (X 232) when there is one.
const BAR_X: u8 = 232;
const BAR_X_BOSS: u8 = 216;
/// `$E3`: slots the game's OAM build used.
const OAM_USED: usize = 0xE3;
const OAM_HI: u32 = 0x7E0900;
const OAM_HIDDEN_Y: u8 = 0xE0;

/// P1 revives a downed P2 by standing within this many px (horizontally) of
/// where it fell, on the same screen, for [`REVIVE_FRAMES`] frames in a row.
pub const REVIVE_DIST: u16 = 24;
pub const REVIVE_FRAMES: u16 = 60;

/// Down / revive state, carried in `CoopState` (rollback).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct Revive {
    /// Where P2 went down (world X, Y), if that was on screen. None (a pit,
    /// left behind by the camera): only the timeout brings P2 back.
    pub spot: Option<(u16, u16)>,
    /// Frames in a row P1 has stood at `spot`.
    pub touch: u16,
    /// The next respawn is a revive: half of max HP.
    pub half_hp: bool,
}

pub fn register(game: &mut Game) {
    game.traps.register(game.hook(HOOK_HUD), "coop_hud", TrapReturn::Observe, hud);
}

fn w16(w: &[u8; 0x20000], a: u32) -> u16 {
    u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]])
}

/// `(x, y)` inside the camera window.
fn on_screen((x, y): (u16, u16), w: &[u8; 0x20000]) -> bool {
    let (cx, cy) = (w16(w, ram::CAMERA_X), w16(w, ram::CAMERA_Y));
    x.wrapping_sub(cx) < 256 && y.wrapping_sub(cy) < 224
}

/// P1 is touching the spot: same screen, within [`REVIVE_DIST`] px in X.
pub fn touching(w: &[u8; 0x20000], spot: (u16, u16)) -> bool {
    on_screen(spot, w) && w16(w, ram::X_POS_X).abs_diff(spot.0) <= REVIVE_DIST
}

/// Called by `coop::go_down`: remember the fall spot.
pub fn mark_down(game: &mut Game) {
    let p2 = &game.coop.p2;
    let pos = (u16::from_le_bytes([p2[OFF_POS_X], p2[OFF_POS_X + 1]]), u16::from_le_bytes([p2[OFF_POS_Y], p2[OFF_POS_Y + 1]]));
    let spot = on_screen(pos, game.wram()).then_some(pos);
    game.coop.revive = Revive { spot, touch: 0, half_hp: true };
}

/// P2 is still down this frame (the update slice returns). Counts down the
/// timeout and the touch; a finished touch ends the wait early.
pub fn still_down(game: &mut Game) -> bool {
    if game.coop.down_frames == 0 {
        return false;
    }
    game.coop.down_frames -= 1;
    let near = game.coop.revive.spot.is_some_and(|s| touching(game.wram(), s));
    let r = &mut game.coop.revive;
    r.touch = if near { r.touch + 1 } else { 0 };
    if r.touch >= REVIVE_FRAMES {
        game.coop.down_frames = 0;
    }
    true
}

/// After a (re)spawn: a revive comes back at half of max HP.
pub fn respawn_hp(game: &mut Game) {
    if std::mem::take(&mut game.coop.revive.half_hp) {
        game.coop.p2[OFF_HP] = (game.wram()[ix(ram::MAX_HP)] & 0x7F) / 2;
    }
}

/// X's bar redraw run on P2's HP: returns the 7 OAM entries it wrote. WRAM,
/// direct page and registers are left as they were.
fn bar_entries(game: &mut Game, hp: u8) -> [u8; BAR_SLOTS * 4] {
    let saved = game.regs();
    let o = ix(ram::OAM_BUF);
    let dp: [u8; 0x100] = game.wram()[..0x100].try_into().unwrap();
    let oam: [u8; BAR_SLOTS * 4] = game.wram()[o..o + BAR_SLOTS * 4].try_into().unwrap();
    let mut blk = game.coop.p2;
    blk[OFF_HP] = hp | 0x80; // redraw flag
    coop::swap_block(game.wram_mut(), &mut blk);
    let mut r = saved;
    r.d = 0;
    r.dbr = 0x80;
    r.p = (r.p | 0x20) & !0x10;
    game.set_regs(r);
    game.call_isolated_ret(game.hook(BAR_REDRAW), TrapReturn::Rts);
    game.set_regs(saved);
    let w = game.wram_mut();
    let bar: [u8; BAR_SLOTS * 4] = w[o..o + BAR_SLOTS * 4].try_into().unwrap();
    w[o..o + BAR_SLOTS * 4].copy_from_slice(&oam);
    coop::swap_block(w, &mut blk);
    w[..0x100].copy_from_slice(&dp);
    bar
}

/// [`HOOK_HUD`]: P2's bar (and the down marker) into 8 slots past P2's.
fn hud(game: &mut Game) {
    if !game.coop.enabled || game.wram()[ix(BAR_STATE)] != BAR_SHOWN || !coop_stages::p2_should_exist(game.wram()) {
        return;
    }
    let c = &game.coop;
    let hp = if c.p2_hidden || coop::p2_is_down(&c.p2) { 0 } else { c.p2[OFF_HP] & 0x7F };
    // Whole high-table bytes, like P2's sprites. P2's display slice leaves
    // [`HUD_SLOTS`] free below 128; when the game itself filled OAM (`$E3` =
    // 128, P2 hidden) the bar takes the game's last (lowest priority) slots.
    let used = usize::from(game.wram()[OAM_USED].min(128));
    let end = if c.p2_oam_count > 0 { c.p2_oam_base + c.p2_oam_count } else { used };
    let start = ((end + 3) & !3).min(128 - HUD_SLOTS);
    let base = if c.p2_oam_count > 0 { c.p2_oam_base } else { used.min(start) };
    let bar = bar_entries(game, hp);
    let x = if w16(game.wram(), BOSS_OBJ) != 0 { BAR_X_BOSS } else { BAR_X };
    let pal = game.coop.zero_palette;
    // Down marker: the bar's icon at the fall spot, blinking.
    let marker = match game.coop.revive.spot {
        Some((sx, sy)) if game.coop.down_frames > 0 && game.coop.down_frames & 8 == 0 => {
            let w = game.wram();
            let (mx, my) = (sx.wrapping_sub(w16(w, ram::CAMERA_X)).wrapping_sub(8), sy.wrapping_sub(w16(w, ram::CAMERA_Y)).wrapping_sub(8));
            (mx <= 240 && my <= 208).then_some([mx as u8, my as u8, bar[2], bar[3]])
        }
        _ => None,
    };
    let w = game.wram_mut();
    let oam = ix(ram::OAM_BUF);
    for slot in end..start {
        w[oam + slot * 4 + 1] = OAM_HIDDEN_Y; // alignment padding
    }
    for k in 0..BAR_SLOTS {
        let e = &mut w[oam + (start + k) * 4..][..4];
        e.copy_from_slice(&bar[k * 4..k * 4 + 4]);
        e[0] = x;
        coop::set_oam_palette(e, pal);
    }
    let e = &mut w[oam + (start + BAR_SLOTS) * 4..][..4];
    match marker {
        Some(m) => {
            e.copy_from_slice(&m);
            coop::set_oam_palette(e, pal);
        }
        None => e[1] = OAM_HIDDEN_Y,
    }
    // All 8 large (bit 1), X bit 8 clear.
    w[ix(OAM_HI) + start / 4..][..2].fill(0xAA);
    // Hidden again by `coop::oam_hide` at the next build.
    game.coop.p2_oam_base = base;
    game.coop.p2_oam_count = start + HUD_SLOTS - base;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coop_hud_touching() {
        let mut w: Box<[u8; 0x20000]> = vec![0u8; 0x20000].into_boxed_slice().try_into().unwrap();
        w[ix(ram::CAMERA_X)..][..2].copy_from_slice(&360u16.to_le_bytes());
        w[ix(ram::CAMERA_Y)..][..2].copy_from_slice(&256u16.to_le_bytes());
        w[ix(ram::X_POS_X)..][..2].copy_from_slice(&500u16.to_le_bytes());
        assert!(touching(&w, (524, 367)));
        assert!(touching(&w, (476, 367)));
        assert!(!touching(&w, (525, 367)));
        // Fall spot below the screen (pit): never.
        assert!(!touching(&w, (500, 513)));
    }
}
