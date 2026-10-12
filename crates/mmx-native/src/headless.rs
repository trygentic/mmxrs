//! `--headless`: step N frames with held pads, dump PNGs, print a WRAM hash.
//! Never opens a window or an audio device.

use crate::cli::Args;
use crate::demo_bots::DemoBots;
use mmx_core::{Game, Pads};
use sha2::{Digest, Sha256};

/// 0x00RRGGBB -> packed RGB8.
pub fn frame_rgb(px: &[u32]) -> Vec<u8> {
    px.iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect()
}

pub fn write_png(path: &str, px: &[u32], w: usize, h: usize) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(&frame_rgb(&px[..w * h]))
        .map_err(|e| e.to_string())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Returns the WRAM sha256 hex after `args.frames` frames.
pub fn run(game: &mut Game, args: &Args) -> Result<String, String> {
    let mut movie = if args.movie_route { None } else { crate::movie::Playback::load(args)? };
    let mut route = if args.movie_route { Some(crate::movie_route::RoutePlayback::load(args)?) } else { None };
    let mut bots = args.demo_bots.then(DemoBots::default);
    let mut vs_bots = args.vs_bots.then(crate::vs_bots::VsBots::default);
    let mut vs_round_hold = 0_u16;
    let trace = std::env::var_os("MMX_BOT_TRACE").is_some();
    let state = |game: &Game| {
        use mmx_core::ram::{self, wram_index as ix};
        let w = game.wram();
        let word = |at| u16::from_le_bytes([w[ix(at)], w[ix(at) + 1]]);
        [
            word(ram::X_POS_X),
            word(ram::X_POS_Y),
            u16::from(w[ix(ram::X_HP)] & 0x7f),
            u16::from(w[ix(ram::X_ACTION)]),
            word(ram::X_VEL_Y),
            word(ram::CAMERA_Y),
            u16::from(w[ix(ram::MODE_D2)]),
            u16::from(w[ix(ram::STAGE_PHASE)]),
        ]
    };
    for f in 1..=args.frames {
        if args.demo_survival { game.maintain_demo_survival(); }
        if let Some(bots) = bots.as_ref() {
            bots.maintain_party(game);
        }
        let inputs = if let Some(bots) = vs_bots.as_mut() {
            bots.pads(game)
        } else if let Some(route) = route.as_mut() {
            route.next(game)
        } else if let Some(movie) = movie.as_mut() {
            movie.next(game)
        } else { match bots.as_mut() {
            Some(bots) => bots.pads(game),
            None => [args.hold, args.p2_hold, args.p3_hold, args.p4_hold],
        }};
        if args.showcase_characters {
            if let Some((group, characters)) = bots.as_ref().and_then(DemoBots::showcase_selection) {
                for (slot, character) in characters.into_iter().enumerate() {
                    game.set_player_character(slot, character);
                }
                eprintln!("showcase group {}: {}", group + 1,
                    characters.map(|character| character.label()).join(", "));
            }
        }
        let pads = Pads { p1: inputs[0], p2: inputs[1] };
        game.set_extra_pads([inputs[2], inputs[3]]);
        let before = trace.then(|| state(game));
        if vs_bots.is_some() && game.vs_status().is_some_and(|status| status.round_over) {
            vs_round_hold += 1;
            if vs_round_hold >= 90 {
                game.restart_vs_round();
                vs_round_hold = 0;
            }
        } else {
            vs_round_hold = 0;
            game.step(pads);
        }
        if let Some(before) = before {
            let after = state(game);
            if after[2] < before[2]
                || after[6..] != before[6..]
                || (after[3] == 0x0c && before[3] != 0x0c)
            {
                eprintln!("bot trace frame {f}: [x,y,hp,action,vy,camera_y,mode,phase] {before:?} -> {after:?}; pad {:04x}", inputs[0]);
            }
        }
        let _ = game.take_audio(); // drain so it doesn't pile up
        for (_, path) in args.dump_at.iter().filter(|(at, _)| *at == f) {
            let (px, w, h) = game.frame();
            {
                let mut rgba: Vec<u8> = px.iter().take(w * h).flat_map(|p| [
                    (p >> 16) as u8, (p >> 8) as u8, *p as u8, 255,
                ]).collect();
                crate::effects::apply(&mut rgba, w, h, game, args.dynamic_lighting, args.charge_effects);
                game.draw_ui_rgba(&mut rgba, w, h);
                if let Some(status) = game.vs_status().filter(|status| status.round_over) {
                    crate::vs_select::draw_result_scaled(&mut rgba, w, h, status, game.frame_scale());
                }
                let effected: Vec<u32> = rgba.chunks_exact(4).map(|p| {
                    (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2])
                }).collect();
                write_png(path, &effected, w, h)?;
            }
            eprintln!("frame {f}: wrote {path} ({w}x{h})");
        }
    }
    Ok(hex(&Sha256::digest(game.wram())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_unpack() {
        assert_eq!(
            frame_rgb(&[0x00123456, 0x00FF0080]),
            vec![0x12, 0x34, 0x56, 0xFF, 0x00, 0x80]
        );
    }

    #[test]
    fn png_roundtrip() {
        let path = std::env::temp_dir().join("mmxrs-native-test.png");
        let path = path.to_str().unwrap();
        write_png(
            path,
            &[0x00FF0000, 0x0000FF00, 0x000000FF, 0x00FFFFFF],
            2,
            2,
        )
        .unwrap();
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        r.next_frame(&mut buf).unwrap();
        assert_eq!(
            &buf[..12],
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]
        );
        let _ = std::fs::remove_file(path);
    }

    /// ROM-gated smoke test: skips (passes) without `MMX_ROM`.
    #[test]
    fn headless_smoke_with_rom() {
        let Some(rom) = std::env::var_os("MMX_ROM") else {
            eprintln!("MMX_ROM not set, skipping");
            return;
        };
        let mut game = Game::from_rom(std::fs::read(rom).unwrap()).unwrap();
        let args = Args {
            headless: true,
            frames: 60,
            ..Args::default()
        };
        let sha = run(&mut game, &args).unwrap();
        assert_eq!(sha.len(), 64);
        assert!(game.frame_count() >= 60);
        let (px, w, h) = game.frame();
        assert!(px.len() >= w * h && w >= 256 && h >= 224);
    }
}
