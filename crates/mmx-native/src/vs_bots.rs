//! Deterministic local VS controllers. All combat and movement remain native.

use mmx_core::{Character, Game, BTN_B, BTN_LEFT, BTN_RIGHT, BTN_START, BTN_X, BTN_Y};
use mmx_core::ram::{self, wram_index as ix};

#[derive(Clone, Copy, Default)]
struct Fighter {
    next_attack: u64,
    next_jump: u64,
    jump_frames: u8,
    charge: u16,
    previous: Option<(i32, i32)>,
    stalled: u8,
}

#[derive(Default)]
pub struct VsBots {
    fighters: [Fighter; 4],
    round: Option<u16>,
    cached_frame: Option<u64>,
    cached_pads: [u16; 4],
}

#[derive(Clone, Copy)]
struct Actor {
    x: i32,
    y: i32,
    hp: u16,
    action: u16,
    grounded: bool,
    character: Character,
}

impl VsBots {
    /// P1..P4 controller words. Repeated reads of one emulator frame are
    /// idempotent. Native round restarts reset every controller timer.
    pub fn pads(&mut self, game: &Game) -> [u16; 4] {
        let Some(status) = game.vs_status() else { return [0; 4]; };
        if self.round != Some(status.round) {
            self.fighters = [Fighter::default(); 4];
            self.round = Some(status.round);
            self.cached_frame = None;
        }
        if status.round_over { return [0; 4]; }
        let frame = game.frame_count();
        if self.cached_frame == Some(frame) { return self.cached_pads; }
        let w = game.wram();
        if w[ix(ram::MODE_D2)] != ram::D2_IN_STAGE || w[ix(ram::STAGE_PHASE)] != ram::PHASE_GAMEPLAY {
            let mut boot = [0; 4];
            if w[ix(ram::MODE_D2)] != ram::D2_IN_STAGE && frame % 90 == 0 { boot[0] = BTN_START; }
            return self.cache(frame, boot);
        }
        let actors: [Option<Actor>; 4] = std::array::from_fn(|player| {
            if player >= status.players || !game.vs_player_active(player) { return None; }
            let [x, y, hp, action, _] = game.player_physics_state(player)?;
            if hp == 0 || action == 0x0c { return None; }
            let block: &[u8] = match player {
                0 => &w[ix(ram::X_BASE)..ix(ram::X_BASE) + ram::X_BLOCK_LEN],
                1 => game.visible_p2_object()?,
                _ => game.visible_extra_player_object(player - 2)?,
            };
            Some(Actor { x: i32::from(x), y: i32::from(y), hp, action,
                grounded: (block[0x2b] | block[0x2c]) & 4 != 0,
                character: game.player_character(player).unwrap_or(Character::X) })
        });
        let arena_left = i32::from(u16::from_le_bytes([w[ix(ram::CAMERA_X)], w[ix(ram::CAMERA_X) + 1]])) + 16;
        let pads = std::array::from_fn(|player| {
            let Some(actor) = actors[player] else { return 0; };
            let Some(target) = nearest(&actors, player) else { return 0; };
            let opponent = actors[target].unwrap();
            let movement = movement(actor, opponent, arena_left, arena_left + 224);
            let direction = if movement & BTN_LEFT != 0 { -1 } else { 1 };
            let feet = actor.y + i32::from(game.player_floor_probe_offset(player).unwrap_or(16)) + 1;
            let ahead = game.terrain_collision(actor.x + direction * 24, feet);
            let gap = !ahead.is_some_and(walkable);
            let obstacle = ahead.is_some_and(walkable)
                && game.terrain_collision(actor.x + direction * 16, feet - 20).is_some_and(walkable);
            let incoming = incoming_shot(game, actor);
            decide(&mut self.fighters[player], actor, opponent, player, frame,
                movement != 0 && (gap || obstacle), incoming, arena_left, arena_left + 224)
        });
        self.cache(frame, pads)
    }

