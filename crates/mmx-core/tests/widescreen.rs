//! Widescreen (docs/widescreen.md). Skips unless `MMX_ROM` is set.

use mmx_core::netgame::{net_checksum, session_game};
use mmx_core::widescreen::{agreement, camera, hud_slots, layout_entry};
use mmx_core::{Game, Pads, Timing, Widescreen};

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
/// netplay checksum are the same as without it over 300 frames walking right
/// (hw-a: enemies pass both edges, so the margin sprite pass runs).
#[test]
fn widescreen_is_display_only() {
    for st in ["highway-mid", "hw-a"] {
        widescreen_display_only(st);
    }
}

fn widescreen_display_only(st: &str) {
    let (Some(mut a), Some(mut b)) = (game(st), game(st)) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    b.set_widescreen(Widescreen::W21_9);
    for f in 0..300 {
        let pads = Pads { p1: if f % 60 < 50 { 0x0100 } else { 0x8100 }, p2: 0 };
        a.step(pads);
        b.step(pads);
        assert_eq!(net_checksum(&a), net_checksum(&b), "{st} frame {f}");
    }
    assert_eq!(a.wram()[..], b.wram()[..]);
    assert_eq!(a.oam(), b.oam());
    assert_eq!(a.cgram(), b.cgram());
    assert_eq!(a.vram()[..], b.vram()[..]);
    let (pa, wa, ha) = a.frame();
    let (pb, wb, hb) = b.frame();
    assert_eq!((wa, ha, wb, hb), (256, 224, 528, 224));
    // The centre of the wide frame is the normal frame, but for the HUD,
    // which moved to the wide edges.
    let hud = hud_boxes(&b);
    assert!(hud.len() >= 4, "{st}: HUD slots {hud:?}");
    for y in 0..224 {
        for x in (0..256).filter(|&x| !hud.iter().any(|&(hx, hy)| (hx..hx + 16).contains(&x) && (hy..hy + 17).contains(&y))) {
            assert_eq!(pa[y * 256 + x], pb[y * 528 + 136 + x], "{st}: pixel ({x},{y})");
        }
    }
    same_state_unwidened(&mut a, &mut b);
    b.set_widescreen(Widescreen::W21_9);
    // Survives a state load; off again gives the 256-px frame.
    let s = a.save_state();
    b.load_state(&s).unwrap();
    b.step(Pads::default());
    assert_eq!(b.frame().1, 528);
    b.set_widescreen(Widescreen::OFF);
    b.step(Pads::default());
    assert_eq!(b.frame().1, 256);
}

/// Margin sprites (docs/widescreen.md "Objects in the margins") in a netplay
/// session game, solo and co-op (P2 idle, left behind in the left margin):
/// hw-a walking right with 21:9 on vs off for 300 frames keeps WRAM, OAM,
/// the checksum and the save state identical; the pass draws in both
/// margins, only there, and never a piece the game put in OAM itself.
#[test]
fn margin_sprites_are_display_only() {
    let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
        return eprintln!("MMX_ROM not set; skipping");
    };
    let p = format!("{}/../../states/hw-a.state", env!("CARGO_MANIFEST_DIR"));
    let Ok(state) = std::fs::read(&p) else { return eprintln!("{p} missing; skipping") };
    for flags in [0, mmx_net::COOP_TWO_PLAYERS] {
        let mk = || {
            let mut g = session_game(&rom, flags, Timing::Auto).unwrap();
            g.load_state(&state).unwrap();
            g
        };
        let (mut a, mut b) = (mk(), mk());
        b.set_widescreen(Widescreen::W21_9);
        let (mut left, mut right) = (0, 0);
        for f in 0..300 {
            let pads = Pads { p1: 0x0100, p2: 0 };
            a.step(pads);
            b.step(pads);
            assert_eq!(net_checksum(&a), net_checksum(&b), "flags {flags} frame {f}");
            let oam = &b.wram()[0x700..0x920];
            let game_oam: Vec<(i16, u8, u8, u8)> = (0..128)
                .map(|k| {
                    let x9 = u16::from(oam[4 * k]) | u16::from(oam[0x200 + k / 4] >> (2 * (k % 4)) & 1) << 8;
                    (if x9 >= 256 { x9 as i16 - 512 } else { x9 as i16 }, oam[4 * k + 1], oam[4 * k + 2], oam[4 * k + 3])
                })
                .collect();
            for s in &b.margin.last {
                let w = if s.large { 16 } else { 8 };
                assert!((s.x < 0 && s.x + w > -136) || (s.x + w > 256 && s.x < 392), "frame {f}: {s:?} not in a margin");
                assert!(!game_oam.contains(&(s.x, s.y, s.tile, s.attr)), "frame {f}: {s:?} is in OAM too");
                left += usize::from(s.x < 0);
                right += usize::from(s.x >= 0);
            }
        }
        assert!(left > 0 && right > 0, "flags {flags}: margin sprites left {left} right {right}");
        assert_eq!(a.wram()[..], b.wram()[..]);
        assert_eq!(a.oam(), b.oam());
        same_state_unwidened(&mut a, &mut b);
        eprintln!("flags {flags}: {left} sprite-frames in the left margin, {right} in the right");
    }
}

