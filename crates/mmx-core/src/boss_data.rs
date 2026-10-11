//! X1 boss art extracted from the user's ROM at load time. No art is bundled.
//! Static enemy graphics use the same 4bpp decompressor and sprite-layout
//! table as Zero, but are loaded as one tile set instead of streamed by frame.

use crate::{character::Character, zero_data::{Frame, Sprite, NO_TILE, SET_TABLE}};

const PAGE_TILES: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BossSet {
    /// SNES 4bpp planar tiles (32 bytes apiece).
    pub tiles: Vec<u8>,
    /// BGR555 sprite palette from the selected ROM.
    pub palette: [u16; 16],
    /// Sprite layouts, with tile numbers rebased into a 48-tile page.
    pub frames: Vec<Frame>,
    pub idle_frame: usize,
    /// ROM pose indices cycled while the character moves horizontally.
    pub run_frames: Vec<usize>,
    /// Align the selected sprite's bottom with X's standing bottom (+16).
    pub y_offset: i16,
}

impl BossSet {
    pub fn page(&self, frame: usize) -> Vec<u8> {
        let mut page = vec![0; PAGE_TILES * 32];
        if let Some(f) = self.frames.get(frame) {
            for (slot, &src) in f.tiles_used.iter().enumerate() {
                if src == NO_TILE { continue }
                let src = usize::from(src) * 32;
                if src + 32 <= self.tiles.len() && slot < PAGE_TILES {
                    page[slot * 32..slot * 32 + 32].copy_from_slice(&self.tiles[src..src + 32]);
                }
            }
        }
        page
    }
}

fn off(a: u32) -> usize { (((a >> 16) & 0x7f) * 0x8000 + (a & 0x7fff)) as usize }
fn byte(rom: &[u8], a: u32) -> Result<u8, String> {
    rom.get(off(a)).copied().ok_or_else(|| format!("ROM address ${a:06X} is missing"))
}
fn word(rom: &[u8], a: u32) -> Result<u16, String> {
    Ok(u16::from(byte(rom, a)?) | u16::from(byte(rom, a + 1)?) << 8)
}
fn long(rom: &[u8], a: u32) -> Result<u32, String> {
    Ok(u32::from(word(rom, a)?) | u32::from(byte(rom, a + 2)?) << 16)
}

pub(crate) fn graphics_table(rom: &[u8]) -> Result<u32, String> {
    // PAL v1.0 and USA Rev 1 have identical layouts; Rev 1 moved the
    // compressed-graphics directory 0x5c bytes within bank $86.
    for base in [0x86F69B, 0x86F6F7] {
        if word(rom, base + 5 * 0x4e)? == 0x0800
            && word(rom, base + 5 * 0x99)? == 0x2000 {
            return Ok(base);
        }
    }
    Err("unsupported Mega Man X graphics directory".into())
}

fn decompress(rom: &[u8], table: u32, id: u8) -> Result<Vec<u8>, String> {
    let entry = table + 5 * u32::from(id);
    let size = usize::from(word(rom, entry)?);
    if size == 0 || size > 0x10000 { return Err(format!("invalid graphics set ${id:02X} size")); }
    let mut addr = u32::from(word(rom, entry + 2)?) | u32::from(byte(rom, entry + 4)?) << 16;
    let mut next = || -> Result<u8, String> {
        let result = byte(rom, addr)?;
        addr += 1;
        if addr & 0xffff == 0 { addr += 0x8000; }
        Ok(result)
    };
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let control = next()?;
        let fill = next()?;
        for bit in (0..8).rev() {
            if out.len() == size { break; }
            out.push(if control & (1 << bit) != 0 { next()? } else { fill });
        }
    }
    out.resize(out.len().div_ceil(32) * 32, 0);
    Ok(out)
}

