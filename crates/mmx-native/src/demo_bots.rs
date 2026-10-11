//! Deterministic input script for recording an actual four-player Highway run.
//! The ROM still handles movement, combat, enemies, and co-op; this only supplies
//! four SNES pad words once per emulated frame.

use mmx_core::character::Character;
use mmx_core::coop::{OFF_HP, OFF_POS_X, OFF_POS_Y};
use mmx_core::ram::{
    wram_index, D2_IN_STAGE, MODE_D2, PHASE_GAMEPLAY, STAGE_PHASE, X_HP, X_POS_X, X_POS_Y,
};
use mmx_core::{Game, BTN_B, BTN_RIGHT, BTN_START, BTN_Y};

#[derive(Default)]
pub struct DemoBots {
    boot_frames: u64,
    gameplay_frames: u64,
    was_playing: bool,
    party_ready: bool,
    next_jump_x: [u32; 4],
    jump_until: [u64; 4],
    gap_jump_after: [u64; 4],
    last_positions: [u16; 4],
    still_frames: [u16; 4],
    rescue_after: [u64; 4],
}

impl DemoBots {
    /// Keep the four-player recording running through repeated enemy waves.
    /// This is deliberately called only under `--demo-bots` and leaves any
    /// player already down to the game's usual co-op respawn.
    pub fn maintain_party(&self, game: &mut Game) {
        if self.party_ready {
            game.restore_local_player_health(5);
        }
    }

    /// Show four distinct characters at a time, long enough to see their
    /// locomotion and attack clips. Five groups cover the entire roster.
    pub fn showcase_selection(&self) -> Option<(usize, [Character; 4])> {
        if !self.party_ready || self.gameplay_frames == 0 || (self.gameplay_frames - 1) % 120 != 0 {
            return None;
        }
        let group = ((self.gameplay_frames - 1) / 120) as usize % Character::ALL.len().div_ceil(4);
        Some((
            group,
            std::array::from_fn(|slot| {
                Character::ALL
                    .get(group * 4 + slot)
                    .copied()
                    .unwrap_or(Character::X)
            }),
        ))
    }

    /// P1..P4 inputs for the next frame. Start is tapped during boot/title,
    /// then every player follows a slightly staggered walk, jump, and shot loop.
    pub fn pads(&mut self, game: &Game) -> [u16; 4] {
        let w = game.wram();
        let playing =
            w[wram_index(MODE_D2)] == D2_IN_STAGE && w[wram_index(STAGE_PHASE)] == PHASE_GAMEPLAY;
        if !playing {
            self.was_playing = false;
            self.party_ready = false;
            self.boot_frames += 1;
            return [
                if self.boot_frames % 90 == 1 {
                    BTN_START
                } else {
                    0
                },
                0,
                0,
                0,
            ];
        }
        if !self.was_playing {
            self.gameplay_frames = 0;
            self.was_playing = true;
            eprintln!("demo bots: Highway gameplay started; waiting for all four players");
        }
        if !self.party_ready {
            self.party_ready = game.visible_extra_player_object(0).is_some()
                && game.visible_extra_player_object(1).is_some();
            if !self.party_ready {
                return [0; 4];
            }
            eprintln!("demo bots: all four players spawned");
            let positions = positions(game);
            // The Highway's first broken platform starts just past x=800.
            // A 120 px cadence starting at x=205 puts a takeoff near x=805.
            self.next_jump_x = positions.map(|x| u32::from(x) + 77);
            self.jump_until = [0; 4];
            self.gap_jump_after = [0; 4];
            self.last_positions = positions;
            self.still_frames = [0; 4];
            self.rescue_after = [0; 4];
        }
        let positions = positions(game);
        let heights = heights(game);
        let f = self.gameplay_frames;
        for player in 0..4 {
            if positions[player].abs_diff(self.last_positions[player]) <= 1 {
                self.still_frames[player] = self.still_frames[player].saturating_add(1);
            } else {
                self.still_frames[player] = 0;
            }
        }
        self.last_positions = positions;
        if f % 300 == 0 {
            let p2 = &game.coop.p2;
            let p3 = game.visible_extra_player_object(0);
            let p4 = game.visible_extra_player_object(1);
            eprintln!(
                "demo bots frame {f}: x={positions:?} y={:?} hp={:?}",
                [
                    read16(w, wram_index(X_POS_Y)),
                    read16(p2, OFF_POS_Y),
                    p3.map_or(0, |p| read16(p, OFF_POS_Y)),
                    p4.map_or(0, |p| read16(p, OFF_POS_Y)),
                ],
                [
                    w[wram_index(X_HP)] & 0x7f,
                    p2[OFF_HP] & 0x7f,
                    p3.map_or(0, |p| p[OFF_HP] & 0x7f),
                    p4.map_or(0, |p| p[OFF_HP] & 0x7f)
                ]
            );
        }
        self.gameplay_frames += 1;
        std::array::from_fn(|player| {
            // The camera follows P1 near the middle of a 256 px screen.
            // Keep P4 behind P1, while P2/P3 lead by 30/60 px. The old
            // P1->P4 forward line placed P4 under the right HUD and clipped
            // Vile's body. P1 waits for P2/P3 to open their gaps at spawn.
            let anchor = i32::from(positions[0]);
            let here = i32::from(positions[player]);
            let formation_walk = match player {
                0 => {
                    i32::from(positions[1]) >= anchor + 27 && i32::from(positions[2]) >= anchor + 57
                }
                1 => here < anchor + 30,
                2 => here < anchor + 60,
                3 => here < anchor - 30,
                _ => false,
            };
            // Never brake in midair: a follower that stops at the spacing
            // boundary during a gap jump drops between platforms.
            // The Highway repeats this broken road every 640 world pixels:
            // the first near wall pins a grounded runner at x=1208, the next
            // at x=1848. Formation must not brake at either approach.
            let gap_approach = highway_gap_approach(positions[player]);
            let walk = formation_walk || gap_approach || heights[player] != 367;
            let mut bits = if walk { BTN_RIGHT } else { 0 };
            // The regular 120 px rhythm would jump at x=1165, land against
            // the pit wall, and then wait forever for the next x=1285 jump.
            // Suppress that jump and launch at the actual near edge. Retry
            // after a landing if a jump is interrupted by combat or a wall.
            if gap_approach && self.next_jump_x[player] < 1280 {
                self.next_jump_x[player] = 1280;
            }
            let gap_jump = highway_gap_takeoff(positions[player])
                && (351..=367).contains(&heights[player])
                && f >= self.gap_jump_after[player];
            // Later Highway hazards can pin a leading player against a wall
            // despite its held Right input. Make an edge-triggered rescue
            // leap after a genuine stall; its cooldown releases B before a
            // retry, which the ROM needs to recognize another jump press.
            let rescue_jump = formation_walk
                && self.still_frames[player] >= 24
                && (351..=367).contains(&heights[player])
                && f >= self.rescue_after[player]
                && f >= self.jump_until[player] + 10;
            if gap_jump
                || rescue_jump
                || (!gap_approach && u32::from(positions[player]) >= self.next_jump_x[player])
            {
                // Holding jump gives X a full-height leap. A one-frame tap
                // made the bots hop only a few pixels and miss every gap.
                self.jump_until[player] = f + 18;
                if gap_jump {
                    self.gap_jump_after[player] = f + 45;
                } else if rescue_jump {
                    self.rescue_after[player] = f + 45;
                } else {
                    while self.next_jump_x[player] <= u32::from(positions[player]) {
                        self.next_jump_x[player] += 120;
                    }
                }
            }
            if f < self.jump_until[player] {
                bits |= BTN_B;
            }
            // X and Zero charge before releasing. Sigma and Vile fire short
            // volleys so the recording has both charge and projectile effects.
            let shooting = if player < 2 {
                f % 210 < 180
            } else {
                (f + (player as u64) * 2) % 7 == 0
            };
            if shooting {
                bits |= BTN_Y;
            }
            bits
        })
    }
}

