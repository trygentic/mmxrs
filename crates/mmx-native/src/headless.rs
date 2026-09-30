//! `--headless`: step N frames with held pads, dump PNGs, print a WRAM hash.
//! Never opens a window or an audio device.

use crate::cli::Args;
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
    if args.movie.is_some() {
        // ponytail: TODO wire mmx-verify's bk2 parser here once it exists.
        return Err("--movie is not implemented yet (needs mmx-verify's bk2 parser)".into());
    }
    let pads = Pads {
        p1: args.hold,
        p2: args.p2_hold,
    };
    for f in 1..=args.frames {
        game.step(pads);
        let _ = game.take_audio(); // drain so it doesn't pile up
        for (_, path) in args.dump_at.iter().filter(|(at, _)| *at == f) {
            let (px, w, h) = game.frame();
            write_png(path, px, w, h)?;
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
