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
    /// Page slot: 0..31 in X1 (VRAM tile `$40 + tile`), 0..47 in X3 (VRAM
    /// tile `tile`); a big sprite covers `tile, tile+1, tile+16, tile+17`.
    pub tile: u8,
    pub hflip: bool,
    pub vflip: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub name: &'static str,
    /// OAM order: the first sprite is drawn on top.
    pub sprites: Vec<Sprite>,
    /// Page slots (32 in X1, 48 in X3) -> index into `ZeroSet::tiles` (32 B each), [`NO_TILE`]
    /// where the frame's page leaves the slot stale.
    pub tiles_used: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ZeroSet {
    /// 4bpp SNES planar tiles, 32 B each.
    pub tiles: Vec<u8>,
    /// BGR555.
    pub palette: [u16; 16],
    pub frames: Vec<Frame>,
    /// X3's saber wave (empty for X1). See [`extract_x3_wave`].
    pub wave: WaveSet,
    /// X3's green saber slash arc (`None` for X1). See [`extract_x3_slash`].
    pub slash: Option<WaveSet>,
    /// X3's saber sound (`None` for X1). See [`extract_x3_sounds`].
    pub sounds: Option<ZeroSounds>,
}

/// An X3 effect set drawn next to Zero: the saber wave (2-row pages,
/// `tiles_used` has 32 slots) or the slash arc (48 slots, columns 0-7);
/// layout tiles are rebased to the page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WaveSet {
    pub tiles: Vec<u8>,
    pub palette: [u16; 16],
    pub frames: Vec<Frame>,
}

impl ZeroSet {
    /// The VRAM page the game streams for `frame`, 32 B per slot: 1 KiB for
    /// X1 (tiles `$40-$5F`), 1.5 KiB for X3 (tiles `$00-$2F`); stale slots are zero.
    pub fn page(&self, frame: usize) -> Vec<u8> {
        page_of(&self.tiles, &self.frames[frame])
    }
}

fn page_of(tiles: &[u8], f: &Frame) -> Vec<u8> {
    let mut p = vec![0u8; f.tiles_used.len() * 32];
    for (slot, &t) in f.tiles_used.iter().enumerate() {
        if t != NO_TILE {
            let s = usize::from(t) * 32;
            p[slot * 32..slot * 32 + 32].copy_from_slice(&tiles[s..s + 32]);
        }
    }
    p
}

impl WaveSet {
    /// The wave's 32-slot page for `frame`, colours remapped through `map`
    /// (wave index -> the wearer's palette index). Pure.
    pub fn page(&self, frame: usize, map: &[u8; 16]) -> Vec<u8> {
        let mut p = page_of(&self.tiles, &self.frames[frame]);
        for t in p.chunks_exact_mut(32) {
            remap_tile(t, map);
        }
        p
    }
}

/// Recolour one 4bpp planar tile in place: pixel value `v` -> `map[v]`. Pure.
pub fn remap_tile(t: &mut [u8], map: &[u8; 16]) {
    let mut out = [0u8; 32];
    for r in 0..8 {
        for bit in 0..8 {
            let v = (t[2 * r] >> bit & 1) | (t[2 * r + 1] >> bit & 1) << 1 | (t[16 + 2 * r] >> bit & 1) << 2 | (t[17 + 2 * r] >> bit & 1) << 3;
            let m = map[usize::from(v)];
            for (k, o) in [2 * r, 2 * r + 1, 16 + 2 * r, 17 + 2 * r].into_iter().enumerate() {
                out[o] |= (m >> k & 1) << bit;
            }
        }
    }
    t[..32].copy_from_slice(&out);
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
    Ok(ZeroSet { tiles, palette, frames, wave: WaveSet::default(), slash: None, sounds: None })
}

/// X action (`+$02`) and frame (`+$17 & $7F`) -> the X1 Zero frame to draw,
/// `None` where X1's Zero has no such pose: the caller draws X's frame in
/// Zero's colours (`zero::palette_for`). X frames measured on Octopus/Highway:
/// stand 0-2, stand shoot 49-50, walk 3-12 (shooting 13-22), jump 23-25
/// (shooting 32-34), fall 25-27 (34-36), land 28-29 (37-38), hurt 51-53, ready 81.
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

// ---------------------------------------------------------------------------
// X3: playable Zero. See docs/zero.md, "X3 Zero data".

/// Header title and CRC32 of Mega Man X3 (USA), headerless 2 MiB.
pub const X3_TITLE: &[u8] = b"MEGAMAN X3";
pub const X3_CRC32: u32 = 0xFA0F_E671;
/// Zero's sprite set (object `+$16`), 117 layouts through `$8D:8000`.
pub const X3_SET: u8 = 0x4A;
/// Zero's per-frame DMA lists for set `$4A` (object `+$31`, bank `$85`).
pub const X3_DMA: u32 = 0x85D6A8;
/// Set `$4B` (21 layouts): Zero's death and event poses, lists `$85:DB47`.
pub const X3_SET2: u8 = 0x4B;
pub const X3_DMA2: u32 = 0x85DB47;
/// Zero's in-game palette (sprite palette 1 while playing him).
pub const X3_PALETTE: u32 = 0x8CB580;
/// X3 streams up to three 16-tile rows (VRAM `$6000-$62FF`, tiles `$00-$2F`).
pub const X3_PAGE_TILES: usize = 48;

