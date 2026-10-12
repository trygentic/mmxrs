//! Translate the leader's buster button into each follower's own attack.
use mmx_core::{Character, BTN_X, BTN_Y};

/// The previous mask must be the previous delayed source mask for this player,
/// before remapping. Zero/Sigma attack on initial fire and charge release.
/// Vile fires only on the initial press; held charge and release never repeat it.
pub fn remap_movie_attacks(character: Character, previous_source_mask: u16, mask: u16) -> u16 {
    let fire = (mask ^ previous_source_mask) & BTN_Y != 0;
    match character {
        Character::Zero | Character::Sigma => {
            (mask & !(BTN_Y | BTN_X)) | if fire { BTN_X } else { 0 }
        }
        Character::Vile => (mask & !BTN_Y) | if mask & BTN_Y != 0 && previous_source_mask & BTN_Y == 0 { BTN_Y } else { 0 },
        _ => mask,
    }
}

/// Vile fires one deliberate large blast at most every two seconds. Caller
/// state uses the game's monotonic frame clock and survives route corrections.
pub fn enforce_attack_cooldown(character: Character, pad: u16, frame: u64, next_allowed: &mut u64) -> u16 {
    if character != Character::Vile || pad & BTN_Y == 0 { return pad; }
    if frame < *next_allowed { return pad & !BTN_Y; }
    *next_allowed = frame.saturating_add(120);
    pad
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmx_core::{BTN_B, BTN_A, BTN_RIGHT};
    #[test]
    fn real_hd_saber_profiles_damage_native_enemy_hp() {
        let (Some(rom_path),Some(pack))=(std::env::var_os("MMX_ROM"),std::env::var_os("MMX_HD_PACK")) else {return;};
        let rom=std::fs::read(rom_path).unwrap();
        let mut g=mmx_core::Game::from_rom(rom.clone()).unwrap();
        let ix=mmx_core::ram::wram_index;
        let b=ix(mmx_core::ram::X_BASE);
        for f in 0..2400 {g.step(mmx_core::Pads{p1:if f%90==0{mmx_core::BTN_START}else{0},p2:0});if mmx_core::coop_stages::p2_anchor_ok(g.wram())&&g.wram()[b+2]==0{break;}}
        let enemy=0x0e68;
        let mut spawned=false;
        for _ in 0..300 {g.step(mmx_core::Pads{p1:mmx_core::BTN_RIGHT,p2:0});if g.wram()[enemy]!=0&&g.wram()[enemy+0x27]&0x7f==2 {spawned=true;break;}}
        assert!(spawned);
        let start=g.save_state();
        for c in [Character::Zero,Character::Sigma] {
            g.set_boss_set(c,mmx_core::boss_data::extract_x1(&rom,c).unwrap());
            crate::bone_sheets::install(&mut g,std::path::Path::new(&pack),&[c]).unwrap();
            let mut hit=false;
            for gap in [45i16,70,90] {
                g.load_state(&start).unwrap();g.set_player_character(0,c);
                let ex=i16::from_le_bytes([g.wram()[enemy+5],g.wram()[enemy+6]]);
                let ey=i16::from_le_bytes([g.wram()[enemy+8],g.wram()[enemy+9]]);
                g.wram_mut()[b+5..b+7].copy_from_slice(&ex.wrapping_sub(gap).to_le_bytes());
                g.wram_mut()[b+8..b+10].copy_from_slice(&ey.to_le_bytes());
                g.wram_mut()[b+2]=0;g.wram_mut()[b+0x11]|=0x40;g.wram_mut()[b+0x1a..b+0x1e].fill(0);g.wram_mut()[b+0x27]=16;
                g.wram_mut()[b+0x88..b+0x8c].fill(0);
                for f in 0..55 {g.step(mmx_core::Pads{p1:if f==0{BTN_X}else{0},p2:0});
                    if g.wram()[enemy]==0 || g.wram()[enemy+0x27]&0x7f<2 {hit=true;break;}}
                eprintln!("realHD {c:?} gap{gap}: enemyHP{} heroHP{}",g.wram()[enemy+0x27]&0x7f,g.wram()[b+0x27]&0x7f);
                if hit {break;}
            }
            assert!(hit,"actualHD {c:?} weapon profile never damaged native Spiky");
        }
    }

    #[test]
    fn vile_single_blast_cooldown_preserves_movement_and_other_characters() {
        let mut next = 0;
        let pad = BTN_RIGHT | BTN_B | BTN_Y;
        assert_eq!(enforce_attack_cooldown(Character::Vile, pad, 0, &mut next), pad);
        assert_eq!(next, 120);
        assert_eq!(enforce_attack_cooldown(Character::Vile, pad, 119, &mut next), pad & !BTN_Y);
        assert_eq!(next, 120);
        assert_eq!(enforce_attack_cooldown(Character::Vile, pad, 120, &mut next), pad);
        assert_eq!(next, 240);
        assert_eq!(enforce_attack_cooldown(Character::Zero, pad, 121, &mut next), pad);
        assert_eq!(next, 240);
    }

    #[test]
    fn followers_keep_movement_and_replace_charge_with_one_character_attack() {
        let movement = BTN_RIGHT | BTN_B | BTN_A;
        for c in [Character::Zero, Character::Sigma] {
            assert_eq!(remap_movie_attacks(c, 0, movement | BTN_Y), movement | BTN_X);
            assert_eq!(remap_movie_attacks(c, BTN_Y, movement | BTN_Y), movement);
            assert_eq!(remap_movie_attacks(c, BTN_Y, movement), movement | BTN_X);
        }
        assert_eq!(remap_movie_attacks(Character::Vile, 0, movement | BTN_Y), movement | BTN_Y);
        assert_eq!(remap_movie_attacks(Character::Vile, BTN_Y, movement | BTN_Y), movement);
        assert_eq!(remap_movie_attacks(Character::Vile, BTN_Y, movement), movement);
        assert_eq!(remap_movie_attacks(Character::X, BTN_Y, movement | BTN_Y), movement | BTN_Y);
    }
}
