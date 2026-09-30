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
use crate::hooks::HookId;
use crate::ram::{self, wram_index as ix, X_BLOCK_LEN};
use crate::traps::TrapReturn;
use crate::{Game, BTN_SELECT, BTN_START};

// ------------------------------------------------------------ addresses

/// P2 update slice hook: the instruction after the object master's
/// `JSL $81:812E` at `$80:D1EB`, i.e. the moment P1's update has returned.
/// Unconditional. Address confirmed by `cargo xtask trace --calls` on the
/// real ROM (the disassembly's bank-`$80` PCs drift from this ROM by up to
/// `0x41`; bank `$81`/`$84` PCs matched).
pub const HOOK_UPDATE: HookId = HookId::P2Update;
/// P2 physics slice hook: the instruction after `JSL $81:9D67` at `$80:D23D`
/// (the object master, after the enemy loops). Confirmed by trace.
pub const HOOK_PHYSICS: HookId = HookId::P2Physics;
/// P2 display slice hook: `$80:9A86`, the instruction after `JSR $80:D544`
/// (the OAM build) at `$80:9A83` in the per-frame stage code. The build ends
/// with the hide loop over slots `$E4..$E3` and leaves `$E3` = slots used,
/// `$E4..$EB` = 0; `trace --calls` shows D544 entered once, from 9A83, and
/// nothing after 9A86 in the frame touches `$E3`/`$E4`. Emitting here (and
/// putting `$E3`/`$E4` back) keeps the ROM's hide loop, and so P1's lag
/// frames, identical to a solo run (B4: emitting inside the build shortened
/// the loop and dropped a lag frame). The camera is final for the frame here.
pub const HOOK_DISPLAY: HookId = HookId::P2Display;
/// Second instruction of the OAM build (`$80:D544 PHP`, `$D545 PHB`): P2's
/// slots from the previous frame are hidden here, every build, including the
/// weapon menu's (paused frames `JMP $D544` and never reach [`HOOK_DISPLAY`]).
/// `$D544` itself is taken by `zero::HOOK_OAM_BUILD`.
pub const HOOK_OAM_HIDE: HookId = HookId::OamHide;
/// Enemy-vs-X contact routine: every enemy's AI `JSL`s it with `D` = the
/// enemy (154 call sites; `$82:9C41` for the Highway bikes), `RTL`, A=1 on a
/// hit. It checks X's i-frames (`$0C30`/`$0C32`, in the block), runs the box
/// test `$84:9C0E` with `X=$0BA8`, then `JMP $84:9D07` applies the damage
/// (`$0BAA`, `$0BCF`, `$0BD8`, `$0C11`, `$0C32`: all in the block). Found by
/// `trace --find` on the frame P1's HP dropped. Observe hook on the entry.
pub const HOOK_CONTACT: HookId = HookId::Contact;
/// Player update: `PHP/PHD`, sets `D=$0BA8` itself, ends `RTL` (JSL entry).
pub const PLAYER_UPDATE: HookId = HookId::PlayerUpdate;
/// Player terrain/physics phase (`JSL` from `$80:D23D`, ends `RTL`): collision
/// against the layout, ground flag, camera requests. Confirmed by trace.
pub const PLAYER_PHYSICS: HookId = HookId::PlayerPhysics;
/// Input remapper (`JSR` from `$80:9A52` in the per-frame stage code, ends
/// `RTS`): pad 1 (`$A7`) + button config -> the block's `$0BDE..$0BE3`.
/// Confirmed by trace.
pub const INPUT_REMAP: HookId = HookId::InputRemap;
/// Player OAM emitter (`JSR` from `$80:D55D`, ends `RTS`). Needs
/// `M=1, X=0, D=0` (true at [`HOOK_DISPLAY`]). Confirmed by trace.
pub const PLAYER_OAM: HookId = HookId::PlayerOam;
/// Companion-object update loop over `$0C38..$0C98` (`JSR` from `$80:D26C`,
/// `RTS`; listing `$80:D2D3`, drift `-$41`). Sets `D` itself.
pub const FX_UPDATE: HookId = HookId::FxUpdate;
/// Companion-object OAM emitter (`JSR` from `$80:D55A`, just before
/// [`PLAYER_OAM`]; listing `$80:D681`). Same register needs as the player's.
pub const FX_OAM: HookId = HookId::FxOam;

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
/// `$0BD3` terrain flags, `$0BD4` object flags: bit 2 = standing on it.
pub const OFF_GROUND: usize = 0x2B;
pub const OFF_INPUT: usize = 0x36; // $0BDE held, $0BE0 prev, $0BE2 pressed
/// `$0BDD`: X's live shot count. The fire routine (`$81:93A8`) refuses when
/// it is `>=` the cap `+$67` and does `INC $35` (direct page, so P2's own
/// copy); the shot's despawn does `DEC $0BDD` (absolute, `$81:A2C0`,
/// `$81:A3FD`, `$81:A4BA`, `$81:A541`) from the shot loop, which runs P2's
/// shots with P2's block in place ([`HOOK_SHOT_SLOT`]).
pub const OFF_SHOTS: usize = 0x35;
/// `$0C0B` (block `+$63`), decremented with `$0BDD` by the despawn at `$81:A541`.
const OFF_CHARGED_SHOTS: usize = 0x63;
/// X's shots: 8 slots of `$40` at `$1228` (`ram::ITEMS_BASE`), `+0` = active.
const X_SHOTS: u32 = 0x7E1228;
const ACTION_DEATH: u8 = 0x0C;

