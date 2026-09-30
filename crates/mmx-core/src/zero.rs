//! Zero's colours for player 2. See `docs/zero.md`.
//!
//! P2 is drawn with X's tiles (coop display slice) and a sprite palette forced
//! on its OAM slots (`CoopState::zero_palette`). This module fills that palette
//! with X's palette recoloured to Zero's ROM colours, through the game's own
//! CGRAM mirror at `$0300` and upload flag `$A1` (NMI copies all 512 bytes to
//! CGRAM when `$A1 != 0`, then clears it).

use crate::hooks::HookId;
use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::saber::Saber;
use crate::zero_data::{zero_x3_frame_for, Sprite, ZeroSet};
use crate::Game;

/// OAM build entry (`JSR` from the object master); once per gameplay frame,
/// before the coop display slice at `$80:9A86`.
pub const HOOK_OAM_BUILD: HookId = HookId::OamBuild;
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

/// X3 Zero frame for a player block (a saber swing's pose, else from its
/// `+$02` action and `+$17` frame), `None` when the block is invisible
/// (`+$0E = 0`, the i-frame blink). An action X3 has no pose for draws the
/// stand frame.
pub fn x3_frame(obj: &[u8], saber: &Saber) -> Option<usize> {
    (obj[0x0E] != 0).then(|| {
        crate::saber::frame(saber).unwrap_or_else(|| zero_x3_frame_for(obj[0x02], obj[0x17]).unwrap_or(0))
    })
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

/// P2 as X3 Zero, from OAM slot `first` (4-aligned): its slash arc (on top)
/// and its frame into `$0700` and the high table `$0900`, their pages into
/// shadow pages 2 and 0 (when the frame changed), their slots into the
/// PPU's mask. Returns the slot past P2's last one.
pub fn emit_x3(game: &mut Game, set: &ZeroSet, first: u8, pal: u8) -> u8 {
    let p2 = game.coop.p2;
    let attr = p2[0x11] & 0x30 | (pal & 7) << 1;
    let saber = game.coop.saber;
    let mut e = match x3_frame(&p2, &saber) {
        Some(_) => slash_entries(game, set, &p2, &saber, 0, attr),
        None => Vec::new(),
    };
    let n_fx = e.len();
    if let Some(f) = x3_frame(&p2, &saber) {
        if game.coop.zero_frame != f as u16 {
            crate::p2tiles::fill_page(game, 0, &set.page(f));
            game.coop.zero_frame = f as u16;
        }
        e.extend(frame_entries(game, set, &p2, f, attr));
    }
    let e = &e[..e.len().min(128 - usize::from(first))];
    let last = write_entries(game, first, e);
    let mask = |a: usize, b: usize| (a..b).fold(0u128, |m, s| m | 1 << s);
    let (f, w) = (usize::from(first), n_fx.min(e.len()));
    game.add_p2_tile_slots(mask(f, f + w), 2);
    game.add_p2_tile_slots(mask(f + w, usize::from(last)), 0);
    last
}

/// Frame `f` of `obj` (a player block) as OAM entries, `attr` = priority /
/// palette bits.
fn frame_entries(game: &Game, set: &ZeroSet, obj: &[u8], f: usize, attr: u8) -> Vec<([u8; 4], u8)> {
    let w = game.wram();
    let cam = |a: u32| u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]]);
    let (ox, oy) = screen_origin(obj, cam(ram::CAMERA_X), cam(ram::CAMERA_Y));
    let right = obj[0x11] & 0x40 != 0;
    set.frames[f].sprites.iter().filter_map(|s| oam_entry(s, (ox, oy + X3_Y_OFFSET), right, attr)).collect()
}

/// The slash arc's colours on Zero's palette (`$8C:B580`, which every
/// Zero wears): the arc draws with 1-5 (`$8C:A460`: white `73BC`, greens
/// `5394 3B8E 2308 1284`) and 15 (`18C6`); Zero's has white 4 (`639C`),
/// greens 12 (`2388`) and 1 (`1284`, the same) and dark 15 (`1084`). No
/// sprite palette is free with co-op on (docs/zero.md).
pub const SLASH_ON_ZERO: [u8; 16] = [0, 4, 12, 12, 12, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 15];

