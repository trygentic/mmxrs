//! mmxrs desktop app: windowed (winit/pixels/cpal/gilrs) or `--headless`.
//! Usage and keys: `docs/native.md`.

mod app;
mod audio;
mod bone_sheets;
mod cli;
mod character_attacks;
mod demo_bots;
mod effects;
#[cfg(target_os = "macos")]
mod gc_pad;
mod headless;
mod keys;
mod movie;
mod movie_route;
#[cfg(test)]
mod movie_route_validation;
mod netplay;
mod video;
mod vs_select;
mod vs_bots;

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
    // X3 Zero is an explicit opt-in; an inherited environment variable must
    // not silently replace the X1 art selected in the launcher.
    let x3 = args.x3_rom.clone().map(|p| {
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
    let net = args.net.as_ref().map(|(host, room)| {
        netplay::connect(&rom, *host, room, &args.signal, args.ice.as_deref(), args.mode, args.net_delay(), args.prediction, x3.as_ref(), args.zero, args.timing, args.wide_gameplay, args.overclock(), args.net_record.as_deref())
            .unwrap_or_else(|e| {
                eprintln!("netplay: {e}");
                std::process::exit(2);
            })
    });
    // Extract selected boss sprite sets from the X1 ROM before moving its
    // bytes into the emulator. Reuse one set when several players choose the
    // same character; a failed extraction must be visible to the player.
    let art_rom = if rom.len() % 0x8000 == 512 { &rom[512..] } else { &rom[..] };
    let mut boss_sets = Vec::new();
    let art_roster: Vec<_> = if args.showcase_characters || args.vs_mode {
        mmx_core::Character::ALL.to_vec()
    } else {
        args.characters.to_vec()
    };
    for character in art_roster {
        if character == mmx_core::Character::X
            || (character == mmx_core::Character::Zero && x3.is_some())
            || matches!(character, mmx_core::Character::Bass | mmx_core::Character::ProtoMan)
            || boss_sets.iter().any(|(existing, _)| *existing == character)
        {
            continue;
        }
        let set = mmx_core::boss_data::extract_x1(art_rom, character).unwrap_or_else(|e| {
            eprintln!("{} sprites: {e}", character.label());
            std::process::exit(2);
        });
        boss_sets.push((character, set));
    }
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
    for (character, set) in boss_sets {
        game.set_boss_set(character, set);
    }
    for (index, character) in args.characters.into_iter().enumerate() {
        game.set_player_character(index, character);
    }
    let active_count = if args.net.is_some() { 2 } else { usize::from(args.players) };
    if args.characters[..active_count].iter().any(|character|
        matches!(character, mmx_core::Character::Bass | mmx_core::Character::ProtoMan))
        && args.bone_sheets.is_none()
    {
        eprintln!("Bass and Proto Man need a 60 FPS bone sheet pack (--bone-sheets DIR)");
        std::process::exit(2);
    }
    if let Some(path) = args.bone_sheets.as_deref() {
        let selected: Vec<_> = if args.showcase_characters || args.vs_mode {
            mmx_core::Character::ALL.to_vec()
        } else {
            args.characters[..active_count].to_vec()
        };
        bone_sheets::install(&mut game, std::path::Path::new(path), &selected)
            .unwrap_or_else(|e| { eprintln!("bone sheets: {e}"); std::process::exit(2); });

    }
    if (args.showcase_characters || args.vs_mode || args.characters[..active_count].contains(&mmx_core::Character::Vile))
        && !game.has_bone_sheet(mmx_core::Character::Vile) {
        let sprite = bone_sheets::vile_sprite_path(args.vile_sprite.as_deref(), std::path::Path::new(&rom_path));
        bone_sheets::install_static_vile(&mut game, &sprite).unwrap_or_else(|e| {
            eprintln!("Vile's on-foot sprite is required at {}: {e}", sprite.display());
            std::process::exit(2);
        });
    }
    game.coop.enabled = args.coop_local || args.players > 1 || net.is_some();
    game.set_local_players(usize::from(args.players.max(if args.coop_local { 2 } else { 1 })));
    if game.coop.enabled {
        mmx_core::coop::register_coop_traps(&mut game).unwrap_or_else(|e| {
            eprintln!("{rom_path}: {e}");
            std::process::exit(2);
        });
    }
    // X3 Zero's optional combat hooks need the X3 set. X1 Zero is provided
    // by the boss sprite set and uses the normal X player routines.
    game.set_p1_zero(args.characters[0] == mmx_core::Character::Zero && game.zero_set().is_some()).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        std::process::exit(2);
    });

    game.set_widescreen(args.widescreen.initial());
    if args.movie.is_some() { game.set_allow_opposing_directions(true); }
    game.set_wide_gameplay(args.wide_gameplay);
    game.set_overclock(args.overclock());
    if args.vs_bots {
        let arena = mmx_core::vs_stages::arena(args.vs_stage.as_deref().unwrap_or("highway"))
            .expect("CLI validated the arena");
        let ready = game.enable_vs(usize::from(args.players))
            .and_then(|_| {
                game.set_vs_arena(*arena);
                mmx_core::vs_stages::start_stage(&mut game, arena)
            });
        if let Err(error) = ready {
            eprintln!("VS bots: {error}");
            std::process::exit(2);
        }
        game.take_audio(); // Discard silent boot before live arena audio.
        eprintln!("VS bots: {} players, {}", args.players, arena.name);
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
    if let Err(e) = app::run(game, &args, net) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
