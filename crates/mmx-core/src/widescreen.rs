//! Widescreen (docs/widescreen.md): the PPU draws `margin` extra pixels each
//! side of the 256-px picture; this module picks the margin and feeds the
//! PPU the stage's real tiles for the margins. Display only.

use crate::{ram, Game};
use snes_core::api::MARGIN_MAP_CELLS;

/// A widescreen setting: margin per side in whole 8-px tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Widescreen(pub u8);

impl Widescreen {
    pub const OFF: Self = Self(0);
    /// 368x224 (1.64; 16:10 = 358 px, rounded up to a tile).
    pub const W16_10: Self = Self(7);
    /// 400x224 (1.79; 16:9 = 398 px).
    pub const W16_9: Self = Self(9);
    /// 528x224 (2.36; 21:9 = 523 px).
    pub const W21_9: Self = Self(17);
    pub const PRESETS: [(&'static str, Self); 4] =
        [("off", Self::OFF), ("16:10", Self::W16_10), ("16:9", Self::W16_9), ("21:9", Self::W21_9)];
    /// Tiles per side the PPU accepts (`MAX_WIDE_MARGIN` / 8).
    pub const MAX_TILES: u8 = 24;

    /// `off`, `16:10`, `16:9`, `21:9` or `N` tiles per side (0-24).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if let Some((_, w)) = Self::PRESETS.iter().find(|(n, _)| *n == s) {
            return Some(*w);
        }
        s.parse::<u8>().ok().filter(|&n| n <= Self::MAX_TILES).map(Self)
    }

    pub fn name(self) -> String {
        Self::PRESETS.iter().find(|(_, w)| *w == self).map_or_else(|| self.0.to_string(), |(n, _)| (*n).to_string())
    }

    /// The next preset (F10 cycles them); a custom N goes to `off`.
    pub fn next(self) -> Self {
        let i = Self::PRESETS.iter().position(|(_, w)| *w == self).map_or(0, |i| (i + 1) % Self::PRESETS.len());
        Self::PRESETS[i].1
    }

    pub fn margin_px(self) -> u16 {
        u16::from(self.0) * 8
    }

    /// Output width (height stays 224 or 239).
    pub fn width(self) -> usize {
        256 + 2 * usize::from(self.margin_px())
    }
}

/// Per layout BG (BG1 terrain, BG2 backdrop): the screen map (32x32 screens
/// of 256 px, one byte each: the screen number), the screens (16x16 maps of
/// 16 px, `u16` map numbers, `$200` bytes per screen), the camera (X, Y).
/// The map number indexes the stage's map table at the long pointer
/// `$0B95` (`$99:8000` on the Highway): 4 tilemap entries per map, TL TR BL BR.
const SCREEN_MAP: [usize; 2] = [0xE800, 0xEC00];
const SCREENS: [usize; 2] = [0x2000, 0xA600];
const MAP_TILES_PTR: usize = 0x0B95;
const CAMERA: [(u32, u32); 2] = [(ram::CAMERA_X, ram::CAMERA_Y), (0x7E1E8D, 0x7E1E90)];

fn read8(g: &Game, a: u32) -> u8 {
    match (a >> 16) & 0xFF {
        0x7E | 0x7F => g.wram()[(a as usize - 0x7E_0000) & 0x1FFFF],
        _ => g.rom_read(a),
    }
}

/// The tilemap entry the stage layout holds for layout BG `bg` (0 = BG1,
/// 1 = BG2) at level pixel `(x, y)`; `None` outside the 8192x8192 map.
pub fn layout_entry(g: &Game, bg: usize, x: i32, y: i32) -> Option<u16> {
    if !(0..8192).contains(&x) || !(0..8192).contains(&y) {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    let w = g.wram();
    let screen = usize::from(w[SCREEN_MAP[bg] + (y >> 8) * 32 + (x >> 8)]);
    let o = (SCREENS[bg] + screen * 0x200 + ((y >> 4) & 15) * 32 + ((x >> 4) & 15) * 2) & 0x1FFFF;
    let map = u32::from(u16::from_le_bytes([w[o], w[(o + 1) & 0x1FFFF]]));
    let p = u32::from_le_bytes([w[MAP_TILES_PTR], w[MAP_TILES_PTR + 1], w[MAP_TILES_PTR + 2], 0]);
    let a = p + map * 8 + (((y >> 3) & 1) * 2 + ((x >> 3) & 1)) as u32 * 2;
    Some(u16::from_le_bytes([read8(g, a), read8(g, a + 1)]))
}

/// The layout BG's camera (level pixel of the picture's top-left).
pub fn camera(g: &Game, bg: usize) -> (i32, i32) {
    let w = g.wram();
    let r = |a: u32| i32::from(u16::from_le_bytes([w[ram::wram_index(a)], w[ram::wram_index(a) + 1]]));
    (r(CAMERA[bg].0), r(CAMERA[bg].1))
}

/// Share of the on-screen cells (8-px grid over 256x224 from the camera)
/// where the layout decoder gives the same tilemap entry as VRAM, in percent.
pub fn agreement(g: &Game, bg: usize) -> u32 {
    let (cx, cy) = camera(g, bg);
    let (mut ok, mut n) = (0, 0);
    for sy in (0..224).step_by(8) {
        for sx in (0..256).step_by(8) {
            let (x, y) = (cx + sx, cy + sy);
            n += 1;
            ok += u32::from(layout_entry(g, bg, x, y) == Some(g.bg_map_entry(bg, x as u16, y as u16)));
        }
    }
    ok * 100 / n
}

/// After each step with widescreen on: give the PPU the stage layout's
/// tiles around the camera for BG1 and BG2 (keyed by level position mod
/// 1024, which is BG scroll space: MMX scrolls each BG by its camera), so
/// the margins show the real stage where the 64-column VRAM ring is stale.
/// Off the layout (left of x 0 at a stage's start) BG1 is blank and BG2 is
/// VRAM's wrap. Only in a stage, and only for a BG whose decoded on-screen cells agree
/// with VRAM (>= 90%); otherwise the margins use VRAM as is. Display only.
pub fn update_margin_tiles(g: &mut Game) {
    let w = g.wram();
    let in_stage = w[ram::wram_index(ram::MODE_D2)] == 4 && w[ram::wram_index(ram::STAGE_PHASE)] != 0;
    let m = i32::from(g.wide_margin());
    for bg in 0..2 {
        if !in_stage || agreement(g, bg) < 90 {
            g.set_margin_tiles(bg, None);
            continue;
        }
        let (cx, cy) = camera(g, bg);
        let n = MARGIN_MAP_CELLS;
        let mut cells = vec![0u32; n * n];
        for y in ((cy - 32) & !7..cy + 256).step_by(8) {
            for x in ((cx - m - 32) & !7..cx + 256 + m + 32).step_by(8) {
                let cell = ((y & 1023) / 8) as usize * n + ((x & 1023) / 8) as usize;
                // Off the map: BG1 (terrain) blank, BG2 (backdrop) whatever VRAM wraps to.
                cells[cell] = match layout_entry(g, bg, x, y) {
                    Some(e) => 0x1_0000 | u32::from(e),
                    None if bg == 0 => 0x1_0000,
                    None => 0,
                };
            }
        }
        g.set_margin_tiles(bg, Some(&cells));
    }
}