/// Set `$4A` frames 0..116, then set `$4B` frames 0..20 (index 117..137).
/// Named from play (`+$02` action / `+$17` frame logs, docs/zero.md) and the
/// rendered sheet (`cargo xtask zerosheet --x3`).
pub const X3_NAMES: [&str; 138] = [
    "stand", "stand blink 1", "stand blink 2", // 0-2
    "walk 1", "walk 2", "walk 3", "walk 4", "walk 5", "walk 6", "walk 7", "walk 8", "walk 9", "walk 10", // 3-12
    "walk shoot 1", "walk shoot 2", "walk shoot 3", "walk shoot 4", "walk shoot 5", // 13-17
    "walk shoot 6", "walk shoot 7", "walk shoot 8", "walk shoot 9", "walk shoot 10", // 18-22
    "jump 1", "jump 2", "jump 3", "fall 1", "fall 2", "land 1", "land 2", // 23-29
    "stand weak 1", "stand weak 2", // 30-31: unverified (not seen in play)
    "jump shoot 1", "jump shoot 2", "jump shoot 3", "fall shoot 1", "fall shoot 2", // 32-36
    "land shoot 1", "land shoot 2", // 37-38
    "wall slide 1", "wall slide 2", "wall slide 3", "wall kick 1", "wall kick 2", // 39-43
    "wall slide shoot 1", "wall slide shoot 2", "wall slide shoot 3", // 44-46
    "wall kick shoot 1", "wall kick shoot 2", // 47-48
    "stand shoot 1", "stand shoot 2", "hurt 1", "hurt 2", "hurt flash", // 49-53
    "dash start", "dash 1", "dash 3", "dash end", "dash shoot 1", "dash shoot 3", // 54-59
    "beam", "beam land", // 60-61
    "materialize 1", "materialize 2", "materialize 3", "materialize 4", "materialize 5", // 62-66
    "ladder grab", "ladder climb 1", "ladder climb 2", "ladder climb 3", "ladder climb 4", // 67-71: unverified
    "ladder top 1", "ladder top 2", "ladder shoot", // 72-74: unverified
    "blank 1", "blank 2", "hurt alt 1", "hurt alt 2", "hurt alt flash", "blank 3", // 75-80
    "ready", // 81
    "charged shot 1", "charged shot 2", "charged shot 3", "charged shot 4", // 82-85: act $4A
    "saber 1", "saber 2", "saber 3", "saber 4", "saber 5", // 86-90: act $4C, 88 = the arc
    "air charged shot 1", "air charged shot 2", "air charged shot 3", "air charged shot 4", // 91-94: act $4E
    "air saber 1", "air saber 2", "air saber 3", "air saber 4", // 95-98: act $50
    "stand alt", // 99
    "jump apex", "fall 3", "jump shoot apex", "fall shoot 3", // 100-103
    "dash 2", "dash 4", "dash 5", "dash 6", // 104-107
    "dash shoot 2", "dash shoot 4", "dash shoot 5", "dash shoot 6", // 108-111
    "charged shot alt", "saber alt 1", "air charged shot alt", "saber alt 2", "air saber alt", // 112-116: unverified
    // set $4B
    "event stand 1", "event stand 2", "event stand 3", "event stand 4", // 117-120
    "event point 1", "event point 2", "event point 3", // 121-123
    "event leap 1", "event leap 2", "event leap 3", "event leap 4", "event leap 5", // 124-128
    "event pose 1", "event pose 2", // 129-130
    "event kneel 1", "event kneel 2", "event kneel 3", "event kneel 4", // 131-134
    "death", "death 2", "death pieces", // 135-137: act $0C shows 135 (set $4B frame 18)
];

/// Page slots loaded by frame `f` of the list table `list`: 6-byte entries
/// `len/16, src24, dest16` (dest bit 15 = last), dest `$6000 + 16 * slot`
/// words. Offsets wrap inside bank `$85`. Returns `(slot, rom address)`;
/// empty = keep the page.
fn x3_dma(rom: &[u8], list: u32, f: usize) -> Result<Vec<(usize, u32)>, String> {
    x3_dma_at(rom, list, f, 0x6000)
}

