//! When does a second player make sense? Pure predicates over raw WRAM,
//! established by scripted runs through the Highway stage (docs/coop-qa.md,
//! `scripts/coop-qa.sh`). "M<n>" below names the milestone in that table
//! that showed the condition. The coordinator wires these into `coop.rs`.

use crate::ram::{self, wram_index as ix};

type Wram = [u8; 0x20000];

/// `$D2`: 4 = in a stage. 0 = title/intro, 2 = stage select, `$0A` = game
/// over password screen (M8: `$D2=$0A` for the whole game-over screen;
/// the Start that continues passes through `$D2=2`, `$D3=$16` for a few frames).
pub const MODE_IN_STAGE: u8 = 4;
/// `$D3` while `$D2 == 4`: 0 = loading (also the reload after a death, with
/// `$D4 = 1`), 2 = "READY" + beam-in, 4 = gameplay, 6 = dying (M1: X sets
/// `$D3=6` the frame it dies in a pit, holds it ~200 frames, then `$D3=0`).
pub const PHASE_GAMEPLAY: u8 = 4;

/// X actions (`$0BAA`) seen on the real ROM.
pub mod act {
    /// Standing (also "pushing a wall" while right is held).
    pub const STAND: u8 = 0x00;
    /// Standing right after the beam-in / landing.
    pub const READY: u8 = 0x02;
    pub const WALK: u8 = 0x04;
    pub const JUMP: u8 = 0x06;
    pub const FALL: u8 = 0x08;
    pub const LAND: u8 = 0x0A;
    pub const DEATH: u8 = 0x0C;
    pub const HURT: u8 = 0x0E;
    pub const WALL_KICK: u8 = 0x10;
    pub const WALL_SLIDE: u8 = 0x12;
    /// Beam-in drop at stage start / after a respawn (`vy = -2048`).
    pub const BEAM_IN: u8 = 0x22;
}

fn b(w: &Wram, a: u32) -> u8 {
    w[ix(a)]
}

fn w16(w: &Wram, a: u32) -> u16 {
    u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]])
}

/// A second player is sensible this frame.
pub fn p2_should_exist(w: &Wram) -> bool {
    // M0/M8: in a stage; not title, stage select or the game-over password screen.
    b(w, ram::MODE_D2) == MODE_IN_STAGE
        // M1/M8: gameplay only; 0 (load, respawn reload), 2 (READY/beam-in),
        // 6 (P1 dying) are out; so are 8/$A/$C (stage clear, from the RAM map:
        // not reached, the Vile trigger at the stage end was not found).
        && b(w, ram::STAGE_PHASE) == PHASE_GAMEPLAY
        // M2: the weapon menu keeps `$D3=4` but sets `$1F24=1` from the Start
        // press until the fade back in is done.
        && b(w, ram::PAUSE) == 0
        // M1/M8: P1 dead (action $0C or HP 0) means the stage-wide death flow
        // is about to start; `$D3` flips to 6 a few frames later.
        && b(w, ram::X_ACTION) != act::DEATH
        && b(w, ram::X_HP) & 0x7F != 0
}

/// P1 is a safe block to copy P2 from: grounded on a real floor, in control.
/// Stricter than coop.rs's "vy == 0 and Y unchanged": a wall slide (M5:
/// `act $12`, vy -512 but Y can sit still for a frame against a ledge), the
/// hurt knockback (M0: `$0E` has vy 0 on its last frames) and the beam-in
/// are all rejected.
pub fn p2_anchor_ok(w: &Wram) -> bool {
    p2_should_exist(w)
        && matches!(b(w, ram::X_ACTION), act::STAND | act::READY | act::WALK)
        && w16(w, ram::X_VEL_Y) == 0
        && p1_in_view(w)
}

/// P1's Y inside the camera window (M4: the camera lags P1 by up to ~100 px
/// while he falls through the collapsed road after the first Bee Blader).
fn p1_in_view(w: &Wram) -> bool {
    in_view_y(w16(w, ram::X_POS_Y), w16(w, ram::CAMERA_Y))
}

/// `y` (object anchor, feet) is inside the 224-line window below `cam_y`.
pub fn in_view_y(y: u16, cam_y: u16) -> bool {
    y >= cam_y && y < cam_y.saturating_add(224)
}