fn palette(rom: &[u8], id: u8) -> Result<[u16; 16], String> {
    let group = 0x860000 | u32::from(word(rom, 0x868133 + 2 * u32::from(id))?);
    let source = 0x850000 | u32::from(word(rom, group + 1)?);
    if byte(rom, group)? < 16 { return Err(format!("palette ${id:02X} is too short")); }
    let mut out = [0; 16];
    for (i, c) in out.iter_mut().enumerate() { *c = word(rom, source + 2 * i as u32)?; }
    Ok(out)
}

/// Repack the tiles read by a layout into the 16-column, three-row shadow
/// page. Big sprites need the four slots `t,t+1,t+16,t+17`; small sprites
/// occupy remaining cells. If a layout exceeds the page, its earliest OAM
/// entries (drawn on top by the game) take priority.
fn repack(rom: &[u8], layout: u32, tile_count: usize) -> Result<Frame, String> {
    let n = usize::from(byte(rom, layout)?);
    if n > 128 { return Err("invalid boss sprite count".into()); }
    let mut source = Vec::with_capacity(n);
    for i in 0..n {
        let e = layout + 1 + 4 * i as u32;
        let a = byte(rom, e + 3)?;
        source.push(Sprite {
            x: byte(rom, e)? as i8,
            y: byte(rom, e + 1)? as i8,
            big: a & 0x20 != 0,
            tile: byte(rom, e + 2)?,
            hflip: a & 0x40 != 0,
            vflip: a & 0x80 != 0,
        });
    }
    let mut slots = vec![NO_TILE; PAGE_TILES];
    let mut sprites = Vec::new();
    let big_count = source.iter().filter(|s| s.big && usize::from(s.tile) + 17 < tile_count).count().min(8);
    let mut big_next = 0usize;
    // Prefer the wholly free third row, then unused cells to the right of
    // the big-sprite blocks in the first two rows.
    let small_slots = (32..PAGE_TILES)
        .chain(2 * big_count..16)
        .chain(16 + 2 * big_count..32)
        .collect::<Vec<_>>();
    let mut small_next = 0usize;
    for mut s in source {
        let original = usize::from(s.tile);
        if s.big {
            if big_next >= 8 || original + 17 >= tile_count { continue; }
            let dst = 2 * big_next;
            big_next += 1;
            for (delta, shift) in [(0, 0), (1, 1), (16, 16), (17, 17)] {
                slots[dst + shift] = (original + delta) as u16;
            }
            s.tile = dst as u8;
        } else {
            if small_next >= small_slots.len() || original >= tile_count { continue; }
            let dst = small_slots[small_next];
            slots[dst] = original as u16;
            s.tile = dst as u8;
            small_next += 1;
        }
        sprites.push(s);
    }
    Ok(Frame { name: "boss", sprites, tiles_used: slots })
}

fn pixel(tile: &[u8], x: usize, y: usize) -> u8 {
    let bit = 7 - x;
    (tile[2 * y] >> bit & 1)
        | (tile[2 * y + 1] >> bit & 1) << 1
        | (tile[16 + 2 * y] >> bit & 1) << 2
        | (tile[17 + 2 * y] >> bit & 1) << 3
}

fn encode_tile(pixels: &[u8; 32 * 32], tile_x: usize, tile_y: usize) -> [u8; 32] {
    let mut out = [0; 32];
    for y in 0..8 {
        for x in 0..8 {
            let c = pixels[(tile_y * 8 + y) * 32 + tile_x * 8 + x];
            let bit = 7 - x;
            out[2 * y] |= (c & 1) << bit;
            out[2 * y + 1] |= (c >> 1 & 1) << bit;
            out[16 + 2 * y] |= (c >> 2 & 1) << bit;
            out[17 + 2 * y] |= (c >> 3 & 1) << bit;
        }
    }
    out
}