/// Sprites the slash arc of `saber` draws now (upper bound for slot
/// planning; the name is from the wave it replaced).
pub fn wave_len(set: &ZeroSet, saber: &Saber) -> usize {
    crate::saber::slash_frame(saber).zip(set.slash.as_ref()).and_then(|(f, s)| s.frames.get(f)).map_or(0, |f| f.sprites.len())
}

/// `saber`'s slash arc on `obj` (its wearer) as OAM entries, mirrored by
/// the wearer's facing; its tiles go into shadow page 2 at columns
/// `8 * which` (P2 0, P1 1) when its frame changed.
fn slash_entries(game: &mut Game, set: &ZeroSet, obj: &[u8], saber: &Saber, which: usize, attr: u8) -> Vec<([u8; 4], u8)> {
    let Some((sl, f)) = set.slash.as_ref().zip(crate::saber::slash_frame(saber)).filter(|(s, f)| *f < s.frames.len()) else {
        return Vec::new();
    };
    let col = 8 * which;
    if game.coop.wave_frames[which] != f as u16 {
        let page = sl.page(f, &SLASH_ON_ZERO);
        for (slot, t) in page.chunks_exact(32).enumerate().filter(|(s, _)| s % 16 < 8) {
            crate::p2tiles::fill_page(game, 2 * crate::p2tiles::SHADOW_WORDS + (slot + col) * 16, t);
        }
        game.coop.wave_frames[which] = f as u16;
    }
    let w = game.wram();
    let cam = |a: u32| u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]]);
    let (ox, oy) = screen_origin(obj, cam(ram::CAMERA_X), cam(ram::CAMERA_Y));
    let right = obj[0x11] & 0x40 != 0;
    sl.frames[f]
        .sprites
        .iter()
        .filter_map(|s| oam_entry(&Sprite { tile: s.tile + col as u8, ..*s }, (ox, oy + X3_Y_OFFSET), right, attr))
        .collect()
}

/// Entries into OAM from slot `first`. Returns the slot past the last one.
fn write_entries(game: &mut Game, first: u8, entries: &[([u8; 4], u8)]) -> u8 {
    let first = usize::from(first);
    let slot = first + entries.len();
    let w = game.wram_mut();
    let mut hi = 0u64;
    for (k, (e, h)) in entries.iter().enumerate() {
        w[ix(ram::OAM_BUF) + (first + k) * 4..][..4].copy_from_slice(e);
        hi |= u64::from(*h) << (2 * k);
    }
    // High table: the player owns whole bytes from `first`; keep the game's
    // bits above its last slot in the last partial byte.
    for (k, b) in (first..slot).step_by(4).enumerate() {
        let i = ix(OAM_HI) + b / 4;
        let n = (slot - b).min(4);
        let keep = if n == 4 { 0 } else { 0xFFu8 << (2 * n) };
        w[i] = (hi >> (8 * k)) as u8 & !keep | w[i] & keep;
    }
    slot as u8
}

// ------------------------------------------------------------ P1 as Zero

/// P1 plays as Zero (`Game::set_p1_zero`). In `CoopState`, so rollback
/// carries the swing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct ZeroP1 {
    pub on: bool,
    /// X3 frame in shadow page 1 (`u16::MAX`: none yet).
    pub frame: u16,
    pub saber: Saber,
}

impl Default for ZeroP1 {
    fn default() -> Self {
        Self { on: false, frame: u16::MAX, saber: Saber::default() }
    }
}

/// `$80:D640` (armour parts, `JSR` from the OAM build): not drawn on a Zero
/// P1. `$80:D671` (the player emitter, next): Zero's frame goes through the
/// build's own cursor instead ([`emit_p1`]), so P1 keeps X's place in the
/// sprite order. Both then return for the ROM (Observe + `RTS` by hand);
/// P2's isolated calls of the same routines run as before.
fn skip_p1_parts(game: &mut Game) {
    if game.coop.p1.on && game.isolated_depth == 0 {
        game.emulate_return(false);
    }
}

fn p1_emitter(game: &mut Game) {
    if game.coop.p1.on && game.isolated_depth == 0 {
        emit_p1(game);
        game.emulate_return(false);
    }
}