/// The save state holds the PPU's last picture and its sprite evaluation,
/// which the HUD shift changes (the HUD is not in the centre; STAT77's
/// overflow flags, which MMX never reads): one frame with `b`'s widescreen
/// off and the states are equal again.
fn same_state_unwidened(a: &mut Game, b: &mut Game) {
    b.set_widescreen(Widescreen::OFF);
    a.step(Pads::default());
    b.step(Pads::default());
    assert_eq!(a.save_state(), b.save_state());
}

/// Visible shifted HUD sprites (16x16) at their OAM position (`(x, y)`).
fn hud_boxes(g: &Game) -> Vec<(usize, usize)> {
    let (l, r) = g.hud_shift();
    let oam = g.oam();
    (0..128)
        .filter(|&k| (l | r) >> k & 1 != 0 && oam[4 * k + 1] < 224 && oam[0x200 + k / 4] >> (2 * (k % 4)) & 1 == 0)
        .map(|k| (usize::from(oam[4 * k]), usize::from(oam[4 * k + 1])))
        .collect()
}

/// The HUD shift (docs/widescreen.md "HUD"): without a margin the PPU
/// ignores it (hw-spiky with the weapon bar, hw-bee2 with a boss bar: every
/// frame pixel-identical with the masks set vs not); at 21:9 X's icon is
/// drawn at X 8 of the wide frame, 8 px in from its new left edge.
#[test]
fn hud_shift_is_display_only() {
    for (st, pokes) in [("hw-spiky", &[(0x0BDB, 2), (0x1F12, 0)][..]), ("hw-bee2", &[(0x1F0E, 0x68), (0x1F0F, 0x0E)][..])] {
        let (Some(mut a), Some(mut b), Some(mut c)) = (game(st), game(st), game(st)) else {
            return eprintln!("MMX_ROM not set; skipping");
        };
        c.set_widescreen(Widescreen::W21_9);
        let mut moved = false;
        for f in 0..90 {
            for g in [&mut a, &mut b, &mut c] {
                if f < 30 {
                    for &(addr, v) in pokes {
                        g.wram_mut()[addr] = v;
                    }
                }
            }
            let (l, r) = hud_slots(b.wram(), 0);
            b.set_hud_shift(l, r);
            for g in [&mut a, &mut b, &mut c] {
                g.step(Pads::default());
            }
            assert_eq!(a.frame(), b.frame(), "{st} frame {f}");
            assert_eq!(net_checksum(&a), net_checksum(&c), "{st} frame {f}");
            // X's icon (slot 0, 16x16 at OAM X 8, Y 80, shown a line lower).
            let (pa, pc) = (a.frame().0, c.frame().0);
            let icon = |p: &[u32], w: usize, x0: usize| -> Vec<u32> {
                (81..97).flat_map(|y| (x0..x0 + 16).map(move |x| p[y * w + x])).collect()
            };
            if c.hud_shift().0 & 1 != 0 && c.oam()[1] == 80 {
                let same = |p: &[u32], q: &[u32]| p.iter().zip(q).filter(|(x, y)| x == y).count();
                let (at_edge, at_old) = (same(&icon(pa, 256, 8), &icon(pc, 528, 8)), same(&icon(pa, 256, 8), &icon(pc, 528, 136 + 8)));
                assert!(at_edge >= 200 && at_old < 128, "{st} frame {f}: icon pixels at the wide edge {at_edge}, at its place {at_old}");
                moved = true;
            }
        }
        assert!(moved, "{st}: HUD never shifted");
        let right = c.hud_shift().1;
        assert_eq!(right != 0, st == "hw-bee2", "{st}: boss bar slots {right:x}");
    }
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
