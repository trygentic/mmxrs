//! Every ROM routine / hook address the co-op, Zero and display traps use,
//! with a byte signature so a dump other than ours still finds them.
//!
//! The addresses were traced on the PAL (Europe) v1.0 dump (`$FFD9 = 2`,
//! sha256 `1c41bb11…`). Other releases move bank `$80` code by tens of bytes
//! (USA v1.1: `+$2B` around `$80:D1EF`, `+$20` around `$80:9A86`), so
//! [`HookMap::resolve`] checks each signature at the PAL address first and
//! otherwise searches the ROM for it (same bank first, then every bank). A
//! hook that is not found, or found more than once, is reported and co-op
//! is refused (`coop::register_coop_traps`). See `docs/rom-variants.md`.
//!
//! Signatures are hex bytes with `??` wildcards on operands that point into
//! ROM (`JSR`/`JSL`/`JMP` targets, long addresses into banks `$80+`,
//! absolute or immediate words `>= $8000`): those move between releases even
//! where the code itself is the same. `sig_offset` is where the signature
//! starts relative to the hook, so "the instruction after a `JSL`" is signed
//! from the `JSL` (offset -4), and a routine whose entry is a jump table is
//! signed further in (the pickups).

/// One hook: `name` (the report's), the PAL cartridge address, and the
/// signature that finds it on other dumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hook {
    pub id: HookId,
    pub name: &'static str,
    pub addr_pal: u32,
    pub sig_offset: i32,
    /// Hex bytes, `??` = any byte.
    pub sig: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookId {
    P2Update,
    P2Physics,
    P2Display,
    OamBuild,
    OamHide,
    Contact,
    PlayerUpdate,
    PlayerPhysics,
    InputRemap,
    PlayerOam,
    FxUpdate,
    FxOam,
    ShotSlot,
    ShotSlotDone,
    ShotsDone,
    P1Oam,
    P1OamDone,
    Hud,
    BarRedraw,
    Platform,
    PlatformTouch,
    Car,
    PickupA,
    PickupB,
    PickupC,
    Streamer,
    StreamerExit,
    EffectSlot,
    QueueTest,
    QueueTestBig,
    DespawnTest,
    SpawnColumn,
    SpawnRow,
    KindCount,
}

const fn hook(id: HookId, name: &'static str, addr_pal: u32, sig_offset: i32, sig: &'static str) -> Hook {
    Hook { id, name, addr_pal, sig_offset, sig }
}

use HookId::*;

