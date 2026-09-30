//! Zero's sprites, read out of the X1 ROM. See `docs/zero.md`, "X1 Zero data".
//!
//! X1 has one Zero: the Highway-ending cutscene object (enemy id `$33`),
//! sprite set `$53` (35 layouts), graphics id `$51` (8448 B, compressed),
//! palette `$85:C0C0`, plus his dialogue portrait (set `$7A`, graphics `$71`).
//! The game decompresses the graphics into `$7F:xxxx` at scene load and then
//! streams 1 KiB pages from there into VRAM `$6400` (tiles `$40-$5F`) per
//! frame, like X. So a frame is a layout plus a 32-slot tile page.
//! Pure: ROM bytes in (copier header already stripped), data out.

/// LoROM bank:addr of the tables.
pub const SET_TABLE: u32 = 0x8D8000; // 3-byte pointer per sprite set (object +$16)
pub const GFX_TABLE: u32 = 0x86F69B; // 5 B per graphics id: size16, src16, bank
pub const ZERO_GFX: u8 = 0x51;
pub const ZERO_SET: u8 = 0x53;
pub const ZERO_DMA: u32 = 0x85A96E; // per-frame DMA lists (object +$31)
pub const ZERO_PALETTE: u32 = 0x85C0C0; // palette id $7B
pub const PORTRAIT_GFX: u8 = 0x71;
pub const PORTRAIT_SET: u8 = 0x7A;
pub const PORTRAIT_DMA: u32 = 0x85A8E8;
/// Decompressor (cartridge address; `$80:B271` in the listing).
pub const DECOMPRESS: u32 = 0x80B230;
/// First VRAM tile of the streamed page (word `$6400`).
pub const PAGE_TILE: u8 = 0x40;
pub const NO_TILE: u16 = u16::MAX;

/// One hardware sprite, relative to the object's origin (facing left, the
/// game's unflipped direction; mirror with `x' = -x - w`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sprite {
    pub x: i8,
    pub y: i8,
    /// 16x16 when set, else 8x8.
    pub big: bool,
    /// Page slot 0..31 (VRAM tile `$40 + tile`); a big sprite covers
    /// `tile, tile+1, tile+16, tile+17`.
    pub tile: u8,
    pub hflip: bool,
    pub vflip: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub name: &'static str,
    /// OAM order: the first sprite is drawn on top.
    pub sprites: Vec<Sprite>,
    /// 32 page slots -> index into `ZeroSet::tiles` (32 B each), [`NO_TILE`]
    /// where the frame's page leaves the slot stale.
    pub tiles_used: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZeroSet {
    /// 4bpp SNES planar tiles, 32 B each.
    pub tiles: Vec<u8>,
    /// BGR555.
    pub palette: [u16; 16],
    pub frames: Vec<Frame>,
}

impl ZeroSet {
    /// The 1 KiB VRAM page (tiles `$40-$5F`) the game streams for `frame`;
    /// stale slots are zero.
    pub fn page(&self, frame: usize) -> Vec<u8> {
        let mut p = vec![0u8; 1024];
        for (slot, &t) in self.frames[frame].tiles_used.iter().enumerate() {
            if t != NO_TILE {
                let s = usize::from(t) * 32;
                p[slot * 32..slot * 32 + 32].copy_from_slice(&self.tiles[s..s + 32]);
            }
        }
        p
    }
}

/// Set `$53` frames 0..34, named from the Highway cutscene's `+$17` sequence
/// (dash in 0-3, idle 4/5/6, shoot 7-11, beam out 13/16/17) and the rendered
/// sheet (`cargo xtask zerosheet`). 19-34 are the Sigma-stage scene (grabbing
/// Vile's ride armour, shocked, lying, getting up).
pub const X1_NAMES: [&str; 35] = [
    "dash 1", "dash 2", "dash 3", "dash 4", // 0-3, with dash dust
    "stand", "stand blink", "stand breathe", // 4-6
    "aim", "shoot charged 1", "shoot charged 2", "shoot charged 3", "shoot recoil", // 7-11
    "materialize 1", "materialize 2", "materialize 3", "materialize 4", // 12-15: beam in/out
    "beam land", "beam", // 16-17
    "turn", "fist", "arms up", "leap", // 18-21
    "clinging 1", "clinging 2", "shocked 1", "shocked 2", "shocked 3", "shocked 4", // 22-27
    "lying 1", "lying 2", "lying 3", "lying 4", // 28-31
    "kneel 1", "kneel 2", "kneel 3", // 32-34
];

pub const PORTRAIT_NAMES: [&str; 12] = [
    "portrait R talk 1", "portrait R talk 2", "portrait R talk 3", "portrait R talk 4",
    "portrait hurt", "portrait L talk 1", "portrait L talk 2", "portrait L talk 3", "portrait L talk 4",
    "portrait eyes shut 1", "portrait eyes shut 2", "portrait eyes shut 3",
];

fn off(addr: u32) -> usize {
    (((addr >> 16) & 0x7F) << 15 | (addr & 0x7FFF)) as usize
}

