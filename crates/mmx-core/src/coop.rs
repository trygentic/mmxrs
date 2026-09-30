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
//! Hooks (all `TrapReturn::Observe`: the body runs, then the ROM bytes):
//! * `$80:D450` shots loop (object master `$80:D203` calls it after the player
//!   update `$81:812E` at `$80:D22C`): P2 update slice.
//! * `$80:D63B` last OAM list pass of `$80:D585` (called at `$80:D5A1`, right
//!   after the player emitter `$80:D6B2` at `$80:D59E`): P2 display slice.
//! * `$84:9D07` enemy-contact hit on X (JMP target inside `$84:9B03`): stub.
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

/// P2 update slice hook: shots loop (`JSR` from `$80:D24A`, only when the
/// freeze flag `$1F13` is 0; the `$1F13 != 0` path runs `$80:D46D` instead,
/// so P2 freezes whenever the game freezes objects). Confidence: high.
pub const HOOK_UPDATE: u32 = 0x80D450;
/// P2 display slice hook: `$80:D63B` (OAM lists 3-5). Runs right after the
/// player emitter, before the low-priority lists. Confidence: high.
pub const HOOK_DISPLAY: u32 = 0x80D63B;
/// Enemy contact damage on X (tail of `$84:9B03` after the hit test
/// `$84:9C0E` with `X=$0BA8`). Entered by `JMP`, starts with an SRAM probe.
pub const HOOK_CONTACT: u32 = 0x849D07;
/// Player update: `PHP/PHD`, sets `D=$0BA8` itself, ends `RTL` (JSL entry).
pub const PLAYER_UPDATE: u32 = 0x81812E;
/// Player OAM emitter: `LDX #$0BA8; LDA $0E,X; BEQ; JSR $D6BD`, ends `RTS`.
/// Needs `M=1, X=0, D=0` (true at [`HOOK_DISPLAY`]).
pub const PLAYER_OAM: u32 = 0x80D6B2;

/// Direct-page OAM cursor: next free OAM slot (`AND #$7F`, `*4` = byte
/// offset into `$0700`). `$E5` = high-table byte index, `$E6` = accumulator.
const OAM_CURSOR: usize = 0x00E4;
/// Button config, 6 bytes, masks in `$80:E559`'s button-byte layout.
const INPUT_CFG: u32 = 0x7EFFC0;
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

// ------------------------------------------------------------ block layout

/// Offsets into the X block (`$0BA8 + off`).
pub const OFF_ACTION: usize = 0x02; // $0BAA: $0C death, $0E hurt
pub const OFF_POS_X: usize = 0x05; // $0BAD word
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
pub const ZERO_PALETTE: u8 = 7;
/// Skip the P2 display pass when fewer OAM slots than this are left: the
/// emitter's overflow exit (`$80:D77B`: `PLX; PLX; JMP $D5E2`) would unwind
/// through `call_isolated`'s sentinel frame. unverified: X's max sprite count.
const P2_MAX_SPRITES: u8 = 32;

// ------------------------------------------------------------ state

/// Off by default; never registered by any verification path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoopState {
    pub enabled: bool,
    /// P2's copy of `$0BA8..$0C38` (HP at [`OFF_HP`], input at [`OFF_INPUT`]).
    pub p2: [u8; X_BLOCK_LEN],
    /// Not in play (outside stage gameplay, or down). Re-anchors on show.
    pub p2_hidden: bool,
    /// First OAM slot P2 got this frame (diagnostic).
    pub p2_oam_base: usize,
    /// Sprite palette (0-7) forced onto P2's OAM entries.
    pub zero_palette: u8,
    /// Frames left while P2 is down.
    pub down_frames: u16,
    /// ponytail: the v1 contact hook only counts; v2 routes hits per player.
    pub contact_hits: u32,
}

impl Default for CoopState {
    fn default() -> Self {
        Self {
            enabled: false,
            p2: [0; X_BLOCK_LEN],
            p2_hidden: true,
            p2_oam_base: 0,
            zero_palette: ZERO_PALETTE,
            down_frames: 0,
            contact_hits: 0,
        }
    }
}

// ------------------------------------------------------------ pure helpers

/// Exchange the live X block with the shadow.
pub fn swap_block(wram: &mut [u8; 0x20000], shadow: &mut [u8; X_BLOCK_LEN]) {
    let b = ix(ram::X_BASE);
    wram[b..b + X_BLOCK_LEN].swap_with_slice(shadow);
}

