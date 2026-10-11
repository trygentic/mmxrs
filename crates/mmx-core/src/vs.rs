//! Local free-for-all rounds on top of the ROM's co-op player slices.
//! The ROM owns movement and shots; this module adds player-on-player hits.

use crate::coop::{self, OFF_ACTION, OFF_HP, OFF_POS_X, OFF_POS_Y, OFF_SHOTS};
use crate::coop_stages;
use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::{Game, Pads};

const SHOTS: u32 = 0x7E1228;
const SHOT_LEN: usize = 0x40;
const HIT_COOLDOWN: u8 = 28;
const SPAWN_OFFSETS: [i16; 4] = [0, 48, -48, 88];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VsStatus {
    /// The winner of the current round, zero-based. `None` while fighting.
    pub winner: Option<usize>,
    pub round_over: bool,
    pub scores: [u8; 4],
    pub round: u16,
    pub players: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct VsState {
    pub(crate) winner: Option<usize>,
    pub(crate) round_over: bool,
    pub(crate) scores: [u8; 4],
    pub(crate) round: u16,
    pub(crate) players: usize,
    placed: bool,
    arena_left: Option<u16>,
    origin: Option<(u16, u16)>,
    eliminated: [bool; 4],
    cooldown: [u8; 4],
}

impl VsState {
    fn new(players: usize) -> Self {
        Self { winner: None, round_over: false, scores: [0; 4], round: 1, players, placed: false, arena_left: None, origin: None,
            eliminated: [false; 4], cooldown: [0; 4] }
    }
    fn status(&self) -> VsStatus {
        VsStatus { winner: self.winner, round_over: self.round_over, scores: self.scores, round: self.round, players: self.players }
    }
}

impl Game {
    pub fn set_vs_arena(&mut self, arena: crate::vs_stages::Arena) {
        self.vs_arena = Some(arena);
        if let Some(vs) = &mut self.vs_state { vs.placed = false; vs.arena_left = None; vs.origin = None; }
    }

    /// Enable local VS for two to four fighters. Character choices may be
    /// changed with `set_player_character` before the first arena frame.
    pub fn enable_vs(&mut self, players: usize) -> Result<(), String> {
        if !(2..=4).contains(&players) { return Err("VS requires 2 to 4 players".into()); }
        coop::register_coop_traps(self)?;
        // Enemy AI may still animate and platform objects still update, but
        // its JSL contact test cannot damage a fighter in VS.
        self.traps.register(self.hook(coop::HOOK_CONTACT), "vs_enemy_contact", TrapReturn::Rtl, no_enemy_contact);
        self.coop.enabled = true;
        self.set_local_players(players);
        self.vs_state = Some(VsState::new(players));
        Ok(())
    }

    pub fn vs_status(&self) -> Option<VsStatus> { self.vs_state.as_ref().map(VsState::status) }

    /// Start the next round in the current arena. The native menu calls this
    /// after showing the winner; all fighters regain full health and space out.
    pub fn restart_vs_round(&mut self) {
        let Some(vs) = &mut self.vs_state else { return };
        vs.winner = None;
        vs.round_over = false;
        vs.round = vs.round.saturating_add(1);
        vs.placed = false;
        vs.eliminated = [false; 4];
        vs.cooldown = [0; 4];
        let max = self.wram()[ix(ram::MAX_HP)] & 0x7f;
        if max == 0 { return; }
        let p1 = ix(ram::X_BASE);
        let w = self.wram_mut();
        w[p1 + OFF_HP] = max | 0x80;
        w[p1 + OFF_ACTION] = coop_stages::act::STAND;
        if let Some((x, y)) = self.vs_state.as_ref().and_then(|vs| vs.origin) {
            let b = &mut self.wram_mut()[p1..p1 + ram::X_BLOCK_LEN];
            b[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&x.to_le_bytes());
            b[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&y.to_le_bytes());
            b[coop::OFF_PREV_X..coop::OFF_PREV_X + 2].copy_from_slice(&x.to_le_bytes());
            b[coop::OFF_PREV_X + 2..coop::OFF_PREV_X + 4].copy_from_slice(&y.to_le_bytes());
            b[coop::OFF_VEL_X..coop::OFF_VEL_X + 4].fill(0);
        }
        for slot in 1..self.local_players {
            let c = if slot == 1 { &mut self.coop } else { &mut self.extra_coop[slot - 2] };
            c.p2_hidden = false;
            c.down_frames = 0;
            c.p2_warp = false;
            c.p2[OFF_HP] = max | 0x80;
            c.p2[OFF_ACTION] = coop_stages::act::STAND;
        }
        place_players(self);
    }
}

fn no_enemy_contact(game: &mut Game) {
    let mut regs = game.regs();
    regs.a = 0;
    game.set_regs(regs);
}

fn block(game: &Game, player: usize) -> Option<&[u8]> {
    match player {
        0 => Some(&game.wram()[ix(ram::X_BASE)..ix(ram::X_BASE) + ram::X_BLOCK_LEN]),
        1 if !game.coop.p2_hidden => Some(&game.coop.p2),
        2..=3 if player < game.local_players && !game.extra_coop[player - 2].p2_hidden => Some(&game.extra_coop[player - 2].p2),
        _ => None,
    }
}

fn block_mut(game: &mut Game, player: usize) -> Option<&mut [u8]> {
    match player {
        0 => Some(&mut game.wram_mut()[ix(ram::X_BASE)..ix(ram::X_BASE) + ram::X_BLOCK_LEN]),
        1 if !game.coop.p2_hidden => Some(&mut game.coop.p2),
        2..=3 if player < game.local_players && !game.extra_coop[player - 2].p2_hidden => Some(&mut game.extra_coop[player - 2].p2),
        _ => None,
    }
}

fn pos(b: &[u8]) -> (i32, i32) {
    (i32::from(u16::from_le_bytes([b[OFF_POS_X], b[OFF_POS_X + 1]])),
     i32::from(u16::from_le_bytes([b[OFF_POS_Y], b[OFF_POS_Y + 1]])))
}

fn place_players(game: &mut Game) {
    let (x, y) = pos(block(game, 0).unwrap());
    let camera = i32::from(u16::from_le_bytes([game.wram()[ix(ram::CAMERA_X)], game.wram()[ix(ram::CAMERA_X) + 1]]));
    let count = game.vs_state.as_ref().unwrap().players;
    let offsets = game.vs_arena.map_or(SPAWN_OFFSETS, |arena| arena.spawn_offsets);
    for player in 1..count {
        if let Some(b) = block_mut(game, player) {
            let sx = (x + i32::from(offsets[player])).clamp(camera + 16, camera + 240) as u16;
            b[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&sx.to_le_bytes());
            b[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&(y as u16).to_le_bytes());
            b[coop::OFF_PREV_X..coop::OFF_PREV_X + 2].copy_from_slice(&sx.to_le_bytes());
            b[coop::OFF_PREV_X + 2..coop::OFF_PREV_X + 4].copy_from_slice(&(y as u16).to_le_bytes());
            b[coop::OFF_VEL_X..coop::OFF_VEL_X + 4].fill(0);
        }
    }
    let vs = game.vs_state.as_mut().unwrap();
    vs.placed = true;
    vs.arena_left.get_or_insert(camera as u16);
    vs.origin.get_or_insert((x as u16, y as u16));
}

fn pin_arena(game: &mut Game) {
    let Some(left) = game.vs_state.as_ref().and_then(|vs| vs.arena_left) else { return };
    let bounds = (i32::from(left) + 16, i32::from(left) + 240);
    for player in 0..game.local_players {
        if let Some(b) = block_mut(game, player) {
            let x = pos(b).0;
            let x = x.clamp(bounds.0, bounds.1) as u16;
            b[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&x.to_le_bytes());
            b[coop::OFF_PREV_X..coop::OFF_PREV_X + 2].copy_from_slice(&x.to_le_bytes());
        }
    }
    // These are the ROM's camera limits in follow mode. Keeping both at the
    // selected screen retains its tilemap, collision and stage effects.
    let w = game.wram_mut();
    for address in [0x7E1E56, 0x7E1E58] {
        w[ix(address)..ix(address) + 2].copy_from_slice(&left.to_le_bytes());
    }
}

fn owner(game: &Game, shot: usize) -> usize {
    if game.coop.p2_shots & (1 << shot) != 0 { return 1; }
    for player in 2..game.local_players {
        if game.extra_coop[player - 2].p2_shots & (1 << shot) != 0 { return player; }
    }
    0
}

fn shot_damage(id: u8) -> u8 {
    match id { 1 => 2, 2 => 4, _ => 1 }
}

fn hit(game: &mut Game, target: usize, damage: u8) {
    let Some(vs) = game.vs_state.as_mut() else { return };
    if vs.cooldown[target] != 0 || vs.eliminated[target] { return; }
    vs.cooldown[target] = HIT_COOLDOWN;
    if let Some(b) = block_mut(game, target) {
        let hp = (b[OFF_HP] & 0x7f).saturating_sub(damage);
        b[OFF_HP] = hp | 0x80;
    }
}

fn saber(game: &Game, player: usize) -> &crate::saber::Saber {
    match player {
        0 => &game.coop.p1.saber,
        1 => &game.coop.saber,
        _ => &game.extra_coop[player - 2].saber,
    }
}

fn saber_mut(game: &mut Game, player: usize) -> &mut crate::saber::Saber {
    match player {
        0 => &mut game.coop.p1.saber,
        1 => &mut game.coop.saber,
        _ => &mut game.extra_coop[player - 2].saber,
    }
}

fn saber_hits(game: &mut Game, players: usize) {
    for shooter in 0..players {
        if player_eliminated(game, shooter)
            || !game.player_character(shooter).is_some_and(crate::saber::uses_saber)
            || saber(game, shooter).hit
            || crate::saber::contact(game, shooter, saber(game, shooter)).is_none() { continue; }
        let Some(b) = block(game, shooter) else { continue };
        let Some(attack) = crate::saber::player_shot(game, shooter, b, saber(game, shooter)) else { continue };
        let sx = i32::from(u16::from_le_bytes([attack[5], attack[6]]));
        let sy = i32::from(u16::from_le_bytes([attack[8], attack[9]]));
        let half_w = i32::from(attack[crate::saber::BOX_AT + 2]);
        let half_h = i32::from(attack[crate::saber::BOX_AT + 3]);
        for target in 0..players {
            if target == shooter || game.vs_state.as_ref().unwrap().eliminated[target]
                || game.vs_state.as_ref().unwrap().cooldown[target] != 0 { continue; }
            let Some(b) = block(game, target) else { continue };
            let (px, py) = pos(b);
            if (sx - px).abs() <= half_w + 8 && (sy - (py - 14)).abs() <= half_h + 16 {
                hit(game, target, 4);
                saber_mut(game, shooter).hit = true;
                break;
            }
        }
    }
}

pub(crate) fn player_eliminated(game: &Game, player: usize) -> bool {
    game.vs_state.as_ref().is_some_and(|vs| vs.eliminated.get(player).copied().unwrap_or(false))
}

pub(crate) fn mask_pads(game: &Game, mut pads: Pads) -> Pads {
    if player_eliminated(game, 0) { pads.p1 = 0; }
    if player_eliminated(game, 1) { pads.p2 = 0; }
    pads
}

/// Called after a complete ROM frame. It does no work outside active VS play.
pub(crate) fn tick(game: &mut Game) {
    let Some(vs) = &game.vs_state else { return };
    let w = game.wram();
    if vs.round_over
        || w[ix(ram::MODE_D2)] != coop_stages::MODE_IN_STAGE
        || (w[ix(ram::STAGE_PHASE)] != coop_stages::PHASE_GAMEPLAY
            && !(vs.placed && w[ix(ram::STAGE_PHASE)] == 6))
        || w[ix(ram::PAUSE)] != 0 { return; }
    let players = vs.players;
    if !vs.placed {
        if (1..players).all(|i| block(game, i).is_some()) { place_players(game); }
        return;
    }
    pin_arena(game);
    for cooldown in &mut game.vs_state.as_mut().unwrap().cooldown { *cooldown = cooldown.saturating_sub(1); }
    for shot in 0..8 {
        let at = ix(SHOTS) + shot * SHOT_LEN;
        let w = game.wram();
        if w[at] == 0 { continue; }
        let shooter = owner(game, shot);
        if player_eliminated(game, shooter) { continue; }
        let (sx, sy) = (i32::from(u16::from_le_bytes([w[at + 5], w[at + 6]])),
                        i32::from(u16::from_le_bytes([w[at + 8], w[at + 9]])));
        let damage = shot_damage(w[at + 0x0a]);
        for target in 0..players {
            if target == shooter || game.vs_state.as_ref().unwrap().eliminated[target] { continue; }
            let Some(b) = block(game, target) else { continue };
            let (px, py) = pos(b);
            if (sx - px).abs() <= 12 && (sy - (py - 14)).abs() <= 18 {
                if game.vs_state.as_ref().unwrap().cooldown[target] != 0 { break; }
                hit(game, target, damage);
                game.wram_mut()[at] = 0;
                if let Some(b) = block_mut(game, shooter) { b[OFF_SHOTS] = b[OFF_SHOTS].saturating_sub(1); }
                if shooter == 1 { game.coop.p2_shots &= !(1 << shot); }
                else if shooter >= 2 { game.extra_coop[shooter - 2].p2_shots &= !(1 << shot); }
                break;
            }
        }
    }
    saber_hits(game, players);
    let mut alive = 0;
    let mut last = None;
    for player in 0..players {
        let hp = block(game, player).map_or(0, |b| b[OFF_HP] & 0x7f);
        if hp == 0 {
            if !game.vs_state.as_ref().unwrap().eliminated[player] {
                game.vs_state.as_mut().unwrap().eliminated[player] = true;
                if player == 0 {
                    let p1 = ix(ram::X_BASE);
                    let w = game.wram_mut();
                    w[p1 + OFF_HP] = 0x81;
                    w[p1 + OFF_ACTION] = coop_stages::act::STAND;
                    w[ix(ram::STAGE_PHASE)] = coop_stages::PHASE_GAMEPLAY;
                } else {
                    let c = if player == 1 { &mut game.coop } else { &mut game.extra_coop[player - 2] };
                    c.p2_hidden = true;
                    c.down_frames = u16::MAX;
                }
            }
        } else if !game.vs_state.as_ref().unwrap().eliminated[player] {
            alive += 1;
            last = Some(player);
        }
    }
    if alive <= 1 {
        let vs = game.vs_state.as_mut().unwrap();
        vs.winner = last;
        vs.round_over = true;
        if let Some(winner) = last { vs.scores[winner] = vs.scores[winner].saturating_add(1); }
        // The native overlay shows the result while this arena frame stays up.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_vs_projectile_ko_and_round_restart() {
        let Some(path) = std::env::var_os("MMX_ROM") else { return };
        let mut game = Game::from_rom(std::fs::read(path).unwrap()).unwrap();
        game.enable_vs(2).unwrap();
        let arena = crate::vs_stages::arena("highway").unwrap();
        game.set_vs_arena(*arena);
        crate::vs_stages::start_stage(&mut game, arena).unwrap();
        for _ in 0..180 {
            if game.vs_state.as_ref().unwrap().placed { break; }
            game.step(Pads::default());
        }
        assert!(game.vs_state.as_ref().unwrap().placed, "both fighters spawned");
        let (x, y) = pos(&game.coop.p2);
        game.coop.p2[OFF_HP] = 1;
        let at = ix(SHOTS);
        let w = game.wram_mut();
        w[at] = 1;
        w[at + 5..at + 7].copy_from_slice(&(x as u16).to_le_bytes());
        w[at + 8..at + 10].copy_from_slice(&((y - 14) as u16).to_le_bytes());
        tick(&mut game);
        assert_eq!(game.vs_status().unwrap().winner, Some(0));
        assert!(game.vs_status().unwrap().round_over);
        assert!(game.coop.p2_hidden);
        game.restart_vs_round();
        let status = game.vs_status().unwrap();
        assert!(!status.round_over);
        assert_eq!(status.round, 2);
        assert_eq!(status.scores[0], 1);
        assert!(!game.coop.p2_hidden);
        assert!(game.coop.p2[OFF_HP] & 0x7f > 0);
    }
}