fn rd(rom: &[u8], addr: u32) -> Result<u8, String> {
    rom.get(off(addr)).copied().ok_or_else(|| format!("${addr:06X} past the end of the ROM"))
}

fn rw(rom: &[u8], addr: u32) -> Result<u16, String> {
    Ok(u16::from(rd(rom, addr)?) | u16::from(rd(rom, addr + 1)?) << 8)
}

fn rl(rom: &[u8], addr: u32) -> Result<u32, String> {
    Ok(u32::from(rw(rom, addr)?) | u32::from(rd(rom, addr + 2)?) << 16)
}

/// The `$80:B230` decompressor: per 8 output bytes, a control byte and a fill
/// byte; each control bit (MSB first) set = copy the next literal, clear =
/// the fill byte. The source crosses banks at `$xx:FFFF -> $xx+1:8000`.
pub fn decompress(rom: &[u8], id: u8) -> Result<Vec<u8>, String> {
    let e = GFX_TABLE + 5 * u32::from(id);
    let size = usize::from(rw(rom, e)?);
    let mut a = u32::from(rw(rom, e + 2)?) | u32::from(rd(rom, e + 4)?) << 16;
    let mut next = || -> Result<u8, String> {
        let v = rd(rom, a)?;
        a += 1;
        if a & 0xFFFF == 0 {
            a += 0x8000;
        }
        Ok(v)
    };
    let mut out = Vec::with_capacity(size + 7);
    for _ in 0..size.div_ceil(8) {
        let (mut c, fill) = (next()?, next()?);
        for _ in 0..8 {
            out.push(if c & 0x80 != 0 { next()? } else { fill });
            c <<= 1;
        }
    }
    out.truncate(size);
    Ok(out)
}

/// Page slots loaded by frame `f`'s DMA list: `(slot, byte offset in the
/// decompressed graphics)`. Entries are 5 B: `len/16, src16, bank, dest_hi`
/// (bit 7 = last); dest is `(dest_hi & $7F) << 8` words. Empty = keep the page.
fn dma_slots(rom: &[u8], list: u32, f: usize) -> Result<Vec<(usize, usize)>, String> {
    let mut x = (list & 0xFF0000) | (list + u32::from(rw(rom, list + 2 * f as u32)?)) & 0xFFFF;
    let mut slots = Vec::new();
    loop {
        let n = usize::from(rd(rom, x)?) * 16;
        if n == 0 {
            break;
        }
        let src = usize::from(rw(rom, x + 1)?);
        let dest = usize::from(rd(rom, x + 4)?);
        let first = ((dest & 0x7F) << 8).checked_sub(0x6400).ok_or("DMA below $6400")? / 16;
        for k in 0..n / 32 {
            slots.push((first + k, src + 32 * k));
        }
        if dest & 0x80 != 0 {
            break;
        }
        x += 5;
    }
    Ok(slots)
}

