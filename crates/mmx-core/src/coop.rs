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
//! physics slice after `$81:9D67`, display slice after the whole OAM build
//! (plus a hide of P2's old slots at the build's start), and a contact pass
//! on every enemy's `JSL $84:9B03`.
//!
//! Default off: every body returns first thing when `enabled` is false, so a
//! registered-but-disabled group leaves the game byte-identical.
//!
//! Zero-cycle slicing: all P2 ROM work goes through [`Game::call_isolated`],
//! which ticks only the CPU (no PPU/APU/master clock), so P2 never makes the
//! main loop lag.

use crate::coop_stages;
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
/// P2 display slice hook: `$80:9A86`, the instruction after `JSR $80:D544`
/// (the OAM build) at `$80:9A83` in the per-frame stage code. The build ends
/// with the hide loop over slots `$E4..$E3` and leaves `$E3` = slots used,
/// `$E4..$EB` = 0; `trace --calls` shows D544 entered once, from 9A83, and
/// nothing after 9A86 in the frame touches `$E3`/`$E4`. Emitting here (and
/// putting `$E3`/`$E4` back) keeps the ROM's hide loop, and so P1's lag
/// frames, identical to a solo run (B4: emitting inside the build shortened
/// the loop and dropped a lag frame). The camera is final for the frame here.
pub const HOOK_DISPLAY: u32 = 0x809A86;
/// Second instruction of the OAM build (`$80:D544 PHP`, `$D545 PHB`): P2's
/// slots from the previous frame are hidden here, every build, including the
/// weapon menu's (paused frames `JMP $D544` and never reach [`HOOK_DISPLAY`]).
/// `$D544` itself is taken by `zero::HOOK_OAM_BUILD`.
pub const HOOK_OAM_HIDE: u32 = 0x80D545;
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
const OAM_HI_INDEX: usize = 0x00E5;
const OAM_HI_ACC: usize = 0x00E6;
/// Slots the game used last build (`$E3`, written at the end of `$80:D544`).
const OAM_USED: usize = 0x00E3;
/// OAM high table (`$0900`, 2 bits per slot: X bit 8, size).
const OAM_HI: u32 = 0x7E0900;
/// Y the ROM's hide loop (`$80:D594`) writes into unused slots. It writes
/// nothing else (no size / high-table bits), so neither do we.
const OAM_HIDDEN_Y: u8 = 0xE0;
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
/// `$0BDD`: X's live shot count. The fire routine (`$81:93A8`) refuses when
/// it is `>=` the cap `+$67` and does `INC $35` (direct page, so P2's own
/// copy); the shot's despawn does `DEC $0BDD` (absolute, `$81:A2C0`,
/// `$81:A3FD`, `$81:A4BA`, `$81:A541`) from the shot loop, where P1's block
/// is live.
pub const OFF_SHOTS: usize = 0x35;
/// X's shots: 8 slots of `$40` at `$1228` (`ram::ITEMS_BASE`), `+0` = active.
const X_SHOTS: u32 = 0x7E1228;
const ACTION_DEATH: u8 = 0x0C;

// ------------------------------------------------------------ tunables

/// P2 spawns this many pixels right of P1.
pub const P2_X_OFFSET: u16 = 16;
/// Frames P2 stays down before respawning next to P1 (no life cost).
pub const P2_DOWN_FRAMES: u16 = 120;
/// unverified: sprite palette for "Zero red". Placeholder; nobody checked
/// what palette 7 holds in each stage, nor wrote Zero's colours into it.
pub const ZERO_PALETTE: u8 = 3;
/// Frame-layout table of the sprite emitter (`$80:D6AA`): `+$16` (set) * 3
/// -> 24-bit pointer to the set's frame list, `+$17 & $7F` (frame) * 3 ->
/// 24-bit pointer to the layout, whose first byte is its sprite count.
const LAYOUT_SETS: u32 = 0x8D8000;

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
    /// First OAM slot past the game's (`$E3`) when P2 was last emitted; P2's
    /// slots (and alignment padding) are `base..base + count`, hidden again
    /// at the next OAM build ([`HOOK_OAM_HIDE`]).
    pub p2_oam_base: usize,
    /// P2's copy of [`P2_SWAPPED`].
    pub p2_scratch: [u8; P2_SWAPPED_LEN],
    /// See [`CoopState::p2_oam_base`].
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
    /// Slots of the shot table ([`X_SHOTS`]) holding P2's shots (bit n = slot n).
    pub p2_shots: u8,
}

