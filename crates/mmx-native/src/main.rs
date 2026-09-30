//! mmxrs desktop app: windowed (winit/pixels/cpal/gilrs) or `--headless`.
//! Usage and keys: `docs/native.md`.

mod app;
mod audio;
mod cli;
#[cfg(target_os = "macos")]
mod gc_pad;
mod headless;
mod keys;
mod netplay;

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
    // `--x3-rom` / `$MMX_X3_ROM`: P2 drawn as X3 Zero, with the saber.
    let x3 = args.x3_rom.clone().or_else(|| std::env::var("MMX_X3_ROM").ok().filter(|s| !s.is_empty())).map(|p| {
        let bytes = std::fs::read(&p).unwrap_or_else(|e| {
            eprintln!("{p}: {e}");
            std::process::exit(1);
        });
        let bytes = if bytes.len() % 0x8000 == 512 { bytes[512..].to_vec() } else { bytes };
        mmx_core::zero_data::extract_x3(&bytes).unwrap_or_else(|e| {
            eprintln!("{p}: {e}");
            std::process::exit(1);
        })
    });
    if args.zero && x3.is_none() {
        eprintln!("--zero needs a Mega Man X3 (USA) ROM: pass --x3-rom PATH or set MMX_X3_ROM");
        std::process::exit(2);
    }
    let net = args.net.as_ref().map(|(host, room)| {
        netplay::connect(&rom, *host, room, &args.signal, args.ice.as_deref(), args.mode, args.net_delay(), args.prediction, x3.as_ref(), args.zero, args.timing, args.net_record.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("netplay: {e}");
                std::process::exit(2);
            })
    });
    let mut game = Game::from_rom_timing(rom, args.timing).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        std::process::exit(1);
    });
    // Online sessions rebuild the game with co-op on at `Running`; the local
    // game played while connecting gets the same traps.
    // Not the PAL dump's layout: say where each co-op hook went (or didn't).
    if !game.hooks().all_as_pal() {
        for l in game.hook_report() {
            eprintln!("hooks: {l}");
        }
    }
    game.set_zero_set(x3);
    game.coop.enabled = args.coop_local || net.is_some();
    if game.coop.enabled {
        mmx_core::coop::register_coop_traps(&mut game).unwrap_or_else(|e| {
            eprintln!("{rom_path}: {e}");
            std::process::exit(2);
        });
    }
    // `--zero`: P1 plays as Zero (single player, local co-op or online).
    game.set_p1_zero(args.zero).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        std::process::exit(2);
    });

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
    if let Err(e) = app::run(game, &args, net) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
