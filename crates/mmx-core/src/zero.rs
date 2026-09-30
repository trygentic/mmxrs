//! Zero's colours for player 2. See `docs/zero.md`.
//!
//! P2 is drawn with X's tiles (coop display slice) and a sprite palette forced
//! on its OAM slots (`CoopState::zero_palette`). This module fills that palette
//! with X's palette recoloured to Zero's ROM colours, through the game's own
//! CGRAM mirror at `$0300` and upload flag `$A1` (NMI copies all 512 bytes to
//! CGRAM when `$A1 != 0`, then clears it).

use crate::traps::TrapReturn;
use crate::Game;

/// OAM build entry (`JSR` from the object master); once per gameplay frame,
/// before the coop display slice at `$80:D5FA`.
pub const HOOK_OAM_BUILD: u32 = 0x80D544;
/// CGRAM mirror in WRAM, uploaded by the NMI when [`CGRAM_DIRTY`] is set.
pub const CGRAM_BUF: usize = 0x0300;
pub const CGRAM_DIRTY: usize = 0x00A1;
/// X's sprite palette (block `+$11` = `$62` -> palette 1).
pub const X_SPRITE_PALETTE: u8 = 1;

/// Zero's own palette, `$85:C0C0` (palette id `$7B` in the `$86:8133` table).
/// Read from sprite palette 6 while Zero is on screen in the Highway ending.
/// Same layout as X's: 10..12 skin, 15 black.
pub const ZERO_ROM: [u16; 16] = [
    0x3988, 0x0128, 0x7ACD, 0x5963, 0x7FBD, 0x5294, 0x3DEF, 0x055E, 0x04D5, 0x08CD, 0x42DF, 0x2597, 0x1110,
    0x271E, 0x1258, 0x14A7,
];

/// X's palette (`$85:B700`) slots 4..9: light cyan ramp 4-6 (`7B6F 7A8A 5963`:
/// chest, thighs, helmet top) and blue ramp 7-9 (`7E00 7900 40C4`: helmet,
/// arms, boots). Blue -> Zero's red, light cyan -> Zero's white/greys (his
/// chest and joints); all-red read as an orange X next to the real Zero.
/// Everything else (visor, gem, skin, whites, black) is kept.
const ARMOUR: [(usize, u16); 6] = [
    (4, ZERO_ROM[4]), // cyan highlight -> white
    (5, ZERO_ROM[5]), // light blue -> grey
    (6, ZERO_ROM[6]), // its shade -> dark grey
    (7, ZERO_ROM[7]), // blue -> red
    (8, ZERO_ROM[8]), // -> dark red
    (9, ZERO_ROM[9]), // navy -> darkest red
];

/// X's 16 colours -> P2's. Pure.
pub fn palette_for(x: &[u16; 16]) -> [u16; 16] {
    let mut p = *x;
    for (i, c) in ARMOUR {
        p[i] = c;
    }
    p
}

/// Byte offset of sprite palette `pal` (0-7) in the `$0300` mirror.
pub fn sprite_palette_offset(pal: u8) -> usize {
    CGRAM_BUF + 2 * (128 + 16 * usize::from(pal & 7))
}

pub fn register_zero_traps(game: &mut Game) {
    game.traps.register(HOOK_OAM_BUILD, "zero_oam_build", TrapReturn::Observe, oam_build);
    crate::p2tiles::register(game);
}

/// [`HOOK_OAM_BUILD`]: P2's palette, and a fresh P2 slot mask for this frame.
fn oam_build(game: &mut Game) {
    crate::p2tiles::clear_slots(game);
    write_palette(game);
}

/// Trap body: keep P2's sprite palette = `palette_for(X's)` in the mirror;
/// flag an upload only when it changed (the game reloads palettes on stage
/// load, fades and weapon swaps, so the check doubles as a repair).
pub fn write_palette(game: &mut Game) {
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    let dst = sprite_palette_offset(game.coop.zero_palette);
    let src = sprite_palette_offset(X_SPRITE_PALETTE);
    let w = game.wram_mut();
    let x: [u16; 16] = std::array::from_fn(|i| u16::from_le_bytes([w[src + 2 * i], w[src + 2 * i + 1]]));
    let want = palette_for(&x);
    let mut dirty = false;
    for (i, c) in want.iter().enumerate() {
        let b = c.to_le_bytes();
        if w[dst + 2 * i..dst + 2 * i + 2] != b {
            w[dst + 2 * i..dst + 2 * i + 2].copy_from_slice(&b);
            dirty = true;
        }
    }
    if dirty {
        w[CGRAM_DIRTY] = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// X's stage palette, `$85:B700`.
    const X: [u16; 16] = [
        0x35AF, 0x739C, 0x239D, 0x091E, 0x7B6F, 0x7A8A, 0x5963, 0x7E00, 0x7900, 0x40C4, 0x42DF, 0x2597, 0x1110,
        0x739C, 0x4210, 0x0C63,
    ];

    fn rgb(c: u16) -> (u16, u16, u16) {
        (c & 31, (c >> 5) & 31, c >> 10)
    }

    #[test]
    fn zero_keeps_skin_white_black() {
        let p = palette_for(&X);
        for i in [0, 1, 2, 3, 10, 11, 12, 13, 14, 15] {
            assert_eq!(p[i], X[i], "slot {i}");
        }
    }

    #[test]
    fn zero_armour_is_red_and_grey() {
        let p = palette_for(&X);
        let lum = |c: u16| rgb(c).0 + rgb(c).1 + rgb(c).2;
        for &c in &p[7..=9] {
            let (r, g, b) = rgb(c);
            assert!(r > 2 * g && r > 2 * b, "{c:04X} not red");
        }
        for &c in &p[4..=6] {
            let (r, g, b) = rgb(c);
            assert!(r.abs_diff(g) <= 2 && g.abs_diff(b) <= 2, "{c:04X} not grey");
        }
        // Ramps stay ordered light -> dark.
        assert!(lum(p[4]) > lum(p[5]) && lum(p[5]) > lum(p[6]));
        assert!(lum(p[7]) > lum(p[8]) && lum(p[8]) > lum(p[9]));
    }

    #[test]
    fn zero_palette_offsets() {
        assert_eq!(sprite_palette_offset(0), 0x0400);
        assert_eq!(sprite_palette_offset(1), 0x0420);
        assert_eq!(sprite_palette_offset(3), 0x0460);
        assert_eq!(sprite_palette_offset(7), 0x04E0);
    }
}