/// Clamp P2's world X into the visible window `[cam, cam + 256 - 16]`.
/// unverified: `$05` is the sprite anchor (emitter offsets are -8/-16), so a
/// tighter window may look better once seen on screen.
pub fn clamp_p2_x(x: u16, camera_x: u16) -> u16 {
    x.clamp(camera_x, camera_x.saturating_add(256 - 16))
}

/// Port of `$80:E559` for pad 2: `pad` in [`crate::Pads`] layout (the
/// little-endian word at `$AD`), `cfg` the six masks at `$7E:FFC0`, `prev`
/// P2's previous held pair. Returns `(held, pressed)` for `$0BDE` / `$0BE2`.
///
/// E559 packs the buttons into `B Y A X L R Sel St` (bit7..0); config entry
/// `c` fires when `c != 0 && buttons & c == c`. Output:
/// `$0BDE = c2<<7 | c3<<5 | c4<<4`, `$0BDF = c1<<7 | c0<<6 | c5<<4 | dpad`,
/// pressed = `!prev & held`. Pause (`$80:9E7E`) tests `$0BE3 & $10` = c5,
/// the menu action. P2 loses Start/Select and the menu bit so it can't pause.
pub fn p2_input_from_pad(pad: u16, prev: [u8; 2], cfg: &[u8; 6]) -> ([u8; 2], [u8; 2]) {
    let pad = pad & !(BTN_START | BTN_SELECT);
    let (lo, hi) = (pad as u8, (pad >> 8) as u8);
    let buttons = ((lo >> 2) & 0x3C) | (hi & 0xC0) | ((hi >> 4) & 0x03);
    let hit = |c: u8| u8::from(c != 0 && buttons & c == c);
    let held = [
        hit(cfg[2]) << 7 | hit(cfg[3]) << 5 | hit(cfg[4]) << 4,
        hit(cfg[1]) << 7 | hit(cfg[0]) << 6 | (hi & 0x0F), // c5 (menu) masked
    ];
    let pressed = [!prev[0] & held[0], !prev[1] & held[1]];
    (held, pressed)
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
    p2
}

/// P2 is dead: `$81:96CB` sets action `$0C` and HP `$80`.
pub fn p2_is_down(p2: &[u8; X_BLOCK_LEN]) -> bool {
    p2[OFF_ACTION] == ACTION_DEATH || p2[OFF_HP] & 0x7F == 0
}

// ------------------------------------------------------------ Game glue

/// Register the co-op trap group. Not part of any verification trap set.
pub fn register_coop_traps(game: &mut Game) {
    game.traps.register(HOOK_UPDATE, "coop_p2_update", TrapReturn::Observe, p2_update);
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

/// `$80:D450`: P2 update slice.
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
        game.coop.p2 = respawn_anchor(game.wram());
        game.coop.p2_hidden = false;
    }
    if p2_is_down(&game.coop.p2) {
        // Never run the death handler on P2: it drives the stage-wide death flow.
        return go_down(game);
    }

    let mut cfg = [0u8; 6];
    cfg.copy_from_slice(&game.wram()[ix(INPUT_CFG)..ix(INPUT_CFG) + 6]);
    let pad = word(game.wram(), PAD2_HELD);
    let saved: Vec<Vec<u8>> =
        P1_OWNED.iter().map(|&(a, n)| game.wram()[ix(a)..ix(a) + n].to_vec()).collect();

    let mut p2 = game.coop.p2;
    let w = game.wram_mut();
    swap_block(w, &mut p2);
    let i = ix(ram::X_INPUT_HELD);
    let prev = [w[i], w[i + 1]];
    let (held, pressed) = p2_input_from_pad(pad, prev, &cfg);
    w[i..i + 6].copy_from_slice(&[held[0], held[1], prev[0], prev[1], pressed[0], pressed[1]]);

    game.call_isolated(PLAYER_UPDATE);

    let w = game.wram_mut();
    let cam = word(w, ram::CAMERA_X);
    let x = word(w, ram::X_POS_X);
    let cx = clamp_p2_x(x, cam);
    if cx != x {
        // ponytail: no collision check; a clamp can push P2 into a wall.
        w[ix(ram::X_POS_X)..ix(ram::X_POS_X) + 2].copy_from_slice(&cx.to_le_bytes());
        w[ix(ram::X_VEL_X)..ix(ram::X_VEL_X) + 2].fill(0);
    }
    swap_block(w, &mut p2);
    for (&(a, n), s) in P1_OWNED.iter().zip(&saved) {
        w[ix(a)..ix(a) + n].copy_from_slice(s);
    }
    game.coop.p2 = p2;
    if p2_is_down(&game.coop.p2) {
        go_down(game);
    }
}