// ------------------------------------------------------------ tunables

/// P2 spawns this many pixels right of P1. 0: P1's own spot is the only
/// one known to be clear of terrain (16 put P2 inside a wall P1 stood at).
pub const P2_X_OFFSET: u16 = 0;
/// Frames P2 may stay outside the camera window before it is warped to P1.
pub const P2_AWAY_FRAMES: u16 = 90;
/// Frames P2 stays down before respawning next to P1 at half HP (no life
/// cost), unless P1 revives it sooner (`coop_hud::still_down`).
pub const P2_DOWN_FRAMES: u16 = 600;
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
    /// Inside P2's contact pass (the isolated call re-enters [`HOOK_CONTACT`]),
    /// or one of `coop_objects`' passes.
    pub in_contact: bool,
    /// Objects P2 stood on last frame (their `+$2C` for P2), bit n = slot
    /// `$0E68 + n * $40`. See `coop_objects`.
    pub p2_on_obj: u64,
    /// Pickups P2 collected and is being filled from (same slot bits).
    pub p2_items: u64,
    /// Slots of the shot table ([`X_SHOTS`]) holding P2's shots (bit n = slot n).
    pub p2_shots: u8,
    /// Effect-pool slots P2's slices spawned (bit n = `$1928 + n * $20`),
    /// see `coop_objects::effect_slot`.
    pub p2_fx: u32,
    /// X3 Zero frame whose tiles are in the PPU's P2 page (`u16::MAX`: none
    /// yet). Only used with `Game::set_zero_set`.
    pub zero_frame: u16,
    /// P2's saber swing (X3 set only), see `saber.rs`.
    pub saber: crate::saber::Saber,
    /// Wave frame in shadow page 2 per player (`[P2, P1]`; `u16::MAX`: none).
    pub wave_frames: [u16; 2],
    /// Player 1 as Zero (`Game::set_p1_zero`), independent of `enabled`.
    pub p1: crate::zero::ZeroP1,
    /// Down spot and revive progress (coop_hud.rs).
    pub revive: crate::coop_hud::Revive,
    /// Frames in a row P2 has been outside the camera window.
    pub p2_away: u16,
    /// P2 was left behind: re-anchor on P1 (HP kept) at the next `p2_anchor_ok`.
    pub p2_warp: bool,
    /// Frames into Zero's saber whoosh + 1 per player (`sfx::P2`, `sfx::P1`), 0 = silent.
    pub sfx: [u16; 2],
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
            p2_on_obj: 0,
            p2_items: 0,
            p2_shots: 0,
            p2_fx: 0,
            zero_frame: u16::MAX,
            saber: Default::default(),
            wave_frames: [u16::MAX; 2],
            p1: Default::default(),
            revive: Default::default(),
            p2_away: 0,
            p2_warp: false,
            sfx: [0; 2],
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
/// Refused (nothing registered) when this ROM lacks one of the hooks
/// (`hooks.rs`; `Game::hook_report` says which).
pub fn register_coop_traps(game: &mut Game) -> Result<(), String> {
    let missing = game.hooks().missing();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|h| h.hook().name).collect();
        return Err(format!(
            "co-op refused: this ROM ({}, v1.{}) lacks the hooks {} (run `cargo xtask rominfo`; see docs/rom-variants.md)",
            game.rom_info().region_name(),
            game.rom_info().version,
            names.join(", ")
        ));
    }
    crate::zero::register_zero_traps(game);
    game.traps.register(game.hook(HOOK_UPDATE), "coop_p2_update", TrapReturn::Observe, p2_update);
    game.traps.register(game.hook(HOOK_PHYSICS), "coop_p2_physics", TrapReturn::Observe, p2_physics);
    game.traps.register(game.hook(HOOK_OAM_HIDE), "coop_p2_oam_hide", TrapReturn::Observe, oam_hide);
    game.traps.register(game.hook(HOOK_DISPLAY), "coop_p2_display", TrapReturn::Observe, p2_display);
    game.traps.register(game.hook(HOOK_CONTACT), "coop_contact", TrapReturn::Observe, contact);
    crate::coop_objects::register(game);
    game.traps.register(game.hook(HOOK_SHOT_SLOT), "coop_shot_slot", TrapReturn::Observe, shot_slot);
    game.traps.register(game.hook(HOOK_SHOT_SLOT_DONE), "coop_shot_slot_done", TrapReturn::Observe, shot_slot_done);
    game.traps.register(game.hook(crate::saber::HOOK_SHOTS_DONE), "coop_saber_shot", TrapReturn::Observe, crate::saber::shots_done);
    crate::coop_hud::register(game);
    Ok(())
}

