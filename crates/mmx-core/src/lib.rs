//! mmx-core: the game. Wraps the vendored jgenesis SNES core, runs one frame
//! per [`Game::step`], and fires Rust [`traps`] at routine entry addresses.
//! Contract: see `docs/PLAN.md`. std-only, no fs, no clock; builds on wasm32.
#![forbid(unsafe_code)]

pub mod coop;
pub mod ram;
pub mod traps;

pub use traps::{TrapFn, TrapLog, TrapReturn, TrapTable};

/// SNES joypad state per player in auto-read layout:
/// bit15 B, 14 Y, 13 Select, 12 Start, 11 Up, 10 Down, 9 Left, 8 Right,
/// 7 A, 6 X, 5 L, 4 R. Low nibble unused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pads {
    pub p1: u16,
    pub p2: u16,
}

pub const BTN_B: u16 = 1 << 15;
pub const BTN_Y: u16 = 1 << 14;
pub const BTN_SELECT: u16 = 1 << 13;
pub const BTN_START: u16 = 1 << 12;
pub const BTN_UP: u16 = 1 << 11;
pub const BTN_DOWN: u16 = 1 << 10;
pub const BTN_LEFT: u16 = 1 << 9;
pub const BTN_RIGHT: u16 = 1 << 8;
pub const BTN_A: u16 = 1 << 7;
pub const BTN_X: u16 = 1 << 6;
pub const BTN_L: u16 = 1 << 5;
pub const BTN_R: u16 = 1 << 4;

/// 65816 register file, decoupled from the vendored CPU crate's type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u16,
    pub x: u16,
    pub y: u16,
    pub s: u16,
    pub d: u16,
    pub pbr: u8,
    pub pc: u16,
    pub dbr: u8,
    /// NVMXDIZC
    pub p: u8,
    pub emulation: bool,
}

impl Registers {
    pub fn pbr_pc(&self) -> u32 {
        (u32::from(self.pbr) << 16) | u32::from(self.pc)
    }
}

/// What the loader learnt from the internal header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomInfo {
    pub title: String,
    /// `$FFDB`: 0 = v1.0 (the disassembly), 1 = v1.1 (mstan's recomp target).
    pub version: u8,
    pub sha256: [u8; 32],
    pub crc32: u32,
    /// True when a 512-byte copier header was stripped.
    pub had_copier_header: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomError {
    TooSmall(usize),
    NotMegaManX { title: String },
    Coprocessor(u8),
    Emulator(String),
}

impl std::fmt::Display for RomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RomError::TooSmall(n) => write!(f, "ROM too small ({n} bytes)"),
            RomError::NotMegaManX { title } => write!(f, "not Mega Man X (USA): title {title:?}"),
            RomError::Coprocessor(t) => write!(f, "unexpected coprocessor byte ${t:02X}"),
            RomError::Emulator(e) => write!(f, "emulator: {e}"),
        }
    }
}
impl std::error::Error for RomError {}

/// The running game. `game.rs` fills the body; the signatures are
/// the contract every other crate builds against.
pub struct Game {
    pub traps: TrapTable,
    pub traplog: TrapLog,
    pub coop: coop::CoopState,
    inner: Option<Box<dyn std::any::Any>>, // ponytail: placeholder until game.rs lands
}

impl Game {
    /// Load a Mega Man X (USA) ROM image. Strips a 512-byte copier header,
    /// validates the internal header (`MEGAMAN X`, no coprocessor), zeroes
    /// WRAM for determinism.
    pub fn from_rom(_rom: Vec<u8>) -> Result<Game, RomError> {
        todo!("game.rs")
    }
    pub fn rom_info(&self) -> &RomInfo {
        todo!()
    }
    /// Run one video frame with these pads. Traps fire at instruction
    /// boundaries. Never blocks, never touches fs or clock.
    pub fn step(&mut self, _pads: Pads) {
        todo!()
    }
    /// Last completed frame as 0x00RRGGBB pixels, plus width and height
    /// (256 or 512 wide; 224/239, doubled when interlaced).
    pub fn frame(&self) -> (&[u32], usize, usize) {
        todo!()
    }
    /// Stereo samples produced since the last call (48 kHz unless
    /// [`Game::set_audio_rate`] changed it).
    pub fn take_audio(&mut self) -> Vec<(f32, f32)> {
        todo!()
    }
    pub fn set_audio_rate(&mut self, _hz: u32) {
        todo!()
    }
    pub fn wram(&self) -> &[u8; 0x20000] {
        todo!()
    }
    pub fn wram_mut(&mut self) -> &mut [u8; 0x20000] {
        todo!()
    }
    /// OAM as the PPU holds it: 512 bytes low table + 32 bytes high table.
    pub fn oam(&self) -> [u8; 544] {
        todo!()
    }
    pub fn cgram(&self) -> [u16; 256] {
        todo!()
    }
    pub fn regs(&self) -> Registers {
        todo!()
    }
    pub fn set_regs(&mut self, _r: Registers) {
        todo!()
    }
    pub fn frame_count(&self) -> u64 {
        todo!()
    }
    /// Run the ROM routine at `addr` (24-bit) to completion on the CPU only:
    /// PPU, APU and the master clock do not advance, so the call costs zero
    /// emulated cycles. Used by co-op to run the player update a second time.
    /// Pushes a return to a sentinel, ticks the CPU until PC reaches it,
    /// then restores the caller's registers.
    pub fn call_isolated(&mut self, _addr: u32) {
        todo!()
    }
    pub fn save_state(&self) -> Vec<u8> {
        todo!()
    }
    pub fn load_state(&mut self, _s: &[u8]) -> Result<(), String> {
        todo!()
    }
}
