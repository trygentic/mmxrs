//! Input-only bots following the first stage's solo movie trajectory.
//! The reference supplies waypoints, never live positions, health or enemies.
use crate::cli::Args;
use mmx_core::coop::{OFF_ACTION, OFF_GROUND, OFF_POS_X, OFF_POS_Y};
use mmx_core::ram::{self, wram_index as ix};
use mmx_core::{Game, Pads, BTN_B, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_Y};
use mmx_verify::movie_bk2;

#[derive(Clone, Copy, Debug)]
struct Point {
    x: u16,
    y: u16,
    pad: u16,
    action: u8,
}

#[derive(Default)]
struct Follower {
    at: usize,
    ticks: usize,
    last_x: u16,
    stalled: u16,
    jump_until: usize,
    jump_after: usize,
    recovering_collapse: bool,
    was_hurt: bool,
}

pub struct RoutePlayback {
    boot: Vec<Pads>,
    boot_at: usize,
    route: Vec<Point>,
    followers: [Follower; 4],
    delay: usize,
    started: bool,
    finished: bool,
    level: u8,
    previous_source: [u16; 4],
    last_logged_tick: Option<usize>,
    pending_respawn_reanchor: bool,
    respawn_gameplay_frames: u16,
    next_attack_allowed: [u64; 4],
}

impl RoutePlayback {
    /// Precompute only the movie's first stage, using a plain solo emulator.
    pub fn load(args: &Args) -> Result<Self, String> {
        let path = args.movie.as_deref().ok_or("route bots require --movie")?;
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let movie = match std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
        {
            Some("lsmv") => movie_bk2::parse_lsmv(&bytes),
            Some("smv") => movie_bk2::parse_smv(&bytes),
            _ => movie_bk2::parse_bk2(&bytes),
        }?;
        if movie.from_snapshot || !movie.resets.is_empty() {
            return Err("route movie must start from power-on without resets".into());
        }
        let rom_path = args.rom.as_deref().ok_or("route bots require a ROM")?;
        let rom = std::fs::read(rom_path).map_err(|e| format!("{rom_path}: {e}"))?;
        let mut reference = Game::from_rom_timing(rom, args.timing).map_err(|e| e.to_string())?;
        reference.set_overclock(mmx_core::Overclock::Off);
        reference.set_allow_opposing_directions(true);
        let mut route = Vec::new();
        let mut boot = Vec::new();
        let mut level = 0;
        eprintln!("movie route: preparing first-stage solo waypoints (up to 12000 frames)");
        for pads in movie.pads.iter().take(12000).copied() {
            let w = reference.wram();
            if gameplay(w) {
                if route.is_empty() {
                    level = w[ix(ram::LEVEL_ID)];
                }
                route.push(Point {
                    x: word(w, ix(ram::X_POS_X)),
                    y: word(w, ix(ram::X_POS_Y)),
                    pad: pads.p1,
                    action: w[ix(ram::X_ACTION)],
                });
            } else if route.is_empty() {
                boot.push(pads);
            } else {
                // A death is not a successful reference route.
                if w[ix(ram::STAGE_PHASE)] == 6 {
                    return Err(
                        "solo movie died in its first stage; cannot guide a successful route"
                            .into(),
                    );
                }
                break;
            }
            reference.step(pads);
            let _ = reference.take_audio();
        }
        if route.len() < 60 {
            return Err("movie did not provide a playable first-stage route".into());
        }
        eprintln!(
            "movie route: {} boot frames, {} first-stage waypoints; input-only correction enabled",
            boot.len(),
            route.len()
        );
        Ok(Self {
            boot,
            boot_at: 0,
            route,
            followers: std::array::from_fn(|_| Follower::default()),
            delay: args.movie_coop_delay.map_or(8, usize::from),
            started: false,
            finished: false,
            level,
            previous_source: [0; 4],
            last_logged_tick: None,
            pending_respawn_reanchor: false,
            respawn_gameplay_frames: 0,
            next_attack_allowed: [0; 4],
        })
    }

