//! Zero's colours for player 2. See `docs/zero.md`.
//!
//! P2 is drawn with X's tiles (coop display slice) or X3 Zero's, coloured by
//! P2's own sprite palette: a ninth one in the vendored PPU
//! (`Game::set_p2_palette`), since the game uses all eight in CGRAM (3 is X's
//! charged shots; docs/coop.md "Palettes"). This module fills it with X's
//! palette recoloured to Zero's ROM colours (or X3 Zero's). A Zero P1 wears
//! CGRAM slot 1 instead, through the game's own mirror at `$0300` and upload
//! flag `$A1` (NMI copies all 512 bytes to CGRAM when `$A1 != 0`, then
//! clears it).

use crate::hooks::HookId;
use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::saber::Saber;
use crate::zero_data::{zero_x3_frame_for, Sprite, ZeroSet};
use crate::Game;
use crate::boss_data::BossSet;

/// Four local players share the SNES's 128 OAM slots with enemies and effects.
/// Keep each custom character within a modest slot budget so the fourth
/// player can still be displayed on busy Highway frames.
pub const FOUR_PLAYER_ART_SLOTS: usize = 16;

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

/// X's own palette (`$85:B700`, sprite palette 1 at the Highway start, no
/// weapon): a second X's source while P1 wears Zero's colours in slot 1.
pub const X_ROM: [u16; 16] = [
    0x35AF, 0x739C, 0x239D, 0x091E, 0x7B6F, 0x7A8A, 0x5963, 0x7E00, 0x7900, 0x40C4, 0x42DF, 0x2597, 0x1110,
    0x739C, 0x4210, 0x0C63,
];

/// A second X (`--p2-character x`): X's blue ramp 7-9 (`7E00 7900 40C4`:
/// helmet, arms, boots) turned green (blue and green swapped), the rest
/// kept. ponytail: one fixed tint; a picker is the upgrade.
const P2_X_TINT: [(usize, u16); 3] = [(7, 0x43E0), (8, 0x23C0), (9, 0x1A04)];

/// X's 16 colours -> a second X's. Pure.
pub fn p2_x_palette(x: &[u16; 16]) -> [u16; 16] {
    let mut p = *x;
    for (i, c) in P2_X_TINT {
        p[i] = c;
    }
    p
}

/// The shadow player now (P2, or 3/4 in their slices) plays as X.
fn shadow_is_x(game: &Game) -> bool {
    game.player_character(game.local_player_index) == Some(crate::Character::X)
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
    let k = game.local_player_index;
    let mut e = wave_entries(game, set, &game.waves[k].clone(), k, attr);
    let n_wave = e.len();
    if x3_frame(&p2, &saber).is_some() {
        e.extend(slash_entries(game, set, &p2, &saber, 0, attr));
    }
    let n_fx = e.len() - n_wave;
    if let Some(f) = x3_frame(&p2, &saber) {
        if game.coop.zero_frame != f as u16 {
            crate::p2tiles::fill_page(game, usize::from(crate::p2tiles::body_page(game)) * crate::p2tiles::SHADOW_WORDS, &set.page(f));
            game.coop.zero_frame = f as u16;
        }
        e.extend(frame_entries(game, set, &p2, f, attr));
    }
    // The charge sparkles (VRAM tiles, in front), in what the co-op slice
    // left below the HUD's slots after the body and the arc.
    let hud_slots = if game.local_players >= 4 { 0 } else { crate::coop_hud::HUD_SLOTS };
    let room = (127 - hud_slots).saturating_sub(usize::from(first) + e.len());
    let frame = game.wram()[ix(ram::FRAME_MAIN)] % SPARKLE_FRAMES;
    let mut sp = sparkle_entries(game, &p2, frame, X3_Y_OFFSET);
    sp.truncate(room);
    let n_sp = sp.len();
    sp.extend(e);
    let budget = if game.local_players >= 4 { FOUR_PLAYER_ART_SLOTS } else { usize::MAX };
    let e = &sp[..sp.len().min(128 - usize::from(first)).min(budget)];
    let last = write_entries(game, first, e);
    let mask = |a: usize, b: usize| (a..b).fold(0u128, |m, s| m | 1 << s);
    let f = usize::from(first) + n_sp.min(e.len());
    let v = n_wave.min(e.len().saturating_sub(n_sp));
    let w = n_fx.min(e.len().saturating_sub(n_sp + v));
    game.add_p2_tile_slots(mask(f, f + v), 3);
    game.add_p2_tile_slots(mask(f + v, f + v + w), 2);
    game.add_p2_tile_slots(mask(f + v + w, usize::from(last)), crate::p2tiles::body_page(game));
    // Wave, arc and body in P2's own palette; the sparkles keep the ROM's.
    add_shadow_palette_slots(game, mask(f, usize::from(last)));
    last
}

// ------------------------------------------------------------ charge
//
// X1's charge (player block, `$81:97F9..`): `+58` charge state (2 = button
// held, 4 = charging, from `+57 = $96`), `+57` counts down from `$B4`,
// `+5B` level (3 = first, 2 = full at `+57 = $50`, 1 = the arm parts' third
// at `+57 = 1`; 0 = none), `+5A` 2-frame timer, `+82` index into the flash
// palette table `$86:BAD2`: every 2 frames the update loads `table[+82]`
// (a flash palette, or 0 = X's own) into sprite palette 1 and does `+82 ^=
// 2`, so the flash is showing while `+82 & 2`. The sparkles are one object
// at `$0C98` (set `$6F`, palette 0; `$70`, palette 2 at full charge; `$71`,
// palette 3 for the third), whose update reads P1's block by absolute
// address. X3 Zero (docs/zero.md "Charge"): the same 2-on, 2-off flash with
// X's flash palette over Zero's whole slot ([`CHARGE_FLASH`]), purple at
// his third level ([`CHARGE_FLASH_3`]).

