//! Zero's colours for player 2. See `docs/zero.md`.
//!
//! P2 is drawn with X's tiles (coop display slice) and a sprite palette forced
//! on its OAM slots (`CoopState::zero_palette`). This module fills that palette
//! with X's palette recoloured to Zero's ROM colours, through the game's own
//! CGRAM mirror at `$0300` and upload flag `$A1` (NMI copies all 512 bytes to
//! CGRAM when `$A1 != 0`, then clears it).

use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::zero_data::{zero_x3_frame_for, Sprite, ZeroSet};
use crate::Game;

/// OAM build entry (`JSR` from the object master); once per gameplay frame,
/// before the coop display slice at `$80:9A86`.
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

// ------------------------------------------------------------ X3 Zero emitter

/// X3 Zero frame for P2's block (`+$02` action, `+$17` frame), `None` when
/// the block is invisible (`+$0E = 0`, the i-frame blink). An action X3 has
/// no pose for draws the stand frame.
pub fn x3_frame(_set: &ZeroSet, obj: &[u8]) -> Option<usize> {
    (obj[0x0E] != 0).then(|| zero_x3_frame_for(obj[0x02], obj[0x17]).unwrap_or(0))
}

/// Screen origin the ROM emitter `$80:D67C` uses for an object:
/// `(+05 - camera X, +08 + (i8)+19 - camera Y)`. Pure.
pub fn screen_origin(obj: &[u8], cam_x: u16, cam_y: u16) -> (i16, i16) {
    let w = |o: usize| u16::from_le_bytes([obj[o], obj[o + 1]]);
    let x = w(0x05).wrapping_sub(cam_x) as i16;
    let y = w(0x08).wrapping_add(obj[0x19] as i8 as u16).wrapping_sub(cam_y) as i16;
    (x, y)
}

/// One layout sprite -> OAM entry and its 2 high-table bits (x bit 8, size),
/// as `$80:D6EA`/`$80:D73F` do it: facing right (`+$11` bit 6) mirrors
/// `x' = -x - w` and toggles hflip; culled (`None`) when `x + 16 >= $10F` or
/// `y + 15 >= $EF` (unsigned). `attr` = priority / palette / name bits. Pure.
pub fn oam_entry(s: &Sprite, (ox, oy): (i16, i16), right: bool, attr: u8) -> Option<([u8; 4], u8)> {
    let w = if s.big { 16 } else { 8 };
    let dx = if right { -i16::from(s.x) - w } else { i16::from(s.x) };
    let x = ox.wrapping_add(dx);
    let y = oy.wrapping_add(i16::from(s.y));
    if x.wrapping_add(16) as u16 >= 0x10F || y.wrapping_add(15) as u16 >= 0xEF {
        return None;
    }
    let a = attr | u8::from(s.hflip) << 6 | u8::from(s.vflip) << 7;
    let a = if right { a ^ 0x40 } else { a };
    Some(([x as u8, y as u8, s.tile, a], (x >> 8) as u8 & 1 | u8::from(s.big) << 1))
}

/// P2 as X3 Zero, from OAM slot `first` (4-aligned): its frame's page into
/// the PPU's P2 page (when the frame changed), its sprites into `$0700` and
/// the high table `$0900`. Returns the slot past P2's last one.
pub fn emit_x3(game: &mut Game, set: &ZeroSet, first: u8, pal: u8) -> u8 {
    let p2 = game.coop.p2;
    let Some(f) = x3_frame(set, &p2) else { return first };
    if game.coop.zero_frame != f as u16 {
        crate::p2tiles::fill_page(game, 0, &set.page(f));
        game.coop.zero_frame = f as u16;
    }
    let w = game.wram();
    let cam = |a: u32| u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]]);
    let (ox, oy) = screen_origin(&p2, cam(ram::CAMERA_X), cam(ram::CAMERA_Y));
    let origin = (ox, oy + X3_Y_OFFSET);
    let attr = p2[0x11] & 0x30 | (pal & 7) << 1;
    let right = p2[0x11] & 0x40 != 0;
    let mut slot = usize::from(first);
    let mut hi = 0u32;
    let w = game.wram_mut();
    for s in &set.frames[f].sprites {
        let Some((e, h)) = oam_entry(s, origin, right, attr) else { continue };
        if slot >= 128 {
            break;
        }
        w[ix(ram::OAM_BUF) + slot * 4..][..4].copy_from_slice(&e);
        hi |= u32::from(h) << (2 * (slot - usize::from(first)));
        slot += 1;
    }
    // High table: P2 owns whole bytes from `first`; keep the game's bits
    // above P2's last slot in the last partial byte.
    for (k, b) in (usize::from(first)..slot).step_by(4).enumerate() {
        let i = ix(OAM_HI) + b / 4;
        let n = (slot - b).min(4);
        let keep = if n == 4 { 0 } else { 0xFFu8 << (2 * n) };
        w[i] = (hi >> (8 * k)) as u8 & !keep | w[i] & keep;
    }
    slot as u8
}

/// X3 Zero stands 8 px lower in his layouts than X (stand frame: feet at
/// `+24` against X's `+16`, same in X1 and X3; X3 gives Zero his own,
/// taller body box). P2 runs X's physics, so lift Zero's frames by 8.
pub const X3_Y_OFFSET: i16 = -8;

