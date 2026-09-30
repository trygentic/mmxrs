//! mmxrs desktop app: windowed (winit/pixels/cpal/gilrs) or `--headless`.
//! Usage and keys: `docs/native.md`.

mod app;
mod audio;
mod cli;
mod headless;
mod keys;

use mmx_core::Game;

fn main() {
    let args = match cli::parse(std::env::args().skip(1), std::env::var("MMX_ROM").ok()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let Some(rom_path) = args.rom.clone() else {
        eprintln!("no ROM: pass --rom PATH or set MMX_ROM\n{}", cli::USAGE);
        std::process::exit(2);
    };
    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        std::process::exit(1);
    });
    let mut game = Game::from_rom(rom).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        std::process::exit(1);
    });
    game.coop.enabled = args.coop_local;
    if args.coop_local {
        mmx_core::coop::register_coop_traps(&mut game);
    }

    if args.headless {
        match headless::run(&mut game, &args) {
            Ok(sha) => println!("frames {} wram_sha256 {sha}", game.frame_count()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(e) = app::run(game, &args) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