/// X1's (and X3's) charge flash palette, the whole slot: what X3 writes
/// over Zero's palette 1 while he charges (`x3-zero-stand`, Y held: the
/// same 16 colours as X1's flash for X).
pub const CHARGE_FLASH: [u16; 16] = [
    0x35AF, 0x7FFF, 0x239D, 0x61BE, 0x7BD4, 0x7B6A, 0x7203, 0x7EE0, 0x7A40, 0x79A4, 0x42DF, 0x2597, 0x1110,
    0x7FFF, 0x5AD6, 0x7143,
];
/// X3 Zero's third-level flash (purple, 142 frames into the hold).
pub const CHARGE_FLASH_3: [u16; 16] = [
    0x35AF, 0x7FFF, 0x239D, 0x61BE, 0x7F9C, 0x7ED9, 0x7A33, 0x7EB6, 0x7E36, 0x75D1, 0x42DF, 0x2597, 0x1110,
    0x7FFF, 0x5AD6, 0x6C12,
];

/// The palette a charging Zero wears now: the flash on the frames X1's
/// charge shows its own (`+5B != 0`, `+82 & 2`), else `zero`. Pure.
pub fn charge_palette(obj: &[u8], zero: &[u16; 16]) -> [u16; 16] {
    match obj[0x5B] {
        0 => *zero,
        _ if obj[0x82] & 2 == 0 => *zero,
        1 => CHARGE_FLASH_3,
        _ => CHARGE_FLASH,
    }
}

/// X1's charge sparkle object (one fixed slot).
pub const CHARGE_FX: u32 = 0x7E0C98;
/// Frames in the sparkle sets `$6F`-`$71` (the object steps one a frame).
pub const SPARKLE_FRAMES: u8 = 22;

/// The sparkle set and palette bits for a charging player block, `None`
/// when it shows none (the object's own rule: `+58 = 4`; set by `+5B`).
/// Pure.
pub fn sparkle_set(obj: &[u8]) -> Option<(u8, u8)> {
    (obj[0x58] == 4).then(|| match obj[0x5B] {
        2 => (0x70, 4),
        1 => (0x71, 6),
        _ => (0x6F, 0),
    })
}

/// Frame `f` of X1 sprite set `set` (layout `count`, then `x, y, tile,
/// attr` per sprite) from the ROM, with each sprite's palette bits.
fn x1_layout(game: &Game, set: u8, f: u8) -> Vec<(Sprite, u8)> {
    let long = |a: u32| u32::from_le_bytes([game.rom_read(a), game.rom_read(a + 1), game.rom_read(a + 2), 0]);
    let lp = long(long(crate::zero_data::SET_TABLE + 3 * u32::from(set)) + 3 * u32::from(f));
    (0..u32::from(game.rom_read(lp)))
        .map(|i| {
            let e = lp + 1 + 4 * i;
            let la = game.rom_read(e + 3);
            let s = Sprite {
                x: game.rom_read(e) as i8,
                y: game.rom_read(e + 1) as i8,
                big: la & 0x20 != 0,
                tile: game.rom_read(e + 2),
                hflip: la & 0x40 != 0,
                vflip: la & 0x80 != 0,
            };
            (s, la & 0x0E)
        })
        .collect()
}

/// The charge sparkles of `obj` (a player block) at sparkle frame `f`, as
/// the ROM draws the `$0C98` object for X: centred on `+05/+08` (no `+19`)
/// plus `dy`, mirrored by the facing, tiles from VRAM. Empty when not
/// charging.
pub fn sparkle_entries(game: &Game, obj: &[u8], f: u8, dy: i16) -> Vec<([u8; 4], u8)> {
    let Some((set, pal)) = sparkle_set(obj) else { return Vec::new() };
    let w = game.wram();
    let r16 = |a: usize| u16::from_le_bytes([w[a], w[a + 1]]);
    let o16 = |o: usize| u16::from_le_bytes([obj[o], obj[o + 1]]);
    let ox = o16(0x05).wrapping_sub(r16(ix(ram::CAMERA_X))) as i16;
    let oy = (o16(0x08).wrapping_sub(r16(ix(ram::CAMERA_Y))) as i16).wrapping_add(dy);
    let right = obj[0x11] & 0x40 != 0;
    let attr = obj[0x11] & 0x30 | pal; // the object's `+11` = the player's `& $70 | +10`
    x1_layout(game, set, f % SPARKLE_FRAMES)
        .into_iter()
        .filter_map(|(s, la)| oam_entry(&s, (ox, oy), right, attr | la))
        .collect()
}