    fn cache(&mut self, frame: u64, pads: [u16; 4]) -> [u16; 4] {
        self.cached_frame = Some(frame);
        self.cached_pads = pads;
        pads
    }
}

fn nearest(actors: &[Option<Actor>; 4], player: usize) -> Option<usize> {
    let actor = actors[player]?;
    (0..4).filter(|&other| other != player)
        .filter_map(|other| actors[other].filter(|a| a.hp > 0)
            .map(|a| (actor.x.abs_diff(a.x) + 2 * actor.y.abs_diff(a.y), other)))
        .min().map(|(_, player)| player)
}

fn walkable(tile: u8) -> bool {
    matches!(tile & 0x3f, 1..=0x0e | 0x13 | 0x34..=0x3d)
}

fn movement(actor: Actor, target: Actor, arena_left: i32, arena_right: i32) -> u16 {
    let face = if target.x >= actor.x { BTN_RIGHT } else { BTN_LEFT };
    let away = if target.x >= actor.x { BTN_LEFT } else { BTN_RIGHT };
    let distance = actor.x.abs_diff(target.x);
    let preferred = match actor.character { Character::Zero => 38, Character::Sigma => 54, Character::Vile => 105, _ => 120 };
    let pad = if distance > preferred + 12 { face }
        else if distance + 20 < preferred { away } else { 0 };
    if actor.x <= arena_left + 5 && pad == BTN_LEFT || actor.x >= arena_right - 5 && pad == BTN_RIGHT { face } else { pad }
}

fn incoming_shot(game: &Game, actor: Actor) -> bool {
    let w = game.wram();
    (0..8).any(|slot| {
        let base = 0x1228 + slot * 0x40;
        if w[base] == 0 { return false; }
        let x = i32::from(u16::from_le_bytes([w[base + 5], w[base + 6]]));
        let y = i32::from(u16::from_le_bytes([w[base + 8], w[base + 9]]));
        let vx = i32::from(i16::from_le_bytes([w[base + 0x1a], w[base + 0x1b]]));
        // A muzzle's own outgoing shot travels away and cannot satisfy this.
        let dx = actor.x - x;
        dx.abs() < 76 && dx * vx > 0 && actor.y.abs_diff(y) < 26
    })
}

fn decide(state: &mut Fighter, actor: Actor, target: Actor, player: usize, frame: u64,
    terrain_jump: bool, incoming: bool, arena_left: i32, arena_right: i32) -> u16 {
    let direction = if target.x >= actor.x { 1 } else { -1 };
    let face = if direction > 0 { BTN_RIGHT } else { BTN_LEFT };
    let distance = actor.x.abs_diff(target.x);
    let melee = matches!(actor.character, Character::Zero | Character::Sigma);
    let mut pad = movement(actor, target, arena_left, arena_right);
    if state.previous.is_some_and(|(x,y)| (x - actor.x).abs() <= 1 && (y - actor.y).abs() <= 1) && pad != 0 {
        state.stalled = state.stalled.saturating_add(1);
    } else { state.stalled = 0; }
    state.previous = Some((actor.x, actor.y));
    let wall = actor.action == 0x12;
    let jump = (actor.grounded && (terrain_jump || incoming || state.stalled >= 18
        || target.y + 28 < actor.y)) || wall;
    if jump && frame >= state.next_jump && state.jump_frames == 0 {
        state.jump_frames = if wall { 8 } else { 32 };
        state.next_jump = frame + 38;
        state.stalled = 0;
    }
    if state.jump_frames > 0 {
        pad |= BTN_B;
        state.jump_frames -= 1;
    }
    // A fresh ground swing must never steal a jump or a native wall kick.
    let can_swing = pad & BTN_B == 0 || matches!(actor.action, 6 | 8);
    if melee && can_swing && frame >= state.next_attack
        && distance <= if actor.character == Character::Sigma { 82 } else { 64 }
        && actor.y.abs_diff(target.y) <= 58 {
        pad = (pad & !(BTN_LEFT | BTN_RIGHT)) | face | BTN_X;
        state.next_attack = frame + 64;
    } else if actor.character == Character::Vile && frame >= state.next_attack
        && actor.y.abs_diff(target.y) < 48 {
        pad = (pad & !(BTN_LEFT | BTN_RIGHT)) | face | BTN_Y;
        state.next_attack = frame + 120;
    } else if !melee && actor.character != Character::Vile {
        // Charge while there is room, then face the target on the release.
        // Between charges fire individual pellets, never keep a rapid-fire
        // bot facing away from the fighter it is trying to hit.
        if state.charge > 0 && state.charge < 180
            || state.charge == 0 && distance > 100 && (frame + player as u64 * 53) % 240 < 180 {
            if state.charge == 0 { pad = (pad & !(BTN_LEFT | BTN_RIGHT)) | face; }
            pad |= BTN_Y;
            state.charge += 1;
        } else if state.charge >= 180 {
            pad = (pad & !(BTN_LEFT | BTN_RIGHT)) | face;
            state.charge = 0;
            state.next_attack = frame + 12;
        } else if frame >= state.next_attack {
            pad = (pad & !(BTN_LEFT | BTN_RIGHT)) | face | BTN_Y;
            state.next_attack = frame + 12;
        }
    }
    pad
}

