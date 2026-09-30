//! `Game`: the vendored jgenesis `SnesEmulator` plus trap dispatch at
//! instruction boundaries. See `docs/traps.md`.

use std::convert::Infallible;

use jgenesis_common::frontend::{
    AudioOutput, Color, ConstantInputPoller, EmulatorTrait, FrameSize, RenderFrameOptions,
    Renderer, SaveWriter, TickEffect,
};
use sha2::{Digest, Sha256};
use snes_config::SnesJoypadState;
use snes_core::api::{CoprocessorRoms, P2Tiles, SnesEmulator, SnesEmulatorConfig, P2_PAGES, P2_PAGE_WORDS};
use snes_core::input::{SnesController, SnesInputs};

use crate::traps::{TrapRecord, TrapReturn};
use crate::{coop, Pads, Registers, RomError, RomInfo, TrapLog, TrapTable};

/// Where `call_isolated` returns to: `$FFFF` in the callee's own bank. In
/// LoROM that is the high byte of the emulation-mode IRQ/BRK vector, so no
/// real code ever starts there. Callees living in WRAM banks `$7E/$7F`
/// would need a different sentinel.
const SENTINEL_PC: u16 = 0xFFFF;
/// CPU cycles `call_isolated` allows before giving up (a few frames' worth).
const CALL_ISOLATED_BUDGET: u32 = 2_000_000;
/// Version marker before the appended P2 tile pages in a save state. v1 (no
/// marker) had a 512-word page; its tail is ignored (empty page). v2
/// (`P2T2`) had one 768-word page: it loads as page 0. v3 (`P2T3`) had two
/// pages (no wave page): they load as pages 0 and 1.
const P2_TILES_MAGIC: &[u8; 4] = b"P2T4";
const P2_TILES_MAGIC_V3: &[u8; 4] = b"P2T3";
const P2_TILES_MAGIC_V2: &[u8; 4] = b"P2T2";

pub struct Game {
    pub traps: TrapTable,
    pub traplog: TrapLog,
    pub coop: coop::CoopState,
    /// Debug: when `Some`, every opcode fetched inside `call_isolated*` is
    /// appended as `PBR:PC`.
    pub isolated_trace: Option<Vec<u32>>,
    /// `call_isolated*` nesting (P2's slices run isolated; `p2tiles` keys on it).
    pub(crate) isolated_depth: u32,
    /// `p2tiles`: `($A3, $1F25)` at the entry of P2's streamer call.
    pub(crate) p2_stream: Option<(u8, u16)>,
    /// X3 Zero art for P2 (static, not in save states; the frame shown is
    /// `coop.zero_frame`). See [`Game::set_zero_set`].
    pub(crate) zero_set: Option<std::sync::Arc<crate::zero_data::ZeroSet>>,
    emu: SnesEmulator,
    // ponytail: own copy of the ROM for rom_read (~1.5 MiB); jgenesis only
    // exposes cartridge reads through &mut (open-bus side effect).
    rom: Vec<u8>,
    info: RomInfo,
    video: Video,
    audio: Audio,
    frame_count: u64,
    /// `PBR:PC` whose trap check already ran at the current boundary visit;
    /// cleared whenever the CPU is mid-instruction.
    fired_at: Option<u32>,
}

struct Video {
    pixels: Vec<u32>,
    width: usize,
    height: usize,
}

impl Renderer for Video {
    type Err = Infallible;
    fn render_frame(
        &mut self,
        frame_buffer: &[Color],
        size: FrameSize,
        _target_fps: f64,
        _options: RenderFrameOptions,
    ) -> Result<(), Infallible> {
        self.width = size.width as usize;
        self.height = size.height as usize;
        self.pixels.clear();
        self.pixels.extend(
            frame_buffer[..self.width * self.height]
                .iter()
                .map(|c| (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)),
        );
        Ok(())
    }
}

#[derive(Default)]
struct Audio(Vec<(f32, f32)>);