/// Wolf Sigma's face is a 35-piece, 120-pixel-wide ROM object. It cannot fit
/// a local player's 16-OAM-entry allowance, so assemble his own ROM pieces
/// and scale the complete face to a 32x32, sixteen-entry portrait sprite.
fn wolf_sigma(rom: &[u8], table: u32) -> Result<BossSet, String> {
    let source = decompress(rom, table, 0x3d)?;
    let colors = palette(rom, 0x50)?;
    let set = long(rom, SET_TABLE + 3 * 0x2f)?;
    let layout = long(rom, set)?;
    let n = usize::from(byte(rom, layout)?);
    let width = 160usize;
    let height = 128usize;
    let mut canvas = vec![0u8; width * height];
    for i in (0..n).rev() {
        let e = layout + 1 + 4 * i as u32;
        let x = byte(rom, e)? as i8 as i32;
        let y = byte(rom, e + 1)? as i8 as i32;
        let t = usize::from(byte(rom, e + 2)?);
        let a = byte(rom, e + 3)?;
        let size = if a & 0x20 != 0 { 16 } else { 8 };
        for py in 0..size {
            for px in 0..size {
                let sx = if a & 0x40 != 0 { size - 1 - px } else { px };
                let sy = if a & 0x80 != 0 { size - 1 - py } else { py };
                let tile = t + sx / 8 + 16 * (sy / 8);
                if tile * 32 + 32 > source.len() { continue; }
                let c = pixel(&source[tile * 32..tile * 32 + 32], sx % 8, sy % 8);
                let dx = 80 + x + px as i32;
                let dy = 64 + y + py as i32;
                if c != 0 && (0..width as i32).contains(&dx) && (0..height as i32).contains(&dy) {
                    canvas[dy as usize * width + dx as usize] = c;
                }
            }
        }
    }
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0usize, 0usize);
    for y in 0..height {
        for x in 0..width {
            if canvas[y * width + x] != 0 {
                left = left.min(x); top = top.min(y);
                right = right.max(x); bottom = bottom.max(y);
            }
        }
    }
    if left > right { return Err("Wolf Sigma has no visible ROM pixels".into()); }
    let (src_w, src_h) = (right - left + 1, bottom - top + 1);
    let scale = (32.0 / src_w as f32).min(32.0 / src_h as f32);
    let (dst_w, dst_h) = ((src_w as f32 * scale) as usize, (src_h as f32 * scale) as usize);
    let mut reduced = [0u8; 32 * 32];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let sx = left + x * src_w / dst_w;
            let sy = top + y * src_h / dst_h;
            reduced[((32 - dst_h) / 2 + y) * 32 + (32 - dst_w) / 2 + x] = canvas[sy * width + sx];
        }
    }
    let mut tiles = Vec::with_capacity(16 * 32);
    let mut sprites = Vec::with_capacity(16);
    let mut used = vec![NO_TILE; PAGE_TILES];
    for y in 0..4 {
        for x in 0..4 {
            let slot = y * 4 + x;
            tiles.extend_from_slice(&encode_tile(&reduced, x, y));
            used[slot] = slot as u16;
            sprites.push(Sprite {
                x: (x * 8) as i8 - 16, y: (y * 8) as i8 - 16,
                big: false, tile: slot as u8, hflip: false, vflip: false,
            });
        }
    }
    Ok(BossSet { tiles, palette: colors, frames: vec![Frame { name: "wolf sigma", sprites, tiles_used: used }], idle_frame: 0, run_frames: vec![0], y_offset: 0 })
}

