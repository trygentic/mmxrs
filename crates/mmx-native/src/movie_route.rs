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
    grounded: bool,
}

#[derive(Clone, Copy, Debug)]
struct JumpPlan {
    start: [u16; 2],
    end: [u16; 2],
    source_start: usize,
    source_end: usize,
}

#[derive(Clone, Copy, Debug)]
struct Traversal {
    plan: JumpPlan,
    begun: usize,
    airborne: bool,
    release_until: usize,
    wall_retry_after: usize,
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
    collapse_exit_x: u16,
    collapse_release_until: usize,
    collapse_retry_after: usize,
    was_hurt: bool,
    traversal: Option<Traversal>,
    completed_jumps: usize,
    landed_floor_at: usize,
    landed_floor_pos: Option<[u16; 2]>,
    boss_left_flank: Option<bool>,
    defense_until: usize,
    defense_release_until: usize,
    defense_after: usize,
    bullet_wait: Option<(usize, usize)>,
    bullet_jump_until: usize,
    bullet_jump_release_until: usize,
    staging_vehicle_direction: u16,
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
    previous_delivered: [u16; 4],
    grounded_road: [Option<(u16, i16)>; 4],
    known_gaps: Vec<JumpPlan>,
    last_logged_tick: Option<usize>,
    pending_respawn_reanchor: bool,
    respawn_gameplay_frames: u16,
    next_attack_allowed: [u64; 4],
    had_health_reservation: bool,
    intro_staging: Option<(u16, u64, u16)>,
    previous_enemy_x: [Option<u16>; 15],
    jump_plans: Vec<JumpPlan>,
    last_visible: [Option<(u16, u16, u8)>; 4],
    death_counts: [usize; 4],
    warp_counts: [usize; 4],
    #[cfg(test)]
    fixture_states: Vec<Vec<u8>>,
}