/// Indexed by `HookId as usize`. Signatures extracted from the PAL dump
/// (unique in the whole ROM with their wildcards; `hooks::tests`).
#[rustfmt::skip]
pub const HOOKS: [Hook; 34] = [
    hook(P2Update, "p2_update", 0x80D1EF, -4, "22 ?? ?? ?? 9C D4 0B AE 13 1F F0 05 20 ?? ?? 80"),
    hook(P2Physics, "p2_physics", 0x80D241, -4, "22 ?? ?? ?? 9C 1C 1F AE 19 1F D0 09 20 ?? ?? 20"),
    hook(P2Display, "p2_display", 0x809A86, -3, "20 ?? ?? AD 30 1F C9 80 D0 07 9C 30 1F A9 0F 85"),
    hook(OamBuild, "oam_build", 0x80D544, 0, "08 8B E2 20 C2 10 20 ?? ?? 20 ?? ?? 20 ?? ?? A9"),
    hook(OamHide, "oam_hide", 0x80D545, -1, "08 8B E2 20 C2 10 20 ?? ?? 20 ?? ?? 20 ?? ?? A9"),
    hook(Contact, "contact", 0x849B03, 0, "C2 10 A5 27 F0 70 A5 0E F0 31 AD 0C 1F D0 2C AD"),
    hook(PlayerUpdate, "player_update", 0x81812E, 0, "08 0B E2 30 F4 A8 0B 2B 22 ?? ?? ?? 85 65 A5 2B"),
    hook(PlayerPhysics, "player_physics", 0x819D67, 0, "0B C2 20 A9 A8 0B 5B E2 20 A5 64 D0 04 22 ?? ??"),
    hook(InputRemap, "input_remap", 0x80E518, 0, "C2 20 AD DE 0B 8D E0 0B E2 30 A5 A7 4A 4A 29 3C"),
    hook(PlayerOam, "player_oam", 0x80D671, 0, "A2 A8 0B B5 0E F0 03 20 ?? ?? 60 B5 11 29 40 85"),
    hook(FxUpdate, "fx_update", 0x80D292, 0, "C2 20 A9 38 0C 5B E2 30 A5 00 F0 07 A5 0A 0A AA"),
    hook(FxOam, "fx_oam", 0x80D640, 0, "AD 38 0C F0 0B AD 46 0C F0 06 A2 38 0C 20 ?? ??"),
    hook(ShotSlot, "shot_slot", 0x80D3B8, -6, "C2 20 A9 28 12 5B E2 30 A5 00 F0 03 20 ?? ?? C2"),
    hook(ShotSlotDone, "shot_slot_done", 0x80D3C1, 0, "C2 21 7B 69 40 00 C9 28 14 90 EB E2 30 60 C2 20"),
    hook(ShotsDone, "shots_done", 0x80D1FF, 0, "AE 13 1F F0 05 20 ?? ?? 80 03 20 ?? ?? AE 14 1F"),
    hook(P1Oam, "p1_oam", 0x80D55A, 0, "20 ?? ?? 20 ?? ?? 20 ?? ?? E2 30 A5 E4 29 03 F0"),
    hook(P1OamDone, "p1_oam_done", 0x80D560, 0, "20 ?? ?? E2 30 A5 E4 29 03 F0 10 46 E6 46 E6 1A"),
    hook(Hud, "hud", 0x809A94, 0, "AD 30 1F F0 09 A9 80 8D 30 1F A9 0E 85 B3 AD CF"),
    hook(BarRedraw, "bar_redraw", 0x80D801, 0, "A9 10 85 E4 A9 04 85 E5 AD CF 0B 30 03 4C ?? ??"),
    hook(Platform, "platform", 0x84AB81, 0, "08 E2 30 A5 2C F0 26 AD D4 0B 0D D3 0B 29 04 D0"),
    hook(PlatformTouch, "platform_touch", 0x84AB56, 0, "08 E2 20 C2 10 AD D4 0B 30 1F A2 A8 0B 22 ?? ??"),
    hook(Car, "car", 0x81DE1C, 0, "A5 2C 85 3A F0 23 AD D4 0B 29 04 D0 1C C2 20 A5"),
    // `LDX $01; JSR (tbl,X); RTL` + a 3-word table, the same in all three:
    // signed at the first case, where the item's constants differ.
    hook(PickupA, "pickup_a", 0x81DEB7, 46, "F0 03 A2 ?? ?? 86 20 E2 10 22 ?? ?? ?? A9 07 85"),
    hook(PickupB, "pickup_b", 0x82E342, 46, "F0 03 A2 ?? ?? 86 20 E2 10 22 ?? ?? ?? A9 12 85"),
    hook(PickupC, "pickup_c", 0x81E33A, 18, "64 18 64 28 A9 FF 85 26 AD B9 0B 29 30 09 02 85"),
    hook(Streamer, "streamer", 0x848FCA, 0, "08 E2 30 AF ?? ?? ?? CF ?? ?? ?? F0 03 EE 9D 1F"),
    hook(StreamerExit, "streamer_exit", 0x849083, 0, "AB 28 6B 08 C2 20 AD A6 0B 0A 18 6D A6 0B EB E2"),
    hook(EffectSlot, "effect_slot", 0x80D326, -12, "C2 20 A9 28 19 5B E2 30 A5 00 F0 0B A9 80 14 0E"),
    // Wide gameplay (wide_gameplay.rs): the X `BCS` of the object window
    // tests (entry + $0F, signed from the entry), and the spawner.
    hook(QueueTest, "queue_test", 0x8280C3, -15, "C2 20 38 A5 05 ED 4D 1E 18 69 20 00 C9 40 01 B0 0F"),
    hook(QueueTestBig, "queue_test_big", 0x82809E, -15, "C2 20 38 A5 05 ED 4D 1E 18 69 60 00 C9 C0 01 B0 0F"),
    hook(DespawnTest, "despawn_test", 0x82807D, -15, "C2 20 38 A5 05 ED 4D 1E 18 69 40 00 C9 80 01 B0 0D"),
    hook(SpawnColumn, "spawn_column", 0x80DC56, 0, "AD 50 1E 38 E9 20 00 85 02 A9 20 01 85 04 20 ?? ?? AD 50 1E"),
    hook(SpawnRow, "spawn_row", 0x80DC9B, -3, "A9 0A 00 85 06 20 ?? ?? A5 00 18 69 20 00 85 00 C6 06 D0 F1"),
    // The `BEQ` (slot empty) of the same-kind count `$84:A1D5`.
    hook(KindCount, "kind_count", 0x84A1E0, -11, "C2 30 A0 00 00 A2 68 0E BD 00 00 F0 1A E2 20 BD 0A 00 C5 0A"),
];