/// [`x3_dma`] with the page starting at VRAM word `base`.
fn x3_dma_at(rom: &[u8], list: u32, f: usize, base: u16) -> Result<Vec<(usize, u32)>, String> {
    let mut x = (list & 0xFF0000) | (list + u32::from(rw(rom, list + 2 * f as u32)?)) & 0xFFFF;
    let mut slots = Vec::new();
    loop {
        let n = u32::from(rd(rom, x)?) * 16;
        if n == 0 {
            break;
        }
        let src = rl(rom, x + 1)?;
        let dest = rw(rom, x + 4)?;
        let first = usize::from((dest & 0x7FFF).checked_sub(base).ok_or("X3 DMA below the page")? / 16);
        for k in 0..n / 32 {
            slots.push((first + k as usize, src + 32 * k));
        }
        if dest & 0x8000 != 0 {
            break;
        }
        x += 6;
    }
    Ok(slots)
}

/// X3 layouts: count, then 4 B per sprite `attr, x, y, tile` (X1 is
/// `x, y, tile, attr`); attr bits as X1. Pointers are `$0D:xxxx`/`$0F:xxxx`
/// (mirrors of `$8D`/`$8F`).
fn x3_layout(rom: &[u8], set: u8, f: usize) -> Result<Vec<Sprite>, String> {
    let table = rl(rom, SET_TABLE + 3 * u32::from(set))?;
    let lp = rl(rom, table + 3 * f as u32)?;
    (0..u32::from(rd(rom, lp)?))
        .map(|i| {
            let e = lp + 1 + 4 * i;
            let a = rd(rom, e)?;
            Ok(Sprite {
                x: rd(rom, e + 1)? as i8,
                y: rd(rom, e + 2)? as i8,
                big: a & 0x20 != 0,
                tile: rd(rom, e + 3)?,
                hflip: a & 0x40 != 0,
                vflip: a & 0x80 != 0,
            })
        })
        .collect()
}

/// Playable Zero from Mega Man X3 (USA): set `$4A` (117 frames) and set
/// `$4B` (21), tiles read raw from ROM (banks `$B0/$B1`, deduplicated),
/// 48-slot pages (`X3_PAGE_TILES`).
pub fn extract_x3(rom: &[u8]) -> Result<ZeroSet, String> {
    if rom.get(0x7FC0..0x7FCA) != Some(X3_TITLE) {
        return Err("not Mega Man X3 (header title at $7FC0)".into());
    }
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(rom);
    if crc != X3_CRC32 {
        return Err(format!("Mega Man X3 CRC32 {crc:08X}, want {X3_CRC32:08X} (USA)"));
    }
    let mut tiles = Vec::new();
    let mut index = std::collections::HashMap::new();
    let mut frames = Vec::new();
    for (set, list, names) in [(X3_SET, X3_DMA, &X3_NAMES[..117]), (X3_SET2, X3_DMA2, &X3_NAMES[117..])] {
        let mut page = vec![NO_TILE; X3_PAGE_TILES];
        for (f, &name) in names.iter().enumerate() {
            let slots = x3_dma(rom, list, f)?;
            if !slots.is_empty() {
                page = vec![NO_TILE; X3_PAGE_TILES];
            }
            for (s, a) in slots {
                let t = match index.get(&a) {
                    Some(&t) => t,
                    None => {
                        for k in 0..32 {
                            tiles.push(rd(rom, a + k)?);
                        }
                        let t = (tiles.len() / 32 - 1) as u16;
                        index.insert(a, t);
                        t
                    }
                };
                *page.get_mut(s).ok_or(format!("set {set:02X} frame {f}: DMA past the page"))? = t;
            }
            let sprites = x3_layout(rom, set, f)?;
            if let Some(s) = sprites.iter().find(|s| usize::from(s.tile) + if s.big { 17 } else { 0 } >= X3_PAGE_TILES) {
                return Err(format!("set {set:02X} frame {f}: tile {:02X} past the page", s.tile));
            }
            frames.push(Frame { name, sprites, tiles_used: page.clone() });
        }
    }
    let palette = std::array::from_fn(|i| rw(rom, X3_PALETTE + 2 * i as u32).unwrap_or(0));
    Ok(ZeroSet {
        tiles,
        palette,
        frames,
        wave: extract_x3_wave(rom)?,
        slash: Some(extract_x3_slash(rom)?),
        sounds: Some(extract_x3_sounds(rom)?),
    })
}

/// The saber wave: X3 object set `$4D` (spawned by Zero's charged attack,
/// act `$4A`/`$4E`, on the blade pose 84/93), lists `$85:DD84`, palette
/// `$8C:B5A0` (X3's sprite palette 3). Streamed like Zero: VRAM `$6300`/`$6400`
/// + `+18 = 8` tiles, layout tiles `$30-$35`/`$40-$45` (+ 8 in OAM).
pub const X3_WAVE_SET: u8 = 0x4D;
pub const X3_WAVE_DMA: u32 = 0x85DD84;
pub const X3_WAVE_PALETTE: u32 = 0x8CB5A0;
/// Frames 0-4 are the flight (0 spawn, 1 unfurl, 2-4 loop); 5-6 were never seen.
pub const X3_WAVE_FRAMES: usize = 5;
/// Two rows of 16 (only columns 0-5 are used).
pub const X3_WAVE_PAGE_TILES: usize = 32;

