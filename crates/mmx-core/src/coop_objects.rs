//! Co-op: objects that act on X's block by absolute address, rerun for P2.
//! Same discipline as the enemy contact pass (`coop::contact`): an `Observe`
//! trap on the routine's entry swaps P2's block in, `call_isolated`s the same
//! routine with the caller's registers, swaps back and puts the scratch the
//! routine touched back as P1's pass (the ROM's own, right after) expects it.
//! See docs/coop.md "Platforms and items".

use crate::coop::{p2_is_down, with_p2_swapped, OFF_POS_X, OFF_POS_Y};
use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::Game;

/// Solid-object routine (listing `$84:AB77`, cartridge +`$0A`), `JSL`ed by
/// platform objects with `D` = the object (16 call sites; on the Highway the
/// collapsing road chunks, `$82:E7A5`). If the object's `+$2C` says X stood
/// on it last frame and X isn't grounded otherwise (`$0BD3|$0BD4` bit 2), X is
/// moved by the object's delta (`+$05 - +$22`, `+$08 - +$24`, written to
/// `$0BAD`/`$0BB0` absolute). Then `+$2C = 0`, the box test `$84:9C0E` vs
/// `X=$0BA8`, and: X on top -> `+$2C = 1`, `$0BD4 |= 4` (the player's ground
/// flag for objects); from the side -> `$0BAD` pushed out, `$0BD4 |= 1/2`.
pub const HOOK_PLATFORM: u32 = 0x84AB81;
/// Companion (listing `$84:AB4C`): box test only, `$0BD4 |= $80` (+ `$40`
/// by side). Called right after [`HOOK_PLATFORM`] by 9 of its callers.
pub const HOOK_PLATFORM_TOUCH: u32 = 0x84AB56;

/// Object slots from the enemy table to the end of the `$1428` table
/// (`$0E68 + 43 * $40 = $1928`), bit n of `CoopState::p2_on_obj`.
const OBJ_FIRST: u16 = 0x0E68;
const OBJ_SLOTS: u16 = 43;
/// The object's "X stood on me last frame" byte.
const OBJ_CARRY: u16 = 0x2C;

/// Pickups: entries of object types 1 (weapon energy), 2 (health) and 4
/// (1-up) of the `$1428` table (dispatch stubs at `$80:F326..`; listing
/// `$81:DEA7`, `$82:E33B`, `$81:E32A`). Each is `LDX $01; JSR (tbl,X); RTL`,
/// so `entry + 5` is its `RTL`. Collected = `LDX #$0BA8; JSL $84:9C0E` hit:
/// health/energy go to state `+$02 = 6` and then fill `$0BCF` (or the
/// weapon `$0BDB`'s energy) by absolute writes over the next frames with the
/// world frozen (`$1F13..$1F19`) and X in action `$18`; the 1-up bumps the
/// shared lives `$1F80` at once.
pub const PICKUPS: [u32; 3] = [0x81DEB7, 0x82E342, 0x81E33A];
/// Byte offset of the pickup state that means "collected" (`+$02 == 6`).
const PICKUP_STATE: usize = 0x02;
const PICKUP_TAKEN: u8 = 0x06;
/// P2 only tries pickups this close (px, both axes).
const PICKUP_NEAR: u16 = 48;

pub fn register(game: &mut Game) {
    game.traps.register(HOOK_PLATFORM, "coop_platform", TrapReturn::Observe, platform);
    game.traps.register(HOOK_PLATFORM_TOUCH, "coop_platform_touch", TrapReturn::Observe, platform);
    for a in PICKUPS {
        game.traps.register(a, "coop_pickup", TrapReturn::Observe, pickup);
    }
}

/// Object slot bit of `D` (see [`OBJ_FIRST`]).
fn slot_bit(d: u16) -> Option<u64> {
    let slot = d.wrapping_sub(OBJ_FIRST) / 0x40;
    (d >= OBJ_FIRST && slot < OBJ_SLOTS).then(|| 1u64 << slot)
}

fn p2_up(game: &Game) -> bool {
    game.coop.enabled && !game.coop.in_contact && !game.coop.p2_hidden && !p2_is_down(&game.coop.p2)
}

