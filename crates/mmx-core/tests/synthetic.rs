//! `Game` against a hand-assembled 32 KiB LoROM (no Mega Man X ROM needed).
//!
//! ```text
//! $00:8000  SEI / CLC / XCE / REP #$30 / LDX #$1FFF / TXS
//!           SEP #$20 / LDA #$80 / STA $4200 (NMI on) / REP #$20
//! $00:8012  loop: JSR sub / JSL sublong / INC $0100 / BRA loop
//! $00:8030  sub:     INC $0200 / RTS
//! $00:8040  sublong: INC $0210 / RTL
//! $00:8050  nmi:     RTI
//! ```
//! Counters are 16-bit words (A is 16-bit), so they sit two bytes apart.

use mmx_core::{Game, Pads, RomError, TrapReturn};

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