/// `$80:E518` entry (the input remapper, reads pad 1 `$A7`): P1's saber.
/// X starts a swing; during one the pad is masked as for P2 (Start and
/// Select pass). `$A7` (held) and `$AB` (pressed) are edited in place.
fn p1_pad(game: &mut Game) {
    if !game.coop.p1.on || game.isolated_depth > 0 || game.zero_set.is_none() {
        return;
    }
    let w = game.wram();
    let held = u16::from_le_bytes([w[ix(ram::PAD1_HELD)], w[ix(ram::PAD1_HELD) + 1]]);
    let b = ix(ram::X_BASE);
    let obj: [u8; ram::X_BLOCK_LEN] = w[b..b + ram::X_BLOCK_LEN].try_into().unwrap();
    let keep = held & (crate::BTN_START | crate::BTN_SELECT);
    let pad = crate::saber::update(&mut game.coop.p1.saber, &obj, held) | keep;
    if crate::saber::whoosh(&game.coop.p1.saber) {
        game.coop.sfx[crate::sfx::P1] = 1;
    }
    let w = game.wram_mut();
    w[ix(ram::PAD1_HELD)..][..2].copy_from_slice(&pad.to_le_bytes());
    let pr = ix(ram::PAD1_PRESSED);
    let pressed = u16::from_le_bytes([w[pr], w[pr + 1]]) & pad;
    w[pr..pr + 2].copy_from_slice(&pressed.to_le_bytes());
}

/// P1 as Zero, the way `$80:D671` writes X: entries at the build's cursor
/// `$E4`, 2 high-table bits per slot shifted into `$E6` and flushed to
/// `$0900 + $E5` every 4 slots (the build's tail flushes a partial byte).
/// Stops before slot 127 (the ROM's overflow exit is left to the lists after
/// it). Tiles from shadow page 1, palette/priority from `+$11` (X's slot 1,
/// which [`write_palette`] fills with Zero's colours); nothing while `+$0E =
/// 0` (the i-frame blink).
pub fn emit_p1(game: &mut Game) {
    let Some(set) = game.zero_set.clone() else { return };
    let b = ix(ram::X_BASE);
    let obj: [u8; ram::X_BLOCK_LEN] = game.wram()[b..b + ram::X_BLOCK_LEN].try_into().unwrap();
    let saber = game.coop.p1.saber;
    let attr = obj[0x11] & 0x3E;
    let mut e = match x3_frame(&obj, &saber) {
        Some(_) => slash_entries(game, &set, &obj, &saber, 1, attr),
        None => Vec::new(),
    };
    let n_fx = e.len();
    if let Some(f) = x3_frame(&obj, &saber) {
        if game.coop.p1.frame != f as u16 {
            crate::p2tiles::fill_page(game, crate::p2tiles::SHADOW_WORDS, &set.page(f));
            game.coop.p1.frame = f as u16;
        }
        e.extend(frame_entries(game, &set, &obj, f, attr));
    }
    let w = game.wram_mut();
    let (mut body, mut fx) = (0u128, 0u128);
    for (k, (e, h)) in e.iter().enumerate() {
        let slot = usize::from(w[OAM_CURSOR] & 0x7F);
        if slot >= 127 {
            break;
        }
        w[ix(ram::OAM_BUF) + slot * 4..][..4].copy_from_slice(e);
        *if k < n_fx { &mut fx } else { &mut body } |= 1 << slot;
        w[OAM_HI_ACC] = w[OAM_HI_ACC] >> 2 | h << 6;
        w[OAM_CURSOR] += 1;
        if w[OAM_CURSOR] & 3 == 0 {
            w[ix(OAM_HI) + usize::from(w[OAM_HI_INDEX])] = w[OAM_HI_ACC];
            w[OAM_HI_INDEX] += 1;
        }
    }
    game.add_p2_tile_slots(body, 1);
    game.add_p2_tile_slots(fx, 2);
}

/// The OAM build's direct-page cursor (`$E4`), high-table byte index (`$E5`)
/// and accumulator (`$E6`).
const OAM_CURSOR: usize = 0xE4;
const OAM_HI_INDEX: usize = 0xE5;
const OAM_HI_ACC: usize = 0xE6;