    pub fn next(&mut self, game: &Game) -> [u16; 4] {
        if self.finished {
            self.previous_source = [0; 4];
            return [0; 4];
        }
        let w = game.wram();
        if !gameplay(w) {
            if !self.started {
                let pad = self.boot.get(self.boot_at).map_or(0, |p| p.p1);
                self.boot_at += 1;
                return [pad, 0, 0, 0];
            }
            if w[ix(ram::MODE_D2)] != ram::D2_IN_STAGE || w[ix(ram::STAGE_PHASE)] >= 8 {
                self.finished = true;
                eprintln!(
                    "movie route: control ended in mode {:#04x}, phase {:#04x}; releasing inputs",
                    w[ix(ram::MODE_D2)],
                    w[ix(ram::STAGE_PHASE)]
                );
            } else if w[ix(ram::STAGE_PHASE)] == 6 {
                // The ROM can reload at a checkpoint, not the initial spawn.
                // Wait for its real beam-in/landing before locating the route.
                self.pending_respawn_reanchor = true;
                self.respawn_gameplay_frames = 0;
            }
            self.previous_source = [0; 4];
            return [0; 4];
        }
        self.started = true;
        if w[ix(ram::LEVEL_ID)] != self.level {
            self.finished = true;
            self.previous_source = [0; 4];
            return [0; 4];
        }
        if mmx_core::coop_stages::p1_scripted(w) {
            self.previous_source = [0; 4];
            return [0; 4];
        }
        // Never replay the TAS's time-based pause into a retimed live stage.
        if w[ix(ram::PAUSE)] != 0 {
            self.previous_source = [0; 4];
            return [0; 4];
        }
        if self.pending_respawn_reanchor {
            self.respawn_gameplay_frames = self.respawn_gameplay_frames.saturating_add(1);
            let ground = ix(ram::X_BASE) + OFF_GROUND;
            let actually_grounded = (w[ground] | w[ground + 1]) & 4 != 0;
            // The first stage-update frame exposes an uninitialized standing
            // object at the top of the screen before the ROM sets BEAM_IN.
            // Do not mistake that zero-velocity placeholder for a landing.
            if self.respawn_gameplay_frames < 3
                || !actually_grounded
                || !mmx_core::coop_stages::p2_anchor_ok(w)
            {
                return [0; 4];
            }
            let x = word(w, ix(ram::X_POS_X));
            let y = word(w, ix(ram::X_POS_Y));
            let Some(at) = checkpoint_waypoint(&self.route, x, y) else {
                return [0; 4];
            };
            self.followers = std::array::from_fn(|_| Follower::default());
            self.followers[0].at = at;
            self.previous_source = [0; 4];
            self.pending_respawn_reanchor = false;
            eprintln!("movie route: real checkpoint respawn at ({x},{y}); leader waypoint {at}");
        }
        let positions = [
            Some((
                word(w, ix(ram::X_POS_X)),
                word(w, ix(ram::X_POS_Y)),
                w[ix(ram::X_ACTION)],
            )),
            game.visible_p2_object().map(position),
            game.visible_extra_player_object(0).map(position),
            game.visible_extra_player_object(1).map(position),
        ];
        let facings = [
            Some(w[ix(ram::X_FACING)]),
            game.visible_p2_object().map(|p| p[0x11]),
            game.visible_extra_player_object(0).map(|p| p[0x11]),
            game.visible_extra_player_object(1).map(|p| p[0x11]),
        ];
        let leader_limit = self.followers[0].at;
        let bees: Vec<_> = (0..ram::ENEMY_COUNT)
            .filter_map(|slot| {
                let at = ix(ram::ENEMIES_BASE) + slot * ram::ENEMY_STRIDE;
                (w[at + ram::ENEMY_ACTIVE] != 0
                    && w[at + ram::ENEMY_ID] == 0x22
                    && w[at + ram::ENEMY_HP] & 0x7f != 0)
                    .then(|| {
                        (
                            word(w, at + ram::ENEMY_POS_X),
                            word(w, at + ram::ENEMY_POS_Y),
                            w[at + ram::ENEMY_HP] & 0x7f,
                        )
                    })
            })
            .collect();
        if self.followers[0].ticks % 300 == 0
            && self.last_logged_tick != Some(self.followers[0].ticks)
        {
            self.last_logged_tick = Some(self.followers[0].ticks);
            eprintln!(
                "movie route tick {}: positions {:?}, waypoints {:?}, targets {:?}, lead shots {}, Bee Bladers {:?}",
                self.followers[0].ticks,
                positions,
                self.followers.each_ref().map(|f| f.at),
                self.followers.each_ref().map(|f| self.route[f.at]),
                w[ix(ram::X_BASE) + 0x35], bees
            );
        }
        std::array::from_fn(|slot| {
            let Some((x, y, action)) = positions[slot] else {
                return 0;
            };
            if slot > 0 && leader_limit < self.delay * slot {
                return 0;
            }
            let limit = if slot == 0 {
                self.route.len() - 1
            } else {
                leader_limit.saturating_sub(self.delay * slot)
            };
            if action == 0x0E {
                // Let the ROM resolve its own knockback and invulnerability.
                // No waypoint time elapses while this player's controls cannot
                // produce the reference movement.
                self.followers[slot].was_hurt = true;
                self.previous_source[slot] = 0;
                return 0;
            }
            if self.followers[slot].was_hurt {
                let old = self.followers[slot].at;
                let at =
                    nearest_compatible(&self.route, x, y, old, limit).unwrap_or(old.min(limit));
                let follower = &mut self.followers[slot];
                follower.at = at;
                follower.was_hurt = false;
                follower.jump_until = 0;
                follower.jump_after = 0;
                follower.stalled = 0;
                follower.last_x = x;
                self.previous_source[slot] = 0;
                eprintln!("movie route: P{} recovered from real knockback at ({x},{y}); waypoint {old}->{at}", slot + 1);
            }
            // Extra players spawn only once the leader has reached safe ground;
            // their first waypoint must match that location, not the reference's
            // beam-in point high above the initial platform.
            if slot > 0 && self.followers[slot].ticks == 0 {
                self.followers[slot].at = limit;
            }
            if slot > 0
                && (x.abs_diff(self.followers[slot].last_x) > 192
                    || self.followers[slot].stalled >= 100)
            {
                // A normal co-op warp/respawn changes position. Reacquire only
                // the bot's waypoint; the ROM still owns the actual position.
                let at = nearest(&self.route, x, y, limit);
                self.followers[slot].at = at;
                self.followers[slot].stalled = 0;
            }
            if let Some(at) =
                landed_continuation(&self.route, x, y, action, self.followers[slot].at, limit)
            {
                let old = self.followers[slot].at;
                self.followers[slot].at = at;
                self.followers[slot].jump_until = 0;
                self.followers[slot].jump_after = 0;
                self.followers[slot].stalled = 0;
                eprintln!("movie route: P{} landed past airborne source segment at ({x},{y}); {old}->{at}", slot + 1);
            }
            if self.followers[slot].stalled >= 24 {
                if let Some(at) = ahead_reached(&self.route, x, y, self.followers[slot].at, limit) {
                    let old = self.followers[slot].at;
                    let follower = &mut self.followers[slot];
                    follower.at = at;
                    follower.jump_until = 0;
                    follower.jump_after = 0;
                    follower.stalled = 0;
                    eprintln!(
                        "movie route: P{} reached later waypoint at ({x},{y}); {old}->{at}",
                        slot + 1
                    );
                }
            }
            let current = self.route[self.followers[slot].at];
            if y.abs_diff(current.y) > 72 && self.followers[slot].stalled >= 24 {
                // Extra attacks can defeat Bee Blader sooner and collapse the
                // road before the solo movie does. Follow the reached lower
                // floor's future route, rather than jumping at a vanished road.
                let from = self.followers[slot].at;
                let to = (from + 1800).min(limit);
                if to >= from {
                    let at = from + nearest(&self.route[from..=to], x, y, to - from);
                    let candidate = self.route[at];
                    let distance =
                        |p: Point| u32::from(x.abs_diff(p.x)) + 2 * u32::from(y.abs_diff(p.y));
                    if distance(candidate) + 64 < distance(current) {
                        self.followers[slot].at = at;
                        self.followers[slot].stalled = 0;
                    }
                }
            }
            if (2560..2820).contains(&x) && y > 400 {
                self.followers[slot].recovering_collapse = true;
            }
            if self.followers[slot].recovering_collapse && x >= 2816 && y <= 390 {
                self.followers[slot].recovering_collapse = false;
                let from = self.followers[slot].at;
                let to = (from + 1800).min(limit).max(from);
                self.followers[slot].at = from + nearest(&self.route[from..=to], x, y, to - from);
            }
            let mut source = steer(&self.route, &mut self.followers[slot], x, y, action, limit);
            // The solo TAS leaps across the first Bee Blader's road. Extra
            // co-op damage can collapse it before that leap, putting the live
            // party on the lower floor absent from the reference trajectory.
            // Recover via the room's right wall using normal wall-kick inputs.
            let collapse_recovery = self.followers[slot].recovering_collapse;
            if collapse_recovery {
                source = (source & !(BTN_LEFT | BTN_RIGHT | BTN_B)) | BTN_RIGHT;
                if self.followers[slot].ticks % 24 < 16 {
                    source |= BTN_B;
                }
            }
            let character = game
                .player_character(slot)
                .unwrap_or(mmx_core::Character::X);
            source = combat_cadence(
                character,
                source,
                self.followers[slot].ticks,
                self.followers[slot].stalled,
            );
            if character == mmx_core::Character::Vile
                && self.followers[slot].stalled >= 60
                && bees.iter().any(|(ex, _, _)| x.abs_diff(*ex) < 200)
            {
                source &= !BTN_Y;
                if self.followers[slot].ticks % 180 == 0 {
                    source |= BTN_Y;
                }
            }
            if let Some((enemy_x, _, _)) = bees
                .iter()
                .filter(|(ex, _, _)| {
                    !collapse_recovery
                        && self.followers[slot].stalled >= 60
                        && x.abs_diff(*ex) < 200
                })
                .min_by_key(|(ex, _, _)| x.abs_diff(*ex))
            {
                // Retimed traversal can pass the living ship before its death;
                // face its actual position instead of shooting away from it.
                // A direction tap turns the player, then stationary fire avoids
                // walking into the boss while the charge builds.
                source = face_enemy(
                    source,
                    x,
                    *enemy_x,
                    facings[slot].is_some_and(|f| f & 0x40 != 0),
                );
                // Shoot the raised head rather than continuing to fire below
                // it. X's jump leads the charge release at tick180; Vile can
                // keep firing his shoulder rockets throughout the ascent.
                let tick = self.followers[slot].ticks;
                let jumping = match character {
                    mmx_core::Character::X => (160..184).contains(&(tick % 200)),
                    mmx_core::Character::Vile => (156..180).contains(&(tick % 180)),
                    _ => tick % 72 < 24,
                };
                source &= !BTN_B;
                if jumping {
                    source |= BTN_B;
                }
            }
            let mut pad = crate::character_attacks::remap_movie_attacks(
                character,
                self.previous_source[slot],
                source,
            );
            self.previous_source[slot] = source;
            if (source & BTN_B != 0 || collapse_recovery)
                && matches!(
                    character,
                    mmx_core::Character::Zero | mmx_core::Character::Sigma
                )
            {
                pad &= !mmx_core::BTN_X;
            }
            crate::character_attacks::enforce_attack_cooldown(
                character,
                pad,
                game.frame_count(),
                &mut self.next_attack_allowed[slot],
            )
        })
    }
}