/// [`HOOK_PLATFORM`] / [`HOOK_PLATFORM_TOUCH`]: the same routine for P2. The
/// object's `+$2C` is per player: P2's lives in `CoopState::p2_on_obj` and is
/// put in place only for P2's pass, so each player is carried by what they
/// stand on.
fn platform(game: &mut Game) {
    if !p2_up(game) {
        return;
    }
    let pc = game.regs().pbr_pc();
    let d = game.regs().d;
    let Some(bit) = slot_bit(d) else { return };
    let carry = usize::from(d + OBJ_CARRY);
    let dp: [u8; 0x20] = game.wram()[..0x20].try_into().unwrap();
    let p1_carry = game.wram()[carry];
    game.wram_mut()[carry] = u8::from(game.coop.p2_on_obj & bit != 0);
    game.coop.in_contact = true;
    with_p2_swapped(game, |game| game.call_isolated(pc));
    game.coop.in_contact = false;
    let w = game.wram_mut();
    let p2_carry = w[carry] != 0;
    w[carry] = p1_carry;
    w[..0x20].copy_from_slice(&dp);
    if pc == HOOK_PLATFORM {
        game.coop.p2_on_obj = if p2_carry { game.coop.p2_on_obj | bit } else { game.coop.p2_on_obj & !bit };
    }
}

/// A [`PICKUPS`] entry, `D` = the pickup. The ROM's pass is P1's; P2 gets its
/// own pass, and the pickup belongs to whoever touches it first:
///
/// 1. Owned by P2 (`CoopState::p2_items`): the whole routine runs isolated
///    with P2's block swapped in, and the ROM's pass is skipped (`PC` set to
///    the routine's `RTL`), so the fill lands in P2's HP and P2 is the one
///    frozen. `with_p2_swapped` puts the world freeze flags back (P1_OWNED),
///    so P1 plays on while P2 fills.
/// 2. Else, if P2 is near: dry-run the routine for P1 (no swap) and then for
///    P2 (swapped), each from the same WRAM snapshot. P1 collecting wins
///    (snapshot back, the ROM's pass does it for real). P2 collecting keeps
///    P2's run, takes ownership and skips the ROM's pass. Neither: snapshot
///    back, nothing happened.
///
/// HP bit 7 (HUD redraw) is only ever set in P2's own block, never in P1's.
fn pickup(game: &mut Game) {
    if !game.coop.enabled || game.coop.in_contact {
        return;
    }
    let r = game.regs();
    let (pc, d) = (r.pbr_pc(), r.d);
    let Some(bit) = slot_bit(d) else { return };
    let base = usize::from(d);
    if game.wram()[base + 1] == 0 {
        // State 0 = a fresh pickup in this slot.
        game.coop.p2_items &= !bit;
    }
    if game.coop.p2_items & bit != 0 {
        run_isolated(game, pc, true);
        return skip_rom(game, pc);
    }
    if !p2_up(game) || !near(game, base) {
        return;
    }
    let snap = Box::new(*game.wram());
    let (p2, scratch) = (game.coop.p2, game.coop.p2_scratch);
    let lives = ix(ram::LIVES);
    let taken = |g: &Game| g.wram()[base + PICKUP_STATE] == PICKUP_TAKEN || g.wram()[lives] != snap[lives];
    run_isolated(game, pc, false);
    let p1_takes = taken(game);
    *game.wram_mut() = *snap;
    if p1_takes {
        return;
    }
    run_isolated(game, pc, true);
    if taken(game) {
        game.coop.p2_items |= bit;
        return skip_rom(game, pc);
    }
    *game.wram_mut() = *snap;
    game.coop.p2 = p2;
    game.coop.p2_scratch = scratch;
}

fn run_isolated(game: &mut Game, pc: u32, p2: bool) {
    game.coop.in_contact = true;
    if p2 {
        with_p2_swapped(game, |game| game.call_isolated(pc));
    } else {
        game.call_isolated(pc);
    }
    game.coop.in_contact = false;
}

/// Continue at the routine's own `RTL` (`entry + 5`): the ROM's pass is skipped.
fn skip_rom(game: &mut Game, pc: u32) {
    let mut r = game.regs();
    r.pc = (pc as u16).wrapping_add(5);
    game.set_regs(r);
}

fn near(game: &Game, base: usize) -> bool {
    let w = game.wram();
    let p2 = &game.coop.p2;
    let at = |o: usize| u16::from_le_bytes([w[base + o], w[base + o + 1]]);
    let p2at = |o: usize| u16::from_le_bytes([p2[o], p2[o + 1]]);
    at(5).abs_diff(p2at(OFF_POS_X)) < PICKUP_NEAR && at(8).abs_diff(p2at(OFF_POS_Y)) < PICKUP_NEAR
}
