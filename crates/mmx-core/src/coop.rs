//! Two-player co-op: player 2 as a shadow copy of X's object block, driven by
//! the ROM's own player code on pad 2 (z2rs model).
//! See `docs/PLAN.md` "Co-op hooks" and `docs/coop.md`.
//!
//! Model: X's whole object block `$0BA8..$0C38` is per-player. Around every
//! P2 pass it is exchanged with [`CoopState::p2`], the ROM runs once more on
//! P2's bytes, and the bytes are exchanged back. Everything else (camera,
//! lives, weapon energy, sub-tanks, RNG, sound queue, enemies) is shared.
//! X's remapped input `$0BDE..$0BE3` lives *inside* the block (offset `$36`),
//! so it needs no separate shadow.
//!
//! Hooks (all `TrapReturn::Observe`: the body runs, then the ROM bytes; real
//! cartridge PCs, see the constants below): update slice after `$81:812E`,
//! physics slice after `$81:9D67`, display slice after the player emitter,
//! and a contact pass on every enemy's `JSL $84:9B03`.
//!
//! Default off: every body returns first thing when `enabled` is false, so a
//! registered-but-disabled group leaves the game byte-identical.
//!
//! Zero-cycle slicing: all P2 ROM work goes through [`Game::call_isolated`],
//! which ticks only the CPU (no PPU/APU/master clock), so P2 never makes the
//! main loop lag.

use crate::ram::{self, wram_index as ix, X_BLOCK_LEN};
use crate::traps::TrapReturn;
use crate::{Game, BTN_SELECT, BTN_START};

// ------------------------------------------------------------ addresses

/// P2 update slice hook: the instruction after the object master's
/// `JSL $81:812E` at `$80:D1EB`, i.e. the moment P1's update has returned.
/// Unconditional. Address confirmed by `cargo xtask trace --calls` on the
/// real ROM (the disassembly's bank-`$80` PCs drift from this ROM by up to
/// `0x41`; bank `$81`/`$84` PCs matched).
pub const HOOK_UPDATE: u32 = 0x80D1EF;
/// P2 physics slice hook: the instruction after `JSL $81:9D67` at `$80:D23D`
/// (the object master, after the enemy loops). Confirmed by trace.
pub const HOOK_PHYSICS: u32 = 0x80D241;
/// P2 display slice hook: `$80:D5FA` (OAM lists 3-5), the call right after the
/// player emitter inside the OAM build `$80:D544`. Confirmed by trace.
pub const HOOK_DISPLAY: u32 = 0x80D5FA;
/// Enemy-vs-X contact routine: every enemy's AI `JSL`s it with `D` = the
/// enemy (154 call sites; `$82:9C41` for the Highway bikes), `RTL`, A=1 on a
/// hit. It checks X's i-frames (`$0C30`/`$0C32`, in the block), runs the box
/// test `$84:9C0E` with `X=$0BA8`, then `JMP $84:9D07` applies the damage
/// (`$0BAA`, `$0BCF`, `$0BD8`, `$0C11`, `$0C32`: all in the block). Found by
/// `trace --find` on the frame P1's HP dropped. Observe hook on the entry.
pub const HOOK_CONTACT: u32 = 0x849B03;
/// Player update: `PHP/PHD`, sets `D=$0BA8` itself, ends `RTL` (JSL entry).
pub const PLAYER_UPDATE: u32 = 0x81812E;
/// Player terrain/physics phase (`JSL` from `$80:D23D`, ends `RTL`): collision
/// against the layout, ground flag, camera requests. Confirmed by trace.
pub const PLAYER_PHYSICS: u32 = 0x819D67;
/// Input remapper (`JSR` from `$80:9A52` in the per-frame stage code, ends
/// `RTS`): pad 1 (`$A7`) + button config -> the block's `$0BDE..$0BE3`.
/// Confirmed by trace.
pub const INPUT_REMAP: u32 = 0x80E518;
/// Player OAM emitter (`JSR` from `$80:D55D`, ends `RTS`). Needs
/// `M=1, X=0, D=0` (true at [`HOOK_DISPLAY`]). Confirmed by trace.
pub const PLAYER_OAM: u32 = 0x80D671;
/// Companion-object update loop over `$0C38..$0C98` (`JSR` from `$80:D26C`,
/// `RTS`; listing `$80:D2D3`, drift `-$41`). Sets `D` itself.
pub const FX_UPDATE: u32 = 0x80D292;
/// Companion-object OAM emitter (`JSR` from `$80:D55A`, just before
/// [`PLAYER_OAM`]; listing `$80:D681`). Same register needs as the player's.
pub const FX_OAM: u32 = 0x80D640;

