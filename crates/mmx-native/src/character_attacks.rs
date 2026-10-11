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