/// P2 has left the playfield vertically and will not come back on its own:
/// fell into a pit (M0: P2 sank to Y 895 with the camera at 256 and never
/// died; P1's pit death fires at `y - cam_y = 257`), or the camera scrolled up
/// without it (M5: P1 wall-climbed out of the Bee Blader crash, camera Y
/// 479 -> 256, P2 stranded at Y 575). Either way: down it / re-anchor it.
pub fn p2_lost(p2_y: u16, cam_y: u16) -> bool {
    p2_y >= cam_y.saturating_add(257) || p2_y.saturating_add(64) < cam_y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wram() -> Box<Wram> {
        vec![0u8; 0x20000].into_boxed_slice().try_into().unwrap()
    }

    fn set(w: &mut Wram, a: u32, v: u8) {
        w[ix(a)] = v;
    }

    fn set16(w: &mut Wram, a: u32, v: u16) {
        w[ix(a)..ix(a) + 2].copy_from_slice(&v.to_le_bytes());
    }

    /// Highway, frame 60 of M0: walking, camera (83,256), X at (211,367).
    fn gameplay() -> Box<Wram> {
        let mut w = wram();
        set(&mut w, ram::MODE_D1, 2);
        set(&mut w, ram::MODE_D2, 4);
        set(&mut w, ram::STAGE_PHASE, 4);
        set(&mut w, ram::X_ACTION, act::WALK);
        set(&mut w, ram::X_HP, 16);
        set16(&mut w, ram::X_POS_X, 211);
        set16(&mut w, ram::X_POS_Y, 367);
        set16(&mut w, ram::CAMERA_X, 83);
        set16(&mut w, ram::CAMERA_Y, 256);
        w
    }

    #[test]
    fn coop_stages_gameplay() {
        let w = gameplay();
        assert!(p2_should_exist(&w));
        assert!(p2_anchor_ok(&w));
    }

    #[test]
    fn coop_stages_modes() {
        for (d2, d3, ok) in [(4, 4, true), (4, 0, false), (4, 2, false), (4, 6, false), (4, 8, false), (2, 4, false), (0x0A, 0, false), (0, 0, false)] {
            let mut w = gameplay();
            set(&mut w, ram::MODE_D2, d2);
            set(&mut w, ram::STAGE_PHASE, d3);
            assert_eq!(p2_should_exist(&w), ok, "$D2={d2:02X} $D3={d3:02X}");
        }
    }

    #[test]
    fn coop_stages_pause_and_death() {
        let mut w = gameplay();
        set(&mut w, ram::PAUSE, 1);
        assert!(!p2_should_exist(&w));
        let mut w = gameplay();
        set(&mut w, ram::X_ACTION, act::DEATH);
        assert!(!p2_should_exist(&w));
        let mut w = gameplay();
        set(&mut w, ram::X_HP, 0x80); // HUD-redraw bit only: dead
        assert!(!p2_should_exist(&w));
    }

    #[test]
    fn coop_stages_anchor() {
        for (a, ok) in [(act::STAND, true), (act::READY, true), (act::WALK, true), (act::JUMP, false), (act::FALL, false), (act::HURT, false), (act::WALL_SLIDE, false), (act::BEAM_IN, false)] {
            let mut w = gameplay();
            set(&mut w, ram::X_ACTION, a);
            assert_eq!(p2_anchor_ok(&w), ok, "act {a:02X}");
        }
        let mut w = gameplay();
        set16(&mut w, ram::X_VEL_Y, (-512i16) as u16);
        assert!(!p2_anchor_ok(&w));
        // M4: P1 ahead of a lagging camera.
        let mut w = gameplay();
        set16(&mut w, ram::X_POS_Y, 500);
        set16(&mut w, ram::CAMERA_Y, 256);
        assert!(!p2_anchor_ok(&w));
    }

    #[test]
    fn coop_stages_lost() {
        assert!(!p2_lost(367, 256)); // on the road
        assert!(p2_lost(895, 256)); // M0: sank through the pit
        assert!(p2_lost(513, 256)); // where P1's pit death fires
        assert!(!p2_lost(512, 256));
        assert!(p2_lost(575, 256) && !p2_lost(575, 479)); // M5: stranded below after the climb
        assert!(p2_lost(100, 256) && !p2_lost(200, 256)); // far above the window
        assert!(in_view_y(256, 256) && !in_view_y(480, 256));
    }
}
