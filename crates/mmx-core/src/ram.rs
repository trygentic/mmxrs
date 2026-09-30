//! WRAM map for Mega Man X (USA) v1.0 as 24-bit addresses. Derived from the
//! MarkAlarm disassembly (uncommented dump; names are ours). Later work extends
//! from `docs/PLAN.md`; keep every constant `pub const NAME: u32 = 0x7Exxxx`.

pub const MODE_D1: u32 = 0x7E00D1;
pub const MODE_D2: u32 = 0x7E00D2;
pub const STAGE_PHASE: u32 = 0x7E00D3;
pub const PAD1_HELD: u32 = 0x7E00A7;
pub const PAD1_PRESSED: u32 = 0x7E00AB;
pub const PAD2_HELD: u32 = 0x7E00AD;
pub const PAD2_PRESSED: u32 = 0x7E00B1;
pub const FRAME_MAIN: u32 = 0x7E0B9B;
pub const FRAME_STAGE: u32 = 0x7E0B9C;
pub const RNG: u32 = 0x7E0BA6;
/// X's object block start; player code runs with `D = $0BA8`.
pub const X_BASE: u32 = 0x7E0BA8;
pub const X_BLOCK_LEN: usize = 0x90;
pub const X_ACTION: u32 = 0x7E0BAA;
pub const X_POS_X: u32 = 0x7E0BAD;
pub const X_POS_Y: u32 = 0x7E0BB0;
pub const X_VEL_X: u32 = 0x7E0BC2;
pub const X_VEL_Y: u32 = 0x7E0BC4;
pub const X_HP: u32 = 0x7E0BCF;
pub const X_WEAPON: u32 = 0x7E0BDB;
pub const X_INPUT_HELD: u32 = 0x7E0BDE;
pub const X_INPUT_PRESSED: u32 = 0x7E0BE2;
pub const X_IFRAMES: u32 = 0x7E0C13;
pub const ENEMIES_BASE: u32 = 0x7E0E68;
pub const ENEMY_STRIDE: usize = 0x40;
pub const ENEMY_COUNT: usize = 15;
pub const CAMERA_X: u32 = 0x7E1E4D;
pub const CAMERA_Y: u32 = 0x7E1E50;
pub const LEVEL_ID: u32 = 0x7E1F7A;
pub const LIVES: u32 = 0x7E1F80;
pub const MAX_HP: u32 = 0x7E1F9A;
pub const PAUSE: u32 = 0x7E1F24;
pub const OAM_BUF: u32 = 0x7E0700;
pub const SOUND_QUEUE: u32 = 0x7E0B72;

/// Index into the 128 KiB WRAM array for a `$7E`/`$7F` address.
pub const fn wram_index(addr: u32) -> usize {
    (addr & 0x1FFFF) as usize
}