impl HookId {
    pub const fn hook(self) -> &'static Hook {
        &HOOKS[self as usize]
    }
    /// The PAL dump's address (docs and tests; the game uses [`HookMap::get`]).
    pub const fn pal(self) -> u32 {
        self.hook().addr_pal
    }
}

impl Hook {
    /// The signature as bytes, `None` = wildcard.
    pub fn pattern(&self) -> Vec<Option<u8>> {
        self.sig
            .split_whitespace()
            .map(|t| if t == "??" { None } else { Some(u8::from_str_radix(t, 16).expect("hex byte in hook signature")) })
            .collect()
    }
}

/// How one hook was found on the loaded ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Signature matches at the PAL address.
    AsPal,
    /// Found once elsewhere; the new address.
    Relocated(u32),
    /// Signature nowhere in the ROM.
    NotFound,
    /// Signature at more than one place (none taken).
    Ambiguous(usize),
    /// The image is a single-bank test ROM (no game code): PAL address, unchecked.
    Unchecked,
}

/// Resolved address per hook, built once per ROM by `Game::from_rom*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookMap {
    res: [Resolution; HOOKS.len()],
}

/// LoROM: file offset of `bank:addr` (banks `$80-$FF` mirror `$00-$7F`).
fn offset(addr: u32) -> usize {
    (((addr >> 16) & 0x7F) << 15 | (addr & 0x7FFF)) as usize
}

fn lorom_addr(off: usize) -> u32 {
    0x80_0000 | ((off as u32 >> 15) << 16) | 0x8000 | (off as u32 & 0x7FFF)
}

fn matches_at(rom: &[u8], off: usize, pat: &[Option<u8>]) -> bool {
    // A signature never straddles a bank (LoROM banks are not contiguous in the CPU map).
    off + pat.len() <= rom.len()
        && off >> 15 == (off + pat.len() - 1) >> 15
        && pat.iter().zip(&rom[off..]).all(|(p, b)| p.is_none_or(|p| p == *b))
}

/// Offsets where `pat` starts inside 32 KiB bank `bank` (a file bank index).
fn find_in_bank(rom: &[u8], bank: usize, pat: &[Option<u8>], out: &mut Vec<usize>) {
    let (k0, first) = pat.iter().enumerate().find_map(|(k, p)| p.map(|b| (k, b))).expect("signature has a fixed byte");
    let lo = bank << 15;
    let hi = ((bank + 1) << 15).min(rom.len());
    for (i, &b) in rom[lo..hi].iter().enumerate() {
        if b == first {
            if let Some(off) = (lo + i).checked_sub(k0) {
                if off >= lo && matches_at(rom, off, pat) {
                    out.push(off);
                }
            }
        }
    }
}

impl HookMap {
    /// Every hook at its PAL address, unchecked (test images).
    pub fn unchecked() -> HookMap {
        HookMap { res: [Resolution::Unchecked; HOOKS.len()] }
    }

    /// Resolve every hook on `rom` (header-stripped image).
    pub fn resolve(rom: &[u8]) -> HookMap {
        if rom.len() <= 0x8000 {
            return HookMap::unchecked();
        }
        let mut res = [Resolution::NotFound; HOOKS.len()];
        for (r, h) in res.iter_mut().zip(&HOOKS) {
            *r = resolve_one(rom, h);
        }
        HookMap { res }
    }

    pub fn resolution(&self, id: HookId) -> Resolution {
        self.res[id as usize]
    }