fn gameplay(w: &[u8; 0x20000]) -> bool {
    w[ix(ram::MODE_D2)] == ram::D2_IN_STAGE && w[ix(ram::STAGE_PHASE)] == ram::PHASE_GAMEPLAY
}
fn word(w: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([w[off], w[off + 1]])
}
fn position(block: &[u8; ram::X_BLOCK_LEN]) -> (u16, u16, u8) {
    (
        word(block, OFF_POS_X),
        word(block, OFF_POS_Y),
        block[OFF_ACTION],
    )
}

fn nearest(route: &[Point], x: u16, y: u16, limit: usize) -> usize {
    route[..=limit.min(route.len() - 1)]
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| u32::from(x.abs_diff(p.x)) + 2 * u32::from(y.abs_diff(p.y)))
        .map_or(0, |(at, _)| at)
}

fn grounded_action(action: u8) -> bool {
    matches!(action, 0x00 | 0x02 | 0x04 | 0x0A)
}

fn checkpoint_waypoint(route: &[Point], x: u16, y: u16) -> Option<usize> {
    (0..route.len())
        .filter(|&i| {
            grounded_action(route[i].action)
                && x.abs_diff(route[i].x) <= 128
                && y.abs_diff(route[i].y) <= 32
        })
        .min_by_key(|&i| distance(route[i], x, y))
}

