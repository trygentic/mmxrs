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
pub(crate) fn four_hud_shown(game: &Game) -> bool {
    game.wram()[ix(BAR_STATE)] == BAR_SHOWN && coop_stages::p2_should_exist(game.wram())
}
/// Slots of X's bar (icon + up to 6 pieces).
const BAR_SLOTS: usize = 7;
/// OAM slots P2's HUD takes past P2's sprites: the bar and the down marker.
pub const HUD_SLOTS: usize = BAR_SLOTS + 1;
/// P2's bar: under X's, same X (the weapon bar is the next column, X 24;
/// the boss bar is on the right edge). X's bar grows up from its icon (Y
/// `$50`); P2's is its vertical mirror, icon 8 px under X's (Y `$68`) and
/// the pieces growing down: `y' = BAR_MIRROR - y`, pieces vflipped. 32 HP
/// (cap at Y 0 for X) ends at Y 200.
const BAR_X: u8 = 8;
const BAR_MIRROR: u8 = 0x50 + 0x68;
/// `$E3`: slots the game's OAM build used.
const OAM_USED: usize = 0xE3;
const OAM_HI: u32 = 0x7E0900;
const OAM_HIDDEN_Y: u8 = 0xE0;

/// With X3 loaded, the bar icon (X's `$86`, 16x16) is X3's "Z" instead: its
/// four tiles live in shadow page 3 (the HUD page; pages 0-2 are full with
/// X3 Zero frames and slash arcs), at the cells the PPU reads for tile
/// `$86` (`(t & $3F) % 48`: 6, 7, 22, 23), so the OAM entry keeps its tile
/// number and only its slot is flagged. One copy serves P2's icon, the down
/// marker and a Zero P1's own icon (slot 0).
const ICON_PAGE: u8 = 3;
const ICON_CELLS: [usize; 4] = [6, 7, 22, 23];
/// The icon's colours (X3's HUD palette) on Zero's palette (`$8C:B580`),
/// which P2's and a Zero P1's sprite palettes hold: the emblem's yellow body
/// 11 -> Zero's red 7, its red stroke ends 6 -> his gold 13 (all-red read
/// as an X at 16 px); the plate's white 1 and greys 13, 14, 15 -> 4, 5, 6,
/// 15.
pub const HUD_ON_ZERO: [u8; 16] = [0, 4, 0, 0, 0, 0, 13, 0, 0, 0, 0, 7, 0, 5, 6, 15];

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

/// The Z icon, 16x16, in Zero's palette: `K` dark outline (15), `W` white
/// plate (4), `R` red stroke (7), `Y` gold highlight (13), `.` transparent.
/// X3 keeps X's emblem for Zero (there is no Z in either ROM), so it is drawn
/// here and needs no X3 ROM.
const Z_GLYPH: [&str; 16] = [
    "KKKKKKKKKKKKKKKK",
    "KWWWWWWWWWWWWWWK",
    "KWWWWWWWWWWWWWWK",
    "KWWYYYYYYYYYYWWK",
    "KWWRRRRRRRRRRWWK",
    "KWWWWWWWWWRRRWWK",
    "KWWWWWWWWRRRWWWK",
    "KWWWWWWWRRRWWWWK",
    "KWWWWWWRRRWWWWWK",
    "KWWWWWRRRWWWWWWK",
    "KWWWWRRRWWWWWWWK",
    "KWWRRRRRRRRRRWWK",
    "KWWYYYYYYYYYYWWK",
    "KWWWWWWWWWWWWWWK",
    "KWWWWWWWWWWWWWWK",
    "KKKKKKKKKKKKKKKK",
];