fn positions(game: &Game) -> [u16; 4] {
    [
        read16(game.wram(), wram_index(X_POS_X)),
        read16(&game.coop.p2, OFF_POS_X),
        game.visible_extra_player_object(0)
            .map_or(0, |p| read16(p, OFF_POS_X)),
        game.visible_extra_player_object(1)
            .map_or(0, |p| read16(p, OFF_POS_X)),
    ]
}

fn heights(game: &Game) -> [u16; 4] {
    [
        read16(game.wram(), wram_index(X_POS_Y)),
        read16(&game.coop.p2, OFF_POS_Y),
        game.visible_extra_player_object(0)
            .map_or(0, |p| read16(p, OFF_POS_Y)),
        game.visible_extra_player_object(1)
            .map_or(0, |p| read16(p, OFF_POS_Y)),
    ]
}

fn highway_gap_approach(x: u16) -> bool {
    x >= 1100 && (x - 1100) % 640 <= 200
}

fn highway_gap_takeoff(x: u16) -> bool {
    x >= 1195 && (x - 1195) % 640 <= 61
}

fn read16(bytes: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([bytes[i], bytes[i + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmx_core::Pads;

    /// Exercises the actual ROM physics and the collapsed Highway road. The
    /// old 120 px jump cadence parked every player at x=1208 indefinitely.
    #[test]
    fn four_bots_cross_collapsed_highway_gap() {
        let Some(rom) = std::env::var_os("MMX_ROM") else {
            eprintln!("MMX_ROM not set, skipping Highway integration test");
            return;
        };
        let mut game = Game::from_rom(std::fs::read(rom).unwrap()).unwrap();
        game.coop.enabled = true;
        game.set_local_players(4);
        mmx_core::coop::register_coop_traps(&mut game).unwrap();
        let mut bots = DemoBots::default();
        for _ in 0..3800 {
            bots.maintain_party(&mut game);
            let [p1, p2, p3, p4] = bots.pads(&game);
            game.set_extra_pads([p3, p4]);
            game.step(Pads { p1, p2 });
            if bots.party_ready && positions(&game).iter().all(|&x| x > 1950) {
                return;
            }
        }
        panic!(
            "four bots failed to cross both collapsed roads: {:?}",
            positions(&game)
        );
    }
}
