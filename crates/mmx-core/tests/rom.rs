//! Real-ROM smoke test. Skips unless `MMX_ROM` points at a Mega Man X (USA) image.

use mmx_core::{Game, Pads};

#[test]
fn boots_real_rom() {
    let Ok(path) = std::env::var("MMX_ROM") else {
        eprintln!("MMX_ROM not set; skipping");
        return;
    };
    let mut g = Game::from_rom(std::fs::read(&path).expect("read MMX_ROM")).expect("load ROM");
    let info = g.rom_info().clone();
    let sha: String = info.sha256.iter().map(|b| format!("{b:02x}")).collect();
    println!(
        "{path}: title {:?} version {} sha256 {sha} crc32 {:08x}",
        info.title, info.version, info.crc32
    );
    for _ in 0..600 {
        g.step(Pads::default());
    }
    let (px, w, h) = g.frame();
    assert_eq!(px.len(), w * h);
    assert!(px.iter().any(|&p| p != 0), "frame 600 is all black");
}

/// F5 in co-op then F7 in a fresh game: P2 comes back where it was (the
/// `COOP` tail), not re-anchored beside P1, and both games run on identically.
#[test]
fn coop_save_state_restores_p2() {
    use mmx_core::coop::{OFF_HP, OFF_POS_X, OFF_POS_Y};
    use mmx_core::netgame::session_game;
    use mmx_core::{Timing, BTN_LEFT, BTN_RIGHT};
    let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
    let Ok(state) = std::fs::read(st) else { return eprintln!("{st} missing; skipping") };
    let fresh = || session_game(&rom, mmx_net::COOP_TWO_PLAYERS, Timing::Auto).unwrap();
    let mut a = fresh();
    a.load_state(&state).unwrap();
    // P1 walks right, then P2 walks left: the players end up apart.
    for f in 0..100 {
        a.step(Pads { p1: if f < 60 { BTN_RIGHT } else { 0 }, p2: BTN_LEFT * u16::from(f > 70) });
    }
    assert!(!a.coop.p2_hidden, "P2 not in play");
    let s = a.save_state();
    assert!(Game::state_info(&s).unwrap().coop.is_some());
    let mut b = fresh();
    b.load_state(&s).unwrap();
    assert_eq!(b.coop, a.coop);
    // The same state without the tail: P2 re-anchors on P1.
    let mut old = fresh();
    old.load_state(&s[..s.windows(4).rposition(|w| w == b"COOP").unwrap()]).unwrap();
    let pos = |g: &Game| {
        let p = &g.coop.p2;
        let w = |o: usize| u16::from_le_bytes([p[o], p[o + 1]]);
        (w(OFF_POS_X), w(OFF_POS_Y), p[OFF_HP] & 0x7F)
    };
    for _ in 0..60 {
        let pads = Pads { p1: 0, p2: BTN_LEFT };
        a.step(pads);
        b.step(pads);
        old.step(pads);
    }
    assert_eq!(b.coop.p2, a.coop.p2, "P2's block diverged after the load");
    assert_eq!(pos(&b), pos(&a));
    assert!(b.wram()[..] == a.wram()[..], "WRAM diverged after the load");
    assert_ne!(pos(&old).0, pos(&a).0, "a re-anchored P2 should not land where P2 walked to");
    println!("P2 after load {:?}, re-anchored {:?}", pos(&b), pos(&old));
}
