//! Co-op: objects that act on X's block by absolute address, rerun for P2.
//! Same discipline as the enemy contact pass (`coop::contact`): an `Observe`
//! trap on the routine's entry swaps P2's block in, `call_isolated`s the same
//! routine with the caller's registers, swaps back and puts the scratch the
//! routine touched back as P1's pass (the ROM's own, right after) expects it.
//! See docs/coop.md "Platforms and items".

use crate::coop::{p2_is_down, with_p2_swapped};
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

pub fn register(game: &mut Game) {
    game.traps.register(HOOK_PLATFORM, "coop_platform", TrapReturn::Observe, platform);
    game.traps.register(HOOK_PLATFORM_TOUCH, "coop_platform_touch", TrapReturn::Observe, platform);
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
    let slot = d.wrapping_sub(OBJ_FIRST) / 0x40;
    if d < OBJ_FIRST || slot >= OBJ_SLOTS {
        return;
    }
    let bit = 1u64 << slot;
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