impl AudioOutput for Audio {
    type Err = Infallible;
    fn push_sample(&mut self, l: f64, r: f64) -> Result<(), Infallible> {
        self.0.push((l as f32, r as f32));
        Ok(())
    }
}

/// MMX has no SRAM: nothing to load, nothing kept.
struct NoSave;

impl SaveWriter for NoSave {
    type Err = &'static str;
    fn load_bytes(&mut self, _: &str) -> Result<Vec<u8>, Self::Err> {
        Err("no save data")
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> Result<(), Self::Err> {
        Ok(())
    }
    fn load_serialized<D: bincode::Decode<()>>(&mut self, _: &str) -> Result<D, Self::Err> {
        Err("no save data")
    }
    fn persist_serialized<E: bincode::Encode>(&mut self, _: &str, _: E) -> Result<(), Self::Err> {
        Ok(())
    }
}

fn joypad(bits: u16) -> SnesController {
    let b = |n: u16| bits & (1 << n) != 0;
    SnesController::Gamepad(SnesJoypadState {
        b: b(15),
        y: b(14),
        select: b(13),
        start: b(12),
        up: b(11),
        down: b(10),
        left: b(9),
        right: b(8),
        a: b(7),
        x: b(6),
        l: b(5),
        r: b(4),
    })
}

fn parse_header(mut rom: Vec<u8>, strict: bool) -> Result<(Vec<u8>, RomInfo), RomError> {
    let had_copier_header = rom.len() % 0x8000 == 512;
    if had_copier_header {
        rom.drain(..512);
    }
    if rom.len() < 0x8000 {
        return Err(RomError::TooSmall(rom.len()));
    }
    let title = String::from_utf8_lossy(&rom[0x7FC0..0x7FD5])
        .trim_end()
        .to_string();
    if strict && !title.starts_with("MEGAMAN X") {
        return Err(RomError::NotMegaManX { title });
    }
    if strict && rom[0x7FD6] != 0 {
        return Err(RomError::Coprocessor(rom[0x7FD6]));
    }
    let info = RomInfo {
        title,
        version: rom[0x7FDB],
        sha256: Sha256::digest(&rom).into(),
        crc32: crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(&rom),
        had_copier_header,
    };
    Ok((rom, info))
}

impl Game {
    pub fn from_rom(rom: Vec<u8>) -> Result<Game, RomError> {
        Self::load(parse_header(rom, true)?)
    }

    /// Research boot (X3 etc.): size check and copier-header strip only, no
    /// title or coprocessor check. The co-op traps assume X1; don't register them.
    pub fn from_rom_any(rom: Vec<u8>) -> Result<Game, RomError> {
        Self::load(parse_header(rom, false)?)
    }

    fn load((rom, info): (Vec<u8>, RomInfo)) -> Result<Game, RomError> {
        let emu = SnesEmulator::create(
            rom.clone(),
            SnesEmulatorConfig::default(),
            CoprocessorRoms::none(),
            &mut NoSave,
        )
        .map_err(|e| RomError::Emulator(e.to_string()))?;
        Ok(Game {
            traps: TrapTable::new(),
            traplog: TrapLog::default(),
            coop: coop::CoopState::default(),
            emu,
            rom,
            info,
            video: Video {
                pixels: vec![0; 256 * 224],
                width: 256,
                height: 224,
            },
            audio: Audio::default(),
            frame_count: 0,
            fired_at: None,
            isolated_trace: None,
            isolated_depth: 0,
            p2_stream: None,
            zero_set: None,
        })
    }

    /// Draw P2 as X3 Zero (`Some`, from [`crate::zero_data::extract_x3`]) or
    /// as X's tiles in Zero's colours (`None`). Static art: both netplay
    /// peers must load the same set (see `netgame`).
    pub fn set_zero_set(&mut self, set: Option<crate::zero_data::ZeroSet>) {
        self.zero_set = set.map(std::sync::Arc::new);
        self.coop.zero_frame = u16::MAX; // refill the page on the next draw
        self.coop.p1.frame = u16::MAX;
        self.coop.wave_frames = [u16::MAX; 2];
    }