/// A Zero P1's sparkles: the ROM's `$0C98` object draws them at X's
/// origin; lift it to Zero's ([`X3_Y_OFFSET`]) through the object's `+19`,
/// which the emitter adds and the sparkle update never writes.
fn lift_p1_sparkles(game: &mut Game) {
    let o = ix(CHARGE_FX);
    let w = game.wram_mut();
    if w[o] != 0 && (0x6F..=0x71).contains(&w[o + 0x16]) {
        w[o + 0x19] = X3_Y_OFFSET as i8 as u8;
    }
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

/// A boss uses its own ROM layout. In the air it takes its jump or fall
/// pose when it has one; moving on the ground cycles its run poses; other X
/// actions retain the idle frame.
pub(crate) fn boss_frame(obj: &[u8], set: &BossSet, tick: u64) -> Option<usize> {
    if obj[0x0E] == 0 || set.frames.is_empty() { return None }
    let last = set.frames.len() - 1;
    let velocity = i16::from_le_bytes([obj[0x1A], obj[0x1B]]);
    let air = match obj[0x02] {
        0x06 | 0x10 => set.jump_frame.or(set.fall_frame),
        0x08 | 0x12 => set.fall_frame.or(set.jump_frame),
        _ => None,
    };
    if let Some(f) = air {
        return Some(f.min(last));
    }
    let airborne = matches!(obj[0x02], 0x06 | 0x08 | 0x10 | 0x12);
    if velocity != 0 && obj[0x02] != 0x0C && !airborne && !set.run_frames.is_empty() {
        return Some(set.run_frames[(tick as usize / 6) % set.run_frames.len()].min(last));
    }
    Some(set.idle_frame.min(last))
}


fn boss_entries(game: &Game, set: &BossSet, obj: &[u8], sprites: &[Sprite], attr: u8) -> Vec<([u8; 4], u8)> {
    let w = game.wram();
    let cam = |a: u32| u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]]);
    let (ox, oy) = screen_origin(obj, cam(ram::CAMERA_X), cam(ram::CAMERA_Y));
    let origin = (ox, oy + set.y_offset);
    let right = obj[0x11] & 0x40 != 0;
    sprites.iter().filter_map(|s| oam_entry(s, origin, right, attr)).collect()
}

/// Draw a selected X1 boss in an isolated local-player display slice.
pub fn emit_boss(game: &mut Game, set: &BossSet, first: u8, pal: u8, page: u8) -> u8 {
    let obj = game.coop.p2;
    let Some(f) = boss_frame(&obj, set, game.frame_count()) else { return first };
    // Always refresh: another local player may have used the page earlier in
    // the frame, and a saved state can restore older page contents.
    crate::p2tiles::fill_page(game, usize::from(page) * crate::p2tiles::SHADOW_WORDS, &set.page(f));
    let attr = obj[0x11] & 0x30 | (pal & 7) << 1;
    let budget = if game.local_players >= 4 { FOUR_PLAYER_ART_SLOTS } else { usize::MAX };
    let entries = boss_entries(game, set, &obj, &set.sprites_within(f, budget), attr);
    let entries = &entries[..entries.len().min(128 - usize::from(first))];
    let last = write_entries(game, first, entries);
    let mask = crate::display::slots(first.into(), last.into());
    game.add_p2_tile_slots(mask, page);
    game.add_local_palette_slots(game.local_player_index, mask, set.palette);
    last
}

/// The slash arc's colours on Zero's palette (`$8C:B580`, which every
/// Zero wears): the arc draws with 1-5 (`$8C:A460`: white `73BC`, greens
/// `5394 3B8E 2308 1284`) and 15 (`18C6`); Zero's has white 4 (`639C`),
/// greens 12 (`2388`) and 1 (`1284`, the same) and dark 15 (`1084`). No
/// CGRAM sprite palette is free with co-op on (docs/zero.md).
pub const SLASH_ON_ZERO: [u8; 16] = [0, 4, 12, 12, 12, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 15];

/// The wave's colours on Zero's palette: it draws with 1 (white) and 6-9
/// (greens `5394 338C 2308 2288`); Zero's has white 4 (`639C`) and greens
/// 12 (`2388`) and 1 (`1284`).
pub const WAVE_ON_ZERO: [u8; 16] = [0, 4, 0, 0, 0, 0, 12, 12, 1, 1, 0, 0, 0, 0, 0, 0];
/// Shadow page 3 columns of each wave (by player: P1, P2): 6 columns x 2
/// rows, around the HUD icon's cells 6, 7, 22, 23 (coop_hud.rs).
const WAVE_COLS: [usize; 2] = [0, 8];

/// Sprites `wave` draws now (slot planning).
pub fn thrown_len(set: &ZeroSet, wave: &crate::saber::Wave) -> usize {
    wave.at().and_then(|(_, _, f, _)| set.wave.frames.get(f)).map_or(0, |f| f.sprites.len())
}

/// `wave` (`which`: the player, 0 P1, 1 P2) as OAM entries at its own place, mirrored
/// by its facing at the throw; its tiles into shadow page 3 every frame.
fn wave_entries(game: &mut Game, set: &ZeroSet, wave: &crate::saber::Wave, which: usize, attr: u8) -> Vec<([u8; 4], u8)> {
    let Some((x, y, f, _)) = wave.at().filter(|w| w.2 < set.wave.frames.len()) else { return Vec::new() };
    let Some(&col) = WAVE_COLS.get(which) else { return Vec::new() };
    let page = set.wave.page(f, &WAVE_ON_ZERO);
    for (slot, t) in page.as_chunks::<32>().0.iter().enumerate().filter(|(s, _)| s % 16 < 6) {
        crate::p2tiles::fill_page(game, 3 * crate::p2tiles::SHADOW_WORDS + (slot + col) * 16, t);
    }
    let w = game.wram();
    let cam = |a: u32| u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]]);
    let origin = (x.wrapping_sub(cam(ram::CAMERA_X)) as i16, y.wrapping_sub(cam(ram::CAMERA_Y)) as i16);
    set.wave.frames[f]
        .sprites
        .iter()
        .filter_map(|s| oam_entry(&Sprite { tile: s.tile + col as u8, ..*s }, origin, wave.right, attr))
        .collect()
}

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