/// See [`X3_WAVE_SET`]. Pure (ROM bytes in; caller checks it is X3).
pub fn extract_x3_wave(rom: &[u8]) -> Result<WaveSet, String> {
    let mut tiles = Vec::new();
    let mut frames = Vec::new();
    let mut page = vec![NO_TILE; X3_WAVE_PAGE_TILES];
    for f in 0..X3_WAVE_FRAMES {
        let slots = x3_dma_at(rom, X3_WAVE_DMA, f, 0x6300)?;
        if !slots.is_empty() {
            page = vec![NO_TILE; X3_WAVE_PAGE_TILES];
        }
        for (s, a) in slots {
            for k in 0..32 {
                tiles.push(rd(rom, a + k)?);
            }
            *page.get_mut(s).ok_or(format!("wave frame {f}: DMA past the page"))? = (tiles.len() / 32 - 1) as u16;
        }
        let mut sprites = x3_layout(rom, X3_WAVE_SET, f)?;
        for s in &mut sprites {
            s.tile = s.tile.checked_sub(0x30).filter(|&t| usize::from(t) + if s.big { 17 } else { 0 } < X3_WAVE_PAGE_TILES).ok_or(format!("wave frame {f}: tile {:02X}", s.tile))?;
        }
        frames.push(Frame { name: "wave", sprites, tiles_used: page.clone() });
    }
    let palette = std::array::from_fn(|i| rw(rom, X3_WAVE_PALETTE + 2 * i as u32).unwrap_or(0));
    Ok(WaveSet { tiles, palette, frames })
}

/// The green slash arc: X3 set `$4F` (X's beam-saber slash; Zero's own
/// swing in X3 has no arc, only the blue blade in his body frames), lists
/// `$85:E5EE`, palette `$8C:A460` (the only `$8C` palette with greens in 1-5,
/// the colours the set draws with; X3's sprite palette 3 while X plays).
/// Frames 0-5 ground swing (2, 3 = the arc), 6-11 the same 5 px further
/// forward and 2 up (air), 12-15 X's flying wave (not used). Drawn at the
/// wearer's origin, it hugs Zero's arc pose 88 / 97.
pub const X3_SLASH_SET: u8 = 0x4F;
pub const X3_SLASH_DMA: u32 = 0x85E5EE;
pub const X3_SLASH_PALETTE: u32 = 0x8CA460;
pub const X3_SLASH_FRAMES: usize = 12;

/// See [`X3_SLASH_SET`]. X3 streams up to four rows (tiles `$30-$6F`);
/// each frame is repacked into 8 columns x 3 rows of a 48-slot page (16x16
/// sprites in rows 0-1, 8x8 ones in the cells left), so it fits half of a
/// shadow page. Pure.
pub fn extract_x3_slash(rom: &[u8]) -> Result<WaveSet, String> {
    let mut tiles = Vec::new();
    let mut frames = Vec::new();
    let mut raw = vec![None; 64]; // streamed slot (tile - $30) -> ROM address
    for f in 0..X3_SLASH_FRAMES {
        let slots = x3_dma_at(rom, X3_SLASH_DMA, f, 0x6300)?;
        if !slots.is_empty() {
            raw = vec![None; 64];
        }
        for (s, a) in slots {
            *raw.get_mut(s).ok_or(format!("slash frame {f}: DMA past row 3"))? = Some(a);
        }
        let mut page = vec![NO_TILE; X3_PAGE_TILES];
        let mut free: Vec<usize> = (0..X3_PAGE_TILES).filter(|s| s % 16 < 8).collect();
        let mut sprites = x3_layout(rom, X3_SLASH_SET, f)?;
        // Big ones first: they need a 2x2 block in rows 0-1.
        let mut order: Vec<usize> = (0..sprites.len()).collect();
        order.sort_by_key(|&i| !sprites[i].big);
        for i in order {
            let s = &mut sprites[i];
            let src = s.tile.checked_sub(0x30).ok_or(format!("slash frame {f}: tile {:02X}", s.tile))?;
            let (d, at): (&[usize], usize) = if s.big {
                let at = *free.iter().find(|&&t| t < 16 && [1, 16, 17].iter().all(|k| free.contains(&(t + k)))).ok_or(format!("slash frame {f}: no room"))?;
                (&[0, 1, 16, 17], at)
            } else {
                (&[0], *free.first().ok_or(format!("slash frame {f}: no room"))?)
            };
            for &k in d {
                free.retain(|&t| t != at + k);
                let a = raw.get(usize::from(src) + k).copied().flatten();
                if let Some(a) = a {
                    tiles.extend((0..32).map(|b| rd(rom, a + b)).collect::<Result<Vec<_>, _>>()?);
                    page[at + k] = (tiles.len() / 32 - 1) as u16;
                }
            }
            s.tile = at as u8;
        }
        frames.push(Frame { name: "slash", sprites, tiles_used: page });
    }
    let palette = std::array::from_fn(|i| rw(rom, X3_SLASH_PALETTE + 2 * i as u32).unwrap_or(0));
    Ok(WaveSet { tiles, palette, frames })
}

