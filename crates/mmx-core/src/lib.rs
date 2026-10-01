//! mmx-core: the game. Wraps the vendored jgenesis SNES core, runs one frame
//! per [`Game::step`], and fires Rust [`traps`] at routine entry addresses.
//! Contract: see `docs/PLAN.md`. std-only, no fs, no clock; builds on wasm32.
#![forbid(unsafe_code)]

pub mod coop;
pub mod coop_hud;
pub mod coop_objects;
pub mod coop_stages;
pub mod display;
mod game;
pub mod hooks;
pub mod margin_sprites;
pub mod netgame;
pub mod netlog;
pub mod p2tiles;
pub mod profile;
pub mod ram;
pub mod saber;
pub mod sfx;
pub mod traps;
pub mod widescreen;
pub mod wide_gameplay;
pub mod zero;
pub mod zero_data;

pub use game::{Game, StateInfo};
pub use widescreen::Widescreen;
/// CPU overclock (`Game::set_overclock`; docs/widescreen.md "Slowdown and overclock").
pub use snes_core::api::CpuOverclock as Overclock;
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
    /// `$FFD9` destination code: 0 Japan, 1 USA (NTSC); 2 and up Europe etc. (PAL).
    pub region: u8,
    pub sha256: [u8; 32],
    pub crc32: u32,
    /// True when a 512-byte copier header was stripped.
    pub had_copier_header: bool,
}

impl RomInfo {
    pub fn region_name(&self) -> &'static str {
        match self.region {
            0 => "Japan (NTSC)",
            1 => "USA (NTSC)",
            2 => "Europe (PAL)",
            _ => "other (PAL)",
        }
    }
    pub fn is_pal(&self) -> bool {
        self.region >= 2
    }
}

/// Video timing the emulator runs the cartridge at. `Auto` follows the header
/// region byte (a PAL dump runs at 50.007 fps, as on a PAL console).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Timing {
    #[default]
    Auto,
    Ntsc,
    Pal,
}

impl Timing {
    pub fn parse(s: &str) -> Option<Timing> {
        match s {
            "auto" => Some(Timing::Auto),
            "ntsc" => Some(Timing::Ntsc),
            "pal" => Some(Timing::Pal),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Timing::Auto => "auto",
            Timing::Ntsc => "ntsc",
            Timing::Pal => "pal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomError {
    TooSmall(usize),
    NotMegaManX { title: String },
    Coprocessor(u8),
    Emulator(String),
    /// A hook the co-op traps need is not in this ROM (`hooks.rs`).
    Hooks(String),
}

impl std::fmt::Display for RomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RomError::TooSmall(n) => write!(f, "ROM too small ({n} bytes)"),
            RomError::NotMegaManX { title } => write!(f, "not Mega Man X (USA): title {title:?}"),
            RomError::Coprocessor(t) => write!(f, "unexpected coprocessor byte ${t:02X}"),
            RomError::Emulator(e) => write!(f, "emulator: {e}"),
            RomError::Hooks(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for RomError {}