/// X3 Zero stands 8 px lower in his layouts than X (stand frame: feet at
/// `+24` against X's `+16`, same in X1 and X3; X3 gives Zero his own,
/// taller body box). P2 runs X's physics, so lift Zero's frames by 8.
pub const X3_Y_OFFSET: i16 = -8;

/// OAM high table (`$0900`, 2 bits per slot).
const OAM_HI: u32 = 0x7E0900;

pub fn register_zero_traps(game: &mut Game) {
    game.traps.register(game.hook(HOOK_OAM_BUILD), "zero_oam_build", TrapReturn::Observe, oam_build);
    // Observe: the bodies return early themselves when P1 is Zero.
    game.traps.register(game.hook(crate::coop::PLAYER_OAM), "zero_p1_emitter", TrapReturn::Observe, p1_emitter);
    game.traps.register(game.hook(crate::coop::FX_OAM), "zero_p1_skip_parts", TrapReturn::Observe, skip_p1_parts);
    game.traps.register(game.hook(crate::coop::INPUT_REMAP), "zero_p1_pad", TrapReturn::Observe, p1_pad);
    crate::p2tiles::register(game);
}

/// [`HOOK_OAM_BUILD`]: P2's palette, and a fresh P2 slot mask for this frame.
fn oam_build(game: &mut Game) {
    crate::p2tiles::clear_slots(game);
    write_palette(game);
}

/// Trap body: keep P2's sprite palette = `palette_for(X's)` in the mirror
/// (and a Zero P1's: Zero's palette over X's slot 1); flag an upload only
/// when it changed (the game reloads palettes on stage load, fades and
/// weapon swaps, so the check doubles as a repair).
pub fn write_palette(game: &mut Game) {
    if let Some(z) = game.zero_set.as_ref().filter(|_| game.coop.p1.on) {
        let pal = z.palette;
        put_palette(game, X_SPRITE_PALETTE, &pal);
    }
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    let src = sprite_palette_offset(X_SPRITE_PALETTE);
    let w = game.wram_mut();
    let x: [u16; 16] = std::array::from_fn(|i| u16::from_le_bytes([w[src + 2 * i], w[src + 2 * i + 1]]));
    let want = game.zero_set.as_ref().map_or_else(|| palette_for(&x), |z| z.palette);
    put_palette(game, game.coop.zero_palette, &want);
}

fn put_palette(game: &mut Game, pal: u8, want: &[u16; 16]) {
    let dst = sprite_palette_offset(pal);
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
                g.call_isolated_ret(g.hook(crate::coop::PLAYER_OAM), TrapReturn::Rts);
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

    /// P1 as Zero on whatever dump `MMX_ROM` is (booted from power-on, no
    /// state): sprite palette 1 in the PPU's CGRAM is Zero's on every one of
    /// 300 gameplay frames. On USA v1.1 with PAL addresses the OAM-build
    /// hook missed and the palette flipped between X's and Zero's.
    #[test]
    fn zero_p1_palette_stable() {
        let (Some(rom), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let mut g = Game::from_rom(std::fs::read(rom).unwrap()).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        // Start for 4 frames every 200 (the intro-stage script) until the
        // intro stage is running (`$D2/$D3 = 4/4`; ~f1500 NTSC, ~f1600 PAL).
        let playing = |g: &Game| g.wram()[ix(ram::MODE_D2)] == 4 && g.wram()[ix(ram::STAGE_PHASE)] == 4;
        let mut f = 0usize;
        while !playing(&g) {
            f += 1;
            assert!(f < 4000, "intro stage not reached");
            let start = f % 200 < 4 && f >= 200;
            g.step(crate::Pads { p1: if start { crate::BTN_START } else { 0 }, p2: 0 });
        }
        g.set_p1_zero(true).unwrap();
        let want = g.zero_set().unwrap().palette;
        let pal1 = |g: &Game| -> [u16; 16] { std::array::from_fn(|i| g.cgram()[128 + 16 + i]) };
        for _ in 0..2 {
            g.step(crate::Pads::default());
        }
        for f in 0..300 {
            g.step(crate::Pads { p1: if f % 60 < 30 { crate::BTN_RIGHT } else { 0 }, p2: 0 });
            assert_eq!(pal1(&g), want, "frame {f}: CGRAM sprite palette 1 is not Zero's");
        }
    }
}