    /// The hook's address on this ROM, `None` when not found.
    pub fn get(&self, id: HookId) -> Option<u32> {
        match self.res[id as usize] {
            Resolution::AsPal | Resolution::Unchecked => Some(id.pal()),
            Resolution::Relocated(a) => Some(a),
            Resolution::NotFound | Resolution::Ambiguous(_) => None,
        }
    }

    /// By report name (`"p2_update"`).
    pub fn addr(&self, name: &str) -> Option<u32> {
        HOOKS.iter().find(|h| h.name == name).and_then(|h| self.get(h.id))
    }

    /// Hooks with no address on this ROM.
    pub fn missing(&self) -> Vec<HookId> {
        HOOKS.iter().filter(|h| self.get(h.id).is_none()).map(|h| h.id).collect()
    }

    /// True when every hook sits at its PAL address (or the image is a test ROM).
    pub fn all_as_pal(&self) -> bool {
        self.res.iter().all(|r| matches!(r, Resolution::AsPal | Resolution::Unchecked))
    }

    /// One line per hook: resolved address and how it was found.
    pub fn report(&self) -> Vec<String> {
        HOOKS
            .iter()
            .map(|h| {
                let how = match self.res[h.id as usize] {
                    Resolution::AsPal => "as PAL".to_string(),
                    Resolution::Relocated(a) => format!("relocated by {:+#x} (PAL ${:06X})", i64::from(a) - i64::from(h.addr_pal), h.addr_pal),
                    Resolution::NotFound => "NOT FOUND".to_string(),
                    Resolution::Ambiguous(n) => format!("NOT FOUND (signature at {n} places)"),
                    Resolution::Unchecked => "unchecked (test image)".to_string(),
                };
                match self.get(h.id) {
                    Some(a) => format!("{:<15} ${a:06X}  {how}", h.name),
                    None => format!("{:<15} -        {how} (PAL ${:06X})", h.name, h.addr_pal),
                }
            })
            .collect()
    }
}