fn landed_continuation(
    route: &[Point],
    x: u16,
    y: u16,
    action: u8,
    at: usize,
    limit: usize,
) -> Option<usize> {
    let p = route[at];
    if !grounded_action(action)
        || !matches!(p.action, 0x06 | 0x08 | 0x10 | 0x12)
        || p.pad & BTN_RIGHT == 0
        || p.x > x.saturating_add(8)
        || y.abs_diff(p.y) <= 28
    {
        return None;
    }
    (at.saturating_add(1)..=(at + 240).min(limit).min(route.len() - 1))
        .filter(|&i| {
            grounded_action(route[i].action)
                && route[i].x >= x.saturating_add(16)
                && route[i].x <= x.saturating_add(192)
                && y.abs_diff(route[i].y) <= 64
        })
        .min_by_key(|&i| (distance(route[i], x, y), i.abs_diff(at)))
}

fn distance(p: Point, x: u16, y: u16) -> u32 {
    u32::from(x.abs_diff(p.x)) + 2 * u32::from(y.abs_diff(p.y))
}

fn ahead_reached(route: &[Point], x: u16, y: u16, at: usize, limit: usize) -> Option<usize> {
    let to = (at + 240).min(limit).min(route.len() - 1);
    let best = (at.saturating_add(1)..=to)
        .filter(|&i| {
            compatible_action(route[i].action)
                && x.abs_diff(route[i].x) <= 64
                && y.abs_diff(route[i].y) <= 32
        })
        .min_by_key(|&i| (distance(route[i], x, y), i.abs_diff(at)))?;
    (distance(route[best], x, y) + 24 < distance(route[at], x, y)).then_some(best)
}