fn word(w: &[u8; 0x20000], addr: u32) -> u16 {
    u16::from_le_bytes([w[ix(addr)], w[ix(addr) + 1]])
}

fn go_down(game: &mut Game) {
    game.coop.p2_hidden = true;
    game.coop.p2_warp = false;
    game.coop.down_frames = P2_DOWN_FRAMES;
    crate::coop_hud::mark_down(game);
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
            game.coop.p2_warp = false;
            game.coop.down_frames = 0;
            game.coop.revive.half_hp = false;
        }
        return;
    }
    if crate::coop_hud::still_down(game) {
        return;
    }
    if game.coop.p2_hidden {
        // Anchor only from a grounded, in-control, on-screen P1: a copy taken
        // during the stage-entry drop inherits an action that never lands
        // (seen on the real ROM: P2 fell through the floor).
        if !coop_stages::p2_anchor_ok(game.wram()) {
            return;
        }
        anchor(game);
        // Debug: `MMX_COOP_P2_HP=N` spawns P2 with N HP (to test HP-0 deaths).
        if let Some(hp) = std::env::var("MMX_COOP_P2_HP").ok().and_then(|v| v.parse().ok()) {
            game.coop.p2[OFF_HP] = hp;
        }
        crate::coop_hud::respawn_hp(game);
        game.coop.p2_hidden = false;
    } else if game.coop.p2_warp && coop_stages::p2_anchor_ok(game.wram()) {
        // Left behind (see `follow`): back on P1's spot, same HP, no down state.
        let hp = game.coop.p2[OFF_HP];
        anchor(game);
        game.coop.p2[OFF_HP] = hp;
    }
    if p2_is_down(&game.coop.p2) {
        // Never run the death handler on P2: it drives the stage-wide death flow.
        return go_down(game);
    }

    let mut pad = word(game.wram(), PAD2_HELD) & !(BTN_START | BTN_SELECT);
    if game.zero_set.is_some() {
        // X swings the saber; during a swing the ROM gets a masked pad.
        pad = crate::saber::update(&mut game.coop.saber, &game.coop.p2, pad);
        if crate::saber::whoosh(&game.coop.saber) {
            game.coop.sfx[crate::sfx::P2] = 1;
        }
    }
    let (live, fired) = (shots_live(game.wram()), game.coop.p2[OFF_SHOTS]);
    with_p2_swapped(game, |game| {
        // Run the ROM's own input remapper (`$80:E518`, JSR/RTS) with pad 2
        // in pad 1's slot: it reads `$A7` and the block's previous input and
        // writes the block's `$0BDE..$0BE3`, so P2 gets P1's button config.
        let w = game.wram_mut();
        let i = ix(ram::PAD1_HELD);
        let saved_pad1 = [w[i], w[i + 1]];
        w[i..i + 2].copy_from_slice(&pad.to_le_bytes());
        game.call_isolated_ret(game.hook(INPUT_REMAP), crate::TrapReturn::Rts);
        let w = game.wram_mut();
        w[i..i + 2].copy_from_slice(&saved_pad1);
        game.call_isolated(game.hook(PLAYER_UPDATE));
    });
    // B3: remember which slots P2 fired into (the count went up); see `shot_books`.
    if game.coop.p2[OFF_SHOTS] > fired {
        game.coop.p2_shots |= shots_live(game.wram()) & !live;
    }
}