/// Direct-page OAM cursor: next free OAM slot (`AND #$7F`, `*4` = byte
/// offset into `$0700`). `$E5` = high-table byte index, `$E6` = accumulator.
const OAM_CURSOR: usize = 0x00E4;
/// `$D2` / `$D3` values for "in stage" / "gameplay".
const MODE_IN_STAGE: u8 = 4;
const PHASE_GAMEPLAY: u8 = 4;
/// Pad-2 held word (`$AD` low = `$4218`, `$AE` high = `$4219`).
const PAD2_HELD: u32 = ram::PAD2_HELD;

// ponytail: shared bytes the player update writes that P2 must not own.
// From a static walk of `$81:812E` (see docs/coop.md). Restored after every
// P2 slice so P1 keeps the camera and P2 can't trigger freezes/boss doors.
// unverified: exact widths; restoring may also hide a legit P2 side effect.
const P1_OWNED: [(u32, usize); 2] = [
    (0x7E1E48, 0x1E8A - 0x1E48), // camera requests / limits ($1E5E..$1E88 written)
    (0x7E1F13, 7),               // object freeze flags $1F13..$1F19
];

/// Per-player scratch outside the block that the player update writes and
/// reads back next frame (collision / landing state, from the disassembly's
/// write list). Swapped with a P2 shadow around every slice like the block;
/// without this P2 read P1's grounded state and "landed" at its jump apex.
// unverified: exact extent; these two ranges fixed the apex landing on the real ROM.
///
/// `$0C38..$0C98` are X's three companion objects (`$20` each): the armour
/// part overlays spawned by `$81:9D7E` from `$1F99`, updated by `$80:D292`
/// and drawn by `$80:D640` right before the player emitter. Swapping them
/// gives P2 its own set (run in P2's physics and display slices).
const P2_SWAPPED: [(u32, usize); 3] = [(0x7E1F31, 7), (0x7E1FA1, 16), (0x7E0C38, 0x60)];
const P2_SWAPPED_LEN: usize = 7 + 16 + 0x60;

fn swap_scratch(w: &mut [u8; 0x20000], shadow: &mut [u8; P2_SWAPPED_LEN]) {
    let mut o = 0;
    for &(a, n) in &P2_SWAPPED {
        for k in 0..n {
            std::mem::swap(&mut w[ix(a) + k], &mut shadow[o + k]);
        }
        o += n;
    }
}

// ------------------------------------------------------------ block layout

/// Offsets into the X block (`$0BA8 + off`).
pub const OFF_ACTION: usize = 0x02; // $0BAA: $0C death, $0E hurt
pub const OFF_POS_X: usize = 0x05; // $0BAD word
pub const OFF_POS_Y: usize = 0x08; // $0BB0 word
pub const OFF_VEL_X: usize = 0x1A; // $0BC2 word
pub const OFF_VEL_Y: usize = 0x1C; // $0BC4 word
pub const OFF_HP: usize = 0x27; // $0BCF, bit7 = HUD redraw
pub const OFF_INPUT: usize = 0x36; // $0BDE held, $0BE0 prev, $0BE2 pressed
const ACTION_DEATH: u8 = 0x0C;

// ------------------------------------------------------------ tunables

/// P2 spawns this many pixels right of P1.
pub const P2_X_OFFSET: u16 = 16;
/// Frames P2 stays down before respawning next to P1 (no life cost).
pub const P2_DOWN_FRAMES: u16 = 120;
/// unverified: sprite palette for "Zero red". Placeholder; nobody checked
/// what palette 7 holds in each stage, nor wrote Zero's colours into it.
pub const ZERO_PALETTE: u8 = 3;
/// Skip the P2 display pass when fewer OAM slots than this are left: the
/// emitter's overflow exit (`$80:D77B`: `PLX; PLX; JMP $D5E2`) would unwind
/// through `call_isolated`'s sentinel frame. unverified: X's max sprite count.
const P2_MAX_SPRITES: u8 = 32;

