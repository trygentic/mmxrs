use mmx_core::{ram, vs_stages, Game, Pads};

#[test]
fn every_vs_arena_loads_rom_gameplay() {
    let Some(path) = std::env::var_os("MMX_ROM") else { return };
    let rom = std::fs::read(path).expect("MMX_ROM");
    for arena in vs_stages::ARENAS {
        let mut game = Game::from_rom(rom.clone()).expect("X1 ROM");
        game.enable_vs(4).expect("VS hooks");
        vs_stages::start_stage(&mut game, arena).unwrap_or_else(|e| panic!("{}: {e}", arena.id));
        game.set_vs_arena(*arena);
        let w = game.wram();
        assert_eq!(w[ram::wram_index(ram::LEVEL_ID)], arena.stage, "{}", arena.id);
        assert_eq!(w[ram::wram_index(ram::MODE_D2)], ram::D2_IN_STAGE, "{}", arena.id);
        assert_eq!(w[ram::wram_index(ram::STAGE_PHASE)], ram::PHASE_GAMEPLAY, "{}", arena.id);
        for _ in 0..8 { game.step(Pads::default()); }
        let status = game.vs_status().expect("VS remains active");
        assert!(!status.round_over, "{} ended before the players fought", arena.id);
    }
}
