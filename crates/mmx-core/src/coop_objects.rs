//! Co-op: objects that act on X's block by absolute address, rerun for P2.
//! Same discipline as the enemy contact pass (`coop::contact`): an `Observe`
//! trap on the routine's entry swaps P2's block in, `call_isolated`s the same
//! routine with the caller's registers, swaps back and puts the scratch the
//! routine touched back as P1's pass (the ROM's own, right after) expects it.
//! See docs/coop.md "Platforms and items".

use crate::coop::{p2_is_down, with_p2_swapped, OFF_POS_X, OFF_POS_Y};
use crate::hooks::HookId;
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
pub const HOOK_PLATFORM: HookId = HookId::Platform;
/// Companion (listing `$84:AB4C`): box test only, `$0BD4 |= $80` (+ `$40`
/// by side). Called right after [`HOOK_PLATFORM`] by 9 of its callers.
pub const HOOK_PLATFORM_TOUCH: HookId = HookId::PlatformTouch;
/// The road attacker's own copy of [`HOOK_PLATFORM`] (listing `$81:DE0C`),
/// `JSR`ed by the car-body object (enemy id `$12`, update `$81:DCD0`) once
/// its driver is dead (state `+$01 >= 4`, not `$0A`). Same `+$2C` carry and
/// box test with absolute writes to `$0BAD`/`$0BB0`/`$0BD4`; it also keeps
/// the previous `+$2C` in `+$3A` and sets `+$37 = 2` (the landing dip) when
/// the rider lands. Ends in `RTS`.
pub const HOOK_CAR: HookId = HookId::Car;
/// The collapsing road chunk's trigger (`$1428` type 9, update `$82:E727`;
/// listing `$82:E9E6`, cartridge +7), `JSR`ed from its update with `D` = the
/// chunk while its timer `+$2F` is 0: `LDX #$0BA8; JSL $84:9C0E` (box test
/// vs X), X on the terrain (`$0BD3` bit 2), above it (`$0002 < 0`) and within
/// 16 px of its centre (`$0BAD`) -> `+$2F = $1E` (frames to the fall), `+$1F =
/// $30`, sound `$22`. It reads X's block by absolute address, so only P1
/// ever started a collapse. Ends in `RTS` (`SEP #$30` first).
pub const HOOK_COLLAPSE: HookId = HookId::CollapseTrigger;
/// The chunk's countdown to the fall (`+$2F`, 0 = not triggered).
const COLLAPSE_TIMER: u16 = 0x2F;

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
pub const PICKUPS: [HookId; 3] = [HookId::PickupA, HookId::PickupB, HookId::PickupC];
/// Byte offset of the pickup state that means "collected" (`+$02 == 6`).
const PICKUP_STATE: usize = 0x02;
const PICKUP_TAKEN: u8 = 0x06;
/// P2 only tries pickups this close (px, both axes).
const PICKUP_NEAR: u16 = 48;

/// Effect pool (dash dust, sparks, the hurt flash): 31 slots of `$20` from
/// `$1928`, `+0` = active, `+$0A` = type. Updated by the loop `$80:D31A`
/// (`JSR` from the object master), `D` = the slot; per active slot the hook
/// is `LDA #$80; TRB $0E; LDA $0A; ASL; TAX; JSR (tbl,X)` and the loop
/// step follows at `+11`.
pub const HOOK_EFFECT_SLOT: HookId = HookId::EffectSlot;
const FX_FIRST: u16 = 0x1928;
const FX_SLOTS: u16 = 31;
const FX_STRIDE: u16 = 0x20;

/// Active effect slots, bit n = slot n.
pub fn effects_live(w: &[u8; 0x20000]) -> u32 {
    (0..FX_SLOTS).filter(|&k| w[usize::from(FX_FIRST + k * FX_STRIDE)] != 0).fold(0, |m, k| m | 1 << k)
}