/// OAM high table (`$0900`, 2 bits per slot).
const OAM_HI: u32 = 0x7E0900;

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
    let want = game.zero_set.as_ref().map_or_else(|| palette_for(&x), |z| z.palette);
    let w = game.wram_mut();
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
    fn zero_oam_entry_mirror_and_cull() {
        let s = Sprite { x: -22, y: 16, big: false, tile: 0x0B, hflip: false, vflip: false };
        // Stand state in X3: tile $0B at screen (96 - 22, 167 + 16) facing left.
        assert_eq!(oam_entry(&s, (96, 167), false, 0x20), Some(([74, 183, 0x0B, 0x20], 0)));
        // Facing right: x' = -x - 8, hflip on.
        assert_eq!(oam_entry(&s, (96, 167), true, 0x20), Some(([110, 183, 0x0B, 0x60], 0)));
        let big = Sprite { big: true, x: 0, y: 0, ..s };
        assert_eq!(oam_entry(&big, (-8, 10), false, 0), Some(([0xF8, 10, 0x0B, 0], 3)));
        assert_eq!(oam_entry(&big, (-17, 10), false, 0), None);
        assert_eq!(oam_entry(&big, (255, 10), false, 0), None);
        assert_eq!(oam_entry(&big, (254, 10), false, 0).map(|e| e.1), Some(2));
        assert_eq!(oam_entry(&big, (0, 224), false, 0), None);
        assert_eq!(oam_entry(&big, (0, -15), false, 0).map(|e| e.0[1]), Some(0xF1));
    }

    /// Calibration: the Rust emitter on X's own (X1-format) layouts writes
    /// what the ROM emitter `$80:D671` writes, in both facings.
    #[test]
    fn zero_emitter_matches_rom() {
        let Some(rom) = std::env::var_os("MMX_ROM") else {
            eprintln!("MMX_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/highway-start.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let mut g = Game::from_rom(std::fs::read(rom).unwrap()).unwrap();
        g.load_state(&state).unwrap();
        let mut checked = 0;
        for (n, pad) in [(1, 0), (20, 0x0100), (20, 0x0200), (10, 0x8200)] {
            for _ in 0..n {
                g.step(crate::Pads { p1: pad, p2: 0 });
            }
            for facing in [0u8, 0x40] {
                let b = ix(ram::X_BASE);
                g.wram_mut()[b + 0x11] = g.wram()[b + 0x11] & !0x40 | facing;
                let saved = g.regs();
                let mut r = saved;
                r.d = 0;
                r.dbr = 0x8D;
                r.p = (r.p | 0x20) & !0x10;
                g.set_regs(r);
                let w = g.wram_mut();
                (w[0xE4], w[0xE5], w[0xE6]) = (0x40, 0x10, 0);
                g.call_isolated_ret(crate::coop::PLAYER_OAM, TrapReturn::Rts);
                g.set_regs(saved);
                let w = g.wram();
                let obj: Vec<u8> = w[b..b + 0x90].to_vec();
                let long = |a: u32| u32::from_le_bytes([g.rom_read(a), g.rom_read(a + 1), g.rom_read(a + 2), 0]);
                let lp = long(long(0x8D8000 + 3 * u32::from(obj[0x16])) + 3 * u32::from(obj[0x17] & 0x7F));
                let origin = screen_origin(&obj, u16::from_le_bytes([w[0x1E4D], w[0x1E4E]]), u16::from_le_bytes([w[0x1E50], w[0x1E51]]));
                let mut slot = 0x40;
                for i in 0..u32::from(g.rom_read(lp)) {
                    let e = lp + 1 + 4 * i;
                    let (la, t) = (g.rom_read(e + 3), g.rom_read(e + 2));
                    let s = Sprite {
                        x: g.rom_read(e) as i8,
                        y: g.rom_read(e + 1) as i8,
                        big: la & 0x20 != 0,
                        tile: t.wrapping_add(obj[0x18]),
                        hflip: la & 0x40 != 0,
                        vflip: la & 0x80 != 0,
                    };
                    let attr = obj[0x11] & 0x3F | la & 0x0E;
                    let Some((want, hi)) = oam_entry(&s, origin, obj[0x11] & 0x40 != 0, attr) else { continue };
                    assert_eq!(w[0x700 + slot * 4..][..4], want, "slot {slot:02X} frame {}", obj[0x17]);
                    // (the ROM stores a high-table byte per 4 slots; the build flushes the rest)
                    if slot / 4 < usize::from(w[0xE5]) {
                        assert_eq!(w[0x900 + slot / 4] >> (2 * (slot % 4)) & 3, hi, "slot {slot:02X} high bits");
                    }
                    slot += 1;
                    checked += 1;
                }
                assert_eq!(usize::from(w[0xE4]), slot);
            }
        }
        assert!(checked > 20);
    }

    #[test]
    fn zero_palette_offsets() {
        assert_eq!(sprite_palette_offset(0), 0x0400);
        assert_eq!(sprite_palette_offset(1), 0x0420);
        assert_eq!(sprite_palette_offset(3), 0x0460);
        assert_eq!(sprite_palette_offset(7), 0x04E0);
    }
}
