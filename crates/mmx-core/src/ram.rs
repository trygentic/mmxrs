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
/// Sound ring write / read indices (16 entries of {cmd, param}; no full check).
pub const SND_WRITE: u32 = 0x7E0BA3;
pub const SND_READ: u32 = 0x7E0BA4;

// Scheduler: 7 task slots of $10 bytes; current slot index.
pub const TASK_SLOTS: u32 = 0x7E0030;
pub const TASK_STRIDE: usize = 0x10;
pub const TASK_COUNT: usize = 7;
pub const TASK_CURRENT: u32 = 0x7E00A0;
pub const MODE_D4: u32 = 0x7E00D4;
/// `$D2` values: 0 intro, 2 stage select, 4 in stage, `$0A` game over.
pub const D2_IN_STAGE: u8 = 0x04;
/// `$D3` values: 0 load, 2 ready, 4 gameplay, 6 dying, 8 clear.
pub const PHASE_GAMEPLAY: u8 = 0x04;
pub const NMI_DONE: u32 = 0x7E0B9D;
pub const FRAME_NMI: u32 = 0x7E0B9E;
pub const PAD1_PREV: u32 = 0x7E00A9;
pub const PAD2_PREV: u32 = 0x7E00AF;
pub const X_INPUT_PREV: u32 = 0x7E0BE0;
pub const X_SUBPIX_X: u32 = 0x7E0BAC;
pub const X_SUBPIX_Y: u32 = 0x7E0BAF;
/// Bit 6 = facing.
pub const X_FACING: u32 = 0x7E0BB9;
// Enemy slot field offsets (from `ENEMIES_BASE + i * ENEMY_STRIDE`).
pub const ENEMY_ACTIVE: usize = 0x00;
pub const ENEMY_POS_X: usize = 0x05;
pub const ENEMY_POS_Y: usize = 0x08;
pub const ENEMY_ID: usize = 0x0A;
pub const ENEMY_CONTACT_DAMAGE: usize = 0x26;
pub const ENEMY_HP: usize = 0x27;
pub const SHOTS_BASE: u32 = 0x7E0C98;
pub const SHOT_STRIDE: usize = 0x20;
pub const SHOT_COUNT: usize = 12;
/// `$1228`: X's shots and enemy shots (8 x `$40`). Items (pellets, 1-ups) live in `$1428` (see `coop_objects.rs`).
pub const ITEMS_BASE: u32 = 0x7E1228;
pub const PICKUPS_BASE: u32 = 0x7E1428;
pub const ITEM_STRIDE: usize = 0x40;
pub const ITEM_COUNT: usize = 8;
pub const EFFECTS_BASE: u32 = 0x7E1928;
pub const EFFECT_STRIDE: usize = 0x20;
pub const EFFECT_COUNT: usize = 31;
pub const SUB_TANKS: u32 = 0x7E1F83;
pub const SUB_TANK_COUNT: usize = 4;
/// Weapon `i` energy at `WEAPON_ENERGY + 2 * i`.
pub const WEAPON_ENERGY: u32 = 0x7E1F88;
pub const ARMOR: u32 = 0x7E1F99;
pub const HEART_TANKS: u32 = 0x7E1F9C;
pub const BOSS_PTR: u32 = 0x7E1F0E;
pub const OAM_BUF_LEN: usize = 544;
pub const CGRAM_BUF: u32 = 0x7E0300;
/// 16 entries of {cmd, param}; enqueued by `$80:888F`.
pub const SOUND_QUEUE_LEN: usize = 16;
/// Anti-piracy counter (SRAM probes at `$70:xxxx`; no SRAM mapped = pass).
pub const ANTI_PIRACY: u32 = 0x7E1F9F;

/// Index into the 128 KiB WRAM array for a `$7E`/`$7F` address.
pub const fn wram_index(addr: u32) -> usize {
    (addr & 0x1FFFF) as usize
}