/// A hit box `(x, y, half w, half h)`, x toward the facing.
pub type HitBox = (i8, i8, u8, u8);

/// The wave's flight, measured in X3 (`states/x3-zero-stand.state`, charge,
/// release; docs/zero.md "The saber"): frame `t` (0 = spawn) -> `(dx, dy,
/// frame, box)`, offsets from Zero's origin at the spawn (x toward his
/// facing), `box` = X3's `+20` hit box `(x, y, half w, half h)` (x toward
/// the facing). On the ground it drops 3 px a frame to `+1`; in the air it
/// keeps its height. From `t = 4` it moves 6 px a frame, frames 3, 4, 2, ...
/// Pure.
pub fn x3_wave_step(t: u8, air: bool) -> (i16, i16, usize, HitBox) {
    let t = i16::from(t);
    let reach = if air { 27 } else { 29 };
    let dx = if t == 0 { 8 } else { reach + 6 * (t - 3).max(0) };
    let dy = if air { -8 } else { (-8 + 3 * t).min(1) };
    let frame = match t {
        0 | 1 => 0,
        2 | 3 => 1,
        _ => [3, 4, 2][((t - 4) % 3) as usize],
    };
    let bx = match t {
        0 => (0, 0, 4, 4),     // $B7FB
        1 => (1, 0, 16, 9),    // $B807 (x -1 facing left)
        _ => (15, 0, 16, 9),   // $B80B / $B80F
    };
    (dx, dy, frame, bx)
}

/// X1 action (`+$02`) and frame (`+$17 & $7F`) -> X3 Zero frame
/// ([`X3_NAMES`] index). X1's set 0 and Zero's set `$4A` share the frame
/// numbering (measured on both ROMs): stand 0-2, walk 3-12 (shooting 13-22),
/// jump 23-25 (32-34), fall 25-27 (34-36), land 28-29 (37-38), wall slide
/// 39-41 (44-46), wall kick 42-43 (47-48), stand shoot 49-50, hurt 51-53,
/// beam-in 60-66, ready 81. So frames 0-74 and 81 map to themselves; death
/// is set `$4B` frame 18; an unknown frame falls back to the action's base pose.
pub fn zero_x3_frame_for(x_action: u8, x_frame: u8) -> Option<usize> {
    let f = usize::from(x_frame & 0x7F);
    if x_action == 0x0C {
        return Some(135);
    }
    if f <= 74 || f == 81 {
        return Some(f);
    }
    Some(match x_action {
        0x00 | 0x02 => 0,
        0x04 => 3,
        0x06 => 23,
        0x08 => 26,
        0x0A => 28,
        0x0E => 51,
        0x10 => 42,
        0x12 => 41,
        0x14 => 55,
        0x22 => 60,
        _ => return None,
    })
}

// ------------------------------------------------------------ X3 sounds

/// Mono PCM (`rate` Hz), envelope and pitch already applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sample {
    pub pcm: Vec<i16>,
    pub rate: u32,
}

/// Zero's X3 sounds. Only the saber whoosh: X3 plays the same hurt and death
/// sounds for X and Zero (docs/zero.md "Sounds").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ZeroSounds {
    pub swing: Sample,
}

/// X3's SPC sample `$0E` (DIR `$3900`: start `$415A`, loop `$4190`): 8 BRR
/// blocks at `$89:AF25` (file `$4AF25`), looping on the last 2.
pub const X3_WHOOSH_BRR: u32 = 0x89AF25;
pub const X3_WHOOSH_BLOCKS: usize = 8;
pub const X3_WHOOSH_LOOP_BLOCK: usize = 6;
/// Its voice's `PITCH` per NTSC frame from key-on (`xtask dump --apu`, the
/// ball after Zero's swing, `x3-zero-stand` + charge + Y at 215), then key-off.
pub const X3_WHOOSH_PITCH: [u16; 42] = [
    0x02A0, 0x0330, 0x0480, 0x0540, 0x0670, 0x0A70, 0x0AA0, 0x0930, 0x0720, 0x05F0, 0x0610, 0x0630, 0x05E0, 0x0490,
    0x0390, 0x0380, 0x03D0, 0x0380, 0x0260, 0x0270, 0x0230, 0x0220, 0x01C0, 0x0160, 0x0140, 0x0140, 0x0150, 0x00C0,
    0x0090, 0x0090, 0x0090, 0x0090, 0x0080, 0x0060, 0x0050, 0x0050, 0x0060, 0x0040, 0x0030, 0x0030, 0x0030, 0x0030,
];
/// The DSP's output rate; `PITCH` `$1000` = one BRR sample per output sample.
pub const DSP_RATE: u32 = 32_000;