fn compatible_action(action: u8) -> bool {
    matches!(
        action,
        0x00 | 0x02 | 0x04 | 0x06 | 0x08 | 0x0A | 0x10 | 0x12
    )
}

fn nearest_compatible(route: &[Point], x: u16, y: u16, at: usize, limit: usize) -> Option<usize> {
    let limit = limit.min(route.len() - 1);
    let from = at.saturating_sub(120).min(limit);
    let to = at.saturating_add(240).min(limit);
    (from..=to)
        .filter(|&i| {
            let p = route[i];
            compatible_action(p.action) && x.abs_diff(p.x) <= 96 && y.abs_diff(p.y) <= 96
        })
        .min_by_key(|&i| {
            let p = route[i];
            (
                u32::from(x.abs_diff(p.x)) + 2 * u32::from(y.abs_diff(p.y)),
                i.abs_diff(at),
            )
        })
}

fn combat_cadence(
    character: mmx_core::Character,
    mut source: u16,
    tick: usize,
    stalled: u16,
) -> u16 {
    // Vile inherits one held source charge. The shared attack remapper emits
    // its single rising-edge blast; never synthesize a rocket firing rhythm.
    if character == mmx_core::Character::Vile {
        return source;
    }
    if source & BTN_Y != 0 || stalled >= 60 {
        let firing = match character {
            mmx_core::Character::Zero | mmx_core::Character::Sigma => tick % 32 < 16,
            _ if stalled >= 60 => tick % 200 < 180,
            _ => true,
        };
        if firing {
            source |= BTN_Y;
        } else {
            source &= !BTN_Y;
        }
    }
    source
}