fn resolve_one(rom: &[u8], h: &Hook) -> Resolution {
    let pat = h.pattern();
    let at_pal = h.addr_pal.wrapping_add_signed(h.sig_offset);
    if matches_at(rom, offset(at_pal), &pat) {
        return Resolution::AsPal;
    }
    let home = offset(at_pal) >> 15;
    let banks = rom.len().div_ceil(0x8000);
    let mut hits = Vec::new();
    find_in_bank(rom, home, &pat, &mut hits);
    if hits.is_empty() {
        for b in (0..banks).filter(|&b| b != home) {
            find_in_bank(rom, b, &pat, &mut hits);
        }
    }
    match hits[..] {
        [] => Resolution::NotFound,
        [off] => Resolution::Relocated(lorom_addr(off).wrapping_add_signed(-h.sig_offset)),
        _ => Resolution::Ambiguous(hits.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal_rom() -> Option<Vec<u8>> {
        let path = std::env::var("MMX_ROM").ok()?;
        let rom = std::fs::read(path).ok()?;
        let rom = if rom.len() % 0x8000 == 512 { rom[512..].to_vec() } else { rom };
        // These tests pin the PAL dump the signatures were taken from.
        (rom[0x7FD9] == 2 && rom[0x7FDB] == 0).then_some(rom)
    }

    #[test]
    fn table_is_indexed_by_id() {
        for (i, h) in HOOKS.iter().enumerate() {
            assert_eq!(h.id as usize, i, "{}", h.name);
            let p = h.pattern();
            assert!((12..=24).contains(&p.len()), "{}: {} bytes", h.name, p.len());
            assert!(p.iter().flatten().count() >= 10, "{}: too many wildcards", h.name);
        }
        let mut names: Vec<_> = HOOKS.iter().map(|h| h.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), HOOKS.len());
    }

    #[test]
    fn test_images_are_unchecked() {
        let m = HookMap::resolve(&[0u8; 0x8000]);
        assert!(m.all_as_pal() && m.missing().is_empty());
        assert_eq!(m.addr("p2_update"), Some(0x80D1EF));
    }

    /// Every signature matches at its PAL address and nowhere else in the ROM.
    #[test]
    fn signatures_unique_on_pal_dump() {
        let Some(rom) = pal_rom() else { return eprintln!("MMX_ROM (PAL v1.0) not set; skipping") };
        for h in &HOOKS {
            let pat = h.pattern();
            let mut hits = Vec::new();
            for b in 0..rom.len() / 0x8000 {
                find_in_bank(&rom, b, &pat, &mut hits);
            }
            let want = offset(h.addr_pal.wrapping_add_signed(h.sig_offset));
            assert_eq!(hits, [want], "{}: signature hits {:X?}", h.name, hits.iter().map(|&o| lorom_addr(o)).collect::<Vec<_>>());
        }
        let m = HookMap::resolve(&rom);
        assert!(m.all_as_pal(), "{:#?}", m.report());
    }

    /// The USA v1.1 dump (`MMX_ROM_V11`, sha256 `b8f70a6e…`): bank-`$80`
    /// hooks move (`+$20` near `$80:9A86`, `+$2B` from `$80:D1EF` on), the
    /// rest stay. Cross-checked: the `JSL`/`JSR` before each "after a call"
    /// hook targets the resolved callee.
    #[test]
    fn resolves_on_usa_v11() {
        let Some(rom) = std::env::var("MMX_ROM_V11").ok().and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM_V11 not set; skipping");
        };
        let m = HookMap::resolve(&rom);
        for h in &HOOKS {
            let want = match h.addr_pal {
                0x809A00..=0x809AFF => h.addr_pal + 0x20,
                a if a >> 16 == 0x80 => a + 0x2B,
                a => a,
            };
            assert_eq!(m.get(h.id), Some(want), "{}", h.name);
        }
        let jsl = |a: u32| u32::from_le_bytes([rom[offset(a)], rom[offset(a + 1)], rom[offset(a + 2)], 0]);
        assert_eq!(jsl(m.get(P2Update).unwrap() - 3), m.get(PlayerUpdate).unwrap());
        assert_eq!(jsl(m.get(P2Physics).unwrap() - 3), m.get(PlayerPhysics).unwrap());
        assert_eq!(jsl(m.get(P2Display).unwrap() - 2) & 0xFFFF, m.get(OamBuild).unwrap() & 0xFFFF);
        assert_eq!(jsl(m.get(P1Oam).unwrap() + 1) & 0xFFFF, m.get(FxOam).unwrap() & 0xFFFF);
        assert_eq!(jsl(m.get(P1Oam).unwrap() + 4) & 0xFFFF, m.get(PlayerOam).unwrap() & 0xFFFF);
    }

    /// A variant with `$80:9000..$80:FFC0` shifted up by `$41` (header kept):
    /// bank-`$80` hooks above the insertion move by `$41`, the rest stay; a
    /// signature wiped from the ROM reports NOT FOUND for that hook alone.
    #[test]
    fn search_finds_shifted_and_missing_hooks() {
        let Some(rom) = pal_rom() else { return eprintln!("MMX_ROM (PAL v1.0) not set; skipping") };
        const SHIFT: usize = 0x41;
        let mut v = rom.clone();
        let (from, end) = (offset(0x809000), offset(0x80FFC0));
        v.copy_within(from..end - SHIFT, from + SHIFT);
        v[from..from + SHIFT].fill(0xEA); // NOP padding
        let m = HookMap::resolve(&v);
        for h in &HOOKS {
            let moved = h.addr_pal >> 16 == 0x80 && h.addr_pal.wrapping_add_signed(h.sig_offset) >= 0x809000;
            let want = if moved { h.addr_pal + SHIFT as u32 } else { h.addr_pal };
            assert_eq!(m.get(h.id), Some(want), "{}", h.name);
            let r = m.resolution(h.id);
            assert_eq!(r == Resolution::AsPal, !moved, "{}: {r:?}", h.name);
        }
        assert!(!m.all_as_pal());

        let mut w = rom.clone();
        let o = offset(HookId::Car.pal());
        w[o] ^= 0xFF; // first byte of the car's signature (a fixed one)
        let m = HookMap::resolve(&w);
        assert_eq!(m.missing(), [HookId::Car]);
        assert_eq!(m.resolution(HookId::Car), Resolution::NotFound);
        assert!(m.report().iter().any(|l| l.starts_with("car") && l.contains("NOT FOUND")));
    }
}