impl RoutePlayback {
    #[cfg(test)]
    pub(crate) fn outcomes(&self) -> ([usize; 4], [usize; 4], bool) {
        (self.death_counts, self.warp_counts, self.finished)
    }
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
        #[cfg(test)]
        let mut fixture_states = Vec::new();
        eprintln!("movie route: preparing first-stage solo waypoints (up to 12000 frames)");
        for pads in movie.pads.iter().take(12000).copied() {
            let w = reference.wram();
            #[cfg(test)]
            if fixture_states.len() < 5
                && word(w, ix(ram::X_POS_X)) >= [725, 1110, 2500, 3270, 3649][fixture_states.len()]
                && mmx_core::coop_stages::p2_anchor_ok(w)
            {
                fixture_states.push(reference.save_state());
            }
            if gameplay(w) {
                if route.is_empty() {
                    level = w[ix(ram::LEVEL_ID)];
                }
                route.push(Point {
                    x: word(w, ix(ram::X_POS_X)),
                    y: word(w, ix(ram::X_POS_Y)),
                    pad: pads.p1,
                    action: w[ix(ram::X_ACTION)],
                    grounded: (w[ix(ram::X_BASE) + OFF_GROUND]
                        | w[ix(ram::X_BASE) + OFF_GROUND + 1])
                        & 4
                        != 0,
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
        let mut jump_plans = recorded_jumps(&route);
        // The source run rides moving cars over these two broken roads. A
        // follower can miss that vehicle's timing, so take a real jump/wall
        // kick from the known road edge instead of chasing the departed car.
        for (start, end) in [([782, 367], [900, 335]), ([1198, 335], [1300, 351])] {
            let source_start = route
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| distance(**p, start[0], start[1]))
                .map(|(i, _)| i)
                .unwrap();
            let source_end = route
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| distance(**p, end[0], end[1]))
                .map(|(i, _)| i)
                .unwrap();
            jump_plans.push(JumpPlan {
                start,
                end,
                source_start,
                source_end,
            });
        }
        if std::env::var_os("MMX_ROUTE_CONTINUE_TRACE").is_some() {
            for bucket in 405..=420 {
                let sample = route.iter().enumerate().skip(4500).find(|(_, p)| p.x/16 == bucket && p.grounded && compatible_action(p.action));
                eprintln!("ROAD_SOURCE bucket{} grounded sample {:?}",bucket,sample);
            }
        }
        eprintln!("movie smart: {} recorded jump segments", jump_plans.len());
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
            previous_delivered: [0; 4],
            grounded_road: [None; 4],
            known_gaps: vec![],
            last_logged_tick: None,
            pending_respawn_reanchor: false,
            respawn_gameplay_frames: 0,
            next_attack_allowed: [0; 4],
            had_health_reservation: false,
            intro_staging: None,
            previous_enemy_x: [None; 15],
            jump_plans,
            last_visible: [None; 4],
            death_counts: [0; 4],
            warp_counts: [0; 4],
            #[cfg(test)]
            fixture_states,
        })
    }

    pub fn next(&mut self, game: &Game) -> [u16; 4] {
        let pads = self.next_inner(game);
        self.previous_delivered = pads;
        let w = game.wram();
        for i in 0..ram::ENEMY_COUNT {
            let at = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
            self.previous_enemy_x[i] = (w[at] != 0).then(|| word(w, at + ram::ENEMY_POS_X));
        }
        pads
    }
    fn next_inner(&mut self, game: &Game) -> [u16; 4] {
        if self.finished {
            self.previous_source = [0; 4];
            let w = game.wram();
            // The source movie holds Start through the intro's beam-out and
            // ending handoff. Stop immediately when stage select is entered.
            let confirm = if w[ix(ram::MODE_D2)] == ram::D2_IN_STAGE && w[ix(ram::STAGE_PHASE)] >= 8 {
                mmx_core::BTN_START
            } else { dialog_confirm(w, game.frame_count()) };
            return [confirm, 0, 0, 0];
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
                if w[ix(ram::MODE_D2)] == 0x0a {
                    eprintln!("movie smart: native game over; first-stage clear was not reached");
                }
                eprintln!(
                    "movie route: control ended in mode {:#04x}, phase {:#04x}; completing native menu handoff",
                    w[ix(ram::MODE_D2)],
                    w[ix(ram::STAGE_PHASE)]
                );
                eprintln!(
                    "movie smart outcomes: deaths={:?}, warps={:?}",
                    self.death_counts, self.warp_counts
                );
            } else if w[ix(ram::STAGE_PHASE)] == 6 {
                if !self.pending_respawn_reanchor {
                    self.death_counts[0] += 1;
                    eprintln!(
                        "movie smart: P1 death frame{} at ({},{}), camera=({},{}), action={:#x}, HP={}, counts {:?}",
                        game.frame_count(),
                        word(w, ix(ram::X_POS_X)),
                        word(w, ix(ram::X_POS_Y)),
                        word(w,ix(ram::CAMERA_X)),word(w,ix(ram::CAMERA_Y)),w[ix(ram::X_ACTION)],w[ix(ram::X_HP)]&0x7f,
                        self.death_counts
                    );
                }
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
            self.last_visible[0] = None; // Native scripted carry/plane alignment.
            self.previous_source = [0; 4];
            // Dialog boxes still read P1's pad while actor movement is locked.
            // A normal confirm pulse advances text without moving companions.
            return [dialog_confirm(w, game.frame_count()), 0, 0, 0];
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
        for slot in 0..4 {
            match (self.last_visible[slot], positions[slot]) {
                (Some((x, y, _)), None) if slot > 0 => {
                    self.death_counts[slot] += 1;
                    let nearby: Vec<_> = (0..ram::ENEMY_COUNT)
                        .filter_map(|i| {
                            let b = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                            (w[b + ram::ENEMY_ACTIVE] != 0
                                && x.abs_diff(word(w, b + ram::ENEMY_POS_X)) < 160
                                && y.abs_diff(word(w, b + ram::ENEMY_POS_Y)) < 128)
                                .then(|| {
                                    (
                                        w[b + ram::ENEMY_ID],
                                        w[b + ram::ENEMY_HP] & 0x7f,
                                        word(w, b + ram::ENEMY_POS_X),
                                        word(w, b + ram::ENEMY_POS_Y),
                                        game.object_contact_bounds(b as u16),
                                    )
                                })
                        })
                        .collect();
                    eprintln!(
                        "movie smart: P{} fatal nearby[id,hp,x,y,bounds]={nearby:?}",
                        slot + 1
                    );
                    eprintln!(
                        "movie smart: P{} down frame{} previous=({x},{y}), raw[x,y,hp,action,vy]={:?}, camera=({},{}), counts {:?}",
                        slot+1,game.frame_count(),game.player_physics_state(slot),
                        word(w,ix(ram::CAMERA_X)),word(w,ix(ram::CAMERA_Y)),self.death_counts
                    );
                }
                (Some((x, y, _)), Some((nx, ny, _)))
                    if x.abs_diff(nx) > 64 || y.abs_diff(ny) > 64 =>
                {
                    self.warp_counts[slot] += 1;
                    eprintln!(
                        "movie smart: P{} ROM warp/discontinuity ({x},{y})->({nx},{ny}), counts {:?}",
                        slot + 1,
                        self.warp_counts
                    );
                }
                _ => {}
            }
            self.last_visible[slot] = positions[slot];
        }
        let live_ground = [
            (w[ix(ram::X_BASE) + OFF_GROUND] | w[ix(ram::X_BASE) + OFF_GROUND + 1]) & 4 != 0,
            game.visible_p2_object().is_some_and(on_ground),
            game.visible_extra_player_object(0).is_some_and(on_ground),
            game.visible_extra_player_object(1).is_some_and(on_ground),
        ];
        for slot in 0..4 {
            if let (Some((x, y, _)), Some(off)) =
                (positions[slot], game.player_floor_probe_offset(slot))
            {
                if x >= 3900 && live_ground[slot] {
                    self.grounded_road[slot] = Some((y, off));
                }
                if x >= 3900 {
                    if let Some((road_y, offset)) = self.grounded_road[slot] {
                        if let Some(plan) = live_road_jump(
                            game,
                            &self.route,
                            self.followers[slot].at,
                            x,
                            road_y,
                            true,
                            Some(offset),
                        ) {
                            if let Some(known) = self.known_gaps.iter_mut().find(|p| {
                                p.end[0].abs_diff(plan.end[0]) <= 16
                                    && p.end[1].abs_diff(plan.end[1]) <= 16
                            }) {
                                if plan.start[0] < known.start[0] {
                                    *known = plan;
                                }
                            } else {
                                self.known_gaps.push(plan);
                            }
                        }
                    }
                }
            }
        }
        if self.had_health_reservation {
            let available = (0..16).any(|i| {
                let base = 0x1628 + i * 0x30;
                w[base] != 0 && w[base + 0x0a] == 2 && w[base + 1] != 4
                    && w[base + 2] != 6 && w[base + 0x0b] & 0x7f == 0
                    && word(w, base + 5) >= 5800
            });
            if !available {
                self.had_health_reservation = false;
                for slot in 0..4 {
                    if let Some((x, y, _)) = positions[slot] {
                        let f = &mut self.followers[slot];
                        if let Some(at) = nearest_compatible(&self.route, x, y, f.at, self.route.len()-1) {
                            f.at = at;
                            f.landed_floor_at = at;
                            f.landed_floor_pos = Some([x,y]);
                            f.jump_until = 0;
                            f.jump_after = 0;
                            f.stalled = 0;
                            eprintln!("movie smart: P{} resumed actual pickup position ({x},{y}) at waypoint {at}",slot+1);
                        }
                    }
                }
            }
        }
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
        if self.followers[0].ticks % 300 == 0 && (2560..3580).contains(&word(w, ix(ram::X_POS_X))) {
            let dead: Vec<_> = (0..ram::ENEMY_COUNT)
                .filter_map(|i| {
                    let b = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                    (w[b] != 0 && w[b + ram::ENEMY_ID] == 0x22 && w[b + ram::ENEMY_HP] & 0x7f == 0)
                        .then(|| {
                            (
                                i,
                                word(w, b + 5),
                                word(w, b + 8),
                                w[b + 1],
                                w[b + 2],
                                w[b + 0x0b],
                                w[b + 0x2b],
                                w[b + 0x2a],
                            )
                        })
                })
                .collect();
            if !dead.is_empty() {
                eprintln!("movie smart dead wreck[slot,x,y,state1,state2,variant,flags2b,flags2a]={dead:?}, cameraY={},bottom={}",word(w,ix(ram::CAMERA_Y)),word(w,0x1e5c));
            }
        }
        let falling_wreck = (0..ram::ENEMY_COUNT)
            .any(|slot| main_wreck_falling(w, ix(ram::ENEMIES_BASE) + slot * ram::ENEMY_STRIDE));
        if falling_wreck
            || positions
                .iter()
                .flatten()
                .any(|(x, y, _)| ((2450..2820).contains(x) || (3150..3520).contains(x)) && *y > 400)
        {
            for slot in 0..4 {
                if let Some((x, _, _)) = positions[slot] {
                    let exit = if (2450..2820).contains(&x) {
                        2816
                    } else if (3150..3520).contains(&x) {
                        3504
                    } else {
                        0
                    };
                    if exit != 0 {
                        self.followers[slot].recovering_collapse = true;
                        self.followers[slot].collapse_exit_x = exit;
                        self.followers[slot].traversal = None;
                    }
                }
            }
        }
        let enemies: Vec<_> = (0..ram::ENEMY_COUNT)
            .filter_map(|slot| {
                let at = ix(ram::ENEMIES_BASE) + slot * ram::ENEMY_STRIDE;
                (w[at + ram::ENEMY_ACTIVE] != 0 && w[at + ram::ENEMY_HP] & 0x7f != 0).then(|| {
                    (
                        word(w, at + ram::ENEMY_POS_X),
                        word(w, at + ram::ENEMY_POS_Y),
                        at as u16,
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
        let mut decision_trace = [None; 4];
        let mut pads = std::array::from_fn(|slot| {
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
            if !self.followers[slot].recovering_collapse
                && limit >= self.followers[slot].landed_floor_at
                && self.followers[slot].at < self.followers[slot].landed_floor_at
                && self.followers[slot]
                    .landed_floor_pos
                    .is_some_and(|p| x.abs_diff(p[0]) <= 48 && y.abs_diff(p[1]) <= 64)
            {
                self.followers[slot].at = self.followers[slot].landed_floor_at;
            }
            if slot > 0
                && limit < self.followers[slot].landed_floor_at
                && !self.followers[slot].recovering_collapse
                && self.followers[slot]
                    .landed_floor_pos
                    .is_some_and(|p| x.abs_diff(p[0]) <= 48 && y.abs_diff(p[1]) <= 64)
            {
                // A successful follower jump can land ahead of its delayed
                // source cursor. Wait safely there; never walk back into the
                // gap just to regain the requested movie-frame spacing.
                self.previous_source[slot] = 0;
                return 0;
            }
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
                if x >= 4600 {
                    if let Some(traversal) = &mut follower.traversal {
                        // Knockback consumes the old crossing deadline while
                        // inputs are frozen. Resume the actual wall recovery
                        // with a fresh native jump edge instead of timing out.
                        traversal.begun = follower.ticks;
                        traversal.release_until = follower.ticks + 1;
                        traversal.wall_retry_after = follower.ticks;
                        if y > traversal.plan.end[1].saturating_add(24) {
                            if let Some(next) = self.known_gaps.iter()
                                .filter(|p| p.start[0] >= x && p.start[0].saturating_sub(x) <= 160
                                    && p.end[0] > traversal.plan.end[0].saturating_add(16))
                                .min_by_key(|p| p.start[0])
                            {
                                traversal.plan.end = next.end;
                                traversal.plan.source_end = next.source_end;
                                eprintln!("movie smart: P{} resumed wall recovery toward {:?}",slot+1,next.end);
                            }
                        }
                    }
                }
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
            if std::env::var_os("MMX_ROUTE_CONTINUE_TRACE").is_some() && x >= 6300 && self.followers[slot].ticks % 60 == 0 {
                let from = self.followers[slot].at;
                let offset = game.player_floor_probe_offset(slot);
                let floors: Vec<_> = offset.map(|off| (0..=64).step_by(16).map(|dx| game.terrain_flat_floor_y(i32::from(x)+dx,i32::from(y)+i32::from(off)+1)).collect()).unwrap_or_default();
                let candidate = (from+1..=(from+1800).min(limit)).find_map(|i| {
                    let p = self.route[i];
                    (p.grounded && compatible_action(p.action) && p.pad & BTN_RIGHT != 0 && p.x >= x.saturating_add(16) && p.x <= x.saturating_add(96)).then_some((i,p))
                });
                eprintln!("ROAD_CONTINUE P{} pos({x},{y}) from{from} limit{limit} ground{} stalled{} last_x{} enemies{} offset{offset:?} floors{floors:?} candidate{candidate:?}",slot+1,live_ground[slot],self.followers[slot].stalled,self.followers[slot].last_x,enemies.len());
            }
            if x >= 6300 && (enemies.is_empty() || x >= 6900) && live_ground[slot]
                && (x >= 6900 || x >= self.route[self.followers[slot].at].x.saturating_add(8))
                && game.player_floor_probe_offset(slot).is_some_and(|off| {
                    let feet = i32::from(y) + i32::from(off) + 1;
                    (0..=64).step_by(16).all(|dx| flat_floor(game, i32::from(x) + dx, feet))
                })
            {
                let from = self.followers[slot].at;
                if let Some(at) = (from+1..=(from+1800).min(limit))
                    .find(|&i| {
                        let p = self.route[i];
                        p.grounded && compatible_action(p.action) && p.pad & BTN_RIGHT != 0
                            && p.x >= x.saturating_add(16) && p.x <= x.saturating_add(96)
                            && p.y.abs_diff(y) <= 32
                    })
                {
                    let f = &mut self.followers[slot];
                    f.at = at;
                    f.stalled = 0;
                    f.jump_until = 0;
                    f.jump_after = 0;
                    eprintln!("movie smart: P{} continued empty safe road from ({x},{y}), waypoint {from}->{at}",slot+1);
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
            if (2450..2820).contains(&x) && y > 400 {
                self.followers[slot].recovering_collapse = true;
                self.followers[slot].collapse_exit_x = 2816;
                self.followers[slot].traversal = None;
            }
            if (3150..3520).contains(&x) && y > 400 {
                self.followers[slot].recovering_collapse = true;
                self.followers[slot].collapse_exit_x = 3504;
                self.followers[slot].traversal = None;
            }
            if self.followers[slot].recovering_collapse
                && live_ground[slot]
                && x >= self.followers[slot].collapse_exit_x
                && y <= 390
            {
                self.followers[slot].recovering_collapse = false;
                let from = self.followers[slot].at;
                let to = (from + 1800).min(self.route.len() - 1);
                let at = from + nearest(&self.route[from..=to], x, y, to - from);
                self.followers[slot].at = at;
                self.followers[slot].landed_floor_at = at;
                self.followers[slot].landed_floor_pos = Some([x, y]);
            }
            let mut source = steer(&self.route, &mut self.followers[slot], x, y, action, limit);
            let near_bee = bees
                .iter()
                // Entering ships hover above the viewport until the leader
                // crosses their descent trigger. Approach normally then;
                // stopping early would leave the encounter suspended forever.
                .filter(|(ex, ey, _)| {
                    x.abs_diff(*ex) < 320 && *ey >= word(w, ix(ram::CAMERA_Y)).saturating_add(32)
                })
                .min_by_key(|(ex, _, _)| x.abs_diff(*ex));
            if near_bee.is_some()
                && self.followers[slot]
                    .traversal
                    .is_some_and(|t| t.plan.start[0] >= 1400)
            {
                self.followers[slot].traversal = None;
            }
            let traversal_pad = if near_bee.is_some() || self.followers[slot].recovering_collapse {
                None
            } else {
                let live_plan = live_road_jump(
                    game,
                    &self.route,
                    self.followers[slot].at,
                    x,
                    y,
                    live_ground[slot],
                    game.player_floor_probe_offset(slot),
                );
                let essential: Vec<_> = self
                    .jump_plans
                    .iter()
                    .copied()
                    .filter(|p| p.start[0] < 1400)
                    .chain(self.known_gaps.iter().copied())
                    .collect();
                let plans = live_plan.as_slice();
                let planned = if plans.is_empty() {
                    essential.as_slice()
                } else {
                    plans
                };
                let safe_wait_floor = x < 4700 || game.player_floor_probe_offset(slot).is_some_and(|off| {
                    flat_floor(game, i32::from(x), i32::from(y) + i32::from(off) + 1)
                });
                let waiting_party = self.followers[slot].traversal.is_none()
                    && live_ground[slot]
                    && safe_wait_floor
                    && planned.iter().any(|p| {
                        p.end[0].saturating_sub(p.start[0]) >= 80
                            && x >= p.start[0].saturating_sub(12)
                            && x <= p.start[0].saturating_add(8)
                            && positions
                                .iter()
                                .flatten()
                                .any(|(px, _, _)| *px < p.start[0].saturating_sub(64))
                    });
                if waiting_party {
                    Some(0)
                } else if self.followers[slot].traversal.is_none()
                    && live_plan.is_some_and(|p| x < p.start[0].saturating_sub(4))
                {
                    Some(BTN_RIGHT)
                } else {
                    committed_jump(
                        planned,
                        &mut self.followers[slot],
                        x,
                        y,
                        action,
                        live_ground[slot],
                        self.previous_delivered[slot] & BTN_B != 0,
                        game.player_floor_probe_offset(slot).is_some_and(|off| {
                            flat_floor(game, i32::from(x), i32::from(y) + i32::from(off) + 1)
                        }),
                    )
                }
            };
            // The solo TAS leaps across the first Bee Blader's road. Extra
            // co-op damage can collapse it before that leap, putting the live
            // party on the lower floor absent from the reference trajectory.
            // Recover via the room's right wall using normal wall-kick inputs.
            let collapse_recovery = self.followers[slot].recovering_collapse;
            if collapse_recovery {
                source = (source & !(BTN_LEFT | BTN_RIGHT | BTN_B)) | BTN_RIGHT;
                // Hold jump through ascent. A wall slide or stalled ground
                // needs a release before another native takeoff can occur.
                let state = &mut self.followers[slot];
                if (action == 0x12 || (live_ground[slot] && state.stalled >= 8))
                    && state.ticks >= state.collapse_retry_after
                {
                    state.collapse_release_until = state.ticks + 1;
                    state.collapse_retry_after = state.ticks + 18;
                }
                if state.ticks >= state.collapse_release_until {
                    source |= BTN_B;
                }
            }
            let character = game
                .player_character(slot)
                .unwrap_or(mmx_core::Character::X);
            if let Some(movement) = traversal_pad {
                source = (source & !(BTN_LEFT | BTN_RIGHT | BTN_B)) | movement;
            }
            let prepare_gap = x >= 3900
                && self.followers[slot].traversal.is_none()
                && !live_ground[slot]
                && self.grounded_road[slot].is_some_and(|(road_y, off)| {
                    flat_floor(game, i32::from(x), i32::from(road_y) + i32::from(off) + 1)
                        && self.known_gaps.iter().any(|p| {
                            p.start[0] >= x
                                && p.start[0].saturating_sub(x) <= 64
                                && p.start[1].abs_diff(road_y) <= 16
                        })
                });
            if prepare_gap {
                // An early source combat hop is not a new airborne takeoff.
                // Brake over the still-supported road and land before jumping
                // from the measured edge with a fresh native B press.
                source &= !(BTN_LEFT | BTN_RIGHT | BTN_B);
                source |= if self.followers[slot].ticks % 2 == 0 {
                    BTN_LEFT
                } else {
                    BTN_RIGHT
                };
            }
            source = combat_cadence(
                character,
                source,
                self.followers[slot].ticks,
                self.followers[slot].stalled,
            );
            if let Some((enemy_x, _, _)) = near_bee {
                let left = *self.followers[slot]
                    .boss_left_flank
                    .get_or_insert(x <= *enemy_x);
                let distance = 80;
                let bounds = (0..ram::ENEMY_COUNT).find_map(|slot| {
                    let at = ix(ram::ENEMIES_BASE) + slot * ram::ENEMY_STRIDE;
                    (w[at + ram::ENEMY_ACTIVE] != 0
                        && w[at + ram::ENEMY_ID] == 0x22
                        && word(w, at + ram::ENEMY_POS_X) == *enemy_x)
                        .then(|| game.object_contact_bounds(at as u16))
                        .flatten()
                });
                let target = if left {
                    bounds
                        .map_or_else(
                            || enemy_x.saturating_sub(128),
                            |b| (b[0] - distance).clamp(0, 65535) as u16,
                        )
                        .max(word(w, ix(ram::CAMERA_X)).saturating_add(24))
                } else {
                    bounds.map_or_else(
                        || enemy_x.saturating_add(128),
                        |b| (b[2] + distance).clamp(0, 65535) as u16,
                    )
                };
                source &= !(BTN_LEFT | BTN_RIGHT | BTN_B | BTN_Y);
                if x.abs_diff(target) > 8 {
                    source |= if x < target { BTN_RIGHT } else { BTN_LEFT };
                } else {
                    source = face_enemy(
                        source,
                        x,
                        *enemy_x,
                        facings[slot].is_some_and(|f| f & 0x40 != 0),
                    );
                }
                let tick = self.followers[slot].ticks;
                let firing = match character {
                    mmx_core::Character::X => tick % 200 < 180,
                    mmx_core::Character::Vile => tick % 180 == 0,
                    _ => tick % 64 < 32,
                };
                if firing {
                    source |= BTN_Y;
                }
                let jumping = match character {
                    mmx_core::Character::X => (160..184).contains(&(tick % 200)),
                    mmx_core::Character::Vile => (156..180).contains(&(tick % 180)),
                    _ => false,
                };
                if jumping {
                    source |= BTN_B;
                }
            } else {
                self.followers[slot].boss_left_flank = None;
            }
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
                    near_bee.is_none()
                        && !collapse_recovery
                        && traversal_pad.is_none()
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
                // it. X jumps before charge release at tick180; Vile times
                // one large shoulder blast at the top of his jump.
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
            let mut rear_driver_aim = None;
            let actor_box = game.player_contact_bounds(slot).unwrap_or([
                i32::from(x) - 8,
                i32::from(y) - 16,
                i32::from(x) + 9,
                i32::from(y) + 17,
            ]);
            if near_bee.is_none() && !collapse_recovery && traversal_pad.is_none() {
                let ordinary = enemies
                    .iter()
                    .filter_map(|(ex, ey, base)| {
                        let b = usize::from(*base);
                        if matches!(w[b + ram::ENEMY_ID], 0x12 | 0x19 | 0x1a | 0x22 | 0x32)
                            || (x >= 5500 && w[b + ram::ENEMY_ID] == 0x36)
                        {
                            // The late mobile chaser follows from behind.
                            // Retreating to its left stand-off runs into it.
                            return None;
                        }
                        let bounds = game.object_contact_bounds(*base)?;
                        let dx = i32::from(*ex) - i32::from(x);
                        (dx >= if x>=7200 && w[b+ram::ENEMY_ID]==0x11 {-240} else {-24}
                            && dx < 180 && i32::from(y).abs_diff(i32::from(*ey)) < 96)
                            .then_some((*ex, *ey, bounds, w[b + ram::ENEMY_ID], *base))
                    })
                    .min_by_key(|(ex, _, _, _, _)| x.abs_diff(*ex));
                if let Some((ex, _, b, id, base)) = ordinary {
                    let extent = actor_box[2] - i32::from(x);
                    let target = if x >= 5700 && id == 0x11 && x > ex {
                        // Keep the reached side of a late moving vehicle.
                        // Its left stand-off must not pull the party backward
                        // across the already cleared road hole.
                        b[2] + (i32::from(x) - actor_box[0]) + 64
                    } else {
                        (b[0] - extent - 64).max(i32::from(word(w, ix(ram::CAMERA_X))) + 16)
                    };
                    source &= !(BTN_LEFT | BTN_RIGHT | BTN_B | BTN_Y);
                    if i32::from(x) < target - 6 {
                        source |= BTN_RIGHT;
                    } else if i32::from(x) > target + 6 {
                        source |= BTN_LEFT;
                    } else {
                        source =
                            face_enemy(source, x, ex, facings[slot].is_some_and(|f| f & 0x40 != 0));
                    }
                    let muzzle = i32::from(y) - 19;
                    let vehicle_weakspot = (x >= 6900 && id == 0x11)
                        .then(|| game.object_contact_bounds_with_box(base, word(w,usize::from(base)+0x38)))
                        .flatten();
                    let weapon_box = vehicle_weakspot.unwrap_or(b);
                    let radius = if vehicle_weakspot.is_some() && matches!(character,mmx_core::Character::X|mmx_core::Character::Vile) {20} else {12};
                    let aligned = weapon_box[1] <= muzzle + radius && weapon_box[3] >= muzzle - radius;
                    match character {
                        mmx_core::Character::X => {
                            if !aligned || self.followers[slot].ticks % 8 < 4 {
                                source |= BTN_Y;
                            }
                        }
                        mmx_core::Character::Vile => {
                            if aligned
                                && game.frame_count() >= self.next_attack_allowed[slot]
                                && self.previous_source[slot] & BTN_Y == 0
                            {
                                source |= BTN_Y;
                            }
                        }
                        _ => {
                            if self.followers[slot].ticks % 32 < 16 {
                                source |= BTN_Y;
                            }
                        }
                    }
                    if x>=7200 && id==0x11 && ex<x && aligned
                        && matches!(character,mmx_core::Character::X|mmx_core::Character::Vile)
                    {
                        // Finish the real rear driver so its moving car cannot
                        // chase the weaker teammate into a landing. Turn first,
                        // then shoot its low native weakspot from the road.
                        rear_driver_aim=Some(ex);
                        let wrong_facing=facings[slot].is_some_and(|f|f&0x40!=0);
                        source=face_enemy(source,x,ex,wrong_facing);
                        source&=!BTN_B;
                        if wrong_facing {source&=!BTN_Y;}
                    }
                    if b[3] < muzzle - 12
                        && live_ground[slot]
                        && (id != 0x0f || b[0] - actor_box[2] >= 48)
                    {
                        let f = &mut self.followers[slot];
                        if f.ticks >= f.defense_after {
                            f.defense_release_until =
                                f.ticks + usize::from(self.previous_delivered[slot] & BTN_B != 0);
                            f.defense_until = f.ticks + 24;
                            f.defense_after = f.ticks + 40;
                        }
                    }
                }
            }
            if !collapse_recovery && traversal_pad.is_none() {
                let overhead = enemies.iter().any(|(_, _, base)| {
                    let at = usize::from(*base);
                    w[at + ram::ENEMY_ID] == 0x0f
                        && game.object_contact_bounds(*base).is_some_and(|b| {
                            b[2] > actor_box[0] - 96
                                && b[0] < actor_box[2] + 96
                                && b[3] <= actor_box[1]
                        })
                });
                let future = self.route[(self.followers[slot].at + 18).min(limit)];
                let terrain_rise = future.grounded && y > future.y.saturating_add(8);
                if overhead && !terrain_rise {
                    source &= !BTN_B;
                }
                let imminent = enemies.iter().any(|(_, _, base)| {
                    let at = usize::from(*base);
                    if matches!(w[at + ram::ENEMY_ID], 0x1a | 0x32) {
                        return false;
                    }
                    let ordinary = game.object_contact_bounds(*base);
                    let attack = if w[at + ram::ENEMY_ID] == 0x27 && w[at + 1] == 4 {
                        game.object_contact_bounds_with_box(*base, 0xcd5c)
                    } else {
                        ordinary
                    };
                    [ordinary, attack].into_iter().flatten().any(|b| {
                        b[2] > actor_box[0] - 64
                            && b[0] < actor_box[2] + 64
                            && b[3] > actor_box[1] - 4
                            && b[1] < actor_box[3] + 4
                    })
                });
                let f = &mut self.followers[slot];
                if imminent && !overhead && live_ground[slot] && f.ticks >= f.defense_after {
                    f.defense_release_until =
                        f.ticks + usize::from(self.previous_delivered[slot] & BTN_B != 0);
                    f.defense_until = f.ticks + 24;
                    f.defense_after = f.ticks + 40;
                }
                if f.ticks < f.defense_until {
                    source &= !BTN_B;
                    if f.ticks >= f.defense_release_until {
                        source |= BTN_B;
                    }
                }
            }
            let short_transfer = self.followers[slot]
                .traversal
                .is_some_and(|t| t.plan.end[0].saturating_sub(t.plan.start[0]) < 80);
            if traversal_pad.is_some() && short_transfer {
                let aligned_foe = enemies.iter().any(|(ex, _, base)| {
                    let id = w[usize::from(*base) + ram::ENEMY_ID];
                    !matches!(id, 0x11 | 0x12 | 0x22)
                        && x.abs_diff(*ex) < 180
                        && game
                            .object_contact_bounds(*base)
                            .is_some_and(|b| b[1] <= i32::from(y) - 3 && b[3] >= i32::from(y) - 35)
                });
                if aligned_foe {
                    source &= !BTN_Y;
                    match character {
                        mmx_core::Character::Vile => {
                            if game.frame_count() >= self.next_attack_allowed[slot]
                                && self.previous_source[slot] & BTN_Y == 0
                            {
                                source |= BTN_Y;
                            }
                        }
                        _ => {
                            if self.followers[slot].ticks % 8 < 4 {
                                source |= BTN_Y;
                            }
                        }
                    }
                }
            }
            if traversal_pad.is_some() && !short_transfer {
                source &= !BTN_Y;
                // Releasing X's existing charge would fire at the moving
                // platforms; retain that held button until safe landing.
                if character == mmx_core::Character::X {
                    source |= self.previous_source[slot] & BTN_Y;
                }
            }
            if x >= 5500 && source & BTN_LEFT != 0 && traversal_pad.is_none()
                && self.grounded_road[slot].is_some_and(|(road_y, off)| {
                    let feet = i32::from(road_y) + i32::from(off) + 1;
                    flat_floor(game, i32::from(x), feet)
                        && !flat_floor(game, i32::from(x) - 32, feet)
                })
            {
                // A moving foe can pull its stand-off target behind a cleared
                // hole. Stay on the verified road edge and face the foe for
                // fire instead of retreating into that hole.
                source &= !(BTN_LEFT | BTN_RIGHT);
                if let Some((ex, _, _)) = enemies.iter().filter(|(ex, _, _)| x.abs_diff(*ex) < 180)
                    .min_by_key(|(ex, _, _)| x.abs_diff(*ex))
                {
                    source = face_enemy(source, x, *ex, facings[slot].is_some_and(|f| f & 0x40 != 0));
                }
            }
            if x >= 4600 && character == mmx_core::Character::Vile
                && matches!(action, 0x06 | 0x08 | 0x10 | 0x12)
            {
                // Native shot $0B state8 is an undamageable, short-lived
                // explosion. Keep the climbing body outside its true box;
                // once it expires or the player rises clear, resume Right.
                if let Some(body) = game.player_contact_bounds(slot) {
                    let burst = (0..8).find_map(|i| {
                        let base = 0x1428 + i * 0x40;
                        (w[base] != 0 && w[base + 0x0a] == 0x0b && w[base + 1] == 8)
                            .then(|| game.object_contact_bounds(base as u16)).flatten()
                            .filter(|b| body[2] + 24 >= b[0] && body[0] - 24 <= b[2]
                                && body[3] + 16 >= b[1] && body[1] - 16 <= b[3])
                    });
                    if let Some(b) = burst {
                        source &= !(BTN_LEFT | BTN_RIGHT);
                        source |= if body[0] + body[2] < b[0] + b[2] {
                            BTN_LEFT
                        } else {
                            BTN_RIGHT
                        };
                    }
                }
            }
            if prepare_gap {
                source &= !(BTN_LEFT | BTN_RIGHT | BTN_B);
                source |= if self.followers[slot].ticks % 2 == 0 {
                    BTN_LEFT
                } else {
                    BTN_RIGHT
                };
            }
            if x >= 7200 && character == mmx_core::Character::Sigma && traversal_pad.is_none() && self.intro_staging.is_none()
                && game.player_floor_probe_offset(slot).is_some_and(|off| {
                    (0..=64).step_by(8).any(|dy| flat_floor(game,i32::from(x),i32::from(y)+i32::from(off)+1+dy))
                })
            {
                let rear_vehicle=(0..ram::ENEMY_COUNT).any(|i| {
                    let base=ix(ram::ENEMIES_BASE)+i*ram::ENEMY_STRIDE;
                    w[base]!=0 && matches!(w[base+ram::ENEMY_ID],0x11|0x12) && w[base+0x26]!=0
                        && game.object_contact_bounds(base as u16).is_some_and(|b| {
                            b[2]>=actor_box[0]-96 && (b[0]+b[2])/2<i32::from(x)
                                && b[3]>i32::from(self.grounded_road[slot].map_or(y,|(ry,_)|ry))-16
                        })
                });
                let f = &mut self.followers[slot];
                let rear_bullet = (0..8).any(|i| {
                    let base=0x1428+i*0x40;
                    w[base]!=0 && w[base+0x0a]==0x0a
                        && game.object_contact_bounds(base as u16).is_some_and(|b| {
                            b[2]>=actor_box[0]-128 && b[0]<actor_box[0]
                                && b[1]>=actor_box[1]-32 && b[3]<=actor_box[3]+32
                        })
                });
                if rear_bullet && f.ticks>=f.bullet_jump_until
                    && (grounded_action(action) || action==0x06)
                {
                    f.bullet_jump_release_until=f.ticks+usize::from(grounded_action(action) && self.previous_delivered[slot]&BTN_B!=0);
                    f.bullet_jump_until=f.ticks+28;
                    eprintln!("movie smart: Sigma full jump over approaching rear bullet at ({x},{y})");
                }
                if f.bullet_wait.is_none() && matches!(action,0x06|0x08|0x10)
                {
                    f.bullet_wait = (0..8).find_map(|i| {
                        let base=0x1428+i*0x40;
                        (w[base]!=0 && w[base+0x0a]==0x0a)
                            .then(|| game.object_contact_bounds(base as u16)).flatten()
                            .filter(|b| b[2]>=actor_box[0]-96 && b[0]<actor_box[0]
                                && actor_box[3]<b[1]-4)
                            .map(|_|(base,f.ticks+80))
                    });
                }
                if let Some((base,until))=f.bullet_wait {
                    let waiting=w[base]!=0 && w[base+0x0a]==0x0a && f.ticks<until
                        && game.object_contact_bounds(base as u16).is_some_and(|b| b[0]<=actor_box[2]+8 && b[2]>=actor_box[0]-160);
                    if waiting {
                        // A rearward driver bullet outruns normal walking.
                        // Brake over safe road while above its narrow lane,
                        // letting it pass before the natural landing.
                        source &= !(BTN_LEFT|BTN_RIGHT);
                    } else {
                        f.bullet_wait=None;
                    }
                }
                if rear_vehicle && f.bullet_wait.is_none() {
                    source=(source&!(BTN_LEFT|BTN_RIGHT))|BTN_RIGHT;
                }
                if f.ticks<f.bullet_jump_until {
                    source &= !BTN_B;
                    if f.ticks>=f.bullet_jump_release_until {source|=BTN_B;}
                }
            }
            if let Some(ex)=rear_driver_aim {
                let wrong_facing=facings[slot].is_some_and(|f|f&0x40!=0);
                source=face_enemy(source,x,ex,wrong_facing);
                source&=!BTN_B;
                if wrong_facing {source&=!BTN_Y;}
            }
            let low_driver_defense = !enemies.iter().any(|(_,_,base)| w[usize::from(*base)+ram::ENEMY_ID]==0x32)
                && enemies.iter().any(|(ex,_,base)| {
                    w[usize::from(*base)+ram::ENEMY_ID]==0x11 && x>=6900 && x.abs_diff(*ex)<240
                });
            let gathering_for_intro = positions[0].is_some_and(|(lx, _, _)| lx >= 6800)
                && positions
                    .iter()
                    .any(|p| p.is_none_or(|(px, _, _)| px < 7000))
                && !low_driver_defense;
            if gathering_for_intro {
                source &= !BTN_Y;
                if character == mmx_core::Character::X {
                    source |= self.previous_source[slot] & BTN_Y;
                }
            }
            let mut pad = crate::character_attacks::remap_movie_attacks(
                character,
                self.previous_source[slot],
                source,
            );
            self.previous_source[slot] = source;
            if gathering_for_intro
                && matches!(
                    character,
                    mmx_core::Character::Zero | mmx_core::Character::Sigma
                )
            {
                pad &= !mmx_core::BTN_X;
            }
            if ((source & BTN_B != 0 && !matches!(action, 0x06 | 0x08))
                || collapse_recovery
                || (traversal_pad.is_some() && !short_transfer))
                && matches!(
                    character,
                    mmx_core::Character::Zero | mmx_core::Character::Sigma
                )
            {
                pad &= !mmx_core::BTN_X;
            }
            if matches!(
                character,
                mmx_core::Character::Zero | mmx_core::Character::Sigma
            ) {
                // A saber swing stops the player. Do not translate distant
                // buster charging into repeated empty melee attacks which
                // leave the swordsmen behind the camera.
                let right = facings[slot].is_some_and(|f| f & 0x40 != 0);
                let reachable = enemies.iter().any(|(ex, ey, base)| {
                    let b = game.object_contact_bounds(*base).unwrap_or([
                        i32::from(*ex) - 16,
                        i32::from(*ey) - 16,
                        i32::from(*ex) + 16,
                        i32::from(*ey) + 16,
                    ]);
                    b[3] >= i32::from(y) - 64
                        && b[1] <= i32::from(y) + 16
                        && if right {
                            b[2] >= i32::from(x) - 16 && b[0] <= i32::from(x) + 75
                        } else {
                            b[0] <= i32::from(x) + 16 && b[2] >= i32::from(x) - 75
                        }
                });
                if !reachable {
                    pad &= !mmx_core::BTN_X;
                }
            }
            decision_trace[slot] = Some((prepare_gap, source));
            if self.intro_staging.is_some() && slot == 3 { pad } else {
                crate::character_attacks::enforce_attack_cooldown(
                    character, pad, game.frame_count(), &mut self.next_attack_allowed[slot],
                )
            }
        });
        if let Some(npc_x)=(0..ram::ENEMY_COUNT).find_map(|i| {
            let base=ix(ram::ENEMIES_BASE)+i*ram::ENEMY_STRIDE;
            (w[base]!=0 && w[base+ram::ENEMY_ID]==0x32).then(||word(w,base+ram::ENEMY_POS_X))
        }) {
            // Human Vile's native kick extends fifty pixels beyond his restored
            // body box. Companions retreat to the safe visible left edge while P1 performs
            // the canonical capture fight. The core freezes them during holds.
            if let Some((px,_,_))=positions[0] {
                // Face the live encounter while the ROM's low-health stun
                // shot and capture sequence run. Keep X on the road so that
                // required zero-damage shot can reach him.
                let right=facings[0].is_some_and(|f|f&0x40!=0);
                let aligned=(npc_x>=px && right)||(npc_x<px && !right);
                pads[0]=face_enemy(0,px,npc_x,right);
                if aligned && game.frame_count()%8<4 {pads[0]|=BTN_Y;}
            }
        }
        // Reserve a real late-road large health pickup for the most wounded
        // local player. Other bodies wait outside its native contact box;
        // the ROM alone collects and fills health on actual contact.
        if let Some((item_x, item_y, bounds)) = (0..16).find_map(|i| {
            let base = 0x1628 + i * 0x30;
            let ix = word(w, base + 5);
            let iy = word(w, base + 8);
            (w[base] != 0 && w[base + 0x0a] == 2 && w[base + 1] != 4
                && w[base + 2] != 6 && w[base + 0x0b] & 0x7f == 0 && ix >= 5800
                && positions.iter().flatten().any(|(px, py, _)| px.abs_diff(ix) < 192 && py.abs_diff(iy) < 96))
                .then(|| game.object_contact_bounds(base as u16).map(|b| (ix, iy, b))).flatten()
        }) {
            if let Some(owner) = (0..4)
                .filter(|&slot| positions[slot].is_some_and(|(px, py, _)| px.abs_diff(item_x) < 192 && py.abs_diff(item_y) < 96))
                .filter_map(|slot| game.player_physics_state(slot).map(|p| (slot, p[2])))
                .filter(|(_, hp)| *hp > 0 && *hp < 16)
                .min_by_key(|(slot, hp)| (*hp, *slot))
                .map(|(slot, _)| slot)
            {
                self.had_health_reservation = true;
                for slot in 0..4 {
                    if let Some((px, _, _)) = positions[slot] {
                        if slot == owner {
                            if self.followers[slot].traversal.is_none() && px.abs_diff(item_x) < 192 {
                                pads[slot] &= !(BTN_LEFT | BTN_RIGHT | BTN_B);
                                if px < item_x.saturating_sub(2) {
                                    pads[slot] |= BTN_RIGHT;
                                } else if px > item_x.saturating_add(2) {
                                    pads[slot] |= BTN_LEFT;
                                }
                            }
                        } else if let Some(body) = game.player_contact_bounds(slot) {
                            let wait_x = bounds[0] - (body[2] - i32::from(px)) - 16;
                            if i32::from(px) >= wait_x - 4 && i32::from(px) < bounds[2] + 48 {
                                pads[slot] &= !(BTN_LEFT | BTN_RIGHT | BTN_B);
                                if i32::from(px) > wait_x + 4 {
                                    pads[slot] |= BTN_LEFT;
                                }
                            }
                        }
                    }
                }
            }
        }
        for room in [2450..2820, 3150..3520] {
            if let Some(last) = positions
                .iter()
                .flatten()
                .filter(|(x, _, _)| room.contains(x))
                .map(|(x, _, _)| *x)
                .min()
            {
                for slot in 0..4 {
                    if self.followers[slot].recovering_collapse
                        && positions[slot].is_some_and(|(x, _, _)| {
                            room.contains(&x) && x > last.saturating_add(32)
                        })
                    {
                        // Gather at the wall before kicking upward. A lone
                        // early ascent would leave the party outside camera.
                        pads[slot] &= !BTN_B;
                    }
                }
            }
        }
        if falling_wreck {
            for slot in 0..4 {
                if self.followers[slot].recovering_collapse {
                    // The falling wreck remains a lethal native collider.
                    // Let gravity carry players down at their safe flank;
                    // approach its exit wall only after it lands/unlocks.
                    pads[slot] &= !(BTN_LEFT | BTN_RIGHT | BTN_B);
                    if let Some((x, _, _)) = positions[slot] {
                        if let Some(bounds) = (0..ram::ENEMY_COUNT).find_map(|i| {
                            let base = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                            main_wreck_falling(w, base)
                                .then(|| game.object_contact_bounds(base as u16))
                                .flatten()
                        }) {
                            let px = i32::from(x);
                            if px >= bounds[0] - 24 && px <= bounds[2] + 24 {
                                pads[slot] |= if px < (bounds[0] + bounds[2]) / 2 {
                                    BTN_LEFT
                                } else {
                                    BTN_RIGHT
                                };
                            }
                        }
                    }
                }
            }
        }
        if let Some((x, y, _)) = positions[0] {
            if bees.is_empty() {
                let horizontally_ahead = positions[1..]
                    .iter()
                    .flatten()
                    .any(|(fx, _, _)| fx.saturating_add(96) < x);
                let safe_wait_floor = x < 4700 || game.player_floor_probe_offset(0).is_some_and(|off| {
                    flat_floor(game, i32::from(x), i32::from(y) + i32::from(off) + 1)
                });
                if horizontally_ahead && live_ground[0] && safe_wait_floor && (x < 4700 || self.followers[0].traversal.is_none()) {
                    let face_right = facings[0].is_some_and(|f| f & 0x40 != 0);
                    let turn = (pads[0] & BTN_LEFT != 0 && face_right)
                        || (pads[0] & BTN_RIGHT != 0 && !face_right);
                    pads[0] &= !BTN_B;
                    if !turn {
                        pads[0] &= !(BTN_LEFT | BTN_RIGHT);
                    }
                }
            }
        }
        // Stage companions on the safe right flank during the driver fight.
        // The craft locks all inputs immediately after the last driver
        // disappears; waiting until Vile arrives is already too late.
        if self.intro_staging.is_none() && positions[0].is_some_and(|(x, _, _)| x >= 7200) {
            let drivers: Vec<_> = (0..ram::ENEMY_COUNT).filter_map(|i| {
                let base = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                (w[base] != 0 && w[base + ram::ENEMY_ID] == 0x11).then_some(base)
            }).collect();
            let party_ready = positions.iter().all(|p| p.is_some_and(|(x, _, _)| x >= 7200));
            let rear_clear = drivers.iter().all(|&base| positions[1..].iter().all(|p| p.is_some_and(|(x, _, _)| x > word(w, base + ram::ENEMY_POS_X).saturating_add(96))));
            if !drivers.is_empty() && party_ready && rear_clear {
                let target = 7548;
                self.intro_staging = Some((target, game.frame_count(), drivers[0] as u16));
                eprintln!("movie smart: staging companions at right flank {target} before final driver; timer={}", w[drivers[0] + 0x35]);
            }
        }
        if let Some((target, began, driver)) = self.intro_staging {
            for slot in 1..4 {
                if let Some((px, _, _)) = positions[slot] {
                    let visible_target = target.min(word(w, ix(ram::CAMERA_X)).saturating_add(256 + game.wide_margin()).saturating_sub(24));
                    pads[slot] = if px < visible_target.saturating_sub(4) { BTN_RIGHT | mmx_core::BTN_A } else if px > visible_target.saturating_add(4) { BTN_LEFT } else { 0 };
                    let action = positions[slot].unwrap().2;
                    let body = game.player_contact_bounds(slot);
                    let approaching = (0..8).any(|i| {
                        let at = 0x1428 + i * 0x40;
                        w[at] != 0 && w[at + 0x0a] == 0x0a && body.is_some_and(|body| {
                            game.object_contact_bounds(at as u16).is_some_and(|shot|
                                shot[2] >= body[0] - 96 && shot[0] < body[0]
                                    && shot[1] >= body[1] - 32 && shot[3] <= body[3] + 32)
                        })
                    });
                    let vehicle_direction = (0..ram::ENEMY_COUNT).find_map(|i| {
                        let at = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                        if w[at] == 0 || !matches!(w[at + ram::ENEMY_ID], 0x11 | 0x12)
                            || matches!(w[at + 1], 6 | 8 | 0x0e) || w[at + ram::ENEMY_CONTACT_DAMAGE] == 0 { return None; }
                        let car = game.object_contact_bounds(at as u16)?;
                        let body = body?;
                        let car_x = word(w, at + ram::ENEMY_POS_X);
                        let previous_x = self.previous_enemy_x[i]?;
                        let closing = if car_x < px { car_x > previous_x } else { car_x < previous_x };
                        if !closing { return None; } // Keep the safe flank when a car drives away.
                        (car[2] >= body[0] - 64 && car[0] <= body[2] + 64
                            && car[3] > 415 && car[1] < 447)
                            .then_some(if (car[0] + car[2]) / 2 < i32::from(px) { BTN_LEFT } else { BTN_RIGHT })
                    });
                    let f = &mut self.followers[slot];
                    if (approaching || vehicle_direction.is_some()) && grounded_action(action) && f.ticks >= f.bullet_jump_until {
                        f.bullet_jump_release_until = f.ticks + usize::from(self.previous_delivered[slot] & BTN_B != 0);
                        f.staging_vehicle_direction = vehicle_direction.unwrap_or(0);
                        f.bullet_jump_until = f.ticks + if vehicle_direction.is_some() { 44 } else { 28 };
                    }
                    if f.ticks < f.bullet_jump_until {
                        // Brake during the full hop so the 4px/tick shot passes
                        // beneath at the apex rather than on our landing.
                        pads[slot] = f.staging_vehicle_direction | if f.ticks >= f.bullet_jump_release_until { BTN_B } else { 0 };
                    }
                    if game.player_saber_state(slot).is_some_and(|(tick, air)| tick != 0 && !air) {
                        pads[slot] |= BTN_B; // Normal grounded-saber cancellation.
                    }
                }
            }
            let native_npc = (0..ram::ENEMY_COUNT).any(|i| {
                let at = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                w[at] != 0 && w[at + ram::ENEMY_ID] == 0x32
            });
            if !native_npc {
                if let Some((px, _, _)) = positions[0] {
                    if px < 7420 { pads[0] = (pads[0] & (BTN_B | BTN_Y)) | BTN_RIGHT; }
                    else if px > 7440 {
                        pads[0] &= !BTN_RIGHT;
                        if facings[0].is_some_and(|f| f & 0x40 == 0) { pads[0] &= !BTN_LEFT; }
                    }
                    else if facings[0].is_some_and(|f| f & 0x40 == 0) { pads[0] &= !BTN_LEFT; }
                }
                // Vile's single full blast can finish the real drivers from
                // the safe right flank. Turn first, then fire on a later frame.
                if let Some((vx, vy, _)) = positions[3].filter(|(x, _, _)| *x >= target.saturating_sub(8))
                    .filter(|_| self.followers[3].ticks >= self.followers[3].bullet_jump_until) {
                    if let Some(enemy_x) = (0..ram::ENEMY_COUNT).filter_map(|i| {
                        let at = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                        (w[at] != 0 && w[at + ram::ENEMY_ID] == 0x11 && w[at + ram::ENEMY_HP] & 0x7f != 0 && vy.abs_diff(word(w, at + ram::ENEMY_POS_Y)) <= 48)
                            .then(|| word(w, at + ram::ENEMY_POS_X))
                    }).min_by_key(|ex| vx.abs_diff(*ex)) {
                        let right = facings[3].is_some_and(|f| f & 0x40 != 0);
                        let aligned = (enemy_x >= vx) == right;
                        pads[3] = face_enemy(pads[3] & BTN_B, vx, enemy_x, right);
                        if aligned && game.frame_count() >= self.next_attack_allowed[3] {
                            pads[3] |= BTN_Y;
                            self.next_attack_allowed[3] = game.frame_count() + 120;
                        }
                    }
                }
            }
            let base = usize::from(driver);
            if w[base] != 0 && w[base + ram::ENEMY_ID] == 0x11 && w[base + 1] == 6 && w[base + ram::ENEMY_HP] & 0x7f != 0 {
                if let Some((px, _, _)) = positions[0] {
                    let enemy_x = word(w, base + ram::ENEMY_POS_X);
                    let ready = positions[1..].iter().all(|p| p.is_some_and(|(x, _, _)| x >= target.saturating_sub(4)));
                    let charged = game.frame_count().saturating_sub(began) >= 180;
                    let toward = ready && charged;
                    let admit_target = word(w, ix(ram::CAMERA_X)).saturating_add(256 + game.wide_margin()).saturating_sub(24) >= target;
                    let desired_right = if toward { enemy_x >= px } else { enemy_x < px };
                    let right = facings[0].is_some_and(|f| f & 0x40 != 0);
                    pads[0] = if right == desired_right { 0 } else if desired_right { BTN_RIGHT } else { BTN_LEFT };
                    if !admit_target { pads[0] |= BTN_RIGHT; }
                    // Start charging away from the husk: the initial pellet
                    // must not finish it before the other players arrive.
                    if !toward && (right == desired_right || self.previous_delivered[0] & BTN_Y != 0)
                        || toward && right != desired_right {
                        pads[0] |= BTN_Y;
                    }
                }
            }
        }
        if std::env::var_os("MMX_BULLET_TRACE").is_some() && game.frame_count()%4==0 {
            if let Some((x,y,action))=positions[2].filter(|(x,_,_)|*x>=7200) {
                let floors: Vec<_>=game.player_floor_probe_offset(2).map(|off| (0..=64).step_by(8).map(|dy|game.terrain_flat_floor_y(i32::from(x),i32::from(y)+i32::from(off)+1+dy)).collect()).unwrap_or_default();
                let shots: Vec<_>=(0..8).filter_map(|i|{let base=0x1428+i*0x40;(w[base]!=0&&w[base+0x0a]==0x0a).then(||(base,game.object_contact_bounds(base as u16)))}).collect();
                eprintln!("BULLET_TRACE frame{} Sigma({x},{y}) action{action:02x} road{:?} floors{floors:?} body{:?} shots{shots:?} wait{:?} pads{:04x}",game.frame_count(),self.grounded_road[2],game.player_contact_bounds(2),self.followers[2].bullet_wait,pads[2]);
            }
        }
        if std::env::var_os("MMX_ROUTE_CONTINUE_TRACE").is_some() && game.frame_count() % 60 == 0 {
            for slot in 0..4 {
                if positions[slot].is_some_and(|(x,_,_)| x>=6300) {
                    eprintln!("ROAD_DELIVERED frame{} P{} pos{:?} cursor{} stalled{} last_x{} pads{:04x} previous{:04x}",game.frame_count(),slot+1,positions[slot],self.followers[slot].at,self.followers[slot].stalled,self.followers[slot].last_x,pads[slot],self.previous_delivered[slot]);
                }
            }
        }
        if std::env::var_os("MMX_GAP_TRACE").is_some() && game.frame_count() % 4 == 0 {
            for slot in 0..4 {
                if positions[slot].is_some_and(|(x, _, _)| (4130..4338).contains(&x)) {
                    eprintln!("GAP_TRACE frame{} P{} raw={:?} grounded={} road={:?} cursor={} limit={} actualprev={:04x} pads={:04x} decision={:?} traversal={:?} gaps={:?}",game.frame_count(),slot+1,game.player_physics_state(slot),live_ground[slot],self.grounded_road[slot],self.followers[slot].at,self.followers[0].at.saturating_sub(self.delay*slot),self.previous_delivered[slot],pads[slot],decision_trace[slot],self.followers[slot].traversal,self.known_gaps.iter().filter(|p|p.start[0]>4100&&p.start[0]<4338).collect::<Vec<_>>());
                }
            }
        }
        if self.followers[0].ticks % 300 == 0 && std::env::var_os("MMX_BOT_TRACE").is_some() {
            eprintln!("movie smart inputs={pads:x?} recovery={:?} falling_wreck={falling_wreck} ground={live_ground:?} freeze={:?}",self.followers.iter().map(|f|f.recovering_collapse).collect::<Vec<_>>(),&w[0x1f10..0x1f1a]);
        }
        pads
    }
}

/// Bee Blader's explosion also creates a type22 visual child (variant4).
/// Only the main actor's death state controls the falling road/camera lock.
fn main_wreck_falling(w: &[u8; 0x20000], at: usize) -> bool {
    w[at + ram::ENEMY_ACTIVE] != 0
        && w[at + ram::ENEMY_ID] == 0x22
        && w[at + 0x0b] != 4
        && w[at + 1] == 0x0e
        && w[at + ram::ENEMY_HP] & 0x7f == 0
        && w[at + 0x2b] & 4 == 0
}

fn dialog_confirm(w: &[u8; 0x20000], frame: u64) -> u16 {
    // Native textbox VM $80:E9CF accepts remapped P1 action buttons.
    // Its active flag is separate from the first-stage controller lifecycle.
    if w[0x1f2c] & 0x80 != 0 && w[ix(ram::MODE_D2)] != 2 && frame % 60 < 2 { BTN_B } else { 0 }
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

fn on_ground(block: &[u8; ram::X_BLOCK_LEN]) -> bool {
    (block[OFF_GROUND] | block[OFF_GROUND + 1]) & 4 != 0
}

fn recorded_jumps(route: &[Point]) -> Vec<JumpPlan> {
    let mut plans = Vec::new();
    for at in 1..route.len() {
        let previous = route[at - 1];
        let current = route[at];
        let pressed = current.grounded && current.pad & BTN_B != 0 && previous.pad & BTN_B == 0;
        // Source movies can walk off an edge and wall-kick later. Commit a
        // crossing from their last grounded point before either jump or fall.
        let began = grounded_action(previous.action)
            && matches!(current.action, 0x06 | 0x08 | 0x10)
            && previous.action != current.action;
        if !pressed && !began {
            continue;
        }
        let p = if began { previous } else { current };
        let mut airborne = false;
        for end in at + 1..(at + 180).min(route.len()) {
            let q = route[end];
            airborne |= !q.grounded;
            if airborne && q.grounded && grounded_action(q.action) {
                if q.x >= p.x.saturating_add(32) {
                    if plans
                        .last()
                        .is_some_and(|last: &JumpPlan| last.start[0].abs_diff(p.x) < 4)
                    {
                        break;
                    }
                    plans.push(JumpPlan {
                        start: [p.x, p.y],
                        end: [q.x, q.y],
                        source_start: at,
                        source_end: end,
                    });
                }
                break;
            }
        }
    }
    plans
}

fn committed_jump(
    plans: &[JumpPlan],
    state: &mut Follower,
    x: u16,
    y: u16,
    action: u8,
    grounded: bool,
    held_last_frame: bool,
    static_floor: bool,
) -> Option<u16> {
    if state.traversal.is_none() {
        if let Some(plan) = plans
            .iter()
            .filter(|p| {
                let road_gap = p.start[0] < 1400;
                (grounded || (road_gap && matches!(action,0x06|0x08|0x10|0x12)))
                    && x >= p.start[0].saturating_sub(if road_gap {128}else{4})
                    && (x<=p.start[0].saturating_add(if road_gap {64}else{10})
                        || (grounded && p.start[0]>=3900 && x<p.end[0].saturating_sub(12)))
                    // A moving vehicle can raise the live takeoff surface by
                    // one or two tiles relative to the recorded road edge.
                    && y<=p.start[1].saturating_add(if grounded && p.start[0]>=3900 && x>p.start[0].saturating_add(10) {64}else{24})
                    && y.saturating_add(64) >= p.start[1]
            })
            .min_by_key(|p| x.abs_diff(p.start[0]))
        {
            if x < plan.start[0].saturating_sub(4) {
                return Some(BTN_RIGHT);
            }
            if grounded && held_last_frame {
                return Some(BTN_RIGHT);
            }
            state.traversal = Some(Traversal {
                plan: *plan,
                begun: state.ticks,
                airborne: !grounded,
                release_until: 0,
                wall_retry_after: 0,
            });
            eprintln!(
                "movie smart: committed jump at ({x},{y}) -> {:?}, source {}..{}",
                plan.end, plan.source_start, plan.source_end
            );
        }
    }
    let mut traversal = state.traversal?;
    traversal.airborne |= !grounded && matches!(action, 0x06 | 0x08 | 0x10 | 0x12);
    if traversal.airborne
        && grounded
        && x >= traversal.plan.end[0].saturating_sub(12)
        // Falling road pieces set the native grounded flag too. Keep the
        // crossing active there until the actual collision map supports it.
        && ((traversal.plan.start[0] < 4700 && traversal.plan.end[0] < 4800) || static_floor)
        && (y <= traversal.plan.end[1].saturating_add(24)
            || (static_floor && y <= traversal.plan.end[1].saturating_add(64)))
    {
        state.completed_jumps += 1;
        state.at = traversal.plan.source_end;
        state.landed_floor_at = traversal.plan.source_end;
        state.landed_floor_pos = Some([x, y]);
        state.traversal = None;
        state.jump_until = 0;
        state.jump_after = state.ticks + 6;
        eprintln!(
            "movie smart: safe jump landing at ({x},{y}), target {:?}",
            traversal.plan.end
        );
        return Some(BTN_RIGHT);
    }
    if action == 0x12 && state.ticks >= traversal.wall_retry_after {
        traversal.release_until = state.ticks + 3;
        traversal.wall_retry_after = state.ticks + 18;
    }
    if traversal.airborne
        && grounded
        && (x < traversal.plan.end[0].saturating_sub(12)
            || ((traversal.plan.start[0] >= 4700 || traversal.plan.end[0] >= 4800) && !static_floor))
        && state.ticks >= traversal.wall_retry_after
    {
        traversal.release_until = state.ticks + 3;
        traversal.wall_retry_after = state.ticks + 18;
    }
    if state.ticks - traversal.begun > 180 {
        eprintln!(
            "movie smart: traversal timed out at ({x},{y}), target {:?}",
            traversal.plan.end
        );
        if traversal.plan.start[0] >= 4600 && x < traversal.plan.end[0] {

            // A late crossing remains active until actual static landing.
            // Its recovery can pass the destination height while still well
            // left of the road; dropping it would select obsolete short hops.
            traversal.begun = state.ticks;
            traversal.release_until = state.ticks + 1;
            traversal.wall_retry_after = state.ticks + 18;
        } else {
            state.traversal = None;
            return None;
        }
    }
    state.traversal = Some(traversal);
    Some(
        BTN_RIGHT
            | if state.ticks >= traversal.release_until {
                BTN_B
            } else {
                0
            },
    )
}

fn flat_floor(game: &Game, x: i32, feet_y: i32) -> bool {
    game.terrain_flat_floor_y(x, feet_y).is_some()
}

/// Discover a hole from the live native collision map. A recorded car landing
/// is not a safe destination after that actor has moved or been destroyed.
fn live_road_jump(
    game: &Game,
    route: &[Point],
    at: usize,
    x: u16,
    y: u16,
    grounded: bool,
    floor_offset: Option<i16>,
) -> Option<JumpPlan> {
    let offset = floor_offset?;
    let feet = i32::from(y) + i32::from(offset) + 1;
    if x < 1400 || !grounded || !flat_floor(game, i32::from(x), feet) {
        return None;
    }
    let edge = (8..=96)
        .step_by(8)
        .find(|dx| !flat_floor(game, i32::from(x) + dx, feet))?;
    let same_height = (edge + 8..=edge + 144)
        .step_by(8)
        .find(|dx| flat_floor(game, i32::from(x) + dx, feet))
        .map(|dx| (dx, y));
    let different_height = || {
        if x < 4700 {
            return None;
        }
        (edge + 8..=edge + 144).step_by(8).find_map(|dx| {
            [16, -16, 32, -32, 48, -48, 64, -64]
                .into_iter()
                .find_map(|dy| {
                    let top = game.terrain_flat_floor_y(i32::from(x) + dx, feet + dy)?;
                    let actor_y = top - i32::from(offset) - 1;
                    (actor_y >= 0 && actor_y <= 65535 && actor_y.abs_diff(i32::from(y)) <= 64)
                        .then_some((dx, actor_y as u16))
                })
        })
    };
    let (landing, end_y) = same_height.or_else(different_height)?;
    let start = x.saturating_add(edge as u16).saturating_sub(8);
    let end = x.saturating_add(landing as u16).saturating_add(16);
    let source_end = at + nearest(&route[at..], end, end_y, route.len() - at - 1);
    Some(JumpPlan {
        start: [start, y],
        end: [end, end_y],
        source_start: at,
        source_end,
    })
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
    let above = target.grounded && i32::from(y) - i32::from(target.y) > 8;
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
    fn bee_explosion_visual_child_cannot_hold_traversal() {
        let mut w = [0u8; 0x20000];
        let at = ix(ram::ENEMIES_BASE);
        w[at] = 1;
        w[at + ram::ENEMY_ID] = 0x22;
        w[at + 1] = 0x0e;
        w[at + 2] = 2;
        assert!(main_wreck_falling(&w, at));
        w[at + 0x0b] = 4;
        assert!(!main_wreck_falling(&w, at));
        w[at + 0x0b] = 0;
        w[at + 1] = 2;
        assert!(!main_wreck_falling(&w, at));
    }
    #[test]
    fn waits_for_waypoint_and_corrects_direction_without_pause() {
        let route = vec![
            Point {
                x: 100,
                y: 100,
                pad: BTN_START | BTN_RIGHT | BTN_Y,
                action: 0,
                grounded: true,
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
                action: 0,
                grounded: true,
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
                action: 0,
                grounded: true,
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
                grounded: true,
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
                action: 6,
                grounded: false,
            };
            30
        ];
        route[18] = Point {
            x: 1764,
            y: 351,
            pad: BTN_RIGHT,
            action: 4,
            grounded: true,
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
                grounded: true,
            },
            Point {
                x: 1024,
                y: 335,
                pad: BTN_RIGHT,
                action: 4,
                grounded: true,
            },
            Point {
                x: 1024,
                y: 335,
                pad: BTN_RIGHT,
                action: 0x0E,
                grounded: false,
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
    fn real_rom_hd_party_crosses_highway_gaps_and_bee_collapses_without_assistance() {
        let Some(rom_path) = std::env::var_os("MMX_ROM").map(std::path::PathBuf::from) else {
            return eprintln!("MMX_ROM not set; skipping real Highway party traversal");
        };
        let root = rom_path.parent().unwrap().parent().unwrap();
        let movie_path = root.join("movies/hetfield90v3-megamanx-100p.bk2");
        let pack_path = root.join("roms/derived/bone_sheets");
        let args = crate::cli::parse(
            [
                "--rom".to_owned(),
                rom_path.to_string_lossy().into_owned(),
                "--movie".to_owned(),
                movie_path.to_string_lossy().into_owned(),
                "--movie-smart".to_owned(),
                "--movie-coop-delay".to_owned(),
                "8".to_owned(),
                "--p1-character".to_owned(),
                "x".to_owned(),
                "--p2-character".to_owned(),
                "zero".to_owned(),
                "--p3-character".to_owned(),
                "sigma".to_owned(),
                "--p4-character".to_owned(),
                "vile".to_owned(),
            ],
            None,
        )
        .unwrap();
        let mut controller = RoutePlayback::load(&args).unwrap();
        let bytes = std::fs::read(&rom_path).unwrap();
        let states = controller.fixture_states.clone();
        assert_eq!(
            states.len(),
            5,
            "movie did not reach the real pre-gap fixtures"
        );
        let roster = [
            mmx_core::Character::X,
            mmx_core::Character::Zero,
            mmx_core::Character::Sigma,
            mmx_core::Character::Vile,
        ];
        for (gap, state) in states.iter().enumerate() {
            if std::env::var("MMX_HAZARD_INDEX")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|wanted| wanted != gap)
            {
                continue;
            }
            for repetition in 0..2 {
                let mut game = Game::from_rom(bytes.clone()).unwrap();
                game.load_state(state).unwrap();
                game.coop.enabled = true;
                game.set_local_players(4);
                mmx_core::coop::register_coop_traps(&mut game).unwrap();
                for (slot, c) in roster.into_iter().enumerate() {
                    game.set_player_character(slot, c);
                    if c != mmx_core::Character::X {
                        game.set_boss_set(c, mmx_core::boss_data::extract_x1(&bytes, c).unwrap());
                    }
                }
                crate::bone_sheets::install(&mut game, &pack_path, &roster).unwrap();
                let x = word(game.wram(), ix(ram::X_POS_X));
                let y = word(game.wram(), ix(ram::X_POS_Y));
                controller.followers = std::array::from_fn(|_| Follower::default());
                controller.followers[0].at = checkpoint_waypoint(&controller.route, x, y).unwrap();
                controller.started = true;
                controller.finished = false;
                controller.pending_respawn_reanchor = false;
                controller.previous_source = [0; 4];
                controller.last_visible = [None; 4];
                controller.death_counts = [0; 4];
                controller.warp_counts = [0; 4];
                controller.next_attack_allowed = [0; 4];
                eprintln!(
                    "HAZARD_START gap{} HP{}",
                    gap + 1,
                    game.wram()[ix(ram::X_HP)] & 0x7f
                );
                let far = [920, 1360, 2940, 3540, 3900][gap];
                let mut passed = [false; 4];
                let mut last_hp = game.wram()[ix(ram::X_HP)] & 0x7f;
                for tick in 0..if gap < 2 { 600 } else { 2400 } {
                    let pads = controller.next(&game);
                    game.set_extra_pads([pads[2], pads[3]]);
                    game.step(Pads {
                        p1: pads[0],
                        p2: pads[1],
                    });
                    let _ = game.take_audio();
                    let hp = game.wram()[ix(ram::X_HP)] & 0x7f;
                    if hp < last_hp && gap >= 2 {
                        let w = game.wram();
                        let px = word(w, ix(ram::X_POS_X));
                        let py = word(w, ix(ram::X_POS_Y));
                        let nearby: Vec<_> = (0..ram::ENEMY_COUNT)
                            .filter_map(|i| {
                                let b = ix(ram::ENEMIES_BASE) + i * ram::ENEMY_STRIDE;
                                (w[b] != 0 && px.abs_diff(word(w, b + ram::ENEMY_POS_X)) < 200)
                                    .then(|| {
                                        (
                                            w[b + ram::ENEMY_ID],
                                            word(w, b + ram::ENEMY_POS_X),
                                            word(w, b + ram::ENEMY_POS_Y),
                                            w[b + ram::ENEMY_HP],
                                            game.object_contact_bounds(b as u16),
                                        )
                                    })
                            })
                            .collect();
                        eprintln!("REAL_DAMAGE tick{tick} HP{last_hp}->{hp} pos=({px},{py}) nearby={nearby:?}");
                    }
                    last_hp = hp;
                    let points = [
                        Some((
                            word(game.wram(), ix(ram::X_POS_X)),
                            word(game.wram(), ix(ram::X_POS_Y)),
                            game.wram()[ix(ram::X_ACTION)],
                        )),
                        game.visible_p2_object().map(position),
                        game.visible_extra_player_object(0).map(position),
                        game.visible_extra_player_object(1).map(position),
                    ];
                    for slot in 0..4 {
                        if points[slot].is_some_and(|p| p.0 >= far && grounded_action(p.2)) {
                            passed[slot] = true;
                        }
                    }
                    if passed.into_iter().all(|p| p) {
                        // Observe the last actual step too: a ROM warp onto
                        // the far side must not evade the diagnostic count.
                        let _ = controller.next(&game);
                        break;
                    }
                    if controller.finished {
                        break;
                    }
                }
                eprintln!(
                    "real party gap{} repeat{}: passed={passed:?}, deaths={:?}, warps={:?}",
                    gap + 1,
                    repetition + 1,
                    controller.death_counts,
                    controller.warp_counts
                );
                assert!(
                    passed.into_iter().all(|p| p),
                    "gap{} party did not all cross",
                    gap + 1
                );
                assert_eq!(
                    controller.death_counts,
                    [0; 4],
                    "gap{} used a down/respawn instead of traversal",
                    gap + 1
                );
                assert_eq!(
                    controller.warp_counts,
                    [0; 4],
                    "gap{} used a ROM warp instead of traversal",
                    gap + 1
                );
            }
        }
    }
    #[test]
    fn hurt_resync_avoids_knockback_points_and_respects_leader_limit() {
        let mut route: Vec<_> = (0..100)
            .map(|x| Point {
                x,
                y: 100,
                pad: BTN_RIGHT,
                action: 4,
                grounded: true,
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
                grounded: true,
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
            previous_delivered: [0; 4],
            grounded_road: [None; 4],
            known_gaps: vec![],
            last_logged_tick: None,
            pending_respawn_reanchor: false,
            respawn_gameplay_frames: 0,
            next_attack_allowed: [0; 4],
            had_health_reservation: false,
            intro_staging: None,
            previous_enemy_x: [None; 15],
            jump_plans: vec![],
            last_visible: [None; 4],
            death_counts: [0; 4],
            warp_counts: [0; 4],
            fixture_states: vec![],
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