/// P2's X3 Zero entries at the current camera (the margin pass moves it,
/// margin_sprites.rs): `(entry, high bits, shadow page)`, the slash arc
/// (page 2) first. Only parts whose page this frame's [`emit_x3`] already
/// filled, so the pass changes no page and no `CoopState`.
pub(crate) fn margin_entries(game: &mut Game, set: &ZeroSet) -> Vec<([u8; 4], u8, u8)> {
    let (p2, saber) = (game.coop.p2, game.coop.saber);
    let Some(f) = x3_frame(&p2, &saber) else { return Vec::new() };
    let attr = p2[0x11] & 0x30 | (game.coop.zero_palette & 7) << 1;
    let mut e = Vec::new();
    if crate::saber::slash_frame(&saber).is_none_or(|s| game.coop.wave_frames[0] == s as u16) {
        e.extend(slash_entries(game, set, &p2, &saber, 0, attr).into_iter().map(|(e, h)| (e, h, 2)));
    }
    if game.coop.zero_frame == f as u16 {
        e.extend(frame_entries(game, set, &p2, f, attr).into_iter().map(|(e, h)| (e, h, 0)));
    }
    e
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
    if p1_custom_art(game) && game.isolated_depth == 0 {
        game.emulate_return(false);
    }
}

fn p1_emitter(game: &mut Game) {
    if p1_custom_art(game) && game.isolated_depth == 0 {
        if let Some(set) = game.boss_sets.get(&game.player_character(0).unwrap()).cloned() {
            emit_p1_boss(game, &set);
        } else {
            emit_p1(game);
        }
        game.emulate_return(false);
    }
}

fn p1_custom_art(game: &Game) -> bool {
    crate::coop_stages::stage_actor_scene(game.wram())
        && (game.coop.p1.on || game.player_character(0).and_then(|c| game.boss_sets.get(&c)).is_some())
}

fn emit_p1_boss(game: &mut Game, set: &BossSet) {
    let b = ix(ram::X_BASE);
    let obj: [u8; ram::X_BLOCK_LEN] = game.wram()[b..b + ram::X_BLOCK_LEN].try_into().unwrap();
    let Some(f) = boss_frame(&obj, set, game.frame_count()) else { return };
    crate::p2tiles::fill_page(game, crate::p2tiles::SHADOW_WORDS, &set.page(f));
    let entries = boss_entries(game, set, &obj, &set.frames[f].sprites, obj[0x11] & 0x3E);
    write_p1_entries(game, &entries, 1);
}

/// `$80:E518` entry (the input remapper, reads pad 1 `$A7`): P1's saber.
/// X starts a swing; during one the pad is masked as for P2 (Start and
/// Select pass). `$A7` (held) and `$AB` (pressed) are edited in place.
fn p1_pad(game: &mut Game) {
    if !crate::coop_stages::stage_actor_scene(game.wram())
        || !crate::saber::uses_saber(game.player_characters[0]) || game.isolated_depth > 0 {
        return;
    }
    let w = game.wram();
    let held = u16::from_le_bytes([w[ix(ram::PAD1_HELD)], w[ix(ram::PAD1_HELD) + 1]]);
    let b = ix(ram::X_BASE);
    let mut obj: [u8; ram::X_BLOCK_LEN] = w[b..b + ram::X_BLOCK_LEN].try_into().unwrap();
    crate::saber::face_attack_start(&game.coop.p1.saber, &mut obj, held);
    game.wram_mut()[b + 0x11] = obj[0x11];
    let keep = held & (crate::BTN_START | crate::BTN_SELECT);
    let duration = crate::saber::duration(game, 0);
    let armed = crate::saber::armed(game, 0);
    let pad = crate::saber::update_timed(&mut game.coop.p1.saber, &obj, held, duration, armed) | keep;
    crate::saber::wave_for(game, 0);
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
    let mut e = wave_entries(game, &set, &game.waves[0].clone(), 0, attr);
    let n_wave = e.len();
    if x3_frame(&obj, &saber).is_some() {
        e.extend(slash_entries(game, &set, &obj, &saber, 1, attr));
    }
    let n_fx = e.len();
    if let Some(f) = x3_frame(&obj, &saber) {
        if game.coop.p1.frame != f as u16 {
            crate::p2tiles::fill_page(game, crate::p2tiles::SHADOW_WORDS, &set.page(f));
            game.coop.p1.frame = f as u16;
        }
        e.extend(frame_entries(game, &set, &obj, f, attr));
    }
    let w = game.wram_mut();
    let (mut body, mut fx, mut wave) = (0u128, 0u128, 0u128);
    for (k, (e, h)) in e.iter().enumerate() {
        let slot = usize::from(w[OAM_CURSOR] & 0x7F);
        if slot >= 127 {
            break;
        }
        w[ix(ram::OAM_BUF) + slot * 4..][..4].copy_from_slice(e);
        *if k < n_wave {
            &mut wave
        } else if k < n_fx {
            &mut fx
        } else {
            &mut body
        } |= 1 << slot;
        w[OAM_HI_ACC] = w[OAM_HI_ACC] >> 2 | h << 6;
        w[OAM_CURSOR] += 1;
        if w[OAM_CURSOR] & 3 == 0 {
            w[ix(OAM_HI) + usize::from(w[OAM_HI_INDEX])] = w[OAM_HI_ACC];
            w[OAM_HI_INDEX] += 1;
        }
    }
    game.add_p2_tile_slots(body, 1);
    game.add_p2_tile_slots(fx, 2);
    game.add_p2_tile_slots(wave, 3);
}