/// BRR blocks (9 bytes: header `ssss ffle`, 16 nibbles high first) -> samples,
/// the vendored DSP's shift and filter math (15-bit results).
pub fn brr_decode(data: &[u8]) -> Vec<i16> {
    let (mut p1, mut p2) = (0i32, 0i32);
    let mut out = Vec::with_capacity(data.len() / 9 * 16);
    for b in data.chunks_exact(9) {
        let (shift, filter) = (b[0] >> 4, (b[0] >> 2) & 3);
        for i in 0..16 {
            let n = i32::from(if i % 2 == 0 { (b[1 + i / 2] as i8) >> 4 } else { ((b[1 + i / 2] as i8) << 4) >> 4 });
            let s = match shift {
                0 => n >> 1,
                1..=12 => n << (shift - 1),
                _ if n < 0 => -2048,
                _ => 0,
            };
            let s = match filter {
                0 => s,
                1 => s + p1 + (-p1 >> 4),
                2 => s + (p1 << 1) + (-(3 * p1) >> 5) - p2 + (p2 >> 4),
                _ => s + (p1 << 1) + (-(13 * p1) >> 6) - p2 + ((3 * p2) >> 4),
            };
            let s = i32::from(((s.clamp(-32768, 32767) as i16) << 1) >> 1);
            out.push(s as i16);
            (p2, p1) = (p1, s);
        }
    }
    out
}

/// Play `brr` (looping from sample `lp`) on one voice at `pitch[f]` during
/// NTSC frame `f` (the DSP's 12-bit fraction; linear interpolation and a
/// `[1 2 1] / 4` smoothing instead of its Gaussian), full envelope, then the
/// driver's key-off (GAIN `$BF`, 64 samples to silence). `gain` in 1/256.
/// Integer math: both netplay peers get the same samples.
fn render(brr: &[i16], lp: usize, pitch: &[u16], gain: i32) -> Vec<i16> {
    const RELEASE: usize = 64;
    let n = (pitch.len() as u64 * u64::from(DSP_RATE) * 10_000 / 600_988) as usize;
    let mut out = Vec::<i32>::with_capacity(n + RELEASE);
    let mut pos = 0u32; // 1/4096 sample
    for k in 0..n + RELEASE {
        let f = (k as u64 * 600_988 / (u64::from(DSP_RATE) * 10_000)) as usize;
        let (i, frac) = ((pos >> 12) as usize, (pos & 0xFFF) as i32);
        let (a, b) = (i32::from(brr[i]), i32::from(brr[if i + 1 < brr.len() { i + 1 } else { lp }]));
        let env = if k < n { 256 } else { (256 * (n + RELEASE - 1 - k) / RELEASE) as i32 };
        out.push((a + ((b - a) * frac >> 12)) * gain / 256 * env / 256);
        pos += u32::from(pitch[f.min(pitch.len() - 1)]);
        while (pos >> 12) as usize >= brr.len() {
            pos -= ((brr.len() - lp) as u32) << 12;
        }
    }
    let at = |k: usize| out.get(k).copied().unwrap_or(0);
    (0..out.len()).map(|k| ((at(k.wrapping_sub(1)) + 2 * at(k) + at(k + 1)) / 4) as i16).collect()
}