    /// [`Game::set_zero_set`] from a Mega Man X3 (USA) image (copier header ok).
    pub fn set_x3_rom(&mut self, rom: &[u8]) -> Result<(), String> {
        let rom = if rom.len() % 0x8000 == 512 { &rom[512..] } else { rom };
        self.set_zero_set(Some(crate::zero_data::extract_x3(rom)?));
        Ok(())
    }

    pub fn zero_set(&self) -> Option<&crate::zero_data::ZeroSet> {
        self.zero_set.as_deref()
    }

    /// Player 1 plays as X3 Zero (docs/zero.md "Playing as Zero"): needs the
    /// X3 set ([`Game::set_x3_rom`]); registers the co-op trap group (its
    /// P2 hooks stay idle while `coop.enabled` is false).
    pub fn set_p1_zero(&mut self, on: bool) -> Result<(), String> {
        if on && self.zero_set.is_none() {
            return Err("playing as Zero needs a Mega Man X3 (USA) ROM (--x3-rom PATH or MMX_X3_ROM)".into());
        }
        self.coop.p1 = crate::zero::ZeroP1 { on, ..Default::default() };
        if on {
            coop::register_coop_traps(self);
        }
        Ok(())
    }

    pub fn p1_zero(&self) -> bool {
        self.coop.p1.on
    }

    pub fn rom_info(&self) -> &RomInfo {
        &self.info
    }

    pub fn step(&mut self, pads: Pads) {
        let inputs = SnesInputs {
            p1: joypad(pads.p1),
            p2: joypad(pads.p2),
        };
        let mut rendered = false;
        // Run past the frame to the next instruction boundary, so a step
        // always ends between instructions (safe for set_regs, call_isolated,
        // save states). The trap check for that boundary runs next step.
        while !(rendered && self.emu.at_instruction_boundary()) {
            self.dispatch_traps();
            let effect = self
                .emu
                .tick(
                    &mut self.video,
                    &mut self.audio,
                    &mut ConstantInputPoller(&inputs),
                    &mut NoSave,
                )
                .expect("frontend sinks are infallible and there is no SRAM");
            rendered |= effect == TickEffect::FrameRendered;
        }
        self.frame_count += 1;
    }

    /// Debug aid: one frame like [`Game::step`], returning `PBR:PC` of every
    /// opcode fetched (traps still fire). Big (~100k entries per frame).
    pub fn trace_frame(&mut self, pads: Pads) -> Vec<u32> {
        let inputs = SnesInputs { p1: joypad(pads.p1), p2: joypad(pads.p2) };
        let mut out = Vec::with_capacity(120_000);
        let mut rendered = false;
        while !(rendered && self.emu.at_instruction_boundary()) {
            self.dispatch_traps();
            if self.emu.at_instruction_boundary() && self.emu.next_tick_fetches_opcode() {
                out.push(self.regs().pbr_pc());
            }
            let effect = self
                .emu
                .tick(&mut self.video, &mut self.audio, &mut ConstantInputPoller(&inputs), &mut NoSave)
                .expect("frontend sinks are infallible and there is no SRAM");
            rendered |= effect == TickEffect::FrameRendered;
        }
        self.frame_count += 1;
        out
    }

    /// Fire the trap at `PBR:PC` once per boundary visit. Loops because an
    /// emulated return can land on another trapped address.
    fn dispatch_traps(&mut self) {
        if !self.emu.at_instruction_boundary() {
            self.fired_at = None;
            return;
        }
        loop {
            let pc = self.regs().pbr_pc();
            if self.fired_at == Some(pc) {
                return;
            }
            let Some(info) = self.traps.get(pc) else {
                self.fired_at = Some(pc);
                return;
            };
            // An NMI/IRQ (or WAI) goes first: fire when the RTI comes back here.
            if !self.emu.next_tick_fetches_opcode() {
                return;
            }
            self.fired_at = Some(pc);
            self.traplog.push(TrapRecord {
                addr: pc,
                name: info.name,
                frame: self.frame_count,
            });
            (info.body)(self);
            match info.ret {
                TrapReturn::Observe => return,
                TrapReturn::Rts => self.emulate_return(false),
                TrapReturn::Rtl => self.emulate_return(true),
            }
        }
    }

