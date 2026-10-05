//! `Game`: the vendored jgenesis `SnesEmulator` plus trap dispatch at
//! instruction boundaries. See `docs/traps.md`.

use std::convert::Infallible;

use jgenesis_common::frontend::{TimingMode, 
    AudioOutput, Color, ConstantInputPoller, EmulatorTrait, FrameSize, RenderFrameOptions,
    Renderer, SaveWriter, TickEffect,
};
use sha2::{Digest, Sha256};
use snes_config::SnesJoypadState;
use snes_core::api::{CoprocessorRoms, P2Tiles, SnesEmulator, SnesEmulatorConfig, P2_PAGES, P2_PAGE_WORDS};
use snes_core::input::{SnesController, SnesInputs};

use crate::hooks::{HookId, HookMap};
use crate::traps::{TrapRecord, TrapReturn};
use crate::{coop, Pads, Registers, RomError, RomInfo, Timing, TrapLog, TrapTable};

/// Where `call_isolated` returns to: `$FFFF` in the callee's own bank. In
/// LoROM that is the high byte of the emulation-mode IRQ/BRK vector, so no
/// real code ever starts there. Callees living in WRAM banks `$7E/$7F`
/// would need a different sentinel.
const SENTINEL_PC: u16 = 0xFFFF;
/// CPU cycles `call_isolated` allows before giving up (a few frames' worth).
const CALL_ISOLATED_BUDGET: u32 = 2_000_000;

/// Why [`Game::try_call_isolated_ret`] gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedError {
    /// Not back at the sentinel within `CALL_ISOLATED_BUDGET` cycles.
    Budget,
    /// The stack pointer went above the caller's: the routine popped its own
    /// return address and is leaving through a frame that is not there.
    Unwound,
}

impl std::fmt::Display for IsolatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            IsolatedError::Budget => "did not return",
            IsolatedError::Unwound => "unwound past its caller",
        })
    }
}
/// Version marker before the appended P2 tile pages in a save state. v1 (no
/// marker) had a 512-word page; its tail is ignored (empty page). v2
/// (`P2T2`) had one 768-word page: it loads as page 0. v3 (`P2T3`) had two
/// pages (no wave page): they load as pages 0 and 1. v5 (`P2T5`) had no
/// P2 palette (`P2Tiles::pal`): it loads with none (refilled at the next
/// OAM build).
const P2_TILES_MAGIC: &[u8; 4] = b"P2T6";
const P2_TILES_MAGIC_V5: &[u8; 4] = b"P2T5";
const P2_TILES_MAGIC_V4: &[u8; 4] = b"P2T4";
/// `P2T5`'s `P2Tiles`: pages, slots, next pages, next slots, pending,
/// page1, next page1, page2, next page2 (no palette).
type P2TilesV5 = (Box<[u16; P2_PAGE_WORDS * P2_PAGES]>, u128, Box<[u16; P2_PAGE_WORDS * P2_PAGES]>, u128, bool, u128, u128, u128, u128);
const P2_TILES_MAGIC_V3: &[u8; 4] = b"P2T3";
const P2_TILES_MAGIC_V2: &[u8; 4] = b"P2T2";
/// Marker after the P2 tile pages: `COOP`, a `u32` LE length, then the
/// bincode [`coop::CoopState`]. States without it load with a default one
/// (P2 hidden: it re-anchors beside P1 when it next shows).
const COOP_MAGIC: &[u8; 4] = b"COOP";

/// What a save state holds, without loading it ([`Game::state_info`]).
#[derive(Debug, Clone)]
pub struct StateInfo {
    pub frame_count: u64,
    /// Tile-page marker (`P2T5`, older `P2T4`..`P2T2`), `None` before them.
    pub tiles: Option<String>,
    /// The co-op state, when the save has one (`COOP`).
    pub coop: Option<coop::CoopState>,
}

