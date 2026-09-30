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