/// Layouts of `set` with pages from `list`; tile indices offset by `base`.
fn frames(rom: &[u8], set: u8, list: u32, names: &[&'static str], base: u16) -> Result<Vec<Frame>, String> {
    let table = rl(rom, SET_TABLE + 3 * u32::from(set))?;
    let mut page = vec![NO_TILE; 32];
    let mut out = Vec::new();
    for (f, &name) in names.iter().enumerate() {
        let slots = dma_slots(rom, list, f)?;
        if !slots.is_empty() {
            page = vec![NO_TILE; 32];
            for (s, b) in slots {
                *page.get_mut(s).ok_or("DMA past the page")? = base + (b / 32) as u16;
            }
        }
        let lp = rl(rom, table + 3 * f as u32)?;
        let n = u32::from(rd(rom, lp)?);
        let sprites = (0..n)
            .map(|i| {
                let e = lp + 1 + 4 * i;
                let (t, a) = (rd(rom, e + 2)?, rd(rom, e + 3)?);
                Ok(Sprite {
                    x: rd(rom, e)? as i8,
                    y: rd(rom, e + 1)? as i8,
                    big: a & 0x20 != 0,
                    tile: t.checked_sub(PAGE_TILE).ok_or(format!("set {set:02X} frame {f}: tile {t:02X} below the page"))?,
                    hflip: a & 0x40 != 0,
                    vflip: a & 0x80 != 0,
                })
            })
            .collect::<Result<_, String>>()?;
        out.push(Frame { name, sprites, tiles_used: page.clone() });
    }
    Ok(out)
}

/// Zero's cutscene set (35 frames) then his 12 portrait frames.
pub fn extract_x1(rom: &[u8]) -> Result<ZeroSet, String> {
    if rom.get(0x7FC0..0x7FC9) != Some(b"MEGAMAN X".as_slice()) {
        return Err("not Mega Man X (header title at $7FC0)".into());
    }
    let mut tiles = decompress(rom, ZERO_GFX)?;
    let mut frames = frames(rom, ZERO_SET, ZERO_DMA, &X1_NAMES, 0)?;
    let base = (tiles.len() / 32) as u16;
    tiles.extend(decompress(rom, PORTRAIT_GFX)?);
    tiles.resize(tiles.len().div_ceil(32) * 32, 0);
    frames.extend(self::frames(rom, PORTRAIT_SET, PORTRAIT_DMA, &PORTRAIT_NAMES, base)?);
    let palette = std::array::from_fn(|i| rw(rom, ZERO_PALETTE + 2 * i as u32).unwrap_or(0));
    Ok(ZeroSet { tiles, palette, frames })
}

/// X action (`+$02`) and frame (`+$17 & $7F`) -> the X1 Zero frame to draw,
/// `None` where X1's Zero has no such pose: the caller draws X's frame in
/// Zero's colours (`zero::palette_for`). X frames measured on Octopus/Highway:
/// stand 0-2, stand shoot 49-50, walk 13-22 (shooting 3-12), jump 32-34
/// (shooting 23-25), fall 35-36 (26-27), land 37-38, hurt 51-53, ready 81.
/// Fallback (None): walk and walk-shoot, wall kick `$10`, wall slide `$12`,
/// and every action not listed.
pub fn zero_frame_for(x_action: u8, x_frame: u8) -> Option<usize> {
    match (x_action, x_frame) {
        (0x00 | 0x02, 1) => Some(5),
        (0x00 | 0x02, 2) => Some(6),
        (0x00 | 0x02, 49) => Some(11), // stand shoot: recoil, then aim
        (0x00 | 0x02, 50) => Some(7),
        (0x00 | 0x02, _) => Some(4),
        (0x06 | 0x08, _) => Some(21), // one airborne pose: the leap
        (0x0A, _) => Some(34),        // land: kneel
        (0x0C, f) => Some(28 + usize::from(f) % 4), // death: lying
        (0x0E, f) => Some(24 + usize::from(f) % 4), // hurt: shocked
        (0x22, _) => Some(17),        // beam-in: the beam
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_data_decompress_format() {
        // Synthetic ROM: GFX_TABLE entry for id 0 -> 9 bytes at $81:8000.
        let mut rom = vec![0u8; 0x10000];
        let e = off(GFX_TABLE);
        rom.resize(e + 8, 0);
        rom[e..e + 5].copy_from_slice(&[9, 0, 0x00, 0x80, 0x81]);
        let s = off(0x818000);
        // ctrl $A0 fill $11: lit, fill, lit, fill x5; ctrl $80 fill $22: lit, fill...
        rom[s..s + 8].copy_from_slice(&[0xA0, 0x11, 0xAA, 0xBB, 0x80, 0x22, 0xCC, 0]);
        let d = decompress(&rom, 0).unwrap();
        assert_eq!(d, [0xAA, 0x11, 0xBB, 0x11, 0x11, 0x11, 0x11, 0x11, 0xCC]);
    }

    #[test]
    fn zero_data_mapping() {
        assert_eq!(zero_frame_for(0x00, 0).map(|f| X1_NAMES[f]), Some("stand"));
        assert_eq!(zero_frame_for(0x06, 32).map(|f| X1_NAMES[f]), Some("leap"));
        assert_eq!(zero_frame_for(0x04, 13), None);
        assert_eq!(zero_frame_for(0x12, 0), None);
        for a in (0..0x48).step_by(2) {
            for f in 0..0x80 {
                assert!(zero_frame_for(a, f).is_none_or(|i| i < X1_NAMES.len()));
            }
        }
    }

    fn real() -> Option<(Vec<u8>, crate::Game)> {
        let path = std::env::var("MMX_ROM").ok()?;
        let bytes = std::fs::read(path).expect("read MMX_ROM");
        let g = crate::Game::from_rom(bytes).expect("load ROM");
        Some((g.rom_bytes().to_vec(), g))
    }

    #[test]
    fn zero_data_extract_x1() {
        let Some((rom, mut g)) = real() else {
            eprintln!("MMX_ROM not set; skipping");
            return;
        };
        let z = extract_x1(&rom).unwrap();
        assert_eq!(z.frames.len(), 35 + 12);
        assert_eq!(z.tiles.len() / 32, 264 + 41);
        assert_eq!(z.palette[7], 0x055E);
        assert!(z.frames.iter().all(|f| f.sprites.iter().all(|s| {
            let n = if s.big { [0, 1, 16, 17].as_slice() } else { &[0] };
            n.iter().all(|d| f.tiles_used[usize::from(s.tile) + d] != NO_TILE)
        })));
        // decode(ROM) == VRAM: the cutscene has Zero standing (frame 4's page)
        // in VRAM $6400. State made by `xtask dump --load highway-vile.state`,
        // see docs/zero.md.
        let state = std::env::var("MMX_ZERO_STATE")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/zero-scene.state").into());
        let Ok(s) = std::fs::read(&state) else {
            eprintln!("{state} missing; VRAM check skipped");
            return;
        };
        g.load_state(&s).unwrap();
        let vram: Vec<u8> = g.vram()[0x6400..0x6600].iter().flat_map(|w| w.to_le_bytes()).collect();
        let stand = X1_NAMES.iter().position(|&n| n == "stand").unwrap();
        assert_eq!(z.page(stand), vram);
    }
}