// ------------------------------------------------------------ state

/// Off by default; never registered by any verification path.
// netplay: Encode/Decode so rollback save states carry P2 (netgame.rs).
#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct CoopState {
    pub enabled: bool,
    /// P2's copy of `$0BA8..$0C38` (HP at [`OFF_HP`], input at [`OFF_INPUT`]).
    pub p2: [u8; X_BLOCK_LEN],
    /// Not in play (outside stage gameplay, or down). Re-anchors on show.
    pub p2_hidden: bool,
    /// First OAM slot P2 got this frame (diagnostic).
    pub p2_oam_base: usize,
    /// P1's Y on the previous frame while P2 waits to be anchored.
    pub last_p1_y: Option<u16>,
    /// P2's copy of [`P2_SWAPPED`].
    pub p2_scratch: [u8; P2_SWAPPED_LEN],
    /// OAM slots the emitter wrote for P2 this frame (diagnostic).
    pub p2_oam_count: usize,
    /// `(D, DB, P)` seen at the display hook (diagnostic).
    pub dbg_regs: (u16, u8, u8),
    /// Sprite palette (0-7) forced onto P2's OAM entries.
    pub zero_palette: u8,
    /// Frames left while P2 is down.
    pub down_frames: u16,
    /// Hits P2 took from enemy contact (diagnostic).
    pub contact_hits: u32,
    /// Inside P2's contact pass (the isolated call re-enters [`HOOK_CONTACT`]).
    pub in_contact: bool,
}

impl Default for CoopState {
    fn default() -> Self {
        Self {
            enabled: false,
            p2: [0; X_BLOCK_LEN],
            p2_hidden: true,
            p2_oam_base: 0,
            last_p1_y: None,
            p2_scratch: [0; P2_SWAPPED_LEN],
            p2_oam_count: 0,
            dbg_regs: (0, 0, 0),
            zero_palette: ZERO_PALETTE,
            down_frames: 0,
            contact_hits: 0,
            in_contact: false,
        }
    }
}

// ------------------------------------------------------------ pure helpers

/// Exchange the live X block with the shadow.
pub fn swap_block(wram: &mut [u8; 0x20000], shadow: &mut [u8; X_BLOCK_LEN]) {
    let b = ix(ram::X_BASE);
    wram[b..b + X_BLOCK_LEN].swap_with_slice(shadow);
}

/// Sound ring `$0B72` (16 x {cmd, param}): write index `$0BA3`, read index
/// `$0BA4` (NMI sends one entry per frame, `$80:8822`), both `AND #$1E`.
/// Enqueue (`$80:888F`/`$80:88D4`) has no full check: a 17th pending entry
/// laps the reader and the whole backlog is lost.
const SND_WRITE: usize = 0x0BA3;
const SND_READ: usize = 0x0BA4;
/// P2 may enqueue only while this many entries or fewer are pending after its slice.
const P2_SOUND_CAP: u8 = 8;

/// Pending sound entries in the ring.
pub fn sound_pending(write: u8, read: u8) -> u8 {
    (write.wrapping_sub(read) & 0x1E) / 2
}

/// Clamp P2's world X into the visible window `[cam, cam + 256 - 16]`.
/// unverified: `$05` is the sprite anchor (emitter offsets are -8/-16), so a
/// tighter window may look better once seen on screen.
pub fn clamp_p2_x(x: u16, camera_x: u16) -> u16 {
    x.clamp(camera_x, camera_x.saturating_add(256 - 16))
}

/// Force sprite palette `pal` (0-7) on one 4-byte OAM entry (attribute byte
/// `vhppccct`, palette bits 1-3).
pub fn set_oam_palette(entry: &mut [u8], pal: u8) {
    entry[3] = (entry[3] & !0x0E) | ((pal & 7) << 1);
}