fn write_p1_entries(game: &mut Game, entries: &[([u8; 4], u8)], page: u8) {
    let w = game.wram_mut();
    let mut body = 0u128;
    for &(entry, high) in entries {
        let slot = usize::from(w[OAM_CURSOR] & 0x7F);
        if slot >= 127 { break; }
        w[ix(ram::OAM_BUF) + slot * 4..][..4].copy_from_slice(&entry);
        body |= 1 << slot;
        w[OAM_HI_ACC] = w[OAM_HI_ACC] >> 2 | high << 6;
        w[OAM_CURSOR] += 1;
        if w[OAM_CURSOR] & 3 == 0 {
            w[ix(OAM_HI) + usize::from(w[OAM_HI_INDEX])] = w[OAM_HI_ACC];
            w[OAM_HI_INDEX] += 1;
        }
    }
    game.add_p2_tile_slots(body, page);
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
    game.traps.register(game.hook(HookId::PaletteLoadAt), "zero_p2_flash", TrapReturn::Observe, p2_flash_load);
    game.traps.register(game.hook(HookId::PaletteLoad), "zero_p2_flash_end", TrapReturn::Observe, p2_flash_end);
    game.traps.register(game.hook(HookId::HadoukenCheck), "zero_no_hadouken", TrapReturn::Observe, no_hadouken);
    crate::p2tiles::register(game);
}

/// P2's update (isolated, block at `$0BA8`) runs X's charge code, whose
/// flash loads its palettes into sprite palette 1: P1's. With the X3 set,
/// P2's flash is drawn from its block instead ([`charge_palette`]), so the
/// loads are skipped (`RTL` by hand): the flash (`$82:8000`, `X = $10`) and
/// X's own palette back (`$82:8011`, `Y = $100 + weapon`).
fn in_p2_slice(game: &Game) -> bool {
    let own_flash = game.zero_set.is_some() || shadow_is_x(game); // X3 Zero's, or a second X's
    game.coop.enabled && game.isolated_depth > 0 && own_flash && game.regs().d == ram::X_BASE as u16
}

/// The Hadouken motion check (`$81:A0EA`, `JSR`; Z set = go): a Zero with
/// the X3 set has the wave instead (X3 has no Hadouken pose: it would show
/// his stand frame with X's fireball), so it fails for his block (`RTS`
/// by hand, A = 1, Z clear, as its first refusal does). A Zero P1 is the
/// live block outside isolated calls; P2's runs isolated with `D = $0BA8`.
fn no_hadouken(game: &mut Game) {
    let p1 = game.coop.p1.on && game.isolated_depth == 0;
    let p2 = game.current_shadow_is_zero() && in_p2_slice(game);
    if game.zero_set.is_some() && (p1 || p2) {
        let mut r = game.regs();
        r.a = r.a & 0xFF00 | 1;
        r.p &= !0x02;
        game.set_regs(r);
        game.emulate_return(false);
    }
}

fn p2_flash_load(game: &mut Game) {
    if in_p2_slice(game) && game.regs().x == 0x10 {
        game.emulate_return(true);
    }
}

fn p2_flash_end(game: &mut Game) {
    if in_p2_slice(game) && (0x100..0x120).contains(&game.regs().y) {
        game.emulate_return(true);
    }
}

/// [`HOOK_OAM_BUILD`]: P2's palette, and a fresh P2 slot mask for this frame.
fn oam_build(game: &mut Game) {
    crate::p2tiles::clear_slots(game);
    if !crate::coop_stages::stage_actor_scene(game.wram()) {
        crate::coop::clear_inactive_scene_ownership(game);
        return;
    }
    write_palette(game);
    if game.coop.p1.on && game.zero_set.is_some() && !game.boss_sets.contains_key(&crate::Character::Zero) {
        lift_p1_sparkles(game);
    }
}

/// Trap body: P2's palette (the PPU's ninth) = X3 Zero's or `palette_for(X's)`,
/// with the charge flash; and a Zero P1's: Zero's palette over X's slot 1 in
/// the mirror, flagging an upload only when it changed (the game reloads
/// palettes on stage load, fades and weapon swaps, so the check doubles as a
/// repair).
pub fn write_palette(game: &mut Game) {
    if !crate::coop_stages::stage_actor_scene(game.wram()) { return; }
    let mut p1_recoloured = false;
    if let Some(set) = game.player_character(0).and_then(|c| game.boss_sets.get(&c)).cloned() {
        put_palette(game, X_SPRITE_PALETTE, &set.palette);
        p1_recoloured = true;
    }
    if let Some(z) = game.zero_set.as_ref().filter(|_| game.coop.p1.on && game.player_character(0) == Some(crate::Character::Zero) && !game.boss_sets.contains_key(&crate::Character::Zero)) {
        let b = ix(ram::X_BASE);
        let pal = charge_palette(&game.wram()[b..b + ram::X_BLOCK_LEN], &z.palette);
        put_palette(game, X_SPRITE_PALETTE, &pal);
        p1_recoloured = true;
    }
    lend_portrait_palette(game, p1_recoloured);
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    let want = shadow_palette(game);
    game.set_p2_palette(want);
}