/// [`Z_GLYPH`] as four SNES 4bpp planar tiles (top-left, top-right,
/// bottom-left, bottom-right), matching [`ICON_CELLS`]. Pure.
pub fn z_glyph_tiles() -> [[u8; 32]; 4] {
    let px = |x: usize, y: usize| -> u8 {
        match Z_GLYPH[y].as_bytes()[x] {
            b'K' => 15,
            b'W' => 4,
            b'R' => 7,
            b'Y' => 13,
            _ => 0,
        }
    };
    let mut out = [[0u8; 32]; 4];
    for (i, (ox, oy)) in [(0, 0), (8, 0), (0, 8), (8, 8)].into_iter().enumerate() {
        for r in 0..8 {
            let mut planes = [0u8; 4];
            for c in 0..8 {
                let v = px(ox + c, oy + r);
                for (b, plane) in planes.iter_mut().enumerate() {
                    *plane |= ((v >> b) & 1) << (7 - c);
                }
            }
            out[i][2 * r] = planes[0];
            out[i][2 * r + 1] = planes[1];
            out[i][16 + 2 * r] = planes[2];
            out[i][17 + 2 * r] = planes[3];
        }
    }
    out
}

/// Write the Z icon into shadow page 3 (see [`ICON_PAGE`]). Always true: the
/// glyph is ours, not the ROM's.
fn put_icon(game: &mut Game) -> bool {
    for (t, cell) in z_glyph_tiles().iter().zip(ICON_CELLS) {
        crate::p2tiles::fill_page(game, usize::from(ICON_PAGE) * crate::p2tiles::SHADOW_WORDS + cell * 16, t);
    }
    true
}

/// [`HOOK_HUD`]: a Zero P1's icon; P2's bar (and the down marker) into 8
/// slots past P2's.
fn hud(game: &mut Game) {
    let shown = game.wram()[ix(BAR_STATE)] == BAR_SHOWN;
    if game.local_players >= 3 {
        // Four-player HUD is composed after PPU rendering. The seven ROM bar
        // sprites would consume scarce OAM and duplicate its P1 display.
        let o = ix(ram::OAM_BUF);
        for slot in 0..BAR_SLOTS {
            game.wram_mut()[o + slot * 4 + 1] = OAM_HIDDEN_Y;
        }
        return;
    }
    // P1 as Zero: X's own icon (slot 0, never moved by the ROM after the
    // bar's init) becomes the Z, in Zero's palette (slot 1).
    if game.coop.p1.on && shown && put_icon(game) {
        let o = ix(ram::OAM_BUF);
        coop::set_oam_palette(&mut game.wram_mut()[o..o + 4], crate::zero::X_SPRITE_PALETTE);
        game.add_p2_tile_slots(1, ICON_PAGE);
    }
    if !game.coop.enabled || !shown || !coop_stages::p2_should_exist(game.wram()) {
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
    let pal = game.coop.zero_palette;
    let zero = game.player_character(1) == Some(crate::Character::Zero);
    // A second X: X's emblem in the HUD palette (2), whatever P1's slot 0
    // wears (a Zero P1's is palette 1).
    let icon_pal = if zero { pal } else { 2 };
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
        e[0] = BAR_X;
        if e[1] != OAM_HIDDEN_Y {
            e[1] = BAR_MIRROR.wrapping_sub(e[1]);
            e[3] ^= u8::from(k > 0) << 7;
        }
        // The pieces keep X's attr (`$34`: the HUD palette, 2), so they are
        // X's colours; only the icon is Zero's (a second X keeps X's emblem).
        if k == 0 {
            coop::set_oam_palette(e, icon_pal);
        }
    }
    let e = &mut w[oam + (start + BAR_SLOTS) * 4..][..4];
    match marker {
        Some(m) => {
            e.copy_from_slice(&m);
            coop::set_oam_palette(e, icon_pal);
        }
        None => e[1] = OAM_HIDDEN_Y,
    }
    let icons = 1 << start | u128::from(marker.is_some()) << (start + BAR_SLOTS);
    if zero && put_icon(game) {
        game.add_p2_tile_slots(icons, ICON_PAGE);
        game.add_p2_palette_slots(icons);
    }
    // Widescreen: the bar goes to the wide picture's left edge with X's (the
    // marker stays where P2 fell).
    game.margin.p2_bar = crate::display::slots(start, start + BAR_SLOTS);
    let w = game.wram_mut();
    // All 8 large (bit 1), X bit 8 clear.
    w[ix(OAM_HI) + start / 4..][..2].fill(0xAA);
    // Hidden again by `coop::oam_hide` at the next build.
    game.coop.p2_oam_base = base;
    game.coop.p2_oam_count = start + HUD_SLOTS - base;
}