/// P2's block when it (re)appears: P1's block, `P2_X_OFFSET` px to the right,
/// at rest, HP = max HP (`$1F9A`).
/// ponytail: copies P1's action too; if P1 is mid-hurt P2 is as well.
pub fn respawn_anchor(wram: &[u8; 0x20000]) -> [u8; X_BLOCK_LEN] {
    let b = ix(ram::X_BASE);
    let mut p2 = [0u8; X_BLOCK_LEN];
    p2.copy_from_slice(&wram[b..b + X_BLOCK_LEN]);
    let x = u16::from_le_bytes([p2[OFF_POS_X], p2[OFF_POS_X + 1]]);
    p2[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&x.saturating_add(P2_X_OFFSET).to_le_bytes());
    p2[OFF_VEL_X..OFF_VEL_X + 4].fill(0);
    p2[OFF_HP] = wram[ix(ram::MAX_HP)] & 0x7F;
    // `+$18` stays X's tile window; P2's streamed tiles go to the PPU's
    // shadow page instead of VRAM (p2tiles.rs). `+$17` bit 7 asks for a reload.
    p2[0x17] |= 0x80;
    // The emitter keeps bits 0-5 of `+$11`, so it writes P2's palette itself.
    p2[0x11] = (p2[0x11] & !0x0E) | (ZERO_PALETTE << 1);
    p2
}

/// Pit rule of the camera code `$80:E14A`: more than `$20` px below the
/// bottom of the 224-line screen. Signed like the ROM's `CMP`/`BMI`.
pub fn fell_off(y: u16, cam_y: u16) -> bool {
    (y.wrapping_sub(0x20).wrapping_sub(cam_y.wrapping_add(0xE0)) as i16) >= 0
}

/// P2 is dead: `$81:96CB` sets action `$0C` and HP `$80`.
pub fn p2_is_down(p2: &[u8; X_BLOCK_LEN]) -> bool {
    p2[OFF_ACTION] == ACTION_DEATH || p2[OFF_HP] & 0x7F == 0
}

// ------------------------------------------------------------ Game glue

/// Register the co-op trap group. Not part of any verification trap set.
pub fn register_coop_traps(game: &mut Game) {
    crate::zero::register_zero_traps(game);
    game.traps.register(HOOK_UPDATE, "coop_p2_update", TrapReturn::Observe, p2_update);
    game.traps.register(HOOK_PHYSICS, "coop_p2_physics", TrapReturn::Observe, p2_physics);
    game.traps.register(HOOK_DISPLAY, "coop_p2_display", TrapReturn::Observe, p2_display);
    game.traps.register(HOOK_CONTACT, "coop_contact", TrapReturn::Observe, contact);
}

fn word(w: &[u8; 0x20000], addr: u32) -> u16 {
    u16::from_le_bytes([w[ix(addr)], w[ix(addr) + 1]])
}

fn go_down(game: &mut Game) {
    game.coop.p2_hidden = true;
    game.coop.down_frames = P2_DOWN_FRAMES;
}

/// [`HOOK_UPDATE`]: P2 update slice.
fn p2_update(game: &mut Game) {
    if !game.coop.enabled {
        return;
    }
    let w = game.wram();
    if w[ix(ram::MODE_D2)] != MODE_IN_STAGE || w[ix(ram::STAGE_PHASE)] != PHASE_GAMEPLAY {
        game.coop.p2_hidden = true;
        game.coop.down_frames = 0;
        return;
    }
    if game.coop.down_frames > 0 {
        game.coop.down_frames -= 1;
        return;
    }
    if game.coop.p2_hidden {
        // Anchor only once P1 stands still vertically: a copy taken during the
        // stage-entry drop inherits an action that never lands (seen on the
        // real ROM: P2 fell through the floor).
        let y = word(game.wram(), ram::X_POS_Y);
        let grounded = word(game.wram(), ram::X_VEL_Y) == 0 && Some(y) == game.coop.last_p1_y;
        game.coop.last_p1_y = Some(y);
        if !grounded {
            return;
        }
        game.coop.p2 = respawn_anchor(game.wram());
        // Debug: `MMX_COOP_P2_HP=N` spawns P2 with N HP (to test HP-0 deaths).
        if let Some(hp) = std::env::var("MMX_COOP_P2_HP").ok().and_then(|v| v.parse().ok()) {
            game.coop.p2[OFF_HP] = hp;
        }
        // Scratch starts as a copy of P1's (grounded) state.
        let mut copy = [0u8; P2_SWAPPED_LEN];
        let w = game.wram();
        let mut o = 0;
        for &(a, n) in &P2_SWAPPED {
            copy[o..o + n].copy_from_slice(&w[ix(a)..ix(a) + n]);
            o += n;
        }
        game.coop.p2_scratch = copy;
        game.coop.p2_hidden = false;
    }
    if p2_is_down(&game.coop.p2) {
        // Never run the death handler on P2: it drives the stage-wide death flow.
        return go_down(game);
    }

    let pad = word(game.wram(), PAD2_HELD) & !(BTN_START | BTN_SELECT);
    with_p2_swapped(game, |game| {
        // Run the ROM's own input remapper (`$80:E518`, JSR/RTS) with pad 2
        // in pad 1's slot: it reads `$A7` and the block's previous input and
        // writes the block's `$0BDE..$0BE3`, so P2 gets P1's button config.
        let w = game.wram_mut();
        let i = ix(ram::PAD1_HELD);
        let saved_pad1 = [w[i], w[i + 1]];
        w[i..i + 2].copy_from_slice(&pad.to_le_bytes());
        game.call_isolated_ret(INPUT_REMAP, crate::TrapReturn::Rts);
        let w = game.wram_mut();
        w[i..i + 2].copy_from_slice(&saved_pad1);
        game.call_isolated(PLAYER_UPDATE);
    });
}

