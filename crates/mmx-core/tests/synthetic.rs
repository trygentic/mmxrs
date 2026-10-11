//! `Game` against a hand-assembled 32 KiB LoROM (no Mega Man X ROM needed).
//!
//! ```text
//! $00:8000  SEI / CLC / XCE / REP #$30 / LDX #$1FFF / TXS
//!           SEP #$20 / LDA #$80 / STA $4200 (NMI on) / REP #$20
//! $00:8012  loop: JSR sub / JSL sublong / INC $0100 / BRA loop
//! $00:8030  sub:     INC $0200 / RTS
//! $00:8040  sublong: INC $0210 / RTL
//! $00:8050  nmi:     RTI
//! $00:8060  unwind:  PLX / PLX / JMP loop  (leaves through its caller's frame)
//! $00:8070  spin:    BRA spin
//! ```
//! Counters are 16-bit words (A is 16-bit), so they sit two bytes apart.

use mmx_core::{Game, IsolatedError, Pads, RomError, TrapReturn};

const SUB: u32 = 0x00_8030;
const SUBLONG: u32 = 0x00_8040;
const LOOP_CTR: usize = 0x0100;
const SUB_CTR: usize = 0x0200;
const LONG_CTR: usize = 0x0210;

fn rom_with(title: &str, chipset: u8, version: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    let code: &[(usize, &[u8])] = &[
        (
            0x0000,
            &[
                0x78, 0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xFF, 0x1F, 0x9A, // init
                0xE2, 0x20, 0xA9, 0x80, 0x8D, 0x00, 0x42, 0xC2, 0x20, // NMI on
                0x20, 0x30, 0x80, // JSR $8030
                0x22, 0x40, 0x80, 0x00, // JSL $008040
                0xEE, 0x00, 0x01, // INC $0100
                0x80, 0xF4, // BRA $8012
            ],
        ),
        (0x0030, &[0xEE, 0x00, 0x02, 0x60]), // INC $0200 / RTS
        (0x0040, &[0xEE, 0x10, 0x02, 0x6B]), // INC $0210 / RTL
        (0x0050, &[0x40]),                   // RTI
        (0x0060, &[0xFA, 0xFA, 0x4C, 0x12, 0x80]), // PLX / PLX / JMP $8012
        (0x0070, &[0x80, 0xFE]),             // BRA $8070
    ];
    for (at, bytes) in code {
        rom[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    let mut t = [b' '; 21];
    t[..title.len()].copy_from_slice(title.as_bytes());
    rom[0x7FC0..0x7FD5].copy_from_slice(&t);
    rom[0x7FD5] = 0x20; // LoROM
    rom[0x7FD6] = chipset;
    rom[0x7FD7] = 0x05; // 32 KiB
    rom[0x7FD9] = 0x01; // USA
    rom[0x7FDA] = 0x33;
    rom[0x7FDB] = version;
    rom[0x7FEA..0x7FEC].copy_from_slice(&[0x50, 0x80]); // native NMI
    rom[0x7FFA..0x7FFC].copy_from_slice(&[0x50, 0x80]); // emulation NMI
    rom[0x7FFC..0x7FFE].copy_from_slice(&[0x00, 0x80]); // reset
    rom[0x7FDC..0x7FE0].copy_from_slice(&[0xFF, 0xFF, 0x00, 0x00]);
    let sum = rom.iter().fold(0u16, |s, &b| s.wrapping_add(u16::from(b)));
    rom[0x7FDC..0x7FDE].copy_from_slice(&(!sum).to_le_bytes());
    rom[0x7FDE..0x7FE0].copy_from_slice(&sum.to_le_bytes());
    rom
}

fn game() -> Game {
    Game::from_rom(rom_with("MEGAMAN X", 0, 0)).unwrap()
}

fn word(g: &Game, i: usize) -> u16 {
    u16::from_le_bytes([g.wram()[i], g.wram()[i + 1]])
}

fn step(g: &mut Game, n: usize) {
    for _ in 0..n {
        g.step(Pads::default());
    }
}

#[test]
fn boots_and_runs_a_frame() {
    let mut g = game();
    step(&mut g, 1);
    assert_eq!(g.frame_count(), 1);
    assert_eq!(g.frame().1, 256);
    assert_eq!(g.frame().0.len(), g.frame().1 * g.frame().2);
    let n = word(&g, LOOP_CTR);
    assert!(n > 100, "loop ran only {n} times");
    let s = word(&g, SUB_CTR);
    assert!(s == n || s == n + 1, "sub ran {s} times, loop {n}");
    step(&mut g, 1);
    assert!(word(&g, LOOP_CTR) > n);
}

fn mark_202(g: &mut Game) {
    g.wram_mut()[0x202] = 0xAA;
}
fn mark_212(g: &mut Game) {
    g.wram_mut()[0x212] = 0xBB;
}
fn count_204(g: &mut Game) {
    let n = word(g, 0x204).wrapping_add(1);
    g.wram_mut()[0x204..0x206].copy_from_slice(&n.to_le_bytes());
}

#[test]
fn rts_trap_replaces_sub() {
    let mut g = game();
    g.traps.register(SUB, "sub", TrapReturn::Rts, mark_202);
    step(&mut g, 3);
    assert_eq!(word(&g, SUB_CTR), 0);
    assert_eq!(g.wram()[0x202], 0xAA);
    assert!(
        word(&g, LOOP_CTR) > 300,
        "return not emulated: loop stalled"
    );
    assert!(word(&g, LONG_CTR) > 300);
    let last = g.traplog.last().unwrap();
    assert_eq!((last.addr, last.name), (SUB, "sub"));
}

#[test]
fn rtl_trap_replaces_sublong() {
    let mut g = game();
    g.traps
        .register(SUBLONG, "sublong", TrapReturn::Rtl, mark_212);
    step(&mut g, 3);
    assert_eq!(word(&g, LONG_CTR), 0);
    assert_eq!(g.wram()[0x212], 0xBB);
    assert!(word(&g, LOOP_CTR) > 300);
    assert!(word(&g, SUB_CTR) > 300);
}

#[test]
fn observe_fires_once_per_call() {
    let mut g = game();
    g.traps.register(SUB, "sub", TrapReturn::Observe, count_204);
    // Enough frames that the NMI lands on the trapped boundary at least once.
    for _ in 0..20 {
        step(&mut g, 1);
        assert_eq!(word(&g, 0x204), word(&g, SUB_CTR));
    }
    assert!(word(&g, SUB_CTR) > 1000);
}

#[test]
fn disable_and_untrap_run_the_rom() {
    let mut g = game();
    g.traps.register(SUB, "sub", TrapReturn::Rts, mark_202);
    g.traps.set_enabled(false);
    step(&mut g, 1);
    let a = word(&g, SUB_CTR);
    assert!(a > 100);
    assert_eq!(g.wram()[0x202], 0);

    g.traps.set_enabled(true);
    g.traps.set_untrapped(SUB, true);
    step(&mut g, 1);
    let b = word(&g, SUB_CTR);
    assert!(b > a);
    assert_eq!(g.wram()[0x202], 0);

    g.traps.set_untrapped(SUB, false);
    step(&mut g, 1);
    assert_eq!(word(&g, SUB_CTR), b);
    assert_eq!(g.wram()[0x202], 0xAA);
}

#[test]
fn call_isolated_costs_nothing() {
    let mut g = game();
    step(&mut g, 2);
    let regs = g.regs();
    let (sub, long, frames) = (word(&g, SUB_CTR), word(&g, LONG_CTR), g.frame_count());
    g.call_isolated_ret(SUB, TrapReturn::Rts);
    g.call_isolated(SUBLONG);
    assert_eq!(word(&g, SUB_CTR), sub + 1);
    assert_eq!(word(&g, LONG_CTR), long + 1);
    // RTS routine through the 3-byte form still lands on the sentinel.
    g.call_isolated(SUB);
    assert_eq!(word(&g, SUB_CTR), sub + 2);
    assert_eq!(g.frame_count(), frames);
    assert_eq!(g.regs(), regs);
    step(&mut g, 1); // still runs normally afterwards
    assert!(word(&g, SUB_CTR) > sub + 100);
}

/// A routine that pops its own return address and leaves through its
/// caller's frame (the sprite emitter's OAM-full exit), or never returns:
/// `try_call_isolated_ret` gives up with the registers put back, where
/// `call_isolated_ret` would panic.
#[test]
fn try_call_isolated_gives_up() {
    let mut g = game();
    step(&mut g, 1);
    let regs = g.regs();
    let loops = word(&g, LOOP_CTR);
    assert_eq!(g.try_call_isolated_ret(0x00_8060, TrapReturn::Rts), Err(IsolatedError::Unwound));
    assert_eq!(g.regs(), regs);
    assert_eq!(word(&g, LOOP_CTR), loops, "ran its caller's code");
    assert_eq!(g.try_call_isolated_ret(0x00_8070, TrapReturn::Rts), Err(IsolatedError::Budget));
    assert_eq!(g.regs(), regs);
    assert_eq!(g.try_call_isolated_ret(SUB, TrapReturn::Rts), Ok(()));
    step(&mut g, 1);
    assert!(word(&g, LOOP_CTR) > loops + 100);
}

#[test]
#[should_panic(expected = "call_isolated($008060) did not return")]
fn call_isolated_panics_when_unwound() {
    let mut g = game();
    step(&mut g, 1);
    g.call_isolated_ret(0x00_8060, TrapReturn::Rts);
}

#[test]
fn traps_fire_inside_call_isolated() {
    let mut g = game();
    step(&mut g, 1);
    g.traps.register(SUB, "sub", TrapReturn::Rts, mark_202);
    let sub = word(&g, SUB_CTR);
    g.call_isolated_ret(SUB, TrapReturn::Rts);
    assert_eq!(word(&g, SUB_CTR), sub);
    assert_eq!(g.wram()[0x202], 0xAA);
}

#[test]
fn rom_read_maps_lorom() {
    let g = game();
    assert_eq!(g.rom_read(0x00_8000), 0x78);
    assert_eq!(g.rom_read(0x80_8030), 0xEE);
    assert_eq!(g.rom_read(0x81_FFFC), 0x00); // 32 KiB image mirrors in every bank
    assert_eq!(g.rom_read(0x00_FFFD), 0x80);
}

#[test]
fn save_state_round_trip() {
    let mut a = game();
    step(&mut a, 3);
    let s = a.save_state();
    step(&mut a, 5);
    let after_8 = a.wram().to_vec();
    a.load_state(&s).unwrap();
    assert_eq!(a.frame_count(), 3);
    step(&mut a, 5);
    assert_eq!(a.frame_count(), 8);
    assert!(a.wram()[..] == after_8[..], "replay after load diverged");

    let mut b = game();
    step(&mut b, 8);
    assert!(b.wram()[..] == after_8[..], "fresh run diverged");
    assert!(a.load_state(&s[..10]).is_err());
}

#[test]
fn save_state_carries_p2_tile_page() {
    let mut g = game();
    step(&mut g, 2);
    g.p2_tile_page_mut()[5] = 0xBEEF;
    g.p2_tile_page_mut()[3 * 768 + 5] = 0xCAFE; // page 3 (HUD icon)
    g.set_p2_tile_slots(1 << 3);
    // P2's own sprite palette (the PPU's ninth) and its slots.
    let pal: [u16; 16] = std::array::from_fn(|i| 0x1000 + i as u16);
    g.set_p2_palette(pal);
    g.add_p2_palette_slots(1 << 9);
    let s = g.save_state();
    g.clear_p2_palette_slots();
    g.set_p2_palette([0; 16]);
    assert_ne!(g.save_state(), s, "P2's palette is not in the state");
    g.p2_tile_page_mut()[5] = 0;
    g.p2_tile_page_mut()[3 * 768 + 5] = 0;
    g.set_p2_tile_slots(0);
    g.load_state(&s).unwrap();
    assert_eq!(g.save_state(), s, "P2's palette did not come back");
    assert_eq!(g.p2_tile_page_mut()[5], 0xBEEF);
    assert_eq!(g.p2_tile_page_mut()[3 * 768 + 5], 0xCAFE);
    // Not live until the game's next OAM upload.
    assert_eq!(g.p2_tiles().1, 0);
    assert_eq!(g.p2_palette(), (0, [0; 16]));
}

#[test]
fn p2t6_four_page_state_still_loads() {
    let mut game = game();
    step(&mut game, 2);
    let current = game.save_state();
    let marker = current.windows(4).rposition(|w| w == b"P2T7").unwrap();
    let mut old = current[..marker].to_vec();
    old.extend_from_slice(b"P2T6");
    let mut page = Box::new([0u16; 768 * 4]);
    page[5] = 0xBEEF;
    let legacy = (page, 0u128, Box::new([0u16; 768 * 4]), 0u128, false,
        0u128, 0u128, 0u128, 0u128, 0u128, 0u128, [0u16; 16], [0u16; 16]);
    old.extend(bincode::encode_to_vec(legacy, bincode::config::standard()).unwrap());
    assert_eq!(Game::state_info(&old).unwrap().tiles.as_deref(), Some("P2T6"));
    game.load_state(&old).unwrap();
    assert_eq!(game.p2_tiles().0[5], 0xBEEF);
    assert_eq!(game.p2_tile_page_mut()[4 * 768 + 5], 0);
}

#[test]
fn save_state_carries_coop_state() {
    use mmx_core::coop::{CoopState, OFF_HP, OFF_POS_X};
    let mut g = game();
    step(&mut g, 2);
    g.coop.enabled = true;
    g.coop.p2[OFF_POS_X] = 0x34;
    g.coop.p2[OFF_HP] = 9;
    g.coop.p2_hidden = false;
    g.coop.p2_scratch[1] = 0x77;
    g.coop.down_frames = 12;
    g.coop.p2_fx = 0b101;
    g.coop.p2_warp = true;
    g.coop.saber.t = 5;
    g.coop.revive.spot = Some((0x100, 0x80));
    let s = g.save_state();
    let info = Game::state_info(&s).unwrap();
    assert_eq!((info.frame_count, info.tiles.as_deref()), (2, Some("P2T7")));
    assert_eq!(info.coop.as_ref(), Some(&g.coop));

    let mut fresh = game();
    fresh.coop.enabled = true; // the frontend's setting, kept across the load
    fresh.load_state(&s).unwrap();
    assert_eq!(fresh.coop, g.coop);
    assert_eq!(fresh.frame_count(), 2);

    // A state from before the `COOP` tail: a default co-op state (P2 hidden,
    // it re-anchors), settings kept.
    let old = &s[..s.windows(4).rposition(|w| w == b"COOP").unwrap()];
    assert!(Game::state_info(old).unwrap().coop.is_none());
    fresh.load_state(old).unwrap();
    assert_eq!(fresh.coop, CoopState { enabled: true, ..CoopState::default() });
    // A cut tail is an error, not a silent default.
    assert!(fresh.load_state(&s[..s.len() - 1]).is_err());
}

#[test]
fn rom_header_checks() {
    let mut headered = vec![0u8; 512];
    headered.extend(rom_with("MEGAMAN X", 0, 0));
    let g = Game::from_rom(headered).unwrap();
    let info = g.rom_info();
    assert_eq!(
        (info.title.as_str(), info.version, info.had_copier_header),
        ("MEGAMAN X", 0, true)
    );

    let v11 = Game::from_rom(rom_with("MEGAMAN X", 0, 1)).unwrap();
    assert_eq!(v11.rom_info().version, 1);
    assert!(!v11.rom_info().had_copier_header);

    assert!(matches!(
        Game::from_rom(rom_with("ROCKMAN X", 0, 0)),
        Err(RomError::NotMegaManX { title }) if title == "ROCKMAN X"
    ));
    assert!(matches!(
        Game::from_rom(rom_with("MEGAMAN X", 3, 0)),
        Err(RomError::Coprocessor(3))
    ));
    assert!(matches!(
        Game::from_rom(vec![0; 100]),
        Err(RomError::TooSmall(100))
    ));
}

/// Loop iterations over 10 frames (after one to settle) at overclock `o`.
fn loops_per_10(o: mmx_core::Overclock) -> u32 {
    let mut g = game();
    g.set_overclock(o);
    step(&mut g, 1);
    let a = word(&g, LOOP_CTR);
    step(&mut g, 10);
    u32::from(word(&g, LOOP_CTR).wrapping_sub(a))
}

#[test]
fn overclock_runs_more_instructions_per_frame() {
    use mmx_core::Overclock;
    let off = loops_per_10(Overclock::Off);
    let (x15, x2) = (loops_per_10(Overclock::X1_5), loops_per_10(Overclock::X2));
    let r = |n: u32| f64::from(n) / f64::from(off);
    eprintln!("loops per 10 frames: off {off}, 1.5x {x15}, 2x {x2}");
    // The loop is ROM and WRAM accesses (8 -> 5 / 4 master cycles) and fast
    // cycles (6 -> 4 / 3); the 40-cycle WRAM refresh per line stays.
    assert!((1.45..1.7).contains(&r(x15)), "1.5x: {off} -> {x15}");
    assert!((1.9..2.05).contains(&r(x2)), "2x: {off} -> {x2}");
    // Same frame rate: one step is still one frame.
    let mut g = game();
    g.set_overclock(Overclock::X2);
    step(&mut g, 3);
    assert_eq!(g.frame_count(), 3);
}