pub struct Game {
    timing: Timing,
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
    /// OAM slots of each player's character in the last OAM build (`[P1,
    /// P2]`, bit n = slot n; display.rs). Display only, not in save states.
    pub(crate) oam_slots: [u128; 2],
    /// Margin sprites (margin_sprites.rs). Display only, not in save states.
    pub margin: crate::margin_sprites::MarginState,
    /// Wide gameplay (wide_gameplay.rs): a setting, not in save states.
    wide_gameplay: bool,
    emu: SnesEmulator,
    // ponytail: own copy of the ROM for rom_read (~1.5 MiB); jgenesis only
    // exposes cartridge reads through &mut (open-bus side effect).
    rom: Vec<u8>,
    info: RomInfo,
    /// Where this ROM keeps each hook (`hooks.rs`), resolved at load.
    hooks: HookMap,
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

fn emulator_config(timing: Timing, allow_opposing_directions: bool) -> SnesEmulatorConfig {
    SnesEmulatorConfig {
        forced_timing_mode: match timing {
            Timing::Auto => None,
            Timing::Ntsc => Some(TimingMode::Ntsc),
            Timing::Pal => Some(TimingMode::Pal),
        },
        allow_opposing_joypad_directions: allow_opposing_directions,
        ..SnesEmulatorConfig::default()
    }
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
        region: rom[0x7FD9],
        sha256: Sha256::digest(&rom).into(),
        crc32: crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC).checksum(&rom),
        had_copier_header,
    };
    Ok((rom, info))
}

impl Game {
    pub fn from_rom(rom: Vec<u8>) -> Result<Game, RomError> {
        Self::load(parse_header(rom, true)?, Timing::Auto)
    }

    /// Like [`Game::from_rom`] with a forced video timing. `Timing::Ntsc` runs a
    /// PAL dump at 60.0988 fps (the game code is the same as USA v1.0).
    pub fn from_rom_timing(rom: Vec<u8>, timing: Timing) -> Result<Game, RomError> {
        Self::load(parse_header(rom, true)?, timing)
    }

    /// Pass Left+Right / Up+Down through to the game as pressed (the
    /// console reads whatever the pad's switches say; a d-pad rocker just
    /// cannot press both). Off by default, as jgenesis has it: a keyboard
    /// can press both. Input movies need it on (TASes press both).
    pub fn set_allow_opposing_directions(&mut self, on: bool) {
        self.emu.reload_config(&emulator_config(self.timing, on));
    }

    /// The timing this game was created with (netplay rebuilds with the same).
    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// Research boot (X3 etc.): size check and copier-header strip only, no
    /// title or coprocessor check. The co-op traps assume X1; don't register them.
    pub fn from_rom_any(rom: Vec<u8>) -> Result<Game, RomError> {
        Self::load(parse_header(rom, false)?, Timing::Auto)
    }

    fn load((rom, info): (Vec<u8>, RomInfo), timing: Timing) -> Result<Game, RomError> {
        let config = emulator_config(timing, false);
        let emu = SnesEmulator::create(
            rom.clone(),
            config,
            CoprocessorRoms::none(),
            &mut NoSave,
        )
        .map_err(|e| RomError::Emulator(e.to_string()))?;
        Ok(Game {
            traps: TrapTable::new(),
            traplog: TrapLog::default(),
            coop: coop::CoopState::default(),
            emu,
            hooks: HookMap::resolve(&rom),
            rom,
            info,
            timing,
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
            oam_slots: [0; 2],
            margin: Default::default(),
            wide_gameplay: false,
        })
    }

    /// Wide gameplay (docs/widescreen.md "Wide gameplay"): enemies spawn,
    /// are queued and despawn by a window [`crate::wide_gameplay::WIDE_SPAWN_PX`]
    /// / [`crate::wide_gameplay::WIDE_GAMEPLAY_PX`] wider per side, whatever
    /// the display. Changes the game (netplay identity); off registers nothing.
    pub fn set_wide_gameplay(&mut self, on: bool) {
        self.wide_gameplay = on;
        crate::wide_gameplay::register(self, on);
    }

    pub fn wide_gameplay(&self) -> bool {
        self.wide_gameplay
    }