/// Redraw only opaque UI pixels after host effects. ROM HUD sprites retain
/// their artwork, palettes, flips and widescreen placement; transparent
/// pixels leave the lit world visible.
pub(crate) fn draw_ui_rgba(game: &Game, frame: &mut [u8], width: usize, height: usize) {
    if frame.len() < width.saturating_mul(height).saturating_mul(4)
        || game.wram()[ix(ram::MODE_D2)] != ram::D2_IN_STAGE { return; }
    let scale = usize::from(game.frame_scale().max(1));
    const CLEAR: u32 = 0xFF000000;
    let mut layer = vec![CLEAR; width * height];
    let (mut left, right) = crate::widescreen::hud_slots(game.wram(), 0);
    let oam = game.oam();
    let shadow = game.emu.p2_tiles();
    // P2 HUD entries are recognizable independently of the next frame's
    // changing player OAM ranges: their bar column and reserved ROM tiles.
    if game.coop.enabled && game.local_players < 3 {
        for slot in 24..128 {
            let e = &oam[slot * 4..slot * 4 + 4];
            if e[2] == 0x86 && (shadow.slots & shadow.page1 & shadow.page2) >> slot & 1 != 0 {
                if e[0] == BAR_X && e[1] == 0x68 {
                    left |= crate::display::slots(slot, slot + BAR_SLOTS);
                } else {
                    left |= 1 << slot; // Revive marker, also part of the UI.
                }
            }
        }
    }
    let cgram = game.cgram();
    let (base, gap) = game.obsel();
    let margin = usize::from(game.wide_margin());
    let (shift_left, shift_right) = game.emu.hud_shift();
    for slot in (0..128).rev().filter(|&s| (left | right) >> s & 1 != 0) {
        let e = &oam[slot * 4..slot * 4 + 4];
        if e[1] == OAM_HIDDEN_Y { continue; }
        let high = oam[512 + slot / 4] >> ((slot % 4) * 2);
        let size = if high & 2 != 0 { 16 } else { 8 };
        let mut x = i32::from(e[0]) - if high & 1 != 0 { 256 } else { 0 } + margin as i32;
        if shift_left >> slot & 1 != 0 { x -= margin as i32; }
        if shift_right >> slot & 1 != 0 { x += margin as i32; }
        let y = usize::from(e[1]);
        for dy in 0..size { for dx in 0..size {
            let sx = if e[3] & 0x40 != 0 { size - 1 - dx } else { dx };
            let sy = if e[3] & 0x80 != 0 { size - 1 - dy } else { dy };
            let tile = (u16::from(e[2]) & 0xF0).wrapping_add((sy / 8 * 16) as u16) & 0xF0
                | (u16::from(e[2]) + (sx / 8) as u16) & 0xF;
            let words: &[u16] = if shadow.slots >> slot & 1 != 0 {
                let page = ((shadow.page1 >> slot & 1) | (shadow.page2 >> slot & 1) << 1) as usize;
                let at = page * crate::p2tiles::SHADOW_WORDS + usize::from((tile & 0x3F) % 48) * 16;
                &shadow.page[at..at + 16]
            } else {
                let at = (usize::from(base) + usize::from(e[3] & 1) * (4096 + usize::from(gap)) + usize::from(tile) * 16) & 0x7FFF;
                &game.vram()[at..at + 16]
            };
            let row = words[sy % 8];
            let row_hi = words[8 + sy % 8];
            let bit = 7 - sx % 8;
            let colour = ((row >> bit) & 1) | ((row >> (bit + 8)) & 1) << 1
                | ((row_hi >> bit) & 1) << 2 | ((row_hi >> (bit + 8)) & 1) << 3;
            if colour == 0 { continue; }
            let c = if shadow.pal >> slot & 1 != 0 { shadow.palette[usize::from(colour)] }
                else { cgram[128 + usize::from((e[3] >> 1) & 7) * 16 + usize::from(colour)] };
            let channel = |shift: u32| -> u32 { (((u32::from(c) >> shift) & 31_u32) * 255 + 15) / 31 };
            let rgb = channel(0) << 16 | channel(5) << 8 | channel(10);
            let px = x + dx as i32;
            if px < 0 { continue; }
            for yy in (y + dy) * scale..((y + dy + 1) * scale).min(height) {
                for xx in px as usize * scale..((px as usize + 1) * scale).min(width) {
                    layer[yy * width + xx] = rgb;
                }
            }
        }}
    }
    if game.local_players >= 3 && game.coop.enabled && four_hud_shown(game) {
        let hp = [game.wram()[ix(ram::X_HP)] & 0x7F,
            if game.coop.p2_hidden { 0 } else { game.coop.p2[OFF_HP] & 0x7F },
            if game.extra_coop[0].p2_hidden { 0 } else { game.extra_coop[0].p2[OFF_HP] & 0x7F },
            if game.extra_coop[1].p2_hidden { 0 } else { game.extra_coop[1].p2[OFF_HP] & 0x7F }];
        let initials = game.player_characters.map(|c| c.label().chars().next().unwrap_or('?').to_ascii_uppercase());
        draw_four_health_bars_scaled(&mut layer, width, height, hp,
            game.wram()[ix(ram::MAX_HP)] & 0x7F, initials, scale as u8);
    }
    for (dst, colour) in frame.chunks_exact_mut(4).zip(layer) {
        if colour != CLEAR { dst.copy_from_slice(&[(colour >> 16) as u8, (colour >> 8) as u8, colour as u8, 255]); }
    }
}

