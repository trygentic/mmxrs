//! Widescreen (docs/widescreen.md). Skips unless `MMX_ROM` is set.

use mmx_core::netgame::net_checksum;
use mmx_core::widescreen::{agreement, camera, layout_entry};
use mmx_core::{Game, Pads, Widescreen};

fn game(state: &str) -> Option<Game> {
    let rom = std::fs::read(std::env::var_os("MMX_ROM")?).ok()?;
    let mut g = Game::from_rom(rom).expect("load ROM");
    let p = format!("{}/../../states/{state}.state", env!("CARGO_MANIFEST_DIR"));
    g.load_state(&std::fs::read(p).ok()?).expect("load state");
    Some(g)
}

/// The layout decoder gives VRAM's own tilemap entry for every on-screen
/// cell of BG1 and BG2.
#[test]
fn decoder_matches_vram_on_screen() {
    for st in ["highway-start", "highway-mid", "octopus-start"] {
        let Some(mut g) = game(st) else { return eprintln!("MMX_ROM not set; skipping") };
        g.step(Pads::default());
        for bg in 0..2 {
            assert_eq!(agreement(&g, bg), 100, "{st} BG{}", bg + 1);
        }
        // Spot check against the PPU directly.
        let (cx, cy) = camera(&g, 0);
        assert_eq!(layout_entry(&g, 0, cx + 128, cy + 160), Some(g.bg_map_entry(0, (cx + 128) as u16, (cy + 160) as u16)));
    }
}

/// Widescreen changes the picture size only: WRAM, OAM, CGRAM, VRAM and the
/// netplay checksum are the same as without it over 300 frames walking right.
#[test]
fn widescreen_is_display_only() {
    let (Some(mut a), Some(mut b)) = (game("highway-mid"), game("highway-mid")) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    b.set_widescreen(Widescreen::W21_9);
    for f in 0..300 {
        let pads = Pads { p1: if f % 60 < 50 { 0x0100 } else { 0x8100 }, p2: 0 };
        a.step(pads);
        b.step(pads);
        assert_eq!(net_checksum(&a), net_checksum(&b), "frame {f}");
    }
    assert_eq!(a.wram()[..], b.wram()[..]);
    assert_eq!(a.oam(), b.oam());
    assert_eq!(a.cgram(), b.cgram());
    assert_eq!(a.vram()[..], b.vram()[..]);
    assert_eq!(a.save_state(), b.save_state());
    let (pa, wa, ha) = a.frame();
    let (pb, wb, hb) = b.frame();
    assert_eq!((wa, ha, wb, hb), (256, 224, 528, 224));
    // The centre of the wide frame is the normal frame.
    for y in 0..224 {
        assert_eq!(pa[y * 256..(y + 1) * 256], pb[y * 528 + 136..y * 528 + 392], "line {y}");
    }
    // Survives a state load; off again gives the 256-px frame.
    let s = a.save_state();
    b.load_state(&s).unwrap();
    b.step(Pads::default());
    assert_eq!(b.frame().1, 528);
    b.set_widescreen(Widescreen::OFF);
    b.step(Pads::default());
    assert_eq!(b.frame().1, 256);
}

#[test]
fn presets_parse() {
    assert_eq!(Widescreen::parse("16:9"), Some(Widescreen::W16_9));
    assert_eq!(Widescreen::parse("off"), Some(Widescreen::OFF));
    assert_eq!(Widescreen::parse("5").map(Widescreen::width), Some(336));
    assert_eq!(Widescreen::parse("99"), None);
    assert_eq!(Widescreen::W21_9.width(), 528);
    assert_eq!(Widescreen::OFF.next(), Widescreen::W16_10);
    assert_eq!(Widescreen::W21_9.next(), Widescreen::OFF);
}