/// `$80:D63B`: P2 display slice. Runs the ROM's own player emitter on P2's
/// block through the game's OAM cursor `$E4`, so P2 lands in the next free
/// slots (no collision with the game's entries by construction), then forces
/// the Zero palette on exactly the slots it got.
fn p2_display(game: &mut Game) {
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    let first = game.wram()[OAM_CURSOR] & 0x7F;
    if first > 128 - P2_MAX_SPRITES {
        return;
    }
    let pal = game.coop.zero_palette;
    let mut p2 = game.coop.p2;
    swap_block(game.wram_mut(), &mut p2);
    // `$80:D6B2` is entered by JSR and ends in RTS.
    game.call_isolated_ret(PLAYER_OAM, crate::TrapReturn::Rts);
    let w = game.wram_mut();
    swap_block(w, &mut p2);
    let last = w[OAM_CURSOR] & 0x7F;
    for slot in usize::from(first)..usize::from(last.max(first)) {
        let o = ix(ram::OAM_BUF) + slot * 4;
        set_oam_palette(&mut w[o..o + 4], pal);
    }
    game.coop.p2_oam_base = usize::from(first);
}

/// `$84:9D07`: enemy touched P1. v1 stub: count only; P2 takes no contact
/// damage yet (v2: rerun `$84:9C0E` with P2's block swapped in).
fn contact(game: &mut Game) {
    if !game.coop.enabled {
        return;
    }
    game.coop.contact_hits = game.coop.contact_hits.wrapping_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BTN_A, BTN_B, BTN_L, BTN_R, BTN_RIGHT, BTN_X, BTN_Y};

    fn wram() -> Box<[u8; 0x20000]> {
        vec![0u8; 0x20000].into_boxed_slice().try_into().unwrap()
    }

    /// ponytail: a plausible config (shot Y, jump B, dash A, L, R, menu Start)
    /// in E559's button byte `B Y A X L R Sel St`. unverified: the ROM
    /// default table `$86:EE20` and which slot means which action.
    const CFG: [u8; 6] = [0x40, 0x80, 0x20, 0x08, 0x04, 0x01];

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
    fn coop_input_remap_matches_e559() {
        // Nothing held.
        assert_eq!(p2_input_from_pad(0, [0, 0], &CFG), ([0, 0], [0, 0]));
        // Y (c0) -> $0BDF bit6, B (c1) -> $0BDF bit7, A (c2) -> $0BDE bit7.
        let (h, p) = p2_input_from_pad(BTN_Y | BTN_B | BTN_A | BTN_RIGHT, [0, 0], &CFG);
        assert_eq!(h, [0x80, 0xC1]);
        assert_eq!(p, h);
        // L (c3) -> $0BDE bit5, R (c4) -> bit4; X unmapped.
        let (h, _) = p2_input_from_pad(BTN_L | BTN_R | BTN_X, [0, 0], &CFG);
        assert_eq!(h, [0x30, 0x00]);
        // Pressed is the rising edge only.
        let (h, p) = p2_input_from_pad(BTN_A | BTN_L, [0x80, 0], &CFG);
        assert_eq!((h, p), ([0xA0, 0], [0x20, 0]));
    }

    #[test]
    fn coop_p2_cannot_pause() {
        let (h, p) = p2_input_from_pad(BTN_START | BTN_SELECT, [0, 0], &CFG);
        assert_eq!((h, p), ([0, 0], [0, 0]));
        // Even when menu is bound to a normal button: c5 never reaches $0BE3.
        let cfg = [0, 0, 0, 0, 0, 0x80];
        let (h, _) = p2_input_from_pad(BTN_B, [0, 0], &cfg);
        assert_eq!(h[1] & 0x10, 0);
    }

    #[test]
    fn coop_multi_button_config_needs_all() {
        let cfg = [0xC0, 0, 0, 0, 0, 0]; // c0 = B+Y chord
        assert_eq!(p2_input_from_pad(BTN_B, [0, 0], &cfg).0[1] & 0x40, 0);
        assert_eq!(p2_input_from_pad(BTN_B | BTN_Y, [0, 0], &cfg).0[1] & 0x40, 0x40);
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
