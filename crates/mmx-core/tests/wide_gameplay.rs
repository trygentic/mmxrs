//! Wide gameplay (docs/widescreen.md "Wide gameplay"). Skips unless `MMX_ROM` is set.

use std::cell::RefCell;

use mmx_core::hooks::HookId;
use mmx_core::netgame::{net_checksum, trapset_id};
use mmx_core::wide_gameplay::widen;
use mmx_core::{Game, Pads, TrapReturn};

fn game(state: &str) -> Option<Game> {
    let rom = std::fs::read(std::env::var_os("MMX_ROM")?).ok()?;
    let mut g = Game::from_rom(rom).expect("load ROM");
    let p = format!("{}/../../states/{state}.state", env!("CARGO_MANIFEST_DIR"));
    g.load_state(&std::fs::read(p).ok()?).expect("load state");
    Some(g)
}

/// Walk right, jump and shoot now and then (enemies pass both edges).
fn pads(f: u32) -> Pads {
    let mut p1 = 0x0100;
    if f % 90 < 12 {
        p1 |= 0x8000; // B: jump
    }
    if f % 40 < 3 {
        p1 |= 0x4000; // Y: shoot
    }
    Pads { p1, p2: 0 }
}

const TESTS: [HookId; 3] = [HookId::QueueTest, HookId::QueueTestBig, HookId::DespawnTest];

thread_local! {
    /// (calls, mismatches) of the pin traps.
    static SEEN: RefCell<(u32, Vec<String>)> = const { RefCell::new((0, Vec::new())) };
}

/// The window test's immediates (`ADC #add`, `CMP #width`) for its X `BCS` at `pc`.
fn imms(g: &Game, pc: u32) -> (u16, u16) {
    let w = |a: u32| u16::from_le_bytes([g.rom_read(a), g.rom_read(a + 1)]);
    (w(pc - 0x0F + 0x0A), w(pc - 0x0F + 0x0D))
}

fn word(g: &Game, a: usize) -> u16 {
    u16::from_le_bytes([g.wram()[a], g.wram()[a + 1]])
}

/// At the X `BCS`: `widen(.., 0)` is the ROM's own A and P, and `widen(..,
/// 136)` says "inside" exactly when the object's X is within the widened
/// window (`x - cam + add + 136 < width + 272`, from `+$05` and `$1E4D`).
fn pin(g: &mut Game) {
    let r = g.regs();
    let (add, width) = imms(g, r.pbr_pc());
    let same = widen(r, add, width, 0).expect("binary mode");
    let wide = widen(r, add, width, 136).unwrap();
    let x = word(g, usize::from(r.d) + 5).wrapping_sub(word(g, 0x1E4D));
    let inside = x.wrapping_add(add + 136) < width + 272;
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        s.0 += 1;
        if (same != r || (wide.p & 1 == 0) != inside || wide.a != x.wrapping_add(add + 136)) && s.1.len() < 10 {
            s.1.push(format!("frame {} ${:06X}: rom {r:X?} m0 {same:X?} m136 {wide:X?} x {x}", g.frame_count(), r.pbr_pc()));
        }
    });
}

/// Pin of the Rust compare: at every call of `$82:806E` / `$808F` / `$80B4`
/// over 600 frames of hw-a and highway-mid (the ROM untouched: observe-only
/// traps), `widen` at M = 0 gives exactly the ROM's registers (A = the
/// subtraction, N V Z C), and at M = 136 its carry is the widened window.
#[test]
fn window_test_m0_is_register_exact() {
    for st in ["hw-a", "highway-mid"] {
        let Some(mut g) = game(st) else { return eprintln!("MMX_ROM not set; skipping") };
        for id in TESTS {
            g.traps.register(g.hook(id), "pin", TrapReturn::Observe, pin);
        }
        SEEN.with(|s| *s.borrow_mut() = (0, Vec::new()));
        for f in 0..600 {
            g.step(pads(f));
        }
        let (calls, bad) = SEEN.with(|s| s.borrow().clone());
        eprintln!("{st}: {calls} window tests");
        assert!(calls > 1000, "{st}: only {calls} calls");
        assert!(bad.is_empty(), "{st}: {bad:#?}");
    }
}

fn m0_trap(g: &mut Game) {
    let r = g.regs();
    let (add, width) = imms(g, r.pbr_pc());
    if let Some(out) = widen(r, add, width, 0) {
        g.set_regs(out);
    }
}