/// Extract a boss's source graphics, palette and layouts from a headerless X1
/// ROM. Unsupported characters return an error instead of silently drawing X.
pub fn extract_x1(rom: &[u8], character: Character) -> Result<BossSet, String> {
    if rom.get(0x7fc0..0x7fc9) != Some(b"MEGAMAN X".as_slice()) {
        return Err("not a headerless Mega Man X ROM".into());
    }
    if character == Character::Zero { return extract_x1_zero(rom); }
    let graphics_table = graphics_table(rom)?;
    if character == Character::WolfSigma { return wolf_sigma(rom, graphics_table); }
    let (set, graphics, colors, idle_frame) = match character {
        Character::LaunchOctopus => (0x76u8, 0x6a, 0x9f, 0),
        Character::StingChameleon => (0x23u8, 0x20, 0x2a, 0),
        Character::ArmoredArmadillo => (0x62u8, 0x5d, 0x90, 0),
        Character::FlameMammoth => (0x06u8, 0x01, 0xad, 0),
        Character::StormEagle => (0x89u8, 0x82, 0xb7, 0),
        Character::SparkMandrill => (0x91u8, 0x8a, 0xe9, 0),
        Character::BoomerKuwanger => (0x75u8, 0x69, 0x9e, 0),
        Character::ChillPenguin => (0x67u8, 0x61, 0x94, 0),
        // On-foot Vile: palette $51 gives his canonical purple armour.
        Character::Vile => (0x50u8, 0x4e, 0x51, 0),
        // Frames 0-3 are the entrance (coat removed piece by piece); frame 4
        // is his first fully equipped battle stance.
        Character::Sigma => (0xa1u8, 0x99, 0xee, 4),
        Character::Velguarder => (0x99u8, 0x8e, 0xcc, 0),
        Character::Bospider => (0x9fu8, 0x97, 0xec, 0),
        Character::RangdaBangda => (0x9bu8, 0x8f, 0xcd, 0),
        Character::DRex => (0xa0u8, 0x98, 0xed, 0),
        _ => return Err(format!("{} ROM sprite mapping is not known", character.label())),
    };
    let tiles = decompress(rom, graphics_table, graphics)?;
    let palette = palette(rom, colors)?;
    let table = long(rom, SET_TABLE + 3 * u32::from(set))?;
    let next = long(rom, SET_TABLE + 3 * (u32::from(set) + 1))?;
    let count = ((next - table) / 3).min(128) as usize;
    if count == 0 { return Err("boss set has no frames".into()); }
    let mut frames = Vec::with_capacity(count);
    for i in 0..count {
        let layout = long(rom, table + 3 * i as u32)?;
        frames.push(repack(rom, layout, tiles.len() / 32)?);
    }
    if character == Character::Sigma {
        // Four local players share OAM's 128 entries. Sigma's full battle
        // stance has 22 pieces; the first 16 omit both feet. Keep the face,
        // shoulder and torso overlays plus every leg piece in OAM order.
        const BATTLE_IDLE: [usize; 16] = [0, 2, 4, 5, 6, 7, 8, 9, 12, 13, 15, 16, 17, 19, 20, 21];
        frames[4].sprites = frames[4].sprites.iter().enumerate()
            .filter(|(i, _)| BATTLE_IDLE.contains(i))
            .map(|(_, sprite)| *sprite)
            .collect();
        // The lunge cycle also has many more pieces than the four-player
        // OAM allowance. Keep the head, torso, sword and both legs in each
        // pose. All five frames have the same +32 foot baseline.
        const RUN_POSES: [(usize, &[usize; 16]); 5] = [
            (12, &[0, 1, 2, 3, 5, 6, 11, 12, 15, 17, 19, 20, 21, 22, 24, 25]),
            (13, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 14, 15, 16, 17, 19]),
            (14, &[0, 1, 2, 3, 9, 10, 11, 12, 13, 14, 15, 16, 19, 20, 21, 22]),
            (15, &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]),
            (16, &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]),
        ];
        for (frame, keep) in RUN_POSES {
            frames[frame].sprites = frames[frame].sprites.iter().enumerate()
                .filter(|(i, _)| keep.contains(i))
                .map(|(_, sprite)| *sprite)
                .collect();
        }
    }
    if character == Character::Vile {
        // Keep face, torso and both feet within the four-player OAM budget.
        const STEP_POSES: [(usize, &[usize; 16]); 2] = [
            (2, &[0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17]),
            (3, &[0, 2, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18]),
        ];
        for (frame, keep) in STEP_POSES {
            frames[frame].sprites = frames[frame].sprites.iter().enumerate()
                .filter(|(i, _)| keep.contains(i))
                .map(|(_, sprite)| *sprite)
                .collect();
        }
    }
    let run_frames = match character {
        // X1's Sigma has no ordinary walking cycle. These are his native
        // forward/lunge sword poses, with leg movement visible between them.
        Character::Sigma => vec![12, 13, 14, 15, 16, 15, 14, 13],
        Character::Vile => vec![0, 2, 3, 2],
        _ => vec![idle_frame],
    };
    let bottom = frames[idle_frame].sprites.iter()
        .map(|s| i16::from(s.y) + if s.big { 16 } else { 8 })
        .max().unwrap_or(16);
    let y_offset = 16 - bottom;
    Ok(BossSet { tiles, palette, frames, idle_frame, run_frames, y_offset })
}

