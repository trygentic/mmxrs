//! Co-op sprite palettes on the real ROM (docs/coop.md "Palettes"). Skip
//! unless `MMX_ROM` is set and `states/highway-start.state` (made on the PAL
//! dump) loads; the X3 half also needs `MMX_X3_ROM`.

use mmx_core::{Game, Pads, BTN_Y};

/// Sprite palette of X's charged shots: CGRAM 176..191 (palette id `$20`).
const SHOT_PAL: u8 = 3;
const HUD_PAL: u8 = 2;

fn game(x3: bool, coop: bool) -> Option<Game> {
    let rom = std::fs::read(std::env::var_os("MMX_ROM")?).ok()?;
    let state = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/highway-start.state")).ok()?;
    let mut g = Game::from_rom(rom).ok()?;
    if x3 {
        g.set_x3_rom(&std::fs::read(std::env::var_os("MMX_X3_ROM")?).ok()?).unwrap();
    }
    g.load_state(&state).ok()?;
    if coop {
        g.coop.enabled = true;
        mmx_core::coop::register_coop_traps(&mut g).unwrap();
    }
    Some(g)
}

/// Visible OAM entries `(slot, x, y, tile, attr)`.
fn sprites(g: &Game) -> Vec<(usize, u16, u8, u16, u8)> {
    let oam = g.oam();
    (0..128)
        .filter_map(|i| {
            let [x, y, t, a] = [oam[4 * i], oam[4 * i + 1], oam[4 * i + 2], oam[4 * i + 3]];
            let hi = oam[512 + i / 4] >> (2 * (i % 4)) & 3;
            (!(0xE0..0xF0).contains(&y)).then_some((i, u16::from(x) | u16::from(hi & 1) << 8, y, u16::from(t) | u16::from(a & 1) << 8, a))
        })
        .collect()
}

fn pal(a: u8) -> u8 {
    a >> 1 & 7
}

fn cgram_pal(g: &Game, p: u8) -> [u16; 16] {
    let c = g.cgram();
    std::array::from_fn(|i| c[128 + 16 * usize::from(p) + i])
}

/// X charges (Y held 45 frames: the first level, then 120: full) and fires
/// in co-op: on every frame the CGRAM palette the shot uses and the shot's
/// own OAM entries (palette 3, not drawn with P2's palette) are the solo
/// run's. Before the fix P2's palette was written over CGRAM palette 3.
#[test]
fn coop_x_charged_shot_matches_solo() {
    for x3 in [false, true] {
        let (Some(mut solo), Some(mut coop)) = (game(x3, false), game(x3, true)) else {
            eprintln!("MMX_ROM{} / highway-start.state missing; skipping", if x3 { " / MMX_X3_ROM" } else { "" });
            continue;
        };
        let mut shot_frames = 0;
        for f in 0..320 {
            let held = (10..55).contains(&f) || (100..220).contains(&f);
            let pads = Pads { p1: if held { BTN_Y } else { 0 }, p2: 0 };
            solo.step(pads);
            coop.step(pads);
            assert_eq!(cgram_pal(&coop, SHOT_PAL), cgram_pal(&solo, SHOT_PAL), "x3 {x3} frame {f}: CGRAM palette 3");
            let p2_pal = coop.p2_palette().0;
            let shot = |g: &Game, mask: u128| {
                let mut v: Vec<_> = sprites(g).into_iter().filter(|s| pal(s.4) == SHOT_PAL && mask >> s.0 & 1 == 0).map(|s| (s.1, s.2, s.3, s.4)).collect();
                v.sort_unstable();
                v
            };
            let want = shot(&solo, 0);
            assert_eq!(shot(&coop, p2_pal), want, "x3 {x3} frame {f}: the shot's sprites");
            shot_frames += usize::from(!want.is_empty());
        }
        assert!(shot_frames > 20, "x3 {x3}: no charged shot on screen ({shot_frames} frames)");
    }
}

/// P2's HP bar: the pieces (tiles `$80`/`$82`/`$84`) are drawn exactly like
/// X's (palette 2 from CGRAM, VRAM tiles), only the icon (and the down
/// marker) uses P2's own palette.
#[test]
fn coop_p2_bar_matches_x() {
    for x3 in [false, true] {
        let Some(mut g) = game(x3, true) else {
            eprintln!("MMX_ROM{} / highway-start.state missing; skipping", if x3 { " / MMX_X3_ROM" } else { "" });
            continue;
        };
        for _ in 0..30 {
            g.step(Pads::default());
        }
        let (p2_pal, colours) = g.p2_palette();
        let tile_slots = g.p2_tiles().1;
        let s = sprites(&g);
        let bar = |slot: &(usize, u16, u8, u16, u8)| slot.1 == 8 && (0x80..=0x86).contains(&slot.3);
        let x_pieces: Vec<_> = s.iter().filter(|e| bar(e) && e.0 < 7 && e.3 != 0x86).collect();
        let p2_pieces: Vec<_> = s.iter().filter(|e| bar(e) && e.0 >= 16 && e.3 != 0x86).collect();
        let p2_icon: Vec<_> = s.iter().filter(|e| bar(e) && e.0 >= 16 && e.3 == 0x86).collect();
        assert!(!x_pieces.is_empty() && !p2_pieces.is_empty() && p2_icon.len() == 1, "x3 {x3}: bars {s:x?}");
        for e in &p2_pieces {
            assert_eq!(e.4 & 0x3F, x_pieces[0].4 & 0x3F, "x3 {x3}: P2 piece attr {e:x?} (X's {:x?})", x_pieces[0]);
            assert_eq!(pal(e.4), HUD_PAL);
            assert_eq!(p2_pal >> e.0 & 1, 0, "x3 {x3}: P2 piece {e:x?} in P2's palette");
            assert_eq!(tile_slots >> e.0 & 1, 0, "x3 {x3}: P2 piece {e:x?} from a shadow page");
        }
        let mut xt: Vec<_> = x_pieces.iter().map(|e| e.3).collect();
        let mut pt: Vec<_> = p2_pieces.iter().map(|e| e.3).collect();
        xt.sort_unstable();
        pt.sort_unstable();
        assert_eq!(pt, xt, "x3 {x3}: both at full HP, same pieces");
        assert_eq!(p2_pal >> p2_icon[0].0 & 1, 1, "x3 {x3}: the Z icon is in P2's palette");
        assert_ne!(colours, [0; 16]);
        // The HUD palette itself is untouched.
        assert_eq!(cgram_pal(&g, HUD_PAL)[1], 0x7FFF);
    }
}