/// `$80:D241` (after the object master's `JSL $81:9D67`): P2's terrain /
/// physics phase, the second half of the player's frame that runs after the
/// enemy loops. Without it P2 kept a stale ground flag (`$0BD3` bit 2) and
/// "landed" at the apex of every jump. Then clamp P2 into the camera window
/// and check for death.
fn p2_physics(game: &mut Game) {
    if !game.coop.enabled || game.coop.p2_hidden || p2_is_down(&game.coop.p2) {
        return;
    }
    with_p2_swapped(game, |game| {
        game.call_isolated(PLAYER_PHYSICS);
        game.call_isolated_ret(FX_UPDATE, crate::TrapReturn::Rts);
        let w = game.wram_mut();
        let cam = word(w, ram::CAMERA_X);
        let x = word(w, ram::X_POS_X);
        let cx = clamp_p2_x(x, cam);
        if cx != x {
            // ponytail: no collision check; a clamp can push P2 into a wall.
            w[ix(ram::X_POS_X)..ix(ram::X_POS_X) + 2].copy_from_slice(&cx.to_le_bytes());
            w[ix(ram::X_VEL_X)..ix(ram::X_VEL_X) + 2].fill(0);
        }
    });
    let cam_y = word(game.wram(), ram::CAMERA_Y);
    let p2 = &mut game.coop.p2;
    if fell_off(u16::from_le_bytes([p2[OFF_POS_Y], p2[OFF_POS_Y + 1]]), cam_y) {
        // The ROM's pit death lives in the camera code (`$80:E14A`, P1 only):
        // Y - $20 >= camera Y + $E0 -> `$84:9F2A` with $7F damage. Same rule
        // here, but the result is just "P2 down" (no stage-wide death flow).
        p2[OFF_ACTION] = ACTION_DEATH;
        p2[OFF_HP] = 0x80;
    }
    if p2_is_down(&game.coop.p2) {
        go_down(game);
    }
}

