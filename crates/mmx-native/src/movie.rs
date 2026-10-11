//! Recorded pad inputs, with optional staggered copies for local co-op.
use crate::cli::Args;
use mmx_core::{Game, Pads, BTN_START, BTN_SELECT};
use mmx_verify::movie_bk2;

pub struct Playback {
    pads: Vec<Pads>,
    frame: usize,
    delay: Option<usize>,
    stage_start: Option<usize>,
    ended: bool,
    previous_source: [u16; 4],
    next_attack_allowed: [u64; 4],
}

impl Playback {
    pub fn load(args: &Args) -> Result<Option<Self>, String> {
        let Some(path) = args.movie.as_deref() else { return Ok(None) };
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let movie = match std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
            Some("lsmv") => movie_bk2::parse_lsmv(&bytes),
            Some("smv") => movie_bk2::parse_smv(&bytes),
            _ => movie_bk2::parse_bk2(&bytes),
        }.map_err(|e| format!("{path}: {e}"))?;
        if movie.from_snapshot { return Err("movie must start from power-on, not a snapshot".into()); }
        if !movie.resets.is_empty() { return Err("movies containing Reset are unsupported".into()); }
        if movie.pads.is_empty() { return Err("movie contains no input frames".into()); }
        eprintln!("movie: {path}: {} frames, ROM SHA1 {:?}, follower delay {:?}; co-op replay is experimental", movie.pads.len(), movie.sha1, args.movie_coop_delay);
        Ok(Some(Self { pads: movie.pads, frame: 0, delay: args.movie_coop_delay.map(usize::from), stage_start: None, ended: false, previous_source: [0; 4], next_attack_allowed: [0; 4] }))
    }

    pub fn next(&mut self, game: &Game) -> [u16; 4] {
        // The weapon menu retains the gameplay phase, but co-op actors are
        // suspended until its fade out completes. Reset follower delay there.
        let playing = mmx_core::coop_stages::p2_should_exist(game.wram());
        let source = self.inputs(playing);
        self.attacks(std::array::from_fn(|slot| game.player_character(slot).unwrap_or(mmx_core::Character::X)), source, playing)
    }

    fn attacks(&mut self, characters: [mmx_core::Character; 4], source: [u16; 4], playing: bool) -> [u16; 4] {
        let inputs = if self.delay.is_some() && playing && !self.ended {
            std::array::from_fn(|slot| {
                let pad = crate::character_attacks::remap_movie_attacks(
                    characters[slot], self.previous_source[slot], source[slot],
                );
                crate::character_attacks::enforce_attack_cooldown(
                    characters[slot], pad, self.frame as u64, &mut self.next_attack_allowed[slot],
                )
            })
        } else { source };
        self.previous_source = if playing && !self.ended { source } else { [0; 4] };
        inputs
    }

    fn inputs(&mut self, playing: bool) -> [u16; 4] {
        if playing {
            self.stage_start.get_or_insert(self.frame);
        } else {
            self.stage_start = None;
        }
        let mut inputs = [0; 4];
        if let Some(pads) = self.pads.get(self.frame) {
            inputs[0] = pads.p1;
            if let Some(delay) = self.delay {
                if let Some(start) = self.stage_start {
                    for (slot, input) in inputs.iter_mut().enumerate().skip(1) {
                        if let Some(at) = self.frame.checked_sub(delay * slot).filter(|at| *at >= start) {
                            // Menu/pause controls belong to the leader only.
                            *input = self.pads[at].p1 & !(BTN_START | BTN_SELECT);
                        }
                    }
                }
            } else {
                inputs[1] = pads.p2;
            }
        } else if !self.ended {
            eprintln!("movie: end at frame {}; releasing all controls", self.frame);
            self.ended = true;
        }
        self.frame += 1;
        inputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmx_core::{BTN_RIGHT, BTN_B, BTN_Y};

    #[test]
    fn followers_wait_for_gameplay_then_copy_with_individual_delays() {
        let track = [BTN_START, BTN_RIGHT, BTN_B, BTN_Y, BTN_RIGHT | BTN_B, BTN_START | BTN_Y];
        let mut p = Playback { pads: track.map(|p1| Pads { p1, p2: 0 }).to_vec(), frame: 0, delay: Some(1), stage_start: None, ended: false, previous_source: [0; 4], next_attack_allowed: [0; 4] };
        assert_eq!(p.inputs(false), [BTN_START, 0, 0, 0]);
        assert_eq!(p.inputs(true), [BTN_RIGHT, 0, 0, 0]);
        assert_eq!(p.inputs(true), [BTN_B, BTN_RIGHT, 0, 0]);
        assert_eq!(p.inputs(true), [BTN_Y, BTN_B, BTN_RIGHT, 0]);
        assert_eq!(p.inputs(true), [BTN_RIGHT | BTN_B, BTN_Y, BTN_B, BTN_RIGHT]);
        assert_eq!(p.inputs(false), [BTN_START | BTN_Y, 0, 0, 0]);
        assert_eq!(p.inputs(true), [0; 4]);
    }

    #[test]
    fn zero_delay_suppresses_follower_pause_and_normal_playback_preserves_p2() {
        let pad = Pads { p1: BTN_START | BTN_SELECT | BTN_Y, p2: BTN_B };
        let mut p = Playback { pads: vec![pad], frame: 0, delay: Some(0), stage_start: None, ended: false, previous_source: [0; 4], next_attack_allowed: [0; 4] };
        assert_eq!(p.inputs(true), [pad.p1, BTN_Y, BTN_Y, BTN_Y]);
        p.frame = 0;
        p.delay = None;
        assert_eq!(p.inputs(false), [pad.p1, BTN_B, 0, 0]);
    }

    #[test]
    fn gameplay_resume_waits_for_fresh_inputs_after_pause_or_death() {
        let mut p = Playback {
            pads: vec![Pads { p1: BTN_RIGHT, p2: 0 }; 8],
            frame: 0, delay: Some(1), stage_start: None, ended: false, previous_source: [0; 4], next_attack_allowed: [0; 4],
        };
        p.inputs(true);
        p.inputs(true);
        assert_eq!(p.inputs(true), [BTN_RIGHT, BTN_RIGHT, BTN_RIGHT, 0]);
        assert_eq!(p.inputs(false), [BTN_RIGHT, 0, 0, 0]);
        assert_eq!(p.inputs(true), [BTN_RIGHT, 0, 0, 0]);
        assert_eq!(p.inputs(true), [BTN_RIGHT, BTN_RIGHT, 0, 0]);
    }

    #[test]
    fn movie_end_releases_controls_without_a_charge_release_attack() {
        let mut p = Playback { pads: vec![], frame: 0, delay: Some(0), stage_start: None, ended: false, previous_source: [BTN_Y; 4], next_attack_allowed: [0; 4] };
        let source = p.inputs(true);
        assert_eq!(p.attacks([mmx_core::Character::Zero, mmx_core::Character::Sigma, mmx_core::Character::Vile, mmx_core::Character::X], source, true), [0; 4]);
        assert_eq!(p.previous_source, [0; 4]);
    }
}