#[cfg(test)]
mod tests {
    use super::*;
    fn actor(x: i32, character: Character) -> Actor {
        Actor {x,y:335,hp:16,action:0,grounded:true,character}
    }
    #[test]
    fn nearest_alive_opponent_excludes_self_and_knockouts() {
        let mut dead = actor(110,Character::Vile); dead.hp = 0;
        let fighters = [Some(actor(100,Character::X)),Some(dead),Some(actor(180,Character::Sigma)),Some(actor(145,Character::Zero))];
        assert_eq!(nearest(&fighters,0),Some(3));
    }
    #[test]
    fn character_attacks_are_sabers_and_one_vile_blast_per_120_frames() {
        for character in [Character::Zero,Character::Sigma] {
            let pad = decide(&mut Fighter::default(),actor(100,character),actor(150,Character::X),0,1,false,false,0,224);
            assert_ne!(pad & BTN_X,0); assert_eq!(pad & BTN_Y,0);
        }
        let mut state = Fighter::default();
        assert_ne!(decide(&mut state,actor(100,Character::Vile),actor(180,Character::X),0,1,false,false,0,224)&BTN_Y,0);
        for frame in 2..121 {
            assert_eq!(decide(&mut state,actor(100,Character::Vile),actor(180,Character::X),0,frame,false,false,0,224)&BTN_Y,0);
        }
        assert_ne!(decide(&mut state,actor(100,Character::Vile),actor(180,Character::X),0,121,false,false,0,224)&BTN_Y,0);
    }
    #[test]
    fn terrain_jump_has_priority_over_ground_melee() {
        let pad = decide(&mut Fighter::default(),actor(100,Character::Zero),actor(140,Character::X),0,1,true,false,0,224);
        assert_ne!(pad & BTN_B,0); assert_eq!(pad & BTN_X,0);
    }
    #[test]
    fn ranged_fighters_keep_spacing_instead_of_stacking() {
        let mut state = Fighter {next_attack:100,..Default::default()};
        let pad = decide(&mut state,actor(100,Character::Vile),actor(115,Character::X),0,1,false,false,0,224);
        assert_eq!(pad & (BTN_LEFT|BTN_RIGHT),BTN_LEFT);
    }
    #[test]
    fn charged_buster_release_faces_opponent_and_releases_y() {
        let mut state = Fighter {charge:180,..Default::default()};
        let pad = decide(&mut state,actor(100,Character::X),actor(150,Character::Zero),0,1,false,false,0,224);
        assert_eq!(pad & BTN_Y,0);
        assert_eq!(pad & (BTN_LEFT|BTN_RIGHT),BTN_RIGHT);
        assert_eq!(state.charge,0);
    }
}
