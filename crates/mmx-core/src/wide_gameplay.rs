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
    let traps: [(HookId, &'static str, crate::TrapFn); 8] = [
        (HookId::QueueTest, "wide_queue_test", window_x),
        (HookId::QueueTestBig, "wide_queue_test_big", window_x),
        (HookId::DespawnTest, "wide_despawn_test", window_x),
        (HookId::SpawnColumn, "wide_spawn_column", spawn_column),
        (HookId::SpawnRow, "wide_spawn_row", spawn_row),
        (HookId::KindCount, "wide_kind_count", kind_count),
        (HookId::SpawnEntry, "wide_spawn_entry", spawn_entry),
        (HookId::GfxTrigger, "wide_gfx_trigger", gfx_trigger),
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
/// Cycle-exact (no instruction skipped), a no-op at `m = 0`. An enemy whose
/// tiles were given to another set keeps the ROM's window (C set: the ROM
/// says outside), so it is culled and removed where the ROM would.
fn window_x(g: &mut Game) {
    let r = g.regs();
    if r.p & P_C != 0 && obj_overwritten(g, usize::from(r.d)) {
        return;
    }
    let entry = r.pbr_pc() - 0x0F;
    let m = if r.pbr_pc() == g.hook(HookId::DespawnTest) { WIDE_SPAWN_PX } else { WIDE_GAMEPLAY_PX };
    let add = u16::from_le_bytes([g.rom_read(entry + 0x0A), g.rom_read(entry + 0x0B)]);
    let width = u16::from_le_bytes([g.rom_read(entry + 0x0D), g.rom_read(entry + 0x0E)]);
    if let Some(out) = widen(r, add, width, m) {
        g.set_regs(out);
    }
}

/// `$80:DC56`: a horizontal column is about to be spawned at `$00` (`cam +
/// 256` scrolling right, `cam` scrolling left). The ROM spawns its own
/// column; the one [`WIDE_SPAWN_PX`] further out is spawned first, through
/// the same spawner, isolated (so entries [`spawn_entry`] holds back there
/// still come at the ROM's own column).
fn spawn_column(g: &mut Game) {
    let (col, cam) = (word(g, 0), word(g, 0x1E4D));
    let wide = if col == cam { col.wrapping_sub(WIDE_SPAWN_PX) } else { col.wrapping_add(WIDE_SPAWN_PX) };
    spawn_columns(g, &[wide]);
}

/// The ROM's column spawner (`$80:DCB0`, the `JSR` after the
/// [`HookId::SpawnColumn`] hook) on each column X of `xs`, 288 px tall
/// from `camY - 32`, isolated; DP `$00..$1F` and the registers put back.
/// Entries already spawned are skipped by the ROM (their flag).
fn spawn_columns(g: &mut Game, xs: &[u16]) {
    let col_call = g.hook(HookId::SpawnColumn) + 0x0E;
    if g.rom_read(col_call) != 0x20 {
        return; // not the JSR $DCB0 the signature promises
    }
    let spawner = (col_call & 0xFF_0000) | u32::from(u16::from_le_bytes([g.rom_read(col_call + 1), g.rom_read(col_call + 2)]));
    let cam_y = word(g, 0x1E50);
    let saved = g.regs();
    let scratch: [u8; 0x20] = g.wram()[..0x20].try_into().unwrap();
    g.set_regs(Registers { d: 0, p: saved.p & !0x30, ..saved });
    for &x in xs {
        let w = g.wram_mut();
        w[..2].copy_from_slice(&x.to_le_bytes());
        w[2..4].copy_from_slice(&cam_y.wrapping_sub(32).to_le_bytes());
        w[4..6].copy_from_slice(&0x120u16.to_le_bytes());
        g.call_isolated_ret(spawner, TrapReturn::Rts);
    }
    g.wram_mut()[..0x20].copy_from_slice(&scratch);
    g.set_regs(saved);
}

/// `$80:DD18` (Rev 1 `$80:DD43`): the column spawner's `BEQ` after a layout
/// entry's section test (`LDA ($18); AND #$F8; CMP $1FFE; BEQ spawn; BCS
/// next`; DB = `$85`, `($18)` the record, `$00` the column X). An enemy
/// (record type 3, the `$0E68` table) in a column outside the ROM's own
/// spawn window whose graphics set is not loaded yet is skipped the way the
/// ROM skips a later section (Z clear, C set): its layout flag stays clear
/// and the ROM spawns it at its own column, as without wide gameplay.
///
/// Why: an enemy's init (`$82:827D`) copies its VRAM slot (`+$18`) and
/// palette (`+$11`) from the loaded-set tables `$7F:8200` / `$7F:8300`,
/// indexed through the id table its `LDA $A5E4,X` reads (DB = `$86`), once.
/// The sets load when the player crosses invisible trigger objects
/// (`$83:FD7B`), which the ROM's spawn line never outruns; 160 px further
/// out it does (Highway: the jumping mech and the Sky Claws by the trucks),
/// and the enemy then lives with slot 0 and palette 0: X's streamed tiles in
/// the wrong colours, for good.
fn spawn_entry(g: &mut Game) {
    let mut r = g.regs();
    if r.p & P_Z == 0 && r.p & P_C != 0 {
        return; // the ROM skips it anyway
    }
    let rec = 0x85_0000 | u32::from(word(g, 0x18));
    if g.rom_read(rec) & 0x0F != 3 {
        return;
    }
    let rel = (word(g, 0) & !31).wrapping_sub(word(g, 0x1E4D)) as i16;
    if (-63..=256).contains(&rel) || gfx_loaded(g, g.rom_read(rec + 3)) {
        return;
    }
    r.p = (r.p & !P_Z) | P_C;
    g.set_regs(r);
}

/// Whether the graphics set of enemy `id` is loaded: `$7F:8300[set] != 0`
/// (the loader stores the palette `| $20`; the fixed sets have theirs too),
/// with `set` as `$82:827D` finds it (8-bit index: `(id - 1) * 2 + 1`), and
/// not since replaced in VRAM by another set of the current list (the
/// tables are never cleared: an evicted set still reads as loaded).
pub fn gfx_loaded(g: &Game, id: u8) -> bool {
    let set = gfx_set(g, id);
    let (slot, attr) = (g.wram()[0x1_8200 + usize::from(set)], g.wram()[0x1_8300 + usize::from(set)]);
    attr != 0 && !clashes(g, &gfx_list(g, g.wram()[0x1F08]), Some(set), slot, attr, true)
}

/// Whether the loader's list `list` puts another set's tiles where a user of
/// `set` (unknown: `None`) at `slot` (`+$18`, page from `attr` bit 0) draws
/// from, or (`pal`) another set's palette in its row (`attr` bits 1-3;
/// unless its own set is in the list too: made to go together). Only sets
/// the loader manages (`attr & $20`). An unknown set spans one tile.
fn clashes(g: &Game, list: &[(u8, u16, u8)], set: Option<u8>, slot: u8, attr: u8, pal: bool) -> bool {
    if attr & 0x20 == 0 {
        return false; // fixed graphics, or none
    }
    let w = g.wram();
    let tile = u16::from(slot) | u16::from(attr & 1) << 8;
    let end = tile + set.map_or(1, |s| gfx_tiles(g, s));
    let pal = pal && !list.iter().any(|&(s, ..)| set == Some(s));
    list.iter().any(|&(s, vram, row)| {
        let t = vram >> 4;
        let at = |s: u8| u16::from(w[0x1_8200 + usize::from(s)]) | u16::from(w[0x1_8300 + usize::from(s)] & 1) << 8;
        let same = t == tile && set.map_or(at(s) == tile, |x| x == s);
        !same && (t < end && tile < t + gfx_tiles(g, s) || pal && row == attr >> 1 & 7)
    })
}

/// What object `o` (`$0E68..$1D08`) draws from the loader's sets: (set,
/// slot, attr). Enemies (`$0E68` table) know their set (from the id); not
/// initialised yet (`$82:827D` runs on the first update), what they will copy.
fn obj_gfx(g: &Game, o: usize) -> (Option<u8>, u8, u8) {
    let w = g.wram();
    if o >= 0x1228 {
        return (None, w[o + 0x18], w[o + 0x11]);
    }
    let set = gfx_set(g, w[o + 0x0A]);
    match w[o + 0x11] {
        0 => (Some(set), w[0x1_8200 + usize::from(set)], w[0x1_8300 + usize::from(set)]),
        a => (Some(set), w[o + 0x18], a),
    }
}

/// The object at `o` draws from tiles (or a palette row) the loader has since
/// given to another set. The ROM's layout has it gone by then (its window),
/// so the window tests give it the ROM's window back.
pub fn obj_overwritten(g: &Game, o: usize) -> bool {
    if !(0x0E68..0x1D08).contains(&o) || g.wram()[o] == 0 {
        return false;
    }
    let (set, slot, attr) = obj_gfx(g, o);
    clashes(g, &gfx_list(g, g.wram()[0x1F08]), set, slot, attr, true)
}

/// The graphics set of enemy `id` (`$82:827D`'s `LDA $A5E5,X`, DB = `$86`).
pub fn gfx_set(g: &Game, id: u8) -> u8 {
    let init = g.hook(HookId::ObjGfxInit);
    let table = u16::from_le_bytes([g.rom_read(init + 0x0E), g.rom_read(init + 0x0F)]);
    let x = id.wrapping_sub(1).wrapping_mul(2).wrapping_add(1);
    g.rom_read(0x86_0000 | u32::from(table.wrapping_add(u16::from(x))))
}

/// `$83:FDE3` (the `RTS` of the trigger objects' player test `$83:FDD3`:
/// A = the player's X, P from `CMP` against the trigger's X, D = the
/// trigger): a graphics-set trigger (id `$15`, X) fires as if the player
/// were [`WIDE_SPAWN_PX`] further right, so a set is in by the time the wide
/// spawn line reaches its enemies, as the ROM's is by its own line. Only N
/// (the callers' `BPL` / `BMI`) changes.
///
/// - behind (`$02 = 2`, the `BMI` at `$FDC3` next): "ahead" once
///   `x + 160 >= tx`, unless that is early (`x < tx`) and [`early_ok`]
///   refuses; refused, it asks again next frame. An early fire sets `+$0F`
///   (free in these objects; their removal clears it).
/// - ahead (`$02 = 0`, the `BPL` at `$FDAE`): with `+$0F` set, "behind"
///   once `x + 160 < tx` (backed out of the early band: the previous list
///   is reloaded), and `+$0F` is cleared once `x >= tx` (the ROM's own
///   state from then on). Without it, the ROM's test: walking back left
///   reloads where the ROM does.
/// - init (`BPL` at `$FD9C`): the ROM's test; `+$0F` cleared.
fn gfx_trigger(g: &mut Game) {
    let mut r = g.regs();
    let d = usize::from(r.d);
    if r.x & 0xFF != 5 || g.wram()[d + 0x0A] != 0x15 {
        return; // a Y trigger, or not the graphics loader's
    }
    let ret = (r.pbr_pc() & 0xFF_0000) | u32::from(word(g, usize::from(r.s) + 1).wrapping_add(1));
    let (px, tx) = (r.a, word(g, d + 5));
    let ahead_rom = px.wrapping_sub(tx) & 0x8000 == 0;
    let ahead_wide = px.wrapping_add(WIDE_SPAWN_PX).wrapping_sub(tx) & 0x8000 == 0;
    let ahead = match (g.rom_read(ret), g.rom_read(ret + 1)) {
        (0x30, _) if !ahead_rom && ahead_wide && early_ok(g, g.wram()[d + 0x0B] & 0x0F) => {
            g.wram_mut()[d + 0x0F] = 1;
            true
        }
        (0x10, 0x0F) if g.wram()[d + 0x0F] != 0 => {
            if ahead_rom || !ahead_wide {
                g.wram_mut()[d + 0x0F] = 0;
            }
            ahead_wide
        }
        (0x10, 0x04) => {
            g.wram_mut()[d + 0x0F] = 0;
            return;
        }
        _ => return,
    };
    r.p = (r.p & !P_N) | if ahead { 0 } else { P_N };
    g.set_regs(r);
}

/// Graphics-set list `idx` of the current stage (`$1F7A`), as the loader
/// thread reads it: (set, VRAM word address, OBJ palette) per entry.
fn gfx_list(g: &Game, idx: u8) -> Vec<(u8, u16, u8)> {
    let op = g.hook(HookId::GfxLoader) + 0x1E; // `LDA $ACEE,X`, DB = $86
    let tbl = u16::from_le_bytes([g.rom_read(op), g.rom_read(op + 1)]);
    let rd = |o: u16| g.rom_read(0x86_0000 | u32::from(tbl.wrapping_add(o)));
    let r16 = |o: u16| u16::from_le_bytes([rd(o), rd(o.wrapping_add(1))]);
    let base = r16(u16::from(g.wram()[0x1F7A]) * 2);
    let mut e = r16(base.wrapping_add(u16::from(idx) * 2));
    let mut out = Vec::new();
    while rd(e) != 0xFF && out.len() < 16 {
        out.push((rd(e), r16(e.wrapping_add(1)), rd(e.wrapping_add(5)) >> 4 & 7));
        e = e.wrapping_add(6);
    }
    out
}

/// Size of graphics set `set` in 8x8 tiles (the decompressor's byte count / 32).
fn gfx_tiles(g: &Game, set: u8) -> u16 {
    let op = g.hook(HookId::GfxDecomp) + 0x1B; // `LDA $F6F7,Y`, Y = set * 5, DB = $86
    let a = u16::from_le_bytes([g.rom_read(op), g.rom_read(op + 1)]).wrapping_add(u16::from(set) * 5);
    let rd = |o: u16| g.rom_read(0x86_0000 | u32::from(a.wrapping_add(o)));
    u16::from_le_bytes([rd(0), rd(1)]) / 32
}

/// Whether list `idx` may load now, ahead of the ROM: the loader thread
/// (`$0040`) is idle, no set goes below VRAM `$1000` (OBJ page 0: the
/// player's, shots' and items' tiles), and no object in use has tiles or a
/// palette row that the list gives to another set.
fn early_ok(g: &Game, idx: u8) -> bool {
    let w = g.wram();
    if w[0x40] != 0 {
        return false;
    }
    let list = gfx_list(g, idx);
    if list.iter().any(|&(_, vram, _)| vram < 0x1000) {
        return false;
    }
    // (first, count, stride): enemies, items / enemy shots, `$1628` objects, effects.
    let tables = [(0x0E68, 15, 0x40), (0x1228, 8, 0x40), (0x1628, 16, 0x30), (0x1928, 31, 0x20)];
    let mut objs = tables.into_iter().flat_map(|(first, n, stride)| (0..n).map(move |k| first + k * stride));
    // In use: inside the ROM's own object window (X -64..320, `$82:806E`),
    // or an enemy about to copy its set's slot. One further out exists only
    // with wide gameplay; overwritten, it gets the ROM's window back
    // ([`window_x`]) and goes as it would have without wide gameplay.
    let cam = word(g, 0x1E4D);
    let in_use = |o: usize| w[o] != 0 && (word(g, o + 5).wrapping_sub(cam).wrapping_add(64) < 384 || o < 0x1228 && w[o + 0x11] == 0);
    !objs.any(|o| {
        in_use(o) && !obj_overwritten(g, o) && {
            let (set, slot, attr) = obj_gfx(g, o);
            clashes(g, &list, set, slot, attr, true)
        }
    })
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

/// Spawn the columns the moved spawn line skips when wide gameplay starts
/// mid-stage (a save state made without it): the five 32-px columns each
/// side between the ROM's window and the wide one, 288 px tall from `camY -
/// 32`, through the ROM's own column spawner (`$80:DCB0`, isolated). A stage
/// entered with it on needs none (the entry's row sweep is widened). Only in
/// a stage; entries already spawned are skipped by the ROM (their flag).
pub fn sweep(g: &mut Game) {
    let w = g.wram();
    if !(w[0xD2] == 4 && w[0xD3] != 0) {
        return;
    }
    let cam = word(g, 0x1E4D) & !31;
    let xs: Vec<u16> = (1..=WIDE_SPAWN_PX / 32).flat_map(|k| [cam.wrapping_add(256 + 32 * k), cam.wrapping_sub(32 * k)]).collect();
    spawn_columns(g, &xs);
}

/// `$84:A1E0` (`BEQ` past an empty slot in the same-kind count
/// `$84:A1D5`, X = the slot): a live object outside the ROM's own despawn
/// window (X `-64..320` from the camera) counts as an empty slot. Such
/// objects only exist with wide gameplay; without this a capped kind (the
/// Sky Claw allows 2, `$82:96AF`) refuses every new spawn while old ones
/// linger in the wider window. Only the branch changes (Z), cycle-exact.
fn kind_count(g: &mut Game) {
    let mut r = g.regs();
    if r.p & P_Z != 0 {
        return;
    }
    let rel = word(g, usize::from(r.x) + 5).wrapping_sub(word(g, 0x1E4D)).wrapping_add(64);
    if rel >= 384 {
        r.p |= P_Z;
        g.set_regs(r);
    }
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