/// Four vertical HP meters at the screen edges, styled like the original
/// seven-sprite X bar. Pixel composition avoids consuming scarce SNES OAM
/// slots in a four-player game. Each player's meter has its own live HP.
pub fn draw_four_health_bars(pixels: &mut [u32], width: usize, height: usize, hp: [u8; 4], max_hp: u8, initials: [char; 4]) {
    draw_four_health_bars_scaled(pixels, width, height, hp, max_hp, initials, 1);
}

/// Keep HUD geometry on the ROM grid when high-resolution actors enlarge
/// the presentation buffer; every original HUD pixel remains a solid block.
pub fn draw_four_health_bars_scaled(pixels: &mut [u32], width: usize, height: usize,
                                  hp: [u8; 4], max_hp: u8, initials: [char; 4], scale: u8) {
    let scale = usize::from(scale.max(1));
    if width / scale < 256 || height / scale < 144 || pixels.len() < width * height { return }
    let physical_width = width;
    let width = width / scale;
    let height = height / scale;
    let fill = |pixels: &mut [u32], x: usize, y: usize, w: usize, h: usize, colour: u32| {
        for row in y * scale..(y + h).min(height) * scale {
            for col in x * scale..(x + w).min(width) * scale { pixels[row * physical_width + col] = colour; }
        }
    };
    let glyph = |c: char| -> [u8; 5] {
        match c {
            '0' => [7,5,5,5,7], '1' => [2,6,2,2,7], '2' => [7,1,7,4,7],
            '3' => [7,1,7,1,7], '4' => [5,5,7,1,1], '5' => [7,4,7,1,7],
            '6' => [7,4,7,5,7], '7' => [7,1,1,1,1], '8' => [7,5,7,5,7],
            '9' => [7,5,7,1,7], 'P' => [7,5,7,4,4], 'X' => [5,5,2,5,5],
            'Z' => [7,1,2,4,7], 'S' => [7,4,7,1,7], 'V' => [5,5,5,5,2],
            'A' => [2,5,7,5,5], 'B' => [6,5,6,5,6], 'C' => [7,4,4,4,7],
            'D' => [6,5,5,5,6], 'E' => [7,4,7,4,7], 'F' => [7,4,7,4,4],
            'G' => [7,4,5,5,7], 'H' => [5,5,7,5,5], 'I' => [7,2,2,2,7],
            'L' => [4,4,4,4,7], 'M' => [5,7,7,7,5], 'N' => [5,7,7,7,5],
            'O' => [7,5,5,5,7], 'R' => [6,5,6,5,5], 'T' => [7,2,2,2,2],
            'U' => [5,5,5,5,7], 'W' => [5,5,7,7,5], 'Y' => [5,5,2,2,2],
            _ => [0; 5],
        }
    };
    let text = |pixels: &mut [u32], x: usize, y: usize, s: &str, colour: u32| {
        for (i, c) in s.chars().enumerate() {
            for (dy, row) in glyph(c).into_iter().enumerate() {
                for dx in 0..3 {
                    if row & (4 >> dx) != 0 { fill(pixels, x + i * 4 + dx, y + dy, 1, 1, colour); }
                }
            }
        }
    };
    const BAR_Y: usize = 26;
    const BAR_H: usize = 100;
    const BAR_W: usize = 14;
    const SEGMENTS: usize = 32;
    let colours = [0x3DA5FF, 0xFF606C, 0x67DB8D, 0xC28BFF];
    let xs = [6, 24, width - 38, width - 20];
    for p in 0..4 {
        let x = xs[p];
        // Metallic cap and border, dark recessed well, then bright segments
        // growing upwards from the bottom, as in the game's own HP meter.
        fill(pixels, x, BAR_Y, BAR_W, BAR_H, 0x0A1326);
        fill(pixels, x, BAR_Y, BAR_W, 2, 0xCBD5E6);
        fill(pixels, x, BAR_Y + BAR_H - 2, BAR_W, 2, 0x66768C);
        fill(pixels, x + 1, BAR_Y + 2, 1, BAR_H - 4, 0x8293A9);
        fill(pixels, x + BAR_W - 2, BAR_Y + 2, 1, BAR_H - 4, 0x8293A9);
        fill(pixels, x + 3, BAR_Y + 2, 8, 96, 0x17243A);
        let units = (usize::from(hp[p].min(max_hp)) * SEGMENTS + usize::from(max_hp.max(1)) / 2)
            / usize::from(max_hp.max(1));
        let colour = if hp[p] <= max_hp / 4 { 0xFF675F } else { colours[p] };
        for segment in 0..units {
            let y = BAR_Y + 96 - segment * 3;
            fill(pixels, x + 3, y, 8, 2, colour);
            fill(pixels, x + 3, y, 2, 2, 0xEFF8FF);
        }
        text(pixels, x + 1, BAR_Y - 9, &format!("P{}", p + 1), 0xF5F6FC);
        text(pixels, x + 5, BAR_Y + BAR_H + 4, &initials[p].to_string(), colours[p]);
    }
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

    #[test]
    fn four_health_bars_use_each_players_hp() {
        let mut pixels = vec![0; 256 * 224];
        draw_four_health_bars(&mut pixels, 256, 224, [32, 16, 8, 0], 32, ['X', 'Z', 'S', 'V']);
        let at = |x: usize, y: usize| pixels[y * 256 + x];
        assert_eq!(at(12, 30), 0x3DA5FF); // full P1 reaches the top
        assert_eq!(at(30, 80), 0xFF606C); // half P2 reaches the middle
        assert_eq!(at(30, 50), 0x17243A);
        assert_eq!(at(224, 122), 0xFF675F); // low P3 uses the warning colour
        assert_eq!(at(224, 80), 0x17243A);
        assert_eq!(at(242, 122), 0x17243A); // empty P4
    }

    #[test]
    fn high_resolution_health_bars_keep_original_game_size() {
        let (mut native, mut hd) = (vec![0; 256 * 224], vec![0; 1024 * 896]);
        draw_four_health_bars(&mut native, 256, 224, [32, 16, 8, 0], 32, ['X', 'Z', 'S', 'V']);
        draw_four_health_bars_scaled(&mut hd, 1024, 896, [32, 16, 8, 0], 32, ['X', 'Z', 'S', 'V'], 4);
        for y in 0..896 { for x in 0..1024 { assert_eq!(hd[y * 1024 + x], native[y / 4 * 256 + x / 4]); } }
    }
}