/// Swap P2's block and scratch in, run `f`, swap out, and put back every
/// P1-owned byte the slice may have touched (camera requests, freeze flags).
fn with_p2_swapped(game: &mut Game, f: impl FnOnce(&mut Game)) {
    // Debug: `MMX_COOP_DIFF=1` prints every byte a P2 slice leaves changed
    // outside the block (after the P1-owned restore), i.e. P2's global side effects.
    static DIFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let before = DIFF.get_or_init(|| std::env::var_os("MMX_COOP_DIFF").is_some()).then(|| Box::new(*game.wram()));
    let saved: Vec<Vec<u8>> =
        P1_OWNED.iter().map(|&(a, n)| game.wram()[ix(a)..ix(a) + n].to_vec()).collect();
    let mut p2 = game.coop.p2;
    let mut scratch = game.coop.p2_scratch;
    let snd = game.wram()[SND_WRITE];
    let w = game.wram_mut();
    swap_block(w, &mut p2);
    swap_scratch(w, &mut scratch);
    f(game);
    // P2's sounds share P1's ring; drop them (rewind the write index) rather
    // than let the ring lap its reader.
    let w = game.wram_mut();
    if sound_pending(w[SND_WRITE], w[SND_READ]) > P2_SOUND_CAP {
        w[SND_WRITE] = snd;
    }
    let w = game.wram_mut();
    swap_block(w, &mut p2);
    swap_scratch(w, &mut scratch);
    for (&(a, n), s) in P1_OWNED.iter().zip(&saved) {
        w[ix(a)..ix(a) + n].copy_from_slice(s);
    }
    game.coop.p2 = p2;
    game.coop.p2_scratch = scratch;
    if let Some(b) = before {
        let d: Vec<String> = (0..0x20000)
            .filter(|&i| b[i] != game.wram()[i])
            .map(|i| format!("${:05X}:{:02X}->{:02X}", 0x7E0000 + i, b[i], game.wram()[i]))
            .collect();
        if !d.is_empty() {
            eprintln!("coop diff frame {} {}", game.frame_count(), d.join(" "));
        }
    }
}

/// [`HOOK_DISPLAY`]: P2 display slice. Runs the ROM's own emitters on P2's
/// block through the game's OAM cursor `$E4`, so P2 lands in the next free
/// slots (no collision with the game's entries by construction), then forces
/// the Zero palette on exactly the slots it got.
fn p2_display(game: &mut Game) {
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    let r = game.regs();
    game.coop.dbg_regs = (r.d, r.dbr, r.p);
    let first = game.wram()[OAM_CURSOR] & 0x7F;
    if first > 128 - P2_MAX_SPRITES {
        return;
    }
    let pal = game.coop.zero_palette;
    let mut p2 = game.coop.p2;
    let mut scratch = game.coop.p2_scratch;
    swap_block(game.wram_mut(), &mut p2);
    swap_scratch(game.wram_mut(), &mut scratch);
    // Both emitters are entered by JSR and end in RTS; armour parts first,
    // as in the ROM's OAM build.
    game.call_isolated_ret(FX_OAM, crate::TrapReturn::Rts);
    game.call_isolated_ret(PLAYER_OAM, crate::TrapReturn::Rts);
    let w = game.wram_mut();
    swap_block(w, &mut p2);
    swap_scratch(w, &mut scratch);
    let last = w[OAM_CURSOR] & 0x7F;
    for slot in usize::from(first)..usize::from(last.max(first)) {
        let o = ix(ram::OAM_BUF) + slot * 4;
        set_oam_palette(&mut w[o..o + 4], pal);
    }
    game.coop.p2_oam_base = usize::from(first);
    game.coop.p2_oam_count = usize::from(last.max(first) - first);
    crate::p2tiles::mark_p2_slots(game);
}