fn face_enemy(source: u16, x: u16, enemy_x: u16, facing_right: bool) -> u16 {
    let mut pad = source & !(BTN_LEFT | BTN_RIGHT);
    if enemy_x < x && facing_right {
        pad |= BTN_LEFT;
    } else if enemy_x > x && !facing_right {
        pad |= BTN_RIGHT;
    }
    pad
}

fn steer(route: &[Point], state: &mut Follower, x: u16, y: u16, action: u8, limit: usize) -> u16 {
    let limit = limit.min(route.len() - 1);
    state.at = state.at.min(limit);
    state.ticks += 1;
    state.stalled = if x.abs_diff(state.last_x) <= 1 {
        state.stalled.saturating_add(1)
    } else {
        0
    };
    state.last_x = x;
    // Skip already reached waypoints, bounded so a coincident later platform
    // cannot jump the controller past a planned vertical section.
    for _ in 0..8 {
        let p = route[state.at];
        if !compatible_action(p.action) && state.at < limit {
            state.at += 1;
            continue;
        }
        let next = route[(state.at + 1).min(limit)];
        let passed = (p.pad & BTN_RIGHT != 0 && next.x > p.x && x >= p.x)
            || (p.pad & BTN_LEFT != 0 && next.x < p.x && x <= p.x);
        if state.at < limit && (x.abs_diff(p.x) <= 10 || passed) && y.abs_diff(p.y) <= 28 {
            state.at += 1;
            // Preserve stationary combat/charge timing instead of fast-forwarding
            // a whole boss fight merely because all waypoints share a position.
            if route[state.at].x == p.x && route[state.at].y == p.y {
                break;
            }
        } else {
            break;
        }
    }
    let current = route[state.at];
    let ahead = (state.at + 18).min(limit);
    let target = route[ahead];
    let dx = i32::from(target.x) - i32::from(x);
    let mut pad = current.pad & !(BTN_START | BTN_SELECT | BTN_LEFT | BTN_RIGHT | BTN_B);
    if dx > 4 {
        pad |= BTN_RIGHT;
    } else if dx < -4 {
        pad |= BTN_LEFT;
    } else {
        pad |= current.pad & (BTN_LEFT | BTN_RIGHT);
    }
    // Upcoming source jump or an actual stalled ledge requests a full-height
    // jump. A release gap guarantees another button edge after an interruption.
    let grounded = matches!(action, 0x00 | 0x02 | 0x04 | 0x0A);
    let planned_jump = route[state.at..=ahead].iter().any(|p| p.pad & BTN_B != 0);
    let above = i32::from(y) - i32::from(target.y) > 18;
    // Keep attempting actual terrain obstacles. Live Bee fights override this
    // with their slower aimed charge/jump cadence in the caller.
    let stuck = state.stalled >= 24 && dx.abs() > 8;
    let on_wall = action == 0x12;
    if (grounded || on_wall)
        && (planned_jump || above || stuck || on_wall)
        && state.ticks >= state.jump_after
    {
        state.jump_until = state.ticks + 20;
        state.jump_after = state.ticks + 32;
    }
    // The Highway's repeated broken-road approaches need a committed takeoff.
    // Use DemoBots' verified geometry and keep moving through the approach.
    if (1100..2500).contains(&x) && (x - 1100) % 640 <= 200 {
        pad = (pad & !BTN_LEFT) | BTN_RIGHT;
        if x >= 1195 && (x - 1195) % 640 <= 61 && grounded && state.ticks >= state.jump_after {
            state.jump_until = state.ticks + 28;
            state.jump_after = state.ticks + 44;
        }
    }
    if state.ticks < state.jump_until {
        pad |= BTN_B;
    }
    // Holding Y maintains charge while stalled, but a repeated source attack
    // loop eventually releases it even when the live fight takes longer.
    pad
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_waypoint_and_corrects_direction_without_pause() {
        let route = vec![
            Point {
                x: 100,
                y: 100,
                pad: BTN_START | BTN_RIGHT | BTN_Y,
                action: 0
            };
            100
        ];
        let mut s = Follower::default();
        let pad = steer(&route, &mut s, 150, 100, 0, 99);
        assert_eq!(s.at, 0);
        assert_eq!(pad & (BTN_LEFT | BTN_RIGHT | BTN_START), BTN_LEFT);
        assert_ne!(pad & BTN_Y, 0);
        steer(&route, &mut s, 100, 100, 0, 99);
        assert_eq!(s.at, 1);
    }
    #[test]
    fn stalled_ledge_jump_has_a_release_before_retry() {
        let route = vec![
            Point {
                x: 200,
                y: 80,
                pad: BTN_RIGHT,
                action: 0
            };
            100
        ];
        let mut s = Follower::default();
        assert_ne!(steer(&route, &mut s, 100, 100, 0, 99) & BTN_B, 0);
        for _ in 0..20 {
            steer(&route, &mut s, 100, 100, 0, 99);
        }
        assert_eq!(steer(&route, &mut s, 100, 100, 0, 99) & BTN_B, 0);
    }
    #[test]
    fn follower_cannot_advance_past_leader_delay() {
        let route = vec![
            Point {
                x: 100,
                y: 100,
                pad: BTN_RIGHT,
                action: 0
            };
            100
        ];
        let mut s = Follower::default();
        for _ in 0..8 {
            steer(&route, &mut s, 100, 100, 0, 3);
        }
        assert_eq!(s.at, 3);
    }
    #[test]
    fn reacquisition_only_chooses_waypoints_before_leader() {
        let route: Vec<_> = (0..100)
            .map(|x| Point {
                x,
                y: 100,
                pad: BTN_RIGHT,
                action: 0,
            })
            .collect();
        assert_eq!(nearest(&route, 80, 100, 40), 40);
        assert_eq!(nearest(&route, 20, 100, 40), 20);
    }
    #[test]
    fn turns_toward_a_boss_behind_then_stands_to_shoot() {
        let source = BTN_RIGHT | BTN_Y;
        assert_eq!(face_enemy(source, 3452, 3372, true), BTN_LEFT | BTN_Y);
        assert_eq!(face_enemy(source, 3452, 3372, false), BTN_Y);
    }
    #[test]
    fn landed_player_can_resume_after_an_unreached_airborne_waypoint() {
        let mut route = vec![
            Point {
                x: 1733,
                y: 305,
                pad: BTN_RIGHT,
                action: 6
            };
            30
        ];
        route[18] = Point {
            x: 1764,
            y: 351,
            pad: BTN_RIGHT,
            action: 4,
        };
        assert_eq!(ahead_reached(&route, 1764, 351, 0, 29), Some(18));
        assert_eq!(ahead_reached(&route, 1764, 351, 0, 10), None);
        assert_eq!(ahead_reached(&route, 1733, 305, 0, 29), None);
    }
    #[test]
    fn checkpoint_reanchor_matches_current_spawn_not_route_start() {
        let route = [
            Point {
                x: 128,
                y: 367,
                pad: 0,
                action: 0,
            },
            Point {
                x: 1024,
                y: 335,
                pad: BTN_RIGHT,
                action: 4,
            },
            Point {
                x: 1024,
                y: 335,
                pad: BTN_RIGHT,
                action: 0x0E,
            },
        ];
        assert_eq!(checkpoint_waypoint(&route, 1024, 335), Some(1));
        assert_eq!(checkpoint_waypoint(&route, 1024, 256), None);
    }
    #[test]
    fn vile_preserves_one_source_charge_without_periodic_firing() {
        for tick in 0..240 {
            assert_eq!(
                combat_cadence(mmx_core::Character::Vile, BTN_RIGHT | BTN_Y, tick, 100),
                BTN_RIGHT | BTN_Y
            );
        }
        assert_eq!(
            combat_cadence(mmx_core::Character::Vile, BTN_RIGHT, 100, 100),
            BTN_RIGHT
        );
    }
    #[test]
    fn hurt_resync_avoids_knockback_points_and_respects_leader_limit() {
        let mut route: Vec<_> = (0..100)
            .map(|x| Point {
                x,
                y: 100,
                pad: BTN_RIGHT,
                action: 4,
            })
            .collect();
        for p in &mut route[20..=40] {
            p.action = 0x0E;
        }
        assert_eq!(nearest_compatible(&route, 30, 100, 30, 99), Some(19));
        assert_eq!(nearest_compatible(&route, 80, 100, 30, 60), Some(60));
        assert_eq!(nearest_compatible(&route, 1000, 100, 30, 99), None);
    }
    #[test]
    fn real_rom_knockback_freezes_then_resynchronizes_inputs() {
        let Some(path) = std::env::var_os("MMX_ROM") else {
            return eprintln!("MMX_ROM not set; skipping route knockback integration");
        };
        let bytes = std::fs::read(path).expect("MMX_ROM must be readable for the integration test");
        let mut game = Game::from_rom(bytes).unwrap();
        game.set_overclock(mmx_core::Overclock::Off);
        for f in 0..2000 {
            game.step(Pads {
                p1: if f % 90 == 0 { BTN_START } else { 0 },
                p2: 0,
            });
            if gameplay(game.wram()) && game.wram()[ix(ram::X_ACTION)] == 0x00 {
                break;
            }
        }
        assert!(gameplay(game.wram()), "Highway did not start");
        // Encounter the first enemy naturally; this fixture never writes WRAM,
        // health, position or enemies to manufacture a collision.
        let mut hit = false;
        for _ in 0..600 {
            game.step(Pads {
                p1: BTN_RIGHT,
                p2: 0,
            });
            if game.wram()[ix(ram::X_ACTION)] == 0x0E {
                hit = true;
                break;
            }
        }
        assert!(hit, "no real enemy knockback occurred");
        let x = word(game.wram(), ix(ram::X_POS_X));
        let y = word(game.wram(), ix(ram::X_POS_Y));
        let route: Vec<_> = (0..300)
            .map(|i| Point {
                x: x.saturating_sub(100).saturating_add(i),
                y,
                pad: BTN_RIGHT,
                action: 4,
            })
            .collect();
        let mut playback = RoutePlayback {
            boot: vec![],
            boot_at: 0,
            route,
            followers: std::array::from_fn(|_| Follower::default()),
            delay: 8,
            started: true,
            finished: false,
            level: game.wram()[ix(ram::LEVEL_ID)],
            previous_source: [BTN_Y; 4],
            last_logged_tick: None,
            pending_respawn_reanchor: false,
            respawn_gameplay_frames: 0,
            next_attack_allowed: [0; 4],
        };
        playback.followers[0].at = 100;
        let frozen = playback.followers[0].at;
        assert_eq!(playback.next(&game), [0; 4]);
        assert_eq!(playback.followers[0].at, frozen);
        assert_eq!(playback.previous_source[0], 0);
        for _ in 0..120 {
            let pads = playback.next(&game);
            game.step(Pads {
                p1: pads[0],
                p2: pads[1],
            });
            if game.wram()[ix(ram::X_ACTION)] != 0x0E {
                break;
            }
            assert_eq!(playback.followers[0].at, frozen);
        }
        assert_ne!(
            game.wram()[ix(ram::X_ACTION)],
            0x0E,
            "hurt animation never ended"
        );
        playback.next(&game);
        assert!(!playback.followers[0].was_hurt);
        let target = playback.route[playback.followers[0].at];
        assert!(target.x.abs_diff(word(game.wram(), ix(ram::X_POS_X))) <= 96);
        assert!(compatible_action(target.action));
    }
}
