//! Wide gameplay (docs/widescreen.md "Wide gameplay"): the object window
//! (spawn, draw queue, despawn) moved out past the widescreen margins.
//! Opt-in; changes the simulation, so it is part of the netplay identity.

use crate::hooks::HookId;
use crate::{Game, Registers, TrapReturn};

/// Draw-queue widening per side (the 21:9 margin, the widest preset).
pub const WIDE_GAMEPLAY_PX: u16 = 136;
/// Spawn column / despawn widening per side: [`WIDE_GAMEPLAY_PX`] rounded up
/// to the spawner's 32-px columns.
pub const WIDE_SPAWN_PX: u16 = WIDE_GAMEPLAY_PX.next_multiple_of(32);

/// Register (or remove) the wide gameplay traps.
pub(crate) fn register(game: &mut Game, on: bool) {
    let traps: [(HookId, &'static str, crate::TrapFn); 5] = [
        (HookId::QueueTest, "wide_queue_test", window_x),
        (HookId::QueueTestBig, "wide_queue_test_big", window_x),
        (HookId::DespawnTest, "wide_despawn_test", window_x),
        (HookId::SpawnColumn, "wide_spawn_column", spawn_column),
        (HookId::SpawnRow, "wide_spawn_row", spawn_row),
    ];
    for (id, name, body) in traps {
        let a = game.hook(id);
        if on {
            game.traps.register(a, name, TrapReturn::Observe, body);
        } else {
            game.traps.unregister(a);
        }
    }
}

fn word(g: &Game, a: usize) -> u16 {
    let w = g.wram();
    u16::from_le_bytes([w[a & 0x1FFFF], w[(a + 1) & 0x1FFFF]])
}

const P_C: u8 = 0x01;
const P_Z: u8 = 0x02;
const P_D: u8 = 0x08;
const P_V: u8 = 0x40;
const P_N: u8 = 0x80;

/// `CLC; ADC #add; CMP #width` on `d` (16-bit, binary): the result in A
/// and P (N Z C from the `CMP`, V from the `ADC`, the rest kept).
fn add_compare(p: u8, d: u16, add: u16, width: u16) -> (u16, u8) {
    let a = d.wrapping_add(add);
    let ovf = (!(d ^ add) & (d ^ a) & 0x8000) != 0;
    let r = a.wrapping_sub(width);
    let mut p = p & !(P_N | P_V | P_Z | P_C);
    p |= if ovf { P_V } else { 0 };
    p |= if r & 0x8000 != 0 { P_N } else { 0 };
    p |= if r == 0 { P_Z } else { 0 };
    p |= if a >= width { P_C } else { 0 };
    (a, p)
}

/// The registers at a window test's X `BCS` (entry + `$0F`; A = `x - cam +
/// add`, P from `CMP #width`) as they would be had the routine's
/// immediates been `add + m` and `width + 2m`. At `m = 0` that is `r`
/// itself (the pin test). `None` in decimal mode (the `ADC` was BCD).
pub fn widen(r: Registers, add: u16, width: u16, m: u16) -> Option<Registers> {
    if r.p & P_D != 0 || r.emulation {
        return None;
    }
    let (a, p) = add_compare(r.p, r.a.wrapping_sub(add), add.wrapping_add(m), width.wrapping_add(2 * m));
    Some(Registers { a, p, ..r })
}

/// Observe trap on a window test's X `BCS`: the X compare redone with the
/// window `m` wider per side; the ROM's branch then goes the widened way.
/// Cycle-exact (no instruction skipped), a no-op at `m = 0`.
fn window_x(g: &mut Game) {
    let r = g.regs();
    let entry = r.pbr_pc() - 0x0F;
    let m = if r.pbr_pc() == g.hook(HookId::DespawnTest) { WIDE_SPAWN_PX } else { WIDE_GAMEPLAY_PX };
    let add = u16::from_le_bytes([g.rom_read(entry + 0x0A), g.rom_read(entry + 0x0B)]);
    let width = u16::from_le_bytes([g.rom_read(entry + 0x0D), g.rom_read(entry + 0x0E)]);
    if let Some(out) = widen(r, add, width, m) {
        g.set_regs(out);
    }
}

/// `$80:DC56`: a horizontal column is about to be spawned at `$00` (`cam +
/// 256` scrolling right, `cam` scrolling left). Move it out.
fn spawn_column(g: &mut Game) {
    let (col, cam) = (word(g, 0), word(g, 0x1E4D));
    let col = if col == cam { col.wrapping_sub(WIDE_SPAWN_PX) } else { col.wrapping_add(WIDE_SPAWN_PX) };
    g.wram_mut()[..2].copy_from_slice(&col.to_le_bytes());
}

/// `$80:DC9B` (`STA $06`, A = 10 columns from `$00 = cam - 32`): the row
/// sweep, widened both sides.
fn spawn_row(g: &mut Game) {
    let x0 = word(g, 0).wrapping_sub(WIDE_SPAWN_PX);
    g.wram_mut()[..2].copy_from_slice(&x0.to_le_bytes());
    let mut r = g.regs();
    r.a = r.a.wrapping_add(2 * WIDE_SPAWN_PX / 32);
    g.set_regs(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_flags() {
        // inside: x - cam = 100, +$20 = 132 < $140
        let (a, p) = add_compare(0, 100, 0x20, 0x140);
        assert_eq!((a, p & P_C, p & P_N), (132, 0, P_N));
        // left of the window: wraps to a large unsigned
        let (a, p) = add_compare(0, 0xFF9C, 0x20, 0x140);
        assert_eq!((a, p & P_C), (0xFFBC, P_C));
        // 100 px left of the picture: out at m = 0, in at m = 136
        let r = Registers { a: 0xFF9Cu16.wrapping_add(0x20), p: P_C, ..Default::default() };
        assert_eq!(widen(r, 0x20, 0x140, 0).unwrap().p & P_C, P_C);
        assert_eq!(widen(r, 0x20, 0x140, WIDE_GAMEPLAY_PX).unwrap().p & P_C, 0);
        assert_eq!(WIDE_SPAWN_PX, 160);
    }
}