/// P2's block and scratch from P1's (grounded) ones, at P1's spot.
fn anchor(game: &mut Game) {
    game.coop.p2 = respawn_anchor(game.wram());
    let mut copy = [0u8; P2_SWAPPED_LEN];
    let w = game.wram();
    let mut o = 0;
    for &(a, n) in &P2_SWAPPED {
        copy[o..o + n].copy_from_slice(&w[ix(a)..ix(a) + n]);
        o += n;
    }
    game.coop.p2_scratch = copy;
    game.coop.p2_warp = false;
    game.coop.p2_away = 0;
}

/// Active slots of the shot table, bit n = slot n.
fn shots_live(w: &[u8; 0x20000]) -> u8 {
    (0..8).filter(|&k| w[ix(X_SHOTS) + k * 0x40] != 0).fold(0, |m, k| m | 1 << k)
}

/// Shot loop `$80:D3B2` (listing `$80:D3F3`, drift `-$41`), per slot: `$D3B8`
/// (`SEP #$30`, `D` = the slot, before `LDA $00; BEQ; JSR $D3F8`) and
/// `$D3C1` (`REP #$21`, the loop step, reached active or not).
pub const HOOK_SHOT_SLOT: HookId = HookId::ShotSlot;
pub const HOOK_SHOT_SLOT_DONE: HookId = HookId::ShotSlotDone;

/// P2's shot slot `D` is about to update: put P2's block in place. A shot
/// reads its owner's block with absolute addresses: its first update
/// (`$81:A54C`) takes position `$0BAD/$0BB0` + the muzzle offset and facing
/// `$0BB9`, a charging shot re-anchors to it every frame (`$81:A56E`), and
/// the despawn does `DEC $0BDD` / `DEC $0C0B` (the shot counts). Run with
/// P1's block, P2's shots came out of X, flew X's way, and took from P1's
/// count (B3 had patched only the count).
fn shot_slot(game: &mut Game) {
    if game.coop.enabled && p2_shot_slot(game) {
        let mut p2 = game.coop.p2;
        swap_block(game.wram_mut(), &mut p2);
        game.coop.p2 = p2;
    }
}

/// The slot's update is done (or it was inactive): Zero's muzzle, then P1's
/// block back.
fn shot_slot_done(game: &mut Game) {
    let p2 = game.coop.enabled && p2_shot_slot(game);
    if (p2 && game.zero_set.is_some()) || (!p2 && game.p1_zero()) {
        zero_muzzle(game);
    }
    if p2 {
        let mut p2 = game.coop.p2;
        swap_block(game.wram_mut(), &mut p2);
        // A P2 that respawned (`+$35 = 0`) with shots still out: no wrap.
        for o in [OFF_SHOTS, OFF_CHARGED_SHOTS] {
            if p2[o] == 0xFF {
                p2[o] = 0;
            }
        }
        game.coop.p2 = p2;
    }
}