/// The colours of the shadow player now in `game.coop` (P2, or 3/4 inside
/// their slices): its boss's, X3 Zero's or `palette_for(X's)`, with the
/// charge flash.
fn shadow_palette(game: &Game) -> [u16; 16] {
    let x = read_palette(game, X_SPRITE_PALETTE);
    // A second X: its own charge flash over the green tint (its loads into
    // slot 1 are skipped, `in_p2_slice`); X's colours from slot 1 unless P1
    // wears another character's there (`coop.p1` is P1's only outside the
    // P3/P4 slices, so ask the roster).
    let p1_not_x = game.coop.p1.on || game.player_character(0).is_some_and(|c| c != crate::Character::X);
    let second_x = || charge_palette(&game.coop.p2, &p2_x_palette(if p1_not_x { &X_ROM } else { &x }));
    game.current_boss_set().map(|b| b.palette).unwrap_or_else(|| {
        if shadow_is_x(game) {
            return second_x();
        }
        game.zero_set.as_ref().filter(|_| game.current_shadow_is_zero()).map_or_else(|| palette_for(&x), |z| charge_palette(&game.coop.p2, &z.palette))
    })
}

/// Draw `mask`'s slots in the current shadow player's own palette: P2's is
/// the PPU's ninth ([`write_palette`] fills it), players 3 and 4 have their
/// own. Their slices used to flag P2's, so a Zero P2 and X P3/P4 wore one
/// player's colours between them.
pub fn add_shadow_palette_slots(game: &mut Game, mask: u128) {
    if game.local_player_index >= 2 {
        let colours = shadow_palette(game);
        game.add_local_palette_slots(game.local_player_index, mask, colours);
    } else {
        game.add_p2_palette_slots(mask);
    }
}

/// X's text-box portrait: enemy-table object id `$3C` with graphics `$6F`
/// (Zero's own portrait is id `$3C` with graphics `$71`, in palette 6). The
/// ROM draws it in sprite palette 1, X's, so a P1 wearing other colours in
/// slot 1 paints X's face in them (green Zero eyes over X's helmet in the
/// Highway ending).
const PORTRAIT_ID: u8 = 0x3C;
const X_PORTRAIT_GFX: u8 = 0x6F;
/// Where X's portrait goes instead: palette 3, X's charged shots (id `$20`,
/// `$85:B860`), unused while the game waits on a text box.
const PORTRAIT_PALETTE: u8 = 3;
pub const CHARGED_SHOT_ROM: [u16; 16] = [
    0x35D0, 0x7FFF, 0x5BED, 0x3780, 0x3720, 0x2EA0, 0x7FD8, 0x7FA6, 0x7B1F, 0x7F20, 0x38F9, 0x6E9F, 0x555F,
    0x7E80, 0x7D80, 0x14A5,
];

/// While P1 recolours slot 1, move X's portrait to [`PORTRAIT_PALETTE`]
/// holding X's own colours; once no portrait is up, put the charged-shot
/// colours back if (and only if) palette 3 still holds X's.
fn lend_portrait_palette(game: &mut Game, p1_recoloured: bool) {
    let mut shown = false;
    if p1_recoloured {
        let base = ix(ram::ENEMIES_BASE);
        let w = game.wram_mut();
        for i in 0..ram::ENEMY_COUNT {
            let o = base + i * ram::ENEMY_STRIDE;
            if w[o + ram::ENEMY_ACTIVE] != 0 && w[o + ram::ENEMY_ID] == PORTRAIT_ID && w[o + 0x10] == X_PORTRAIT_GFX {
                w[o + 0x11] = w[o + 0x11] & !0x0E | PORTRAIT_PALETTE << 1;
                shown = true;
            }
        }
    }
    if shown {
        put_palette(game, PORTRAIT_PALETTE, &X_ROM);
    } else if read_palette(game, PORTRAIT_PALETTE) == X_ROM {
        put_palette(game, PORTRAIT_PALETTE, &CHARGED_SHOT_ROM);
    }
}

