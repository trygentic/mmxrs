//! `Game`: the vendored jgenesis `SnesEmulator` plus trap dispatch at
//! instruction boundaries. See `docs/traps.md`.

use std::convert::Infallible;

use jgenesis_common::frontend::{
    AudioOutput, Color, ConstantInputPoller, EmulatorTrait, FrameSize, RenderFrameOptions,
    Renderer, SaveWriter, TickEffect,
};
use sha2::{Digest, Sha256};
use snes_config::SnesJoypadState;
use snes_core::api::{CoprocessorRoms, SnesEmulator, SnesEmulatorConfig};
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

pub struct Game {
    pub traps: TrapTable,
    pub traplog: TrapLog,
    pub coop: coop::CoopState,
    /// Debug: when `Some`, every opcode fetched inside `call_isolated*` is
    /// appended as `PBR:PC`.
    pub isolated_trace: Option<Vec<u32>>,
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

fn parse_header(mut rom: Vec<u8>) -> Result<(Vec<u8>, RomInfo), RomError> {
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
    if !title.starts_with("MEGAMAN X") {
        return Err(RomError::NotMegaManX { title });
    }
    if rom[0x7FD6] != 0 {
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
        let (rom, info) = parse_header(rom)?;
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
        })
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
    fn emulate_return(&mut self, long: bool) {
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
        self.set_regs(saved);
        self.fired_at = saved_fired;
    }

    /// Byte of the (header-stripped) ROM at a 24-bit LoROM address:
    /// `(bank & $7F) << 15 | (addr & $7FFF)`, mirrored over the image size.
    pub fn rom_read(&self, addr: u32) -> u8 {
        let off = (((addr >> 16) & 0x7F) << 15 | (addr & 0x7FFF)) as usize;
        self.rom[off % self.rom.len()]
    }

    pub fn save_state(&self) -> Vec<u8> {
        let state = (self.emu.to_save_state(), self.frame_count, self.fired_at);
        bincode::encode_to_vec(state, bincode::config::standard()).expect("save state encodes")
    }

    pub fn load_state(&mut self, s: &[u8]) -> Result<(), String> {
        let ((state, frame_count, fired_at), _): ((SnesEmulator, u64, Option<u32>), _) =
            bincode::decode_from_slice(s, bincode::config::standard())
                .map_err(|e| e.to_string())?;
        self.emu.load_state(state);
        self.frame_count = frame_count;
        self.fired_at = fired_at;
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