impl Default for CoopState {
    fn default() -> Self {
        Self {
            enabled: false,
            p2: [0; X_BLOCK_LEN],
            p2_hidden: true,
            p2_oam_base: 0,
            p2_scratch: [0; P2_SWAPPED_LEN],
            p2_oam_count: 0,
            dbg_regs: (0, 0, 0),
            zero_palette: ZERO_PALETTE,
            down_frames: 0,
            contact_hits: 0,
            in_contact: false,
            p2_shots: 0,
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

/// Clamp P2's world X into `[cam + 8, cam + 248]`. `$05` is the sprite
/// centre (the emitter draws X at -8/-16 around it), so the whole 16 px body
/// stays on screen (B5: `[cam, cam + 240]` showed half a sprite on the left).
pub fn clamp_p2_x(x: u16, camera_x: u16) -> u16 {
    x.clamp(camera_x.saturating_add(8), camera_x.saturating_add(248))
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
    p2[OFF_SHOTS] = 0; // P1's shots aren't P2's
    // `+$18` stays X's tile window; P2's streamed tiles go to the PPU's
    // shadow page instead of VRAM (p2tiles.rs). `+$17` bit 7 asks for a reload.
    p2[0x17] |= 0x80;
    // The emitter keeps bits 0-5 of `+$11`, so it writes P2's palette itself.
    p2[0x11] = (p2[0x11] & !0x0E) | (ZERO_PALETTE << 1);
    p2
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
    game.traps.register(HOOK_OAM_HIDE, "coop_p2_oam_hide", TrapReturn::Observe, oam_hide);
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
    if !coop_stages::p2_should_exist(w) {
        // Pause: freeze P2 where it is (its slots are hidden by the OAM
        // build hook, the update doesn't run). Anything else sends it home.
        // ponytail: in practice the object master doesn't run while `$1F24`
        // is set, so this is only the Start frame.
        if w[ix(ram::PAUSE)] == 0 {
            game.coop.p2_hidden = true;
            game.coop.down_frames = 0;
        }
        return;
    }
    if game.coop.down_frames > 0 {
        game.coop.down_frames -= 1;
        return;
    }
    if game.coop.p2_hidden {
        // Anchor only from a grounded, in-control, on-screen P1: a copy taken
        // during the stage-entry drop inherits an action that never lands
        // (seen on the real ROM: P2 fell through the floor).
        if !coop_stages::p2_anchor_ok(game.wram()) {
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
    let (live, fired) = (shots_live(game.wram()), game.coop.p2[OFF_SHOTS]);
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
    // B3: remember which slots P2 fired into (the count went up); see `shot_books`.
    if game.coop.p2[OFF_SHOTS] > fired {
        game.coop.p2_shots |= shots_live(game.wram()) & !live;
    }
}

/// Active slots of the shot table, bit n = slot n.
fn shots_live(w: &[u8; 0x20000]) -> u8 {
    (0..8).filter(|&k| w[ix(X_SHOTS) + k * 0x40] != 0).fold(0, |m, k| m | 1 << k)
}

/// B3: a P2 shot that despawned in this frame's shot loop did `DEC $0BDD` on
/// P1's count (P1's block is live there) and never on P2's. Left alone, P2
/// hits the cap after a couple of shots and P1's count wraps to `$FF`, so
/// neither can fire again (Bee Blader 2 took one hit from P2, and never died
/// with both shooting). Give the count back to P1 and take it off P2.
/// Runs at [`HOOK_PHYSICS`], after the shot loop and before anyone can fire
/// into a freed slot again (firing is in the player update).
fn shot_books(game: &mut Game) {
    let gone = game.coop.p2_shots & !shots_live(game.wram());
    if gone == 0 {
        return;
    }
    game.coop.p2_shots &= !gone;
    let n = gone.count_ones() as u8;
    let i = ix(ram::X_BASE) + OFF_SHOTS;
    game.wram_mut()[i] = game.wram()[i].wrapping_add(n);
    game.coop.p2[OFF_SHOTS] = game.coop.p2[OFF_SHOTS].saturating_sub(n);
}

/// `$80:D241` (after the object master's `JSL $81:9D67`): P2's terrain /
/// physics phase, the second half of the player's frame that runs after the
/// enemy loops. Without it P2 kept a stale ground flag (`$0BD3` bit 2) and
/// "landed" at the apex of every jump. Then the vertical "lost" check (pits,
/// B1; left behind by a vertical scroll, B2). The horizontal clamp is in the
/// display slice, against the camera the ROM scrolled to this frame.
fn p2_physics(game: &mut Game) {
    if !game.coop.enabled {
        return;
    }
    shot_books(game);
    if game.coop.p2_hidden || p2_is_down(&game.coop.p2) {
        return;
    }
    with_p2_swapped(game, |game| {
        game.call_isolated(PLAYER_PHYSICS);
        game.call_isolated_ret(FX_UPDATE, crate::TrapReturn::Rts);
    });
    let cam_y = word(game.wram(), ram::CAMERA_Y);
    let p2 = &mut game.coop.p2;
    if coop_stages::p2_lost(u16::from_le_bytes([p2[OFF_POS_Y], p2[OFF_POS_Y + 1]]), cam_y) {
        // The ROM's pit death lives in the camera code (`$80:E14A`, P1 only),
        // so P2 never dies in a pit by itself, and nothing drags it along a
        // vertical scroll. Either way it's just "P2 down": respawn next to P1
        // after `P2_DOWN_FRAMES`, no life cost, no stage-wide death flow.
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

/// [`HOOK_OAM_HIDE`]: hide last frame's P2 slots before the game rebuilds
/// OAM. The ROM's hide loop only reaches the game's own previous count
/// (`$E3`, which P2 no longer raises), so without this a P2 that moved or
/// hid would leave its old sprite behind. The game overwrites whatever of
/// these slots it uses this frame.
fn oam_hide(game: &mut Game) {
    if !game.coop.enabled {
        return;
    }
    let (base, n) = (game.coop.p2_oam_base, game.coop.p2_oam_count);
    let w = game.wram_mut();
    for slot in base..(base + n).min(128) {
        w[ix(ram::OAM_BUF) + slot * 4 + 1] = OAM_HIDDEN_Y;
    }
    game.coop.p2_oam_count = 0;
}

/// [`HOOK_DISPLAY`]: P2 display slice, after the ROM's OAM build is done.
/// Clamps P2 into the final camera window (B5), then runs the ROM's own
/// emitters on P2's block into the slots past the game's (`$E3`), from a
/// 4-aligned slot so P2 owns whole high-table bytes, forces the Zero palette
/// on them, and puts direct page (`$E3..$EB` included) back as the game left
/// it. ponytail: P2 now sorts behind every game sprite (it used to sit
/// between P1 and OAM lists 3-5).
fn p2_display(game: &mut Game) {
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    // B5: the camera update `$80:DDB0` ran after P2's physics slice, so clamp
    // here, against where the screen really is this frame.
    let cam = word(game.wram(), ram::CAMERA_X);
    let x = u16::from_le_bytes([game.coop.p2[OFF_POS_X], game.coop.p2[OFF_POS_X + 1]]);
    let cx = clamp_p2_x(x, cam);
    if cx != x {
        // ponytail: no collision check; a clamp can push P2 into a wall.
        game.coop.p2[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&cx.to_le_bytes());
        game.coop.p2[OFF_VEL_X..OFF_VEL_X + 2].fill(0);
    }
    let saved = game.regs();
    game.coop.dbg_regs = (saved.d, saved.dbr, saved.p);
    let dp: [u8; 0x100] = game.wram()[..0x100].try_into().unwrap();
    // `$E3` = 128 when the build ran out of slots (its overflow exit skips
    // the hide loop). The emitter's own overflow exit (`$80:D739`, after slot
    // 127's high-table byte: `PLX; PLX; JMP $D5A1`) would unwind through
    // `call_isolated`'s sentinel frame, so P2 must end below slot 127: when
    // the game left too few slots, P2 takes the game's last (lowest priority)
    // ones. Measured at M3 (hw-a f240..300): the game uses 100-128 slots.
    let used = dp[OAM_USED].min(128);
    let fx = &game.coop.p2_scratch[P2_SWAPPED_LEN - 0x60..];
    let need = layout_len(game, &game.coop.p2)
        + (0..3).map(|k| &fx[k * 0x20..k * 0x20 + 0x20]).filter(|o| o[0] != 0).map(|o| layout_len(game, o)).sum::<usize>();
    let Some(room) = 127usize.checked_sub(need) else { return };
    let first = ((used + 3) & !3).min(room as u8 & !3);
    let pal = game.coop.zero_palette;
    let w = game.wram_mut();
    w[OAM_CURSOR] = first;
    w[OAM_HI_INDEX] = first / 4;
    w[OAM_HI_ACC] = 0;
    // The emitters want what the build gave them: M=1, X=0, D=0, DB=$8D.
    let mut r = saved;
    r.d = 0;
    r.dbr = 0x8D;
    r.p = (r.p | 0x20) & !0x10;
    game.set_regs(r);
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
    let last = (w[OAM_CURSOR] & 0x7F).max(first);
    // Flush the partial high-table byte like the build's tail (`$80:D565`),
    // keeping the bits of any game slots above P2's in that byte.
    let r = last & 3;
    if r != 0 {
        let hi = ix(OAM_HI) + usize::from(last / 4);
        w[hi] = w[OAM_HI_ACC] >> (2 * (4 - r)) | w[hi] & (0xFF << (2 * r));
    }
    for slot in usize::from(used)..usize::from(first) {
        w[ix(ram::OAM_BUF) + slot * 4 + 1] = OAM_HIDDEN_Y; // alignment padding
    }
    for slot in usize::from(first)..usize::from(last) {
        let o = ix(ram::OAM_BUF) + slot * 4;
        set_oam_palette(&mut w[o..o + 4], pal);
    }
    w[..0x100].copy_from_slice(&dp);
    game.set_regs(saved);
    let base = used.min(first);
    game.coop.p2_oam_base = usize::from(base);
    game.coop.p2_oam_count = usize::from(last - base);
    // Route P2's slots to the PPU's shadow tile page (p2tiles.rs).
    crate::p2tiles::mark_p2_slots(game);
}

/// Sprites the emitter `$80:D67C` would write for object `obj` (upper bound:
/// it skips off-screen pieces): 0 unless `+$0E` (visible), else the first
/// byte of its frame layout (see [`LAYOUT_SETS`]).
fn layout_len(game: &Game, obj: &[u8]) -> usize {
    if obj[0x0E] == 0 {
        return 0;
    }
    let long = |a: u32| u32::from_le_bytes([game.rom_read(a), game.rom_read(a + 1), game.rom_read(a + 2), 0]);
    let frames = long(LAYOUT_SETS + 3 * u32::from(obj[0x16]));
    usize::from(game.rom_read(long(frames + 3 * u32::from(obj[0x17] & 0x7F))))
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
        assert_eq!(clamp_p2_x(0x0108, 0x0100), 0x0108);
        assert_eq!(clamp_p2_x(0x00F0, 0x0100), 0x0108);
        assert_eq!(clamp_p2_x(0x0300, 0x0100), 0x01F8);
        assert_eq!(clamp_p2_x(0x0180, 0x0100), 0x0180);
        assert_eq!(clamp_p2_x(0xFFFF, 0xFFF0), 0xFFFF);
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