/// Playable art from X1 Zero's Highway and Sigma-stage cutscenes. The X1 ROM
/// supplies a standing pose and a four-pose dash, but no walk or ladder art.
/// The 32-slot X1 pages are padded to the 48-slot local-player shadow page.
pub fn extract_x1_zero(rom: &[u8]) -> Result<BossSet, String> {
    let zero = crate::zero_data::extract_x1(rom)?;
    let mut frames: Vec<Frame> = zero.frames.into_iter().take(crate::zero_data::X1_NAMES.len()).collect();
    for frame in &mut frames {
        frame.tiles_used.resize(PAGE_TILES, NO_TILE);
    }
    // Each native dash pose has 17-18 OAM entries, including ground dust.
    // Four local players get 16 art entries apiece; remove only extra dust
    // along the floor so the head, hair, body and feet remain intact.
    for frame in &mut frames[..4] {
        while frame.sprites.len() > 16 {
            let Some(i) = frame.sprites.iter().position(|s| s.y == 16 && !s.big) else { break };
            frame.sprites.remove(i);
        }
    }
    let idle_frame = 4;
    let run_frames = vec![0, 1, 2, 3];
    let bottom = frames[idle_frame].sprites.iter()
        .map(|s| i16::from(s.y) + if s.big { 16 } else { 8 })
        .max().unwrap_or(16);
    let y_offset = 16 - bottom;
    Ok(BossSet {
        tiles: zero.tiles,
        palette: zero.palette,
        frames,
        idle_frame,
        run_frames,
        y_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_rom_boss_art() {
        for path in [
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/mega-man-x.smc"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../roms/Mega Man X (USA) (Rev 1).sfc"),
        ] {
            let Ok(mut rom) = std::fs::read(path) else { continue };
            if rom.len() % 0x8000 == 512 { rom.drain(..512); }
            for character in [
                Character::Zero, Character::Vile, Character::Sigma, Character::LaunchOctopus,
                Character::StingChameleon,
                Character::ArmoredArmadillo, Character::FlameMammoth,
                Character::StormEagle, Character::SparkMandrill,
                Character::BoomerKuwanger, Character::ChillPenguin,
                Character::Velguarder,
                Character::WolfSigma,
                Character::Bospider, Character::RangdaBangda, Character::DRex,
            ] {
                let set = extract_x1(&rom, character).unwrap();
                assert!(!set.frames.is_empty());
                assert!(!set.frames[set.idle_frame].sprites.is_empty());
                assert!(set.run_frames.iter().all(|&frame| frame < set.frames.len()));
                let bottom = set.frames[set.idle_frame].sprites.iter()
                    .map(|s| i16::from(s.y) + if s.big { 16 } else { 8 })
                    .max().unwrap();
                assert_eq!(bottom + set.y_offset, 16);
                assert!(set.frames.iter().all(|f| f.tiles_used.len() == PAGE_TILES));
                assert!(set.frames.iter().all(|f| f.sprites.iter().all(|s| {
                    let t = usize::from(s.tile);
                    t < PAGE_TILES && (!s.big || t + 17 < PAGE_TILES)
                })));
                assert!(set.page(set.idle_frame).iter().any(|&b| b != 0));
                if character == Character::Vile {
                    assert_eq!(set.palette, palette(&rom, 0x51).unwrap());
                }
                if character == Character::Zero {
                    assert_eq!(set.frames.len(), 35);
                    assert_eq!(set.run_frames, [0, 1, 2, 3]);
                    assert!(set.frames[..4].iter().all(|f| f.sprites.len() == 16));
                }
            }
        }
    }
}