pub fn register(game: &mut Game) {
    game.traps.register(game.hook(HOOK_EFFECT_SLOT), "coop_effect", TrapReturn::Observe, effect_slot);
    let emitter = emitter(game);
    game.traps.register(emitter, "coop_effect_oam", TrapReturn::Observe, effect_oam);
    game.traps.register(game.hook(HOOK_PLATFORM), "coop_platform", TrapReturn::Observe, platform);
    game.traps.register(game.hook(HOOK_PLATFORM_TOUCH), "coop_platform_touch", TrapReturn::Observe, platform);
    game.traps.register(game.hook(HOOK_CAR), "coop_car", TrapReturn::Observe, platform);
    game.traps.register(game.hook(HOOK_COLLAPSE), "coop_collapse", TrapReturn::Observe, collapse);
    for a in PICKUPS {
        game.traps.register(game.hook(a), "coop_pickup", TrapReturn::Observe, pickup);
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

/// [`HOOK_PLATFORM`] / [`HOOK_PLATFORM_TOUCH`] / [`HOOK_CAR`]: the same routine for P2. The
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
    let ret = if pc == game.hook(HOOK_CAR) { TrapReturn::Rts } else { TrapReturn::Rtl };
    with_p2_swapped(game, |game| game.call_isolated_ret(pc, ret));
    game.coop.in_contact = false;
    let w = game.wram_mut();
    let p2_carry = w[carry] != 0;
    w[carry] = p1_carry;
    w[..0x20].copy_from_slice(&dp);
    if pc != game.hook(HOOK_PLATFORM_TOUCH) {
        game.coop.p2_on_obj = if p2_carry { game.coop.p2_on_obj | bit } else { game.coop.p2_on_obj & !bit };
    }
}

/// [`HOOK_COLLAPSE`]: the chunk's trigger test once more with P2's block
/// swapped in, before P1's (the ROM's). P2 standing on the chunk starts the
/// countdown exactly as X does; then the ROM's pass is skipped (`RTS` with
/// the routine's `SEP #$30`), since P1 on it too would only restart the
/// same countdown and queue the crumble sound twice. Otherwise only the box
/// test's direct-page scratch changed, and it is put back.
fn collapse(game: &mut Game) {
    if !p2_up(game) {
        return;
    }
    let r = game.regs();
    if slot_bit(r.d).is_none() {
        return;
    }
    let timer = usize::from(r.d + COLLAPSE_TIMER);
    if game.wram()[timer] != 0 {
        return;
    }
    let dp: [u8; 0x20] = game.wram()[..0x20].try_into().unwrap();
    game.coop.in_contact = true;
    with_p2_swapped(game, |game| game.call_isolated_ret(r.pbr_pc(), TrapReturn::Rts));
    game.coop.in_contact = false;
    game.wram_mut()[..0x20].copy_from_slice(&dp);
    if game.wram()[timer] != 0 {
        let mut r = game.regs();
        r.p |= 0x30;
        r.x &= 0xFF;
        r.y &= 0xFF;
        game.set_regs(r);
        game.emulate_return(false);
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

/// [`HOOK_EFFECT_SLOT`]: an effect P2 spawned (`CoopState::p2_fx`) updates
/// isolated with P2's block in place, and the ROM's pass is skipped. Effects
/// read their owner's block by absolute address (the hurt flash, type `$0B`,
/// sits on `$0BAD/$0BB0` and ends with the i-frames), so in the ROM's pass
/// P2's flash was drawn on P1, and its real-time cost made P1 lag a frame
/// early on NTSC (M3 of the Rev 1 QA route; docs/rom-variants.md).
fn effect_slot(game: &mut Game) {
    let r = game.regs();
    let k = r.d.wrapping_sub(FX_FIRST) / FX_STRIDE;
    if !game.coop.enabled || game.coop.in_contact || k >= FX_SLOTS || game.coop.p2_fx >> k & 1 == 0 {
        return;
    }
    let (pc, base) = (r.pbr_pc(), usize::from(r.d));
    let bank = pc & 0xFF_0000;
    let table = u16::from_le_bytes([game.rom_read(pc + 9), game.rom_read(pc + 10)]);
    let w = game.wram_mut();
    w[base + 0x0E] &= !0x80;
    let x = w[base + 0x0A] << 1;
    let at = bank | u32::from(table.wrapping_add(u16::from(x)));
    let target = bank | u32::from(u16::from_le_bytes([game.rom_read(at), game.rom_read(at + 1)]));
    let mut regs = r;
    regs.a = (regs.a & 0xFF00) | u16::from(x);
    regs.x = u16::from(x);
    game.set_regs(regs);
    game.coop.in_contact = true;
    with_p2_swapped(game, |game| game.call_isolated_ret(target, TrapReturn::Rts));
    game.coop.in_contact = false;
    if game.wram()[base] == 0 {
        game.coop.p2_fx &= !(1 << k);
    }
    regs.pc = (pc as u16).wrapping_add(11);
    game.set_regs(regs);
}

/// The object sprite emitter (listing `$80:D67C`, `X` = the object; `JSR`
/// from the OAM build's draw lists), found through the player's own call
/// `LDX #$0BA8; LDA $0E,X; BEQ; JSR emitter` ([`crate::coop::PLAYER_OAM`]).
pub(crate) fn emitter(game: &Game) -> u32 {
    let at = game.hook(crate::coop::PLAYER_OAM) + 8;
    (at & 0xFF_0000) | u32::from(u16::from_le_bytes([game.rom_read(at), game.rom_read(at + 1)]))
}

/// OAM cursor (next slot) during the build.
const OAM_CURSOR: usize = 0xE4;

/// The emitter is about to draw a P2 effect (it queued itself on a draw
/// list in its isolated update): draw it isolated too and return from the
/// `JSR`, so P2's effects cost P1 no CPU time. The emitter's overflow exit
/// (after slot 127: `PLX; PLX; JMP`) would unwind through the isolated
/// call's sentinel, so an effect that might not fit is not drawn.
fn effect_oam(game: &mut Game) {
    let r = game.regs();
    let k = r.x.wrapping_sub(FX_FIRST) / FX_STRIDE;
    if !game.coop.enabled || game.coop.in_contact || k >= FX_SLOTS || !r.x.wrapping_sub(FX_FIRST).is_multiple_of(FX_STRIDE) || game.coop.p2_fx >> k & 1 == 0 {
        return;
    }
    let base = usize::from(r.x);
    let need = crate::coop::layout_len(game, &game.wram()[base..base + usize::from(FX_STRIDE)]);
    if usize::from(game.wram()[OAM_CURSOR]) + need < 128 - crate::coop_hud::HUD_SLOTS {
        game.coop.in_contact = true; // the call starts on this trap
        game.call_isolated_ret(r.pbr_pc(), TrapReturn::Rts);
        game.coop.in_contact = false;
    }
    game.emulate_return(false);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ram::X_POS_X;

    /// `r2-1` (PAL): X walks right to 3912, just short of the collapsing road
    /// chunk at 3984 (`$1428` type 9); then co-op starts, P1 stands, P2 walks
    /// onto the chunk. P2 alone starts the collapse (`+$2F` counts down from
    /// `$1E`, as for X at the same spot solo); with the trap off it doesn't.
    #[test]
    fn coop_p2_triggers_collapse() {
        let Some(rom) = std::env::var_os("MMX_ROM") else {
            return eprintln!("MMX_ROM not set; skipping");
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/r2-1.state");
        let Ok(state) = std::fs::read(st) else {
            return eprintln!("{st} missing; skipping");
        };
        let rom = std::fs::read(rom).unwrap();
        for trap in [true, false] {
            let mut g = Game::from_rom(rom.clone()).unwrap();
            if g.load_state(&state).is_err() {
                return eprintln!("{st} is not a state of this dump; skipping");
            }
            for _ in 0..120 {
                g.step(crate::Pads { p1: crate::BTN_RIGHT, p2: 0 });
            }
            let chunk = (0..20u16)
                .map(|k| 0x1428 + k * 0x40)
                .find(|&a| g.wram()[usize::from(a)] != 0 && g.wram()[usize::from(a) + 0x0A] == 9)
                .map(usize::from)
                .expect("a road chunk ahead");
            assert_eq!(g.wram()[chunk + usize::from(COLLAPSE_TIMER)], 0);
            g.coop.enabled = true;
            crate::coop::register_coop_traps(&mut g).unwrap();
            if !trap {
                let a = g.hook(HOOK_COLLAPSE);
                g.traps.set_untrapped(a, true);
            }
            let p1x = |g: &Game| u16::from_le_bytes([g.wram()[ix(X_POS_X)], g.wram()[ix(X_POS_X) + 1]]);
            let x = p1x(&g);
            let mut started = None;
            for f in 1..=70 {
                g.step(crate::Pads { p1: 0, p2: if f <= 50 { crate::BTN_RIGHT } else { 0 } });
                if started.is_none() && g.wram()[chunk + usize::from(COLLAPSE_TIMER)] != 0 {
                    started = Some(f);
                }
            }
            assert_eq!(p1x(&g), x, "P1 stood still");
            if trap {
                let f = started.expect("P2 on the chunk starts the collapse");
                assert!((30..=50).contains(&f), "started at f{f}");
            } else {
                assert_eq!(started, None);
            }
        }
    }
}
