//! Local four-player control smoke test. Uses the real ROM when MMX_ROM is set.

use mmx_core::coop::OFF_POS_X;
use mmx_core::{Character, Game, Pads, BTN_LEFT, BTN_RIGHT, BTN_START};

fn x(object: &[u8]) -> u16 {
    u16::from_le_bytes([object[OFF_POS_X], object[OFF_POS_X + 1]])
}

#[test]
fn four_local_players_keep_separate_movement() {
    let Some(rom) = std::env::var_os("MMX_ROM").and_then(|path| std::fs::read(path).ok()) else {
        return eprintln!("MMX_ROM not set; skipping local four-player smoke test");
    };
    let art_rom = if rom.len() % 0x8000 == 512 { &rom[512..] } else { &rom[..] };
    let sigma = mmx_core::boss_data::extract_x1(art_rom, Character::Sigma).unwrap();
    let vile = mmx_core::boss_data::extract_x1(art_rom, Character::Vile).unwrap();
    let x1_zero = mmx_core::boss_data::extract_x1(art_rom, Character::Zero).unwrap();
    let mut game = Game::from_rom(rom).unwrap();
    assert_eq!(game.player_character(0), Some(Character::X));
    assert_eq!(game.player_character(1), Some(Character::Zero));
    game.coop.enabled = true;
    game.set_local_players(4);
    mmx_core::coop::register_coop_traps(&mut game).unwrap();
    game.set_boss_set(Character::Sigma, sigma);
    game.set_boss_set(Character::Vile, vile);
    game.set_boss_set(Character::Zero, x1_zero);
    game.set_player_character(2, Character::Sigma);
    game.set_player_character(3, Character::Vile);

    // Start from the title screen and wait for X to land in the Highway.
    let mut spawned = false;
    for frame in 0..2400 {
        game.step(Pads { p1: if frame % 90 == 0 { BTN_START } else { 0 }, p2: 0 });
        if !game.coop.p2_hidden && game.visible_extra_player_object(0).is_some()
            && game.visible_extra_player_object(1).is_some()
        {
            spawned = true;
            break;
        }
    }
    assert!(spawned, "four players never reached playable stage state: D2={:02x} D3={:02x} P2={} P3={} P4={}",
        game.wram()[mmx_core::ram::wram_index(mmx_core::ram::MODE_D2)],
        game.wram()[mmx_core::ram::wram_index(mmx_core::ram::STAGE_PHASE)],
        game.coop.p2_hidden,
        game.visible_extra_player_object(0).is_some(),
        game.visible_extra_player_object(1).is_some());
    for _ in 0..3 { game.step(Pads::default()); }
    let page = game.p2_tile_page_mut();
    let words = mmx_core::p2tiles::SHADOW_WORDS;
    assert!(page[4 * words..5 * words].iter().any(|&v| v != 0), "Sigma art page is empty");
    assert!(page[5 * words..6 * words].iter().any(|&v| v != 0), "Vile art page is empty");

    let before = [
        x(&game.coop.p2),
        x(game.visible_extra_player_object(0).unwrap()),
        x(game.visible_extra_player_object(1).unwrap()),
    ];
    for _ in 0..20 {
        game.set_extra_pads([BTN_LEFT, 0]);
        game.step(Pads { p1: 0, p2: BTN_RIGHT });
    }
    let after = [
        x(&game.coop.p2),
        x(game.visible_extra_player_object(0).unwrap()),
        x(game.visible_extra_player_object(1).unwrap()),
    ];
    assert!(after[0] > before[0], "P2 did not move right: {before:?} -> {after:?}");
    assert!(after[1] < before[1], "P3 did not move left: {before:?} -> {after:?}");
    assert_eq!(after[2], before[2], "P4 moved without input: {before:?} -> {after:?}");

    let saved = game.save_state();
    game.set_extra_pads([0, BTN_RIGHT]);
    for _ in 0..10 {
        game.step(Pads::default());
    }
    game.load_state(&saved).unwrap();
    assert_eq!(x(game.visible_extra_player_object(0).unwrap()), after[1]);
    assert_eq!(x(game.visible_extra_player_object(1).unwrap()), after[2]);

    // The same extracted ROM art also replaces the primary player's ROM
    // emitter and player 2's isolated display emitter.
    game.set_player_character(0, Character::Sigma);
    game.set_player_character(1, Character::Vile);
    for _ in 0..3 { game.step(Pads::default()); }
    let page = game.p2_tile_page_mut();
    assert!(page[..words].iter().any(|&v| v != 0), "Vile P2 art page is empty");
    assert!(page[words..2 * words].iter().any(|&v| v != 0), "Sigma P1 art page is empty");
    game.set_player_character(0, Character::Zero);
    game.set_player_character(1, Character::Zero);
    game.set_p1_zero(true).unwrap();
    for _ in 0..3 { game.step(Pads::default()); }
    assert!(game.p2_tile_page_mut()[words..2 * words].iter().any(|&v| v != 0), "X1 Zero P1 art page is empty");
}