fn read_palette(game: &Game, pal: u8) -> [u16; 16] {
    let src = sprite_palette_offset(pal);
    let w = game.wram();
    std::array::from_fn(|i| u16::from_le_bytes([w[src + 2 * i], w[src + 2 * i + 1]]))
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

    /// Bosses leap and fall in their own poses; a jump with sideways speed
    /// used to cycle the run, and a straight jump stood in the idle pose.
    #[test]
    fn boss_pose_follows_x_action() {
        let frame = crate::zero_data::Frame { name: "t", sprites: Vec::new(), tiles_used: Vec::new() };
        let mut set = BossSet {
            tiles: Vec::new(), palette: [0; 16], frames: vec![frame; 20],
            idle_frame: 4, run_frames: vec![12, 13], jump_frame: Some(17), fall_frame: Some(18),
            y_offset: 0, coverage: Vec::new(),
        };
        let obj = |action: u8, velocity: i16| {
            let mut o = [0u8; 0x40];
            o[0x0E] = 1;
            o[0x02] = action;
            o[0x1A..0x1C].copy_from_slice(&velocity.to_le_bytes());
            o
        };
        assert_eq!(boss_frame(&obj(0x00, 0), &set, 0), Some(4));
        assert_eq!(boss_frame(&obj(0x04, 0x100), &set, 0), Some(12));
        assert_eq!(boss_frame(&obj(0x04, 0x100), &set, 6), Some(13));
        for velocity in [0, 0x100] {
            assert_eq!(boss_frame(&obj(0x06, velocity), &set, 0), Some(17), "jump");
            assert_eq!(boss_frame(&obj(0x10, velocity), &set, 0), Some(17), "wall jump");
            assert_eq!(boss_frame(&obj(0x08, velocity), &set, 0), Some(18), "fall");
            assert_eq!(boss_frame(&obj(0x12, velocity), &set, 0), Some(18), "wall slide");
        }
        // Without air poses: idle in the air, never the run cycle.
        set.jump_frame = None;
        set.fall_frame = None;
        assert_eq!(boss_frame(&obj(0x06, 0x100), &set, 0), Some(4));
        assert_eq!(boss_frame(&obj(0x08, 0x100), &set, 0), Some(4));
        let mut gone = obj(0x00, 0);
        gone[0x0E] = 0;
        assert_eq!(boss_frame(&gone, &set, 0), None);
    }

    /// `highway-start` with the Hadouken, full HP 32: down, down-forward,
    /// forward + Y. X does the Hadouken (action `$42`, its shot id 4); a
    /// Zero with the X3 set does not (his move is the wave).
    #[test]
    fn zero_has_no_hadouken() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            return eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/highway-start.state");
        let Ok(state) = std::fs::read(st) else { return eprintln!("{st} missing; skipping") };
        let run = |zero: bool| {
            let mut g = Game::from_rom(std::fs::read(&x1).unwrap()).unwrap();
            g.set_x3_rom(&std::fs::read(&x3).unwrap()).unwrap();
            g.load_state(&state).unwrap();
            g.set_p1_zero(zero).unwrap();
            let mut acts = Vec::new();
            for f in 1..=60 {
                let w = g.wram_mut();
                w[ix(ram::HADOUKEN)] = ram::HADOUKEN_BIT;
                w[ix(ram::MAX_HP)] = 0x20;
                w[ix(ram::X_HP)] = 0x20;
                let p1 = match f {
                    20..=22 => crate::BTN_DOWN,
                    23..=25 => crate::BTN_DOWN | crate::BTN_RIGHT,
                    26..=27 => crate::BTN_RIGHT,
                    28..=29 => crate::BTN_RIGHT | crate::BTN_Y,
                    _ => 0,
                };
                g.step(crate::Pads { p1, p2: 0 });
                acts.push(g.wram()[ix(ram::X_ACTION)]);
            }
            acts
        };
        assert!(run(false).contains(&0x42), "X's Hadouken");
        let z = run(true);
        assert!(!z.contains(&0x42), "{z:02X?}");
    }

    /// A second X: only the blue ramp 7-9 changes, to green; `X_ROM` is X's.
    #[test]
    fn p2_x_tints_the_blue_ramp() {
        assert_eq!(X_ROM, X);
        let p = p2_x_palette(&X);
        for i in 0..16 {
            if (7..=9).contains(&i) {
                let (r, g, b) = rgb(p[i]);
                assert!(g > b && g > r, "slot {i}: {:04X}", p[i]);
            } else {
                assert_eq!(p[i], X[i], "slot {i}");
            }
        }
    }

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

    /// Calibration: [`sparkle_entries`] for X's own block and the `$0C98`
    /// object's frame writes what the ROM draws for X's charge (sets `$6F`
    /// and, from 100 frames in, `$70`), in both facings.
    #[test]
    fn zero_sparkles_match_rom() {
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
        let (mut checked, mut sets) = (0, Vec::new());
        for f in 1..=150 {
            let pad = crate::BTN_Y | if f > 125 { crate::BTN_LEFT } else { 0 };
            g.step(crate::Pads { p1: pad, p2: 0 });
            let b = ix(ram::X_BASE);
            let obj: Vec<u8> = g.wram()[b..b + ram::X_BLOCK_LEN].to_vec();
            let fx = ix(CHARGE_FX);
            let Some((set, _)) = sparkle_set(&obj) else {
                assert!(f < 45, "frame {f}: no sparkles while charging");
                continue;
            };
            if set != g.wram()[fx + 0x16] {
                // the level-up frame: the object switches set a frame late
                sets.push(0);
                continue;
            }
            let want = sparkle_entries(&g, &obj, g.wram()[fx + 0x17] & 0x7F, 0);
            assert!(!want.is_empty() || g.wram()[fx + 0x17] & 0x7F == 0, "frame {f}");
            let oam = &g.wram()[ix(ram::OAM_BUF)..][..512];
            let found = (0..128 - want.len()).any(|s| want.iter().enumerate().all(|(k, (e, _))| oam[(s + k) * 4..][..4] == *e));
            assert!(found, "frame {f}: {want:02X?} not in OAM");
            checked += want.len();
            sets.push(g.wram()[fx + 0x16]);
        }
        assert!(checked > 200, "{checked}");
        assert!(sets.contains(&0x6F) && sets.contains(&0x70), "{sets:02X?}");
        assert!(sets.iter().filter(|&&s| s == 0).count() <= 1, "{sets:02X?}");
    }

    #[test]
    fn zero_charge_palette() {
        let mut b = [0u8; 0x90];
        let z = [1u16; 16];
        assert_eq!(charge_palette(&b, &z), z);
        b[0x5B] = 3;
        assert_eq!(charge_palette(&b, &z), z); // +82 = 0: X's own showing
        b[0x82] = 2;
        assert_eq!(charge_palette(&b, &z), CHARGE_FLASH);
        b[0x5B] = 2;
        assert_eq!(charge_palette(&b, &z), CHARGE_FLASH);
        b[0x5B] = 1;
        assert_eq!(charge_palette(&b, &z), CHARGE_FLASH_3);
        b[0x82] = 4;
        assert_eq!(charge_palette(&b, &z), z);
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

    /// X's portrait in the Highway-ending text box ("I guess I'm not
    /// powerful enough") with P1 as Zero: drawn in X's own colours from
    /// palette 3 while Zero keeps slot 1, and palette 3 back to the charged
    /// shots' once the box closes. X1 Zero (boss set) and X3 Zero both
    /// recolour slot 1. State `states/highway-x-portrait.state`: the 100%
    /// movie to frame 6300 (Vile), then B every 30 frames for 2400.
    #[test]
    fn zero_p1_x_portrait_keeps_x_colours() {
        let Some(rom) = std::env::var_os("MMX_ROM") else {
            return eprintln!("MMX_ROM not set; skipping");
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/highway-x-portrait.state");
        let Ok(state) = std::fs::read(st) else { return eprintln!("{st} missing; skipping") };
        let rom = std::fs::read(rom).unwrap();
        let x3 = std::env::var_os("MMX_X3_ROM").map(|p| std::fs::read(p).unwrap());
        let setups: [(&str, bool); 2] = [("X1 Zero", false), ("X3 Zero", true)];
        for (name, use_x3) in setups {
            let mut g = Game::from_rom(rom.clone()).unwrap();
            let zero = if use_x3 {
                let Some(x3) = &x3 else { eprintln!("MMX_X3_ROM not set; {name} skipped"); continue };
                g.set_x3_rom(x3).unwrap();
                g.zero_set().unwrap().palette
            } else {
                let set = crate::boss_data::extract_x1(&rom, crate::Character::Zero).unwrap();
                let pal = set.palette;
                g.set_boss_set(crate::Character::Zero, set);
                g.set_player_character(0, crate::Character::Zero);
                pal
            };
            g.load_state(&state).unwrap();
            g.set_p1_zero(true).unwrap();
            let pal = |g: &Game, n: usize| -> [u16; 16] { std::array::from_fn(|i| g.cgram()[128 + 16 * n + i]) };
            let mut seen = false;
            for f in 0..700 {
                g.step(crate::Pads { p1: if f % 30 < 2 { crate::BTN_B } else { 0 }, p2: 0 });
                let w = g.wram();
                let portrait = (0..ram::ENEMY_COUNT).map(|i| ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE).find(|&o| {
                    w[o] != 0 && w[o + ram::ENEMY_ID] == PORTRAIT_ID && w[o + 0x10] == X_PORTRAIT_GFX
                });
                if let Some(o) = portrait.filter(|_| f > 2) {
                    seen = true;
                    assert_eq!(w[o + 0x11] >> 1 & 7, PORTRAIT_PALETTE, "{name} frame {f}: portrait palette");
                    assert_eq!(pal(&g, 3), X_ROM, "{name} frame {f}: portrait colours");
                }
                if f > 2 {
                    assert_eq!(pal(&g, 1), zero, "{name} frame {f}: slot 1 is not Zero's");
                }
            }
            assert!(seen, "{name}: X's portrait never shown");
            assert_eq!(pal(&g, 3), CHARGED_SHOT_ROM, "{name}: palette 3 not restored");
        }
    }

    /// Four local players, P2 as X3 Zero, P3 and P4 as X, all moving: P2's
    /// body page (0) holds Zero's frame after every frame. P3/P4's X tile
    /// streams used to land on the same page, so Zero drew with X's tiles
    /// (and P3/P4 with Zero's).
    #[test]
    fn zero_p2_page_survives_four_players() {
        let (Some(rom), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            return eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
        };
        let mut g = Game::from_rom(std::fs::read(rom).unwrap()).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        g.coop.enabled = true;
        g.set_local_players(4);
        crate::coop::register_coop_traps(&mut g).unwrap();
        g.set_player_character(1, crate::Character::Zero);
        g.set_player_character(2, crate::Character::X);
        g.set_player_character(3, crate::Character::X);
        let playing = |g: &Game| g.wram()[ix(ram::MODE_D2)] == 4 && g.wram()[ix(ram::STAGE_PHASE)] == 4;
        let mut f = 0usize;
        while !playing(&g) {
            f += 1;
            assert!(f < 4000, "intro stage not reached");
            let start = f % 200 < 4 && f >= 200;
            g.step(crate::Pads { p1: if start { crate::BTN_START } else { 0 }, p2: 0 });
        }
        let z = g.zero_set().unwrap().clone();
        let mut checked = 0;
        for f in 0..300 {
            let walk = |phase: usize| if (f + phase) % 80 < 40 { crate::BTN_RIGHT } else { crate::BTN_LEFT };
            g.set_extra_pads([walk(20) | if f % 50 < 10 { crate::BTN_B } else { 0 }, walk(40)]);
            g.step(crate::Pads { p1: 0, p2: walk(0) });
            let zf = g.coop.zero_frame;
            if f < 30 || usize::from(zf) >= z.frames.len() {
                continue;
            }
            let want: Vec<u16> = z.page(usize::from(zf)).chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let page = &g.p2_tile_page_mut()[..want.len()];
            assert_eq!(page, &want[..], "frame {f}: P2's page is not Zero's frame {zf}");
            checked += 1;
        }
        assert!(checked > 200, "Zero's frame checked on {checked} frames");
    }
}