/// `$84:9B03` entry, `D` = the enemy: run the same contact routine once more
/// with P2's block swapped in, so the enemy can hit P2 (damage, knockback,
/// i-frames and death all land in P2's block). The result (A) is thrown away:
/// the enemy's AI only sees P1's pass, which the ROM runs right after.
/// ponytail: an enemy that dies on contact dies only when it touches P1.
fn contact(game: &mut Game) {
    let p2 = &game.coop.p2;
    // `$0C30`/`$0C32` set = P2 is hurt; the ROM would then test the ride
    // armour `$0E18` instead, which is shared, so skip.
    if !game.coop.enabled || game.coop.in_contact || game.coop.p2_hidden || p2_is_down(p2) || p2[0x88] != 0 || p2[0x8A] != 0 {
        return;
    }
    // The box test scribbles on direct page `$00..$11` and the damage on `$0000`;
    // P1's pass may bail before writing them, so put them back.
    let dp: [u8; 0x20] = game.wram()[..0x20].try_into().unwrap();
    let hp = p2[OFF_HP];
    game.coop.in_contact = true;
    with_p2_swapped(game, |game| game.call_isolated(HOOK_CONTACT));
    game.coop.in_contact = false;
    game.wram_mut()[..0x20].copy_from_slice(&dp);
    if game.coop.p2[OFF_HP] != hp {
        game.coop.contact_hits = game.coop.contact_hits.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wram() -> Box<[u8; 0x20000]> {
        vec![0u8; 0x20000].into_boxed_slice().try_into().unwrap()
    }

    #[test]
    fn coop_default_off() {
        let s = CoopState::default();
        assert!(!s.enabled && s.p2_hidden && s.zero_palette == ZERO_PALETTE);
    }

    #[test]
    fn coop_swap_roundtrip() {
        let mut w = wram();
        let b = ix(ram::X_BASE);
        w[b..b + X_BLOCK_LEN].fill(0x11);
        w[b - 1] = 0xEE;
        w[b + X_BLOCK_LEN] = 0xEE;
        let mut s = [0x22u8; X_BLOCK_LEN];
        swap_block(&mut w, &mut s);
        assert!(w[b..b + X_BLOCK_LEN].iter().all(|&v| v == 0x22));
        assert!(s.iter().all(|&v| v == 0x11));
        assert_eq!((w[b - 1], w[b + X_BLOCK_LEN]), (0xEE, 0xEE));
        swap_block(&mut w, &mut s);
        assert!(w[b..b + X_BLOCK_LEN].iter().all(|&v| v == 0x11));
    }

    #[test]
    fn coop_input_offset_is_0bde() {
        assert_eq!(ix(ram::X_BASE) + OFF_INPUT, ix(ram::X_INPUT_HELD));
        assert_eq!(ix(ram::X_BASE) + OFF_HP, ix(ram::X_HP));
        assert_eq!(ix(ram::X_BASE) + OFF_POS_X, ix(ram::X_POS_X));
        assert_eq!(ix(ram::X_BASE) + OFF_VEL_X, ix(ram::X_VEL_X));
    }

    #[test]
    fn coop_clamp() {
        assert_eq!(clamp_p2_x(0x0100, 0x0100), 0x0100);
        assert_eq!(clamp_p2_x(0x00F0, 0x0100), 0x0100);
        assert_eq!(clamp_p2_x(0x0300, 0x0100), 0x01F0);
        assert_eq!(clamp_p2_x(0x0180, 0x0100), 0x0180);
        assert_eq!(clamp_p2_x(0xFFFF, 0xFFF0), 0xFFFF);
    }

    #[test]
    fn coop_fell_off() {
        assert!(!fell_off(0x01FF, 0x0100));
        assert!(fell_off(0x0200, 0x0100));
        assert!(!fell_off(0x0010, 0x0000));
    }

    #[test]
    fn coop_sound_pending() {
        assert_eq!(sound_pending(0x04, 0x00), 2);
        assert_eq!(sound_pending(0x02, 0x1E), 2);
        assert_eq!(sound_pending(0x10, 0x10), 0);
    }

    #[test]
    fn coop_oam_palette() {
        let mut e = [0x10, 0x20, 0x30, 0b1100_0011];
        set_oam_palette(&mut e, 7);
        assert_eq!(e, [0x10, 0x20, 0x30, 0b1100_1111]);
        set_oam_palette(&mut e, 2);
        assert_eq!(e[3], 0b1100_0101);
    }

    #[test]
    fn coop_respawn_anchor() {
        let mut w = wram();
        let b = ix(ram::X_BASE);
        w[b..b + X_BLOCK_LEN].fill(0x33);
        w[ix(ram::X_POS_X)..ix(ram::X_POS_X) + 2].copy_from_slice(&0x01F8u16.to_le_bytes());
        w[ix(ram::MAX_HP)] = 0x10;
        let p2 = respawn_anchor(&w);
        assert_eq!(u16::from_le_bytes([p2[OFF_POS_X], p2[OFF_POS_X + 1]]), 0x0208);
        assert_eq!(p2[OFF_HP], 0x10);
        assert_eq!(&p2[OFF_VEL_X..OFF_VEL_X + 4], &[0, 0, 0, 0]);
        assert_eq!(p2[OFF_INPUT], 0x33);
        assert!(!p2_is_down(&p2));
        let mut dead = p2;
        dead[OFF_ACTION] = ACTION_DEATH;
        assert!(p2_is_down(&dead));
        let mut dead = p2;
        dead[OFF_HP] = 0x80;
        assert!(p2_is_down(&dead));
    }
}