/// Shot slot `D` of a Zero player (whose block is live): if the shot sits
/// at X's muzzle (the block's `+05` + `$86:BDDD[+3C]`, mirrored by the
/// shot's facing), the ROM just placed it (first update, or a charging shot
/// re-anchoring): move it to Zero's cannon ([`crate::zero_data::ZERO_MUZZLE`]).
fn zero_muzzle(game: &mut Game) {
    let k = u32::from(game.regs().d.wrapping_sub(X_SHOTS as u16) / 0x40);
    let a = X_SHOTS + k * 0x40;
    if k >= 8 || game.wram()[ix(a)] == 0 {
        return;
    }
    let m = game.wram()[ix(a) + 0x3C];
    let Some(&(zx, zy)) = crate::zero_data::ZERO_MUZZLE.get(usize::from(m / 2)) else { return };
    let t = crate::zero_data::X1_MUZZLE + u32::from(m);
    let (xx, xy) = (i16::from(game.rom_read(t)), i16::from(game.rom_read(t + 1) as i8));
    let w = game.wram_mut();
    let s: i16 = if w[ix(a) + 0x11] & 0x40 != 0 { 1 } else { -1 };
    let (px, py) = (word(w, ram::X_BASE + OFF_POS_X as u32), word(w, ram::X_BASE + OFF_POS_Y as u32));
    let at = |dx: i16, dy: i16| (px.wrapping_add((s * dx) as u16), py.wrapping_add(dy as u16));
    // x decides: the half charge (`+0A = 1`) re-anchors x only (`$81:A57B`).
    let ((x0, y0), (x, y)) = (at(xx, xy), at(zx, zy));
    if word(w, a + 5) == x0 {
        w[ix(a) + 5..ix(a) + 7].copy_from_slice(&x.to_le_bytes());
        if word(w, a + 8) == y0 {
            w[ix(a) + 8..ix(a) + 10].copy_from_slice(&y.to_le_bytes());
        }
    }
}

fn p2_shot_slot(game: &Game) -> bool {
    let k = game.regs().d.wrapping_sub(X_SHOTS as u16) / 0x40;
    k < 8 && game.coop.p2_shots >> k & 1 != 0
}

/// Forget P2's slots the shot loop freed (at [`HOOK_PHYSICS`], before anyone
/// can fire into them again: firing is in the player update).
fn shot_books(game: &mut Game) {
    game.coop.p2_shots &= shots_live(game.wram());
}

/// `$80:D241` (after the object master's `JSL $81:9D67`): P2's terrain /
/// physics phase, the second half of the player's frame that runs after the
/// enemy loops. Without it P2 kept a stale ground flag (`$0BD3` bit 2) and
/// "landed" at the apex of every jump. Then [`follow`].
fn p2_physics(game: &mut Game) {
    crate::saber::take_back(game); // both players' swings
    if !game.coop.enabled {
        return;
    }
    shot_books(game);
    if game.coop.p2_hidden || p2_is_down(&game.coop.p2) {
        return;
    }
    with_p2_swapped(game, |game| {
        game.call_isolated(game.hook(PLAYER_PHYSICS));
        game.call_isolated_ret(game.hook(FX_UPDATE), crate::TrapReturn::Rts);
    });
    follow(game);
    if p2_is_down(&game.coop.p2) {
        go_down(game);
    }
}

/// Where P2 is against the camera, after its physics. P2 is never moved by
/// the camera: the old clamp dragged it along with no collision check, into
/// walls and floors (hw-e: P2 fell off a ledge behind P1 and was pushed into
/// the ledge's side, then ejected up through the road). Instead:
/// - airborne below the ROM's pit line (`p2_lost`, B1): a real death, down;
/// - a screen or more outside the window (`p2_far`), or outside it for
///   [`P2_AWAY_FRAMES`] (left behind by a scroll, B2): `p2_warp`, i.e. back
///   on P1's spot at the next `p2_anchor_ok`, HP kept.
///
/// P2 keeps its own physics while off screen (collision is not camera-
/// streamed: idle P2 800+ px behind stands where it stood, hw-a/hw-e/r3-10).
fn follow(game: &mut Game) {
    let w = game.wram();
    let (cx, cy) = (word(w, ram::CAMERA_X), word(w, ram::CAMERA_Y));
    let p2 = &mut game.coop.p2;
    let (x, y) = (u16::from_le_bytes([p2[OFF_POS_X], p2[OFF_POS_X + 1]]), u16::from_le_bytes([p2[OFF_POS_Y], p2[OFF_POS_Y + 1]]));
    let grounded = (p2[OFF_GROUND] | p2[OFF_GROUND + 1]) & 4 != 0;
    if coop_stages::p2_lost(y, cy) && !grounded {
        // The ROM's pit death lives in the camera code (`$80:E14A`, P1 only).
        // For P2 it's just "P2 down": no life cost, no stage-wide death flow.
        p2[OFF_ACTION] = ACTION_DEATH;
        p2[OFF_HP] = 0x80;
        return;
    }
    let c = &mut game.coop;
    c.p2_away = if coop_stages::p2_on_screen(x, y, cx, cy) { 0 } else { c.p2_away.saturating_add(1) };
    if c.p2_away > P2_AWAY_FRAMES || coop_stages::p2_far(x, y, cx, cy) {
        c.p2_warp = true;
    }
}