/// Zero's saber whoosh from a Mega Man X3 (USA) image (checked by
/// [`extract_x3`]): the SFX X3 plays on Zero's saber swing (the ball it
/// throws, sample `$0E` swept up and down, voice volume `$30`), at about
/// X3's output level.
pub fn extract_x3_sounds(rom: &[u8]) -> Result<ZeroSounds, String> {
    let at = off(X3_WHOOSH_BRR);
    let data = rom.get(at..at + 9 * X3_WHOOSH_BLOCKS).ok_or("whoosh sample past the end of the ROM")?;
    if data[9 * (X3_WHOOSH_BLOCKS - 1)] & 3 != 3 || data.chunks(9).rev().skip(1).any(|b| b[0] & 1 != 0) {
        return Err("whoosh sample: BRR end/loop flags not where expected".into());
    }
    let brr = brr_decode(data);
    let pcm = render(&brr, 16 * X3_WHOOSH_LOOP_BLOCK, &X3_WHOOSH_PITCH, 80);
    Ok(ZeroSounds { swing: Sample { pcm, rate: DSP_RATE } })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One block per filter, values worked by hand from the DSP formulas.
    #[test]
    fn brr_filters() {
        // shift 12, filter 0: nibbles 1, -1 -> 1 << 11 = 2048, -2048.
        let mut b = vec![0xC0, 0x1F, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(&brr_decode(&b)[..3], &[2048, -2048, 0]);
        // shift 0: n >> 1 (7 -> 3, -8 -> -4).
        b[0] = 0x00;
        b[1] = 0x78;
        assert_eq!(&brr_decode(&b)[..2], &[3, -4]);
        // Block 1 ends on 2048 (p1) after 0 (p2); block 2 is all-zero nibbles.
        // Filter 1: p1 - p1/16 = 1920, then 1920 - 120.
        let mut two = vec![0xC0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xC4, 0, 0, 0, 0, 0, 0, 0, 0];
        let d = brr_decode(&two);
        assert_eq!((d[15], d[16], d[17]), (2048, 1920, 1800));
        // Filter 2: 2 p1 - 3 p1 / 32 - p2 + p2 / 16 = 4096 - 192.
        two[9] = 0xC8;
        assert_eq!(brr_decode(&two)[16], 3904);
        // Filter 3: 2 p1 - 13 p1 / 64 - p2 + 3 p2 / 16 = 4096 - 416.
        two[9] = 0xCC;
        assert_eq!(brr_decode(&two)[16], 3680);
        // Clamp, then 15 bits: filter 2 on 14336 gives 41664 -> 32767 -> -1.
        let w = vec![0xC8, 0x77, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(&brr_decode(&w)[..2], &[14336, -1]);
    }

    /// The whoosh bytes in the ROM are the ones X3's SPC RAM holds under
    /// DIR entry `$0E` while Zero plays, and they render to ~0.7 s.
    #[test]
    fn x3_whoosh_matches_spc_ram() {
        let Some(path) = std::env::var_os("MMX_X3_ROM") else {
            eprintln!("MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/x3-zero-stand.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let rom = std::fs::read(path).unwrap();
        let mut g = crate::Game::from_rom_any(rom.clone()).unwrap();
        g.load_state(&state).unwrap();
        g.step(crate::Pads::default());
        let dir = usize::from(g.dsp_register(0x5D)) << 8;
        let spc = g.spc_ram();
        let e = |k: usize| usize::from(u16::from_le_bytes([spc[dir + 0x0E * 4 + k], spc[dir + 0x0E * 4 + k + 1]]));
        let (start, lp) = (e(0), e(2));
        let rom = if rom.len() % 0x8000 == 512 { &rom[512..] } else { &rom[..] };
        let at = off(X3_WHOOSH_BRR);
        assert_eq!(&spc[start..start + 9 * X3_WHOOSH_BLOCKS], &rom[at..at + 9 * X3_WHOOSH_BLOCKS]);
        assert_eq!(lp - start, 9 * X3_WHOOSH_LOOP_BLOCK);
        let s = extract_x3_sounds(rom).unwrap().swing;
        assert_eq!(s.rate, DSP_RATE);
        assert!((22_000..23_000).contains(&s.pcm.len()), "{}", s.pcm.len());
        assert!(s.pcm.iter().any(|&x| x.unsigned_abs() > 3000));
        assert_eq!(*s.pcm.last().unwrap(), 0);
    }

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

    #[test]
    fn zero_data_x3_mapping() {
        let name = |a, f| zero_x3_frame_for(a, f).map(|i| X3_NAMES[i]);
        assert_eq!(name(0x00, 0), Some("stand"));
        assert_eq!(name(0x04, 5), Some("walk 3"));
        assert_eq!(name(0x04, 15), Some("walk shoot 3"));
        assert_eq!(name(0x12, 41), Some("wall slide 3"));
        assert_eq!(name(0x10, 42), Some("wall kick 1"));
        assert_eq!(name(0x0C, 0), Some("death"));
        assert_eq!(name(0x22, 60), Some("beam"));
        for a in (0..0x52).step_by(2) {
            for f in 0..0x80 {
                assert!(zero_x3_frame_for(a, f).is_none_or(|i| i < X3_NAMES.len()));
            }
        }
    }

    #[test]
    fn zero_data_extract_x3() {
        let Some(path) = std::env::var_os("MMX_X3_ROM") else {
            eprintln!("MMX_X3_ROM not set; skipping");
            return;
        };
        let mut g = crate::Game::from_rom_any(std::fs::read(path).expect("read MMX_X3_ROM")).expect("load X3");
        let z = extract_x3(g.rom_bytes()).unwrap();
        assert_eq!(z.frames.len(), 117 + 21);
        assert_eq!(z.palette[0], 0x2188);
        // Every sprite's tiles are on its page, except three fragment frames
        // whose lists load fewer tiles than the layout names (drawn as holes).
        let partial: Vec<usize> = (0..z.frames.len())
            .filter(|&i| {
                let f = &z.frames[i];
                !f.sprites.iter().all(|s| {
                    let n = if s.big { [0, 1, 16, 17].as_slice() } else { &[0] };
                    n.iter().all(|d| f.tiles_used[usize::from(s.tile) + d] != NO_TILE)
                })
            })
            .collect();
        assert_eq!(partial, [113, 115, 137]);
        // decode(ROM) == VRAM $6000.. with Zero standing (frame 0's page).
        // State: docs/zero.md, "X3 Zero data".
        let state = std::env::var("MMX_X3_ZERO_STATE")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/x3-zero-stand.state").into());
        let Ok(s) = std::fs::read(&state) else {
            eprintln!("{state} missing; VRAM check skipped");
            return;
        };
        g.load_state(&s).unwrap();
        let vram: Vec<u8> = g.vram()[0x6000..0x6300].iter().flat_map(|w| w.to_le_bytes()).collect();
        let (page, used) = (z.page(0), &z.frames[0].tiles_used);
        for (slot, &t) in used.iter().enumerate() {
            if t != NO_TILE {
                assert_eq!(page[slot * 32..][..32], vram[slot * 32..][..32], "slot {slot:02X}");
            }
        }
        assert!(used.iter().filter(|&&t| t != NO_TILE).count() >= 25);
    }

    #[test]
    fn zero_data_x3_wave_step() {
        assert_eq!(x3_wave_step(0, false), (8, -8, 0, (0, 0, 4, 4)));
        assert_eq!(x3_wave_step(3, false).0, 29);
        assert_eq!(x3_wave_step(5, false), (41, 1, 4, (15, 0, 16, 9)));
        assert_eq!(x3_wave_step(5, true).1, -8);
        let id: [u8; 16] = std::array::from_fn(|i| i as u8);
        let mut t: [u8; 32] = std::array::from_fn(|i| (i * 37) as u8);
        let t0 = t;
        remap_tile(&mut t, &id);
        assert_eq!(t, t0);
    }

    #[test]
    fn zero_data_extract_x3_wave() {
        let Some(path) = std::env::var_os("MMX_X3_ROM") else {
            eprintln!("MMX_X3_ROM not set; skipping");
            return;
        };
        let mut g = crate::Game::from_rom_any(std::fs::read(path).expect("read MMX_X3_ROM")).expect("load X3");
        let w = extract_x3(g.rom_bytes()).unwrap().wave;
        assert_eq!(w.frames.len(), X3_WAVE_FRAMES);
        assert_eq!(w.palette[7], 0x338C);
        // decode(ROM) == VRAM: the wave in flight, frame 4's tiles at $6380/$6480
        // (`+18 = 8`). State: docs/zero.md "The saber".
        let state = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/x3-zero-wave.state");
        let Ok(s) = std::fs::read(state) else {
            eprintln!("{state} missing; VRAM check skipped");
            return;
        };
        g.load_state(&s).unwrap();
        let id: [u8; 16] = std::array::from_fn(|i| i as u8);
        let page = w.page(4, &id);
        let used = &w.frames[4].tiles_used;
        assert_eq!(used.iter().filter(|&&t| t != NO_TILE).count(), 10);
        for (slot, &t) in used.iter().enumerate() {
            if t != NO_TILE {
                let v = 0x6380 + (slot / 16) * 0x100 + (slot % 16) * 16;
                let vram: Vec<u8> = g.vram()[v..v + 16].iter().flat_map(|w| w.to_le_bytes()).collect();
                assert_eq!(page[slot * 32..][..32], vram[..], "slot {slot:02X}");
            }
        }
    }

    #[test]
    fn zero_data_extract_x3_slash() {
        let Some(path) = std::env::var_os("MMX_X3_ROM") else {
            eprintln!("MMX_X3_ROM not set; skipping");
            return;
        };
        let mut g = crate::Game::from_rom_any(std::fs::read(path).expect("read MMX_X3_ROM")).expect("load X3");
        let rom = g.rom_bytes().to_vec();
        let sl = extract_x3(&rom).unwrap().slash.unwrap();
        assert_eq!(sl.frames.len(), X3_SLASH_FRAMES);
        assert_eq!(sl.palette[3], 0x3B8E);
        // Every slot a sprite reads is filled, in columns 0-7.
        for f in &sl.frames {
            for s in &f.sprites {
                for d in if s.big { [0, 1, 16, 17].as_slice() } else { &[0] } {
                    let t = usize::from(s.tile) + d;
                    assert!(t % 16 < 8 && f.tiles_used[t] != NO_TILE, "{f:?}");
                }
            }
        }
        // decode(ROM) == VRAM: the game's own streamer and emitter showing set
        // $4F frame 3 (Zero's block poked to it; docs/zero.md "The saber").
        let state = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/x3-zero-slash.state");
        let Ok(st) = std::fs::read(state) else {
            eprintln!("{state} missing; VRAM check skipped");
            return;
        };
        g.load_state(&st).unwrap();
        let id: [u8; 16] = std::array::from_fn(|i| i as u8);
        let page = sl.page(3, &id);
        let orig = x3_layout(&rom, X3_SLASH_SET, 3).unwrap();
        assert_eq!(orig.len(), 10);
        for (s, o) in sl.frames[3].sprites.iter().zip(&orig) {
            assert_eq!((s.x, s.y, s.big), (o.x, o.y, o.big));
            for d in if s.big { [0, 1, 16, 17].as_slice() } else { &[0] } {
                let v = 0x6000 + (usize::from(o.tile) + d) * 16;
                let vram: Vec<u8> = g.vram()[v..v + 16].iter().flat_map(|w| w.to_le_bytes()).collect();
                assert_eq!(page[(usize::from(s.tile) + d) * 32..][..32], vram[..], "tile {:02X}", o.tile);
            }
        }
    }
}