/// The trap body at M = 0 (registers written back at every X `BCS`) leaves
/// the game exactly as the untrapped ROM: the queue flags (`+$0E` of every
/// object), the draw lists `$0920..$0A9F`, their counts `$E7..$EC` and the
/// whole WRAM every frame, 600 frames of hw-a. Cycle-exact too: the trap
/// skips no instruction.
#[test]
fn window_test_m0_matches_rom_run() {
    let (Some(mut rom), Some(mut rust)) = (game("hw-a"), game("hw-a")) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    for id in TESTS {
        rust.traps.register(rust.hook(id), "m0", TrapReturn::Observe, m0_trap);
    }
    for f in 0..600 {
        rom.step(pads(f));
        rust.step(pads(f));
        let (a, b) = (rom.wram(), rust.wram());
        assert_eq!(a[0x0920..0x0AA0], b[0x0920..0x0AA0], "draw lists, frame {f}");
        assert_eq!(a[0xE7..0xED], b[0xE7..0xED], "draw counts, frame {f}");
        for o in (0x0E68..0x1928).step_by(0x40).chain((0x1928..0x1D08).step_by(0x20)) {
            assert_eq!(a[o + 0x0E], b[o + 0x0E], "+$0E of ${o:04X}, frame {f}");
        }
        assert_eq!(a[..], b[..], "WRAM, frame {f}");
    }
    assert_eq!(rom.save_state(), rust.save_state());
}

/// Off is nothing: a game that had wide gameplay on and off again has the
/// trap set, the checksum and the save state of one that never had it
/// (2400 frames).
#[test]
fn off_is_untouched() {
    let (Some(mut a), Some(mut b)) = (game("hw-a"), game("hw-a")) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    let plain = trapset_id(&a);
    b.set_wide_gameplay(true);
    assert_ne!(trapset_id(&b), plain);
    b.set_wide_gameplay(false);
    assert_eq!(trapset_id(&b), plain);
    assert_eq!(b.traps.len(), 0);
    for f in 0..2400 {
        let p = if f < 1200 { pads(f) } else { Pads { p1: 0x0200, p2: 0 } };
        a.step(p);
        b.step(p);
        assert_eq!(net_checksum(&a), net_checksum(&b), "frame {f}");
    }
    assert_eq!(a.save_state(), b.save_state());
}

/// Live objects of the `$0E68` tables: (slot, id `+$0A`, X).
fn objects(g: &Game) -> Vec<(usize, u8, i32)> {
    let w = g.wram();
    (0..43)
        .map(|i| 0x0E68 + i * 0x40)
        .filter(|&o| w[o] != 0)
        .map(|o| ((o - 0x0E68) / 0x40, w[o + 0x0A], i32::from(word(g, o + 5))))
        .collect()
}

/// Per object that appears: its X relative to the camera when first seen;
/// and the leftmost relative X any object reached. 600 frames walking right.
pub fn spawn_xs(g: &mut Game) -> (Vec<i32>, i32) {
    let mut live: Vec<(usize, u8)> = Vec::new();
    let (mut first, mut left) = (Vec::new(), i32::MAX);
    for f in 0..600 {
        g.step(pads(f));
        let cam = i32::from(word(g, 0x1E4D));
        let now = objects(g);
        for &(slot, id, x) in &now {
            if !live.contains(&(slot, id)) {
                first.push(x - cam);
            }
            left = left.min(x - cam);
        }
        live = now.iter().map(|&(s, id, _)| (s, id)).collect();
    }
    (first, left)
}

/// With it on, walking right on hw-a: objects appear out past the 21:9
/// picture on the right and live far out on the left; off, they appear near
/// the 256-px edge.
#[test]
fn enemies_spawn_at_the_wide_edges() {
    let (Some(mut off), Some(mut on)) = (game("hw-a"), game("hw-a")) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    on.set_wide_gameplay(true);
    let (f_off, l_off) = spawn_xs(&mut off);
    let (f_on, l_on) = spawn_xs(&mut on);
    eprintln!("off: first X {f_off:?}, leftmost {l_off}\non: first X {f_on:?}, leftmost {l_on}");
    let far = |v: &[i32], lo: i32| v.iter().filter(|&&x| x >= lo).count();
    assert!(far(&f_on, 256 + 136) > 0, "on: nothing spawned past the 21:9 edge");
    assert_eq!(far(&f_off, 256 + 64), 0, "off: something spawned far out");
    assert!(l_on < -64 - 64, "on: nothing lived far out on the left ({l_on})");
}