/// Swap P2's block and scratch in, run `f`, swap out, and put back every
/// P1-owned byte the slice may have touched (camera requests, freeze flags).
pub(crate) fn with_p2_swapped(game: &mut Game, f: impl FnOnce(&mut Game)) {
    // Debug: `MMX_COOP_DIFF=1` prints every byte a P2 slice leaves changed
    // outside the block (after the P1-owned restore), i.e. P2's global side effects.
    static DIFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let before = DIFF.get_or_init(|| std::env::var_os("MMX_COOP_DIFF").is_some()).then(|| Box::new(*game.wram()));
    let saved: Vec<Vec<u8>> =
        P1_OWNED.iter().map(|&(a, n)| game.wram()[ix(a)..ix(a) + n].to_vec()).collect();
    let mut p2 = game.coop.p2;
    let mut scratch = game.coop.p2_scratch;
    let snd = game.wram()[SND_WRITE];
    let fx = crate::coop_objects::effects_live(game.wram());
    game.coop.p2_fx &= fx;
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
    // Effects P2 spawned are P2's (they read the owner's block every frame).
    game.coop.p2_fx |= crate::coop_objects::effects_live(game.wram()) & !fx;
    if let Some(b) = before {
        let d: Vec<String> = (0..0x20000)
            .filter(|&i| b[i] != game.wram()[i])
            .map(|i| format!("${:05X}:{:02X}->{:02X}", 0x7E0000 + i, b[i], game.wram()[i]))
            .collect();
        if !d.is_empty() {
            eprintln!("coop diff frame {} {:?} {}", game.frame_count(), game.traplog.last().map(|t| t.name), d.join(" "));
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
    let zero = game.zero_set.clone();
    let fx = &game.coop.p2_scratch[P2_SWAPPED_LEN - 0x60..];
    let need = match &zero {
        Some(z) => {
            crate::zero::x3_frame(&game.coop.p2, &game.coop.saber).map_or(0, |f| z.frames[f].sprites.len())
                + crate::zero::wave_len(z, &game.coop.saber)
        }
        None => {
            layout_len(game, &game.coop.p2)
                + (0..3).map(|k| &fx[k * 0x20..k * 0x20 + 0x20]).filter(|o| o[0] != 0).map(|o| layout_len(game, o)).sum::<usize>()
        }
    };
    let Some(room) = (127 - crate::coop_hud::HUD_SLOTS).checked_sub(need) else { return };
    let first = ((used + 3) & !3).min(room as u8 & !3);
    let pal = game.coop.zero_palette;
    if let Some(z) = zero {
        // X3 Zero: a Rust emitter writes P2's slots and the page (zero.rs).
        let last = crate::zero::emit_x3(game, &z, first, pal);
        game.oam_slots[1] = crate::display::slots(first.into(), last.into());
        let w = game.wram_mut();
        for slot in usize::from(used)..usize::from(first) {
            w[ix(ram::OAM_BUF) + slot * 4 + 1] = OAM_HIDDEN_Y; // alignment padding
        }
        let base = used.min(first);
        game.coop.p2_oam_base = usize::from(base);
        game.coop.p2_oam_count = usize::from(last - base);
        return;
    }
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
    game.call_isolated_ret(game.hook(FX_OAM), crate::TrapReturn::Rts);
    game.call_isolated_ret(game.hook(PLAYER_OAM), crate::TrapReturn::Rts);
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
    game.oam_slots[1] = crate::display::slots(first.into(), last.into());
    // Route P2's slots to the PPU's shadow tile page (p2tiles.rs).
    crate::p2tiles::mark_p2_slots(game);
}

/// Sprites the emitter `$80:D67C` would write for object `obj` (upper bound:
/// it skips off-screen pieces): 0 unless `+$0E` (visible), else the first
/// byte of its frame layout (see [`LAYOUT_SETS`]).
pub(crate) fn layout_len(game: &Game, obj: &[u8]) -> usize {
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
    with_p2_swapped(game, |game| game.call_isolated(game.hook(HOOK_CONTACT)));
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
        assert_eq!(u16::from_le_bytes([p2[OFF_POS_X], p2[OFF_POS_X + 1]]), 0x01F8);
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

    /// hw-spiky: P2 walks 40 px left of X and fires, X fires right. Each
    /// shot starts at its own shooter, flies its way, and both counts end at 0.
    /// With the X3 set (`MMX_X3_ROM`) P2 is Zero and its shot leaves Zero's
    /// cannon (stand: 27 px out, 7 up; X's: 16, 3), then P1 as Zero too.
    #[test]
    fn coop_p2_shot_from_p2() {
        let Some(rom) = std::env::var_os("MMX_ROM") else {
            eprintln!("MMX_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let x3 = std::env::var_os("MMX_X3_ROM").map(|p| std::fs::read(p).unwrap());
        let (x_muzzle, zero_muzzle) = ((16, -3), crate::zero_data::ZERO_MUZZLE[0]);
        let runs: &[(bool, bool)] = if x3.is_some() { &[(false, false), (true, false), (true, true)] } else { &[(false, false)] };
        for &(p2_zero, p1_zero) in runs {
            let mut g = crate::Game::from_rom(std::fs::read(&rom).unwrap()).unwrap();
            g.load_state(&state).unwrap();
            if p2_zero {
                g.set_x3_rom(x3.as_deref().unwrap()).unwrap();
            }
            g.set_p1_zero(p1_zero).unwrap();
            g.coop.enabled = true;
            register_coop_traps(&mut g).unwrap();
            let shot = |g: &crate::Game, k: usize| {
                let o = ix(X_SHOTS) + k * 0x40;
                let w = g.wram();
                let w16 = |o: usize| u16::from_le_bytes([w[o], w[o + 1]]);
                (w[o] != 0, w16(o + 5), w16(o + 8), w[o + 0x11] & 0x40 != 0)
            };
            let (p2m, p1m) = (if p2_zero { zero_muzzle } else { x_muzzle }, if p1_zero { zero_muzzle } else { x_muzzle });
            for f in 1..=80 {
                let p2 = match f {
                    ..=40 => crate::BTN_LEFT,
                    46 => crate::BTN_Y,
                    _ => 0,
                };
                g.step(crate::Pads { p1: if f == 50 { crate::BTN_Y } else { 0 }, p2 });
                let p = |o: usize| u16::from_le_bytes([g.coop.p2[o], g.coop.p2[o + 1]]);
                if f == 46 {
                    let want = (true, p(OFF_POS_X) - p2m.0 as u16, p(OFF_POS_Y).wrapping_add(p2m.1 as u16), false);
                    assert_eq!(shot(&g, 0), want, "P2's shot, zero {p2_zero}");
                    assert_eq!(g.coop.p2[OFF_SHOTS], 1);
                }
                if f == 50 {
                    let w = |a: u32| u16::from_le_bytes([g.wram()[ix(a)], g.wram()[ix(a) + 1]]);
                    let want = (true, w(ram::X_POS_X) + p1m.0 as u16, w(ram::X_POS_Y).wrapping_add(p1m.1 as u16), true);
                    assert_eq!(shot(&g, 1), want, "P1's shot, zero {p1_zero}");
                }
            }
            assert!(!shot(&g, 0).0 && !shot(&g, 1).0);
            assert_eq!((g.coop.p2[OFF_SHOTS], g.wram()[ix(ram::X_BASE) + OFF_SHOTS], g.coop.p2_shots), (0, 0, 0));
        }
    }
}