    /// CPU overclock (docs/widescreen.md "Slowdown and overclock"): the CPU's
    /// bus cycles cost fewer master cycles, so enemy-heavy frames finish
    /// before the next NMI. Changes the game (netplay identity); a setting,
    /// not in save states (a loaded state runs at this game's factor).
    pub fn set_overclock(&mut self, o: crate::Overclock) {
        self.emu.set_cpu_overclock(o);
    }

    pub fn overclock(&self) -> crate::Overclock {
        self.emu.cpu_overclock()
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
        if on {
            coop::register_coop_traps(self)?;
        }
        self.coop.p1 = crate::zero::ZeroP1 { on, ..Default::default() };
        Ok(())
    }

    pub fn p1_zero(&self) -> bool {
        self.coop.p1.on
    }

    pub fn rom_info(&self) -> &RomInfo {
        &self.info
    }

    /// Hook addresses on this ROM (`hooks.rs`).
    pub fn hooks(&self) -> &HookMap {
        &self.hooks
    }

    /// Address of hook `id` on this ROM. A hook that was not found gives its
    /// PAL address; `coop::register_coop_traps` refuses such a ROM, so no
    /// trap or isolated call ever uses it.
    pub fn hook(&self, id: HookId) -> u32 {
        self.hooks.get(id).unwrap_or(id.pal())
    }

    /// The ROM (title, region, version, sha256) and one line per hook:
    /// resolved address, "as PAL" / "relocated by N" / "NOT FOUND".
    pub fn hook_report(&self) -> Vec<String> {
        let i = &self.info;
        let sha: String = i.sha256[..4].iter().map(|b| format!("{b:02x}")).collect();
        let mut v = vec![format!(
            "ROM {} ({}, v1.{}, sha256 {sha}…): {} of {} hooks as on the PAL dump, {} not found",
            i.title,
            i.region_name(),
            i.version,
            crate::hooks::HOOKS.iter().filter(|h| self.hooks.resolution(h.id) == crate::hooks::Resolution::AsPal).count(),
            crate::hooks::HOOKS.len(),
            self.hooks.missing().len(),
        )];
        v.extend(self.hooks.report());
        v
    }

    pub fn step(&mut self, pads: Pads) {
        let inputs = SnesInputs {
            p1: joypad(pads.p1),
            p2: joypad(pads.p2),
        };
        let _t = crate::profile::scope(crate::profile::EMULATOR);
        let mut rendered = false;
        let audio_from = self.audio.0.len();
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
        if let Some(s) = self.zero_set.as_ref().and_then(|z| z.sounds.as_ref()) {
            let _t = crate::profile::scope("sfx mix");
            let fps = self.emu.target_fps();
            crate::sfx::mix(&mut self.coop.sfx, &s.swing, fps, &mut self.audio.0[audio_from..]);
        }
        self.frame_count += 1;
        if self.wide_margin() > 0 {
            let _t = crate::profile::scope("wide tiles");
            crate::widescreen::update_margin_tiles(self);
        }
    }

    /// Debug aid: one frame like [`Game::step`], returning `PBR:PC` of every
    /// opcode fetched (traps still fire). Big (~100k entries per frame).
    pub fn trace_frame(&mut self, pads: Pads) -> Vec<u32> {
        self.trace_frame_mclk(pads).into_iter().map(|(pc, _)| pc).collect()
    }