    /// Pop the return address the `JSR`/`JSL` pushed; `PC = word + 1`.
    pub(crate) fn emulate_return(&mut self, long: bool) {
        let mut r = self.regs();
        let lo = self.pop(&mut r);
        let hi = self.pop(&mut r);
        if long {
            r.pbr = self.pop(&mut r);
        }
        r.pc = u16::from_le_bytes([lo, hi]).wrapping_add(1);
        self.set_regs(r);
    }

    fn pop(&self, r: &mut Registers) -> u8 {
        r.s = next_s(r, r.s.wrapping_add(1));
        self.wram()[stack_index(r.s)]
    }

    fn push(&mut self, r: &mut Registers, v: u8) {
        self.wram_mut()[stack_index(r.s)] = v;
        r.s = next_s(r, r.s.wrapping_sub(1));
    }

    pub fn frame(&self) -> (&[u32], usize, usize) {
        (&self.video.pixels, self.video.width, self.video.height)
    }

    pub fn take_audio(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.audio.0)
    }

    pub fn set_audio_rate(&mut self, hz: u32) {
        self.emu.update_audio_output_frequency(hz.into());
    }

    /// Frames per emulated second: 60.0988 NTSC, 50.007 PAL (the header's
    /// region byte decides; roms/mega-man-x.smc is the PAL release). Wall-clock
    /// pacers must use this, not a hard-coded 60.
    pub fn fps(&self) -> f64 {
        self.emu.target_fps()
    }

    pub fn wram(&self) -> &[u8; 0x20000] {
        self.emu.wram()
    }

    pub fn wram_mut(&mut self) -> &mut [u8; 0x20000] {
        self.emu.wram_mut()
    }

    pub fn oam(&self) -> [u8; 544] {
        self.emu.oam_bytes()
    }

    pub fn cgram(&self) -> [u16; 256] {
        self.emu.cgram()
    }

    /// Raw VRAM (64 KiB as words). Debug / research.
    pub fn vram(&self) -> &[u16; 0x8000] {
        self.emu.vram()
    }

    /// OBSEL: (sprite tile base, name-table gap), both VRAM word addresses.
    pub fn obsel(&self) -> (u16, u16) {
        self.emu.obsel()
    }

    /// The shadow sprite tile pages (pending; live on the next OAM upload):
    /// page 0 is P2's, page 1 (`P2_PAGE_WORDS..`) a Zero P1's, page 2 the
    /// saber waves (P2's in columns 0-7, P1's in 8-15). See
    /// `p2tiles.rs`. Part of the save state.
    pub fn p2_tile_page_mut(&mut self) -> &mut [u16; P2_PAGE_WORDS * P2_PAGES] {
        self.emu.p2_tile_page_mut()
    }

    /// OAM slots (bit n = slot n) that read the shadow pages instead of VRAM
    /// (all of them page 0).
    pub fn set_p2_tile_slots(&mut self, mask: u128) {
        self.emu.set_p2_tile_slots(mask, 0, 0);
    }

    /// Add `mask` to the pending slot mask, reading shadow page `page` (0-2).
    pub fn add_p2_tile_slots(&mut self, mask: u128, page: u8) {
        let t = self.emu.p2_tiles();
        let on = |p: u8, bits: u128| if page == p { bits | mask } else { bits & !mask };
        self.emu.set_p2_tile_slots(t.next_slots | mask, on(1, t.next_page1), on(2, t.next_page2));
    }

    /// Live shadow pages and slot mask (what the renderer uses now).
    pub fn p2_tiles(&self) -> (&[u16; P2_PAGE_WORDS * P2_PAGES], u128) {
        let t = self.emu.p2_tiles();
        (&t.page, t.slots)
    }

    pub fn regs(&self) -> Registers {
        let r = self.emu.cpu_registers();
        Registers {
            a: r.a,
            x: r.x,
            y: r.y,
            s: r.s,
            d: r.d,
            pbr: r.pbr,
            pc: r.pc,
            dbr: r.dbr,
            p: r.p.into(),
            emulation: r.emulation_mode,
        }
    }

    pub fn set_regs(&mut self, r: Registers) {
        self.emu.set_cpu_registers(wdc65816_emu::core::Registers {
            a: r.a,
            x: r.x,
            y: r.y,
            s: r.s,
            d: r.d,
            pbr: r.pbr,
            pc: r.pc,
            dbr: r.dbr,
            p: r.p.into(),
            emulation_mode: r.emulation,
        });
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// [`Game::call_isolated_ret`] for a routine entered by `JSL` (ends in `RTL`).
    pub fn call_isolated(&mut self, addr: u32) {
        self.call_isolated_ret(addr, TrapReturn::Rtl);
    }

    /// Run the routine at `addr` on the CPU alone (zero emulated time) with
    /// the caller's current registers (P, D, DBR as the ROM caller would set
    /// them), then restore every register. Call it at an instruction
    /// boundary: from a trap body or between steps. Traps fire inside the
    /// call, once per boundary visit as in [`Game::step`].
    ///
    /// `Rts` pushes 2 bytes (`SENTINEL_PC - 1`), `Rtl` (or `Observe`) pushes
    /// 3 (`bank(addr)` first), exactly as `JSR`/`JSL` would; either way the
    /// routine's return lands on `bank(addr):SENTINEL_PC` and the call ends.
    ///
    /// Caveat: the PPU does not advance, so no new NMI/IRQ edge arrives; the
    /// callee must not wait on one (an interrupt already latched before the
    /// call is still taken). Panics if the routine has not returned within
    /// `CALL_ISOLATED_BUDGET` CPU cycles.
    pub fn call_isolated_ret(&mut self, addr: u32, ret: TrapReturn) {
        let saved = self.regs();
        let saved_fired = self.fired_at.take();
        let bank = (addr >> 16) as u8;
        let sentinel = (u32::from(bank) << 16) | u32::from(SENTINEL_PC);
        let mut r = saved;
        if ret != TrapReturn::Rts {
            self.push(&mut r, bank);
        }
        let [lo, hi] = SENTINEL_PC.wrapping_sub(1).to_le_bytes();
        self.push(&mut r, hi);
        self.push(&mut r, lo);
        r.pbr = bank;
        r.pc = addr as u16;
        self.set_regs(r);
        self.isolated_depth += 1;
        let mut budget = CALL_ISOLATED_BUDGET;
        loop {
            self.dispatch_traps();
            if self.emu.at_instruction_boundary() && self.regs().pbr_pc() == sentinel {
                break;
            }
            assert!(budget > 0, "call_isolated(${addr:06X}) did not return");
            budget -= 1;
            if self.isolated_trace.is_some()
                && self.emu.at_instruction_boundary()
                && self.emu.next_tick_fetches_opcode()
            {
                let pc = self.regs().pbr_pc();
                if let Some(t) = self.isolated_trace.as_mut() {
                    t.push(pc);
                }
            }
            // One CPU tick (the sentinel check inside is the same as ours).
            self.emu.run_cpu_isolated(sentinel, 1);
        }
        self.isolated_depth -= 1;
        self.set_regs(saved);
        self.fired_at = saved_fired;
    }

    /// Byte of the (header-stripped) ROM at a 24-bit LoROM address:
    /// `(bank & $7F) << 15 | (addr & $7FFF)`, mirrored over the image size.
    pub fn rom_bytes(&self) -> &[u8] {
        &self.rom
    }

    pub fn rom_read(&self, addr: u32) -> u8 {
        let off = (((addr >> 16) & 0x7F) << 15 | (addr & 0x7FFF)) as usize;
        self.rom[off % self.rom.len()]
    }

    /// `(emulator, frame_count, fired_at)`, then [`P2_TILES_MAGIC`] and the
    /// PPU's P2 tile page (appended, so states from before the page, or with
    /// the old 512-word page and no magic, load with an empty one).
    pub fn save_state(&self) -> Vec<u8> {
        let cfg = bincode::config::standard();
        let state = (self.emu.to_save_state(), self.frame_count, self.fired_at);
        let mut v = bincode::encode_to_vec(state, cfg).expect("save state encodes");
        v.extend(P2_TILES_MAGIC);
        v.extend(bincode::encode_to_vec(self.emu.p2_tiles(), cfg).expect("p2 tiles encode"));
        v
    }

    pub fn load_state(&mut self, s: &[u8]) -> Result<(), String> {
        let cfg = bincode::config::standard();
        let ((state, frame_count, fired_at), n): ((SnesEmulator, u64, Option<u32>), _) =
            bincode::decode_from_slice(s, cfg).map_err(|e| e.to_string())?;
        self.emu.load_state(state);
        if let Some(tail) = s[n..].strip_prefix(P2_TILES_MAGIC.as_slice()) {
            let (t, _) = bincode::decode_from_slice(tail, cfg).map_err(|e| e.to_string())?;
            self.emu.set_p2_tiles(t);
        } else if let Some(tail) = s[n..].strip_prefix(P2_TILES_MAGIC_V3.as_slice()) {
            const W2: usize = P2_PAGE_WORDS * 2;
            type V3 = (Box<[u16; W2]>, u128, Box<[u16; W2]>, u128, bool, u128, u128);
            let ((page, slots, next, next_slots, pending, page1, next_page1), _): (V3, _) =
                bincode::decode_from_slice(tail, cfg).map_err(|e| e.to_string())?;
            let mut t = P2Tiles { slots, next_slots, pending, page1, next_page1, ..P2Tiles::default() };
            t.page[..W2].copy_from_slice(&page[..]);
            t.next_page[..W2].copy_from_slice(&next[..]);
            self.emu.set_p2_tiles(t);
        } else if let Some(tail) = s[n..].strip_prefix(P2_TILES_MAGIC_V2.as_slice()) {
            type V2 = (Box<[u16; P2_PAGE_WORDS]>, u128, Box<[u16; P2_PAGE_WORDS]>, u128, bool);
            let ((page, slots, next, next_slots, pending), _): (V2, _) =
                bincode::decode_from_slice(tail, cfg).map_err(|e| e.to_string())?;
            let mut t = P2Tiles { slots, next_slots, pending, ..P2Tiles::default() };
            t.page[..P2_PAGE_WORDS].copy_from_slice(&page[..]);
            t.next_page[..P2_PAGE_WORDS].copy_from_slice(&next[..]);
            self.emu.set_p2_tiles(t);
        } else {
            self.emu.set_p2_tiles(P2Tiles::default());
        }
        self.frame_count = frame_count;
        self.fired_at = fired_at;
        self.coop.zero_frame = u16::MAX; // the page came from the state: refill it
        self.coop.p1.frame = u16::MAX;
        self.coop.wave_frames = [u16::MAX; 2];
        Ok(())
    }
}

/// Emulation mode keeps the stack in page 1.
fn next_s(r: &Registers, s: u16) -> u16 {
    if r.emulation {
        0x0100 | (s & 0xFF)
    } else {
        s
    }
}

/// The stack lives in bank 0; `$0000-$1FFF` there mirrors WRAM.
fn stack_index(s: u16) -> usize {
    // ponytail: a stack above $1FFF (not WRAM) is unsupported; MMX keeps it low.
    debug_assert!(s < 0x2000, "stack pointer ${s:04X} outside low WRAM");
    usize::from(s)
}