    /// [`Game::trace_frame`] with the master clock at each fetch.
    pub fn trace_frame_mclk(&mut self, pads: Pads) -> Vec<(u32, u64)> {
        let inputs = SnesInputs { p1: joypad(pads.p1), p2: joypad(pads.p2) };
        let mut out = Vec::with_capacity(120_000);
        let mut rendered = false;
        while !(rendered && self.emu.at_instruction_boundary()) {
            self.dispatch_traps();
            if self.emu.at_instruction_boundary() && self.emu.next_tick_fetches_opcode() {
                out.push((self.regs().pbr_pc(), self.emu.master_cycles()));
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

    /// [`Game::trace_frame`] with the PPU position (scanline, master cycles
    /// into the line) at each fetch (oracle comparison, tools/oracle-mesen).
    pub fn trace_frame_pos(&mut self, pads: Pads) -> Vec<(u32, u16, u64)> {
        let inputs = SnesInputs { p1: joypad(pads.p1), p2: joypad(pads.p2) };
        let mut out = Vec::with_capacity(120_000);
        let mut rendered = false;
        while !(rendered && self.emu.at_instruction_boundary()) {
            self.dispatch_traps();
            if self.emu.at_instruction_boundary() && self.emu.next_tick_fetches_opcode() {
                let (line, h) = self.emu.ppu_position();
                out.push((self.regs().pbr_pc(), line, h));
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
            {
                let _t = crate::profile::scope(info.name);
                (info.body)(self);
            }
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

    /// SPC700 audio RAM (64 KiB). Research / inspection.
    pub fn spc_ram(&self) -> &[u8] {
        self.emu.spc_ram()
    }

    /// DSP register `$00-$7F`, read without side effects. Research / inspection.
    pub fn dsp_register(&self, a: u8) -> u8 {
        self.emu.dsp_register(a)
    }

    /// Research knob, process-wide: bit n silences DSP voice n in every game's output.
    pub fn set_voice_mute(mask: u8) {
        SnesEmulator::set_voice_mute(mask);
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
    /// saber slash arcs (P2's in columns 0-7, P1's in 8-15), page 3 the HUD
    /// icon (`coop_hud.rs`). See `p2tiles.rs`. Part of the save state.
    pub fn p2_tile_page_mut(&mut self) -> &mut [u16; P2_PAGE_WORDS * P2_PAGES] {
        self.emu.p2_tile_page_mut()
    }

    /// OAM slots (bit n = slot n) that read the shadow pages instead of VRAM
    /// (all of them page 0).
    pub fn set_p2_tile_slots(&mut self, mask: u128) {
        self.emu.set_p2_tile_slots(mask, 0, 0);
    }

    /// Add `mask` to the pending slot mask, reading shadow page `page` (0-3:
    /// bit 0 = the `page1` mask, bit 1 = `page2`).
    pub fn add_p2_tile_slots(&mut self, mask: u128, page: u8) {
        let t = self.emu.p2_tiles();
        let on = |bit: u8, bits: u128| if page & bit != 0 { bits | mask } else { bits & !mask };
        self.emu.set_p2_tile_slots(t.next_slots | mask, on(1, t.next_page1), on(2, t.next_page2));
    }

    /// Add `mask` to the pending slots drawn with P2's own sprite palette
    /// (the PPU's ninth, [`Game::set_p2_palette`]) instead of CGRAM. Live with
    /// the shadow pages, on the next OAM upload; cleared with them at the OAM
    /// build (`p2tiles::clear_slots`).
    pub fn add_p2_palette_slots(&mut self, mask: u128) {
        let t = self.emu.p2_tiles();
        let (m, p) = (t.next_pal | mask, t.next_palette);
        self.emu.set_p2_palette(m, p);
    }

    /// Clear the pending P2-palette slot mask.
    pub fn clear_p2_palette_slots(&mut self) {
        let p = self.emu.p2_tiles().next_palette;
        self.emu.set_p2_palette(0, p);
    }

    /// The colours of P2's sprite palette (pending, see
    /// [`Game::add_p2_palette_slots`]). Part of the save state.
    pub fn set_p2_palette(&mut self, colours: [u16; 16]) {
        let m = self.emu.p2_tiles().next_pal;
        self.emu.set_p2_palette(m, colours);
    }

    /// Live P2 palette: (slot mask, colours), what the renderer uses now.
    pub fn p2_palette(&self) -> (u128, [u16; 16]) {
        let t = self.emu.p2_tiles();
        (t.pal, t.palette)
    }

    /// Draw `player`'s (0 = P1, 1 = P2) sprites `(dx, dy)` px off from OAM
    /// from the next frame rendered on (netplay smoothing, docs/netplay.md
    /// "Rollback smoothing"). Render time only: OAM, WRAM and save states are
    /// untouched, and a loaded state renders unshifted until set again. The
    /// slots are the last OAM build's (the one the next frame shows), minus
    /// hidden ones (Y = `$E0`).
    pub fn set_display_offset(&mut self, player: usize, dx: i16, dy: i16) {
        let mask = crate::display::visible(self.wram(), self.oam_slots[player & 1]);
        self.emu.set_sprite_offset(mask, dx, dy);
    }

    /// Widescreen (docs/widescreen.md): `margin` extra pixels each side of
    /// the 256-px picture (0 = off). Display only: no WRAM/VRAM/OAM/CGRAM
    /// write, not in save states, kept across `load_state`. [`Game::frame`]
    /// reports the wide size from the next frame on. With a margin, objects
    /// past the picture's edges are drawn in it (margin_sprites.rs), from
    /// the display hook, which this registers when co-op has not.
    pub fn set_wide_margin(&mut self, margin: u16) {
        self.emu.set_wide_margin(margin);
        if margin > 0 {
            crate::coop::register_display_trap(self);
        }
    }

    /// The whole CPU (and open bus) around a display-only isolated pass.
    pub(crate) fn cpu_snapshot(&self) -> snes_core::api::CpuSnapshot {
        self.emu.cpu_snapshot()
    }

    pub(crate) fn restore_cpu_snapshot(&mut self, s: snes_core::api::CpuSnapshot) {
        self.emu.restore_cpu_snapshot(s);
    }

    /// The PPU's margin sprites from the next OAM upload on (display only).
    pub(crate) fn set_margin_sprites(&mut self, sprites: Vec<snes_core::api::MarginSprite>) {
        self.emu.set_margin_sprites(sprites);
    }

    /// The PPU's widescreen HUD slots (left, right) from the next OAM upload
    /// on (display only, docs/widescreen.md "HUD"); set after every step
    /// with a margin (`widescreen::update_margin_tiles`), no effect without.
    pub fn set_hud_shift(&mut self, left: u128, right: u128) {
        self.emu.set_hud_shift(left, right);
    }

    /// The HUD slots the PPU shifts now (left, right).
    pub fn hud_shift(&self) -> (u128, u128) {
        self.emu.hud_shift()
    }

    pub fn wide_margin(&self) -> u16 {
        self.emu.wide_margin()
    }

    /// [`Game::set_wide_margin`] by preset.
    pub fn set_widescreen(&mut self, w: crate::Widescreen) {
        self.set_wide_margin(w.margin_px());
    }

    pub fn widescreen(&self) -> crate::Widescreen {
        crate::Widescreen((self.wide_margin() / 8) as u8)
    }

    /// The margin map for `bg` (0-3): `MARGIN_MAP_CELLS^2` cells, `0` = use
    /// VRAM, else `0x1_0000 | tilemap entry`; `None` clears it.
    pub fn set_margin_tiles(&mut self, bg: usize, cells: Option<&[u32]>) {
        self.emu.set_margin_tiles(bg, cells);
    }

    /// Research: record each rendered line's BG registers ([`Game::line_regs`]).
    pub fn set_line_record(&mut self, on: bool) {
        self.emu.set_line_record(on);
    }

    pub fn line_regs(&self) -> &[snes_core::api::LineRegs] {
        self.emu.line_regs()
    }

    /// The VRAM tilemap entry BG `bg` (0-3) shows at BG-space `(x, y)`.
    pub fn bg_map_entry(&self, bg: usize, x: u16, y: u16) -> u16 {
        self.emu.bg_map_entry(bg, x, y)
    }

    /// Research: the PPU registers (`{:#?}`).
    pub fn ppu_registers_debug(&self) -> String {
        self.emu.ppu_registers_debug()
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

    /// Master clocks since power-on (debug: oracle traces).
    pub fn master_cycles(&self) -> u64 {
        self.emu.master_cycles()
    }

    /// Start (`true`) or stop recording SPC700 instruction starts on this thread
    /// (debug: oracle traces). [`Game::spc_trace_take`] drains them.
    pub fn spc_trace(on: bool) {
        snes_core::api::SPC_TRACE.with(|t| *t.borrow_mut() = on.then(Vec::new));
    }

    /// The SPC700 instruction starts recorded so far, `(PC, SPC cycle since thread start)`.
    pub fn spc_trace_take() -> Vec<(u16, u64)> {
        snes_core::api::SPC_TRACE.with(|t| t.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
    }

    /// PPU position `(scanline, master clocks into the line)` (debug: oracle traces).
    pub fn ppu_position(&self) -> (u16, u64) {
        self.emu.ppu_position()
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
        if let Err(e) = self.run_isolated(addr, ret, false) {
            panic!("call_isolated(${addr:06X}) {e}");
        }
    }

    /// [`Game::call_isolated_ret`] for display-only callers, which skip what
    /// they were drawing instead of panicking. Fails (registers restored,
    /// nothing else) when the routine has not returned within the budget,
    /// or as soon as it unwinds past its own return address (the stack
    /// pointer above the caller's: the sprite emitter's OAM-full exit
    /// `PLX; PLX; JMP` to the build's tail, `$80:D739` in the PAL listing),
    /// before it runs any of its caller's code. Whatever the routine wrote
    /// before that stays written: the caller puts back what it may touch
    /// (the margin pass restores WRAM `$0000..$1FFF` and a CPU snapshot).
    pub fn try_call_isolated_ret(&mut self, addr: u32, ret: TrapReturn) -> Result<(), IsolatedError> {
        self.run_isolated(addr, ret, true)
    }

    fn run_isolated(&mut self, addr: u32, ret: TrapReturn, fallible: bool) -> Result<(), IsolatedError> {
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
        let result = loop {
            self.dispatch_traps();
            let boundary = self.emu.at_instruction_boundary();
            if boundary && self.regs().pbr_pc() == sentinel {
                break Ok(());
            }
            if fallible && boundary {
                // Popped past the return address the call pushed: it is
                // leaving through its caller's frame, which is not there.
                if self.regs().s > saved.s {
                    break Err(IsolatedError::Unwound);
                }
                if budget == 0 {
                    break Err(IsolatedError::Budget);
                }
            }
            if budget == 0 && !fallible {
                break Err(IsolatedError::Budget);
            }
            budget = budget.saturating_sub(1);
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
        };
        self.isolated_depth -= 1;
        self.set_regs(saved);
        self.fired_at = saved_fired;
        result
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
    /// the old 512-word page and no magic, load with an empty one), then
    /// [`COOP_MAGIC`], the length and the [`coop::CoopState`]. See
    /// docs/savestates.md.
    pub fn save_state(&self) -> Vec<u8> {
        let cfg = bincode::config::standard();
        let state = (self.emu.to_save_state(), self.frame_count, self.fired_at);
        let mut v = bincode::encode_to_vec(state, cfg).expect("save state encodes");
        v.extend(P2_TILES_MAGIC);
        v.extend(bincode::encode_to_vec(self.emu.p2_tiles(), cfg).expect("p2 tiles encode"));
        let coop = bincode::encode_to_vec(&self.coop, cfg).expect("coop state encodes");
        v.extend(COOP_MAGIC);
        v.extend((coop.len() as u32).to_le_bytes());
        v.extend(coop);
        v
    }

    /// The co-op tail right after the `P2T5` pages (`COOP`, `u32` LE length,
    /// bincode); `Ok(None)` when absent (saved before it).
    fn coop_tail(t: &[u8]) -> Result<Option<coop::CoopState>, String> {
        let Some(t) = t.strip_prefix(COOP_MAGIC.as_slice()) else { return Ok(None) };
        let n = t.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("co-op state truncated")?;
        let body = t.get(4..4 + n).ok_or("co-op state truncated")?;
        let (c, _) = bincode::decode_from_slice(body, bincode::config::standard()).map_err(|e| format!("co-op state: {e}"))?;
        Ok(Some(c))
    }

    /// Frame count, tile-page marker and co-op state of a save state.
    pub fn state_info(s: &[u8]) -> Result<StateInfo, String> {
        let cfg = bincode::config::standard();
        let ((_, frame_count, _), n): ((SnesEmulator, u64, Option<u32>), _) =
            bincode::decode_from_slice(s, cfg).map_err(|e| e.to_string())?;
        let tiles = [P2_TILES_MAGIC, P2_TILES_MAGIC_V5, P2_TILES_MAGIC_V4, P2_TILES_MAGIC_V3, P2_TILES_MAGIC_V2]
            .into_iter()
            .find(|m| s[n..].starts_with(m.as_slice()))
            .map(|m| String::from_utf8_lossy(m).into_owned());
        let coop = if let Some(t) = s[n..].strip_prefix(P2_TILES_MAGIC.as_slice()) {
            let (_, m): (P2Tiles, _) = bincode::decode_from_slice(t, cfg).map_err(|e| e.to_string())?;
            Self::coop_tail(&t[m..])?
        } else if let Some(t) = s[n..].strip_prefix(P2_TILES_MAGIC_V5.as_slice()) {
            let (_, m): (P2TilesV5, _) = bincode::decode_from_slice(t, cfg).map_err(|e| e.to_string())?;
            Self::coop_tail(&t[m..])?
        } else {
            None
        };
        Ok(StateInfo { frame_count, tiles, coop })
    }

    /// Restores the emulator, the P2 tile pages and [`coop::CoopState`]. The
    /// frontend's settings in `CoopState` (`enabled`, P1 as Zero, P2's
    /// palette) are kept, not taken from the state.
    pub fn load_state(&mut self, s: &[u8]) -> Result<(), String> {
        let cfg = bincode::config::standard();
        let mut coop = None;
        let ((state, frame_count, fired_at), n): ((SnesEmulator, u64, Option<u32>), _) =
            bincode::decode_from_slice(s, cfg).map_err(|e| e.to_string())?;
        self.emu.load_state(state);
        if let Some(tail) = s[n..].strip_prefix(P2_TILES_MAGIC.as_slice()) {
            let (t, m) = bincode::decode_from_slice(tail, cfg).map_err(|e| e.to_string())?;
            self.emu.set_p2_tiles(t);
            coop = Self::coop_tail(&tail[m..])?;
        } else if let Some(tail) = s[n..].strip_prefix(P2_TILES_MAGIC_V5.as_slice()) {
            let ((page, slots, next_page, next_slots, pending, page1, next_page1, page2, next_page2), m): (P2TilesV5, _) =
                bincode::decode_from_slice(tail, cfg).map_err(|e| e.to_string())?;
            self.emu.set_p2_tiles(P2Tiles { page, slots, next_page, next_slots, pending, page1, next_page1, page2, next_page2, ..P2Tiles::default() });
            coop = Self::coop_tail(&tail[m..])?;
        } else if let Some(tail) = s[n..].strip_prefix(P2_TILES_MAGIC_V4.as_slice()) {
            const W3: usize = P2_PAGE_WORDS * 3;
            type V4 = (Box<[u16; W3]>, u128, Box<[u16; W3]>, u128, bool, u128, u128, u128, u128);
            let ((page, slots, next, next_slots, pending, page1, next_page1, page2, next_page2), _): (V4, _) =
                bincode::decode_from_slice(tail, cfg).map_err(|e| e.to_string())?;
            let mut t = P2Tiles { slots, next_slots, pending, page1, next_page1, page2, next_page2, ..P2Tiles::default() };
            t.page[..W3].copy_from_slice(&page[..]);
            t.next_page[..W3].copy_from_slice(&next[..]);
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
        let (enabled, p1_on, palette) = (self.coop.enabled, self.coop.p1.on, self.coop.zero_palette);
        // With a `COOP` tail the pages and the frames they hold came from the
        // same state; without one, a default (P2 hidden: it re-anchors).
        self.coop = coop.unwrap_or_default();
        self.coop.enabled = enabled;
        self.coop.p1.on = p1_on;
        self.coop.zero_palette = palette;
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
