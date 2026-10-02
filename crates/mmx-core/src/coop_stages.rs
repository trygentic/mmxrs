//! When does a second player make sense? Pure predicates over raw WRAM,
//! established by scripted runs through the Highway stage (docs/coop-qa.md,
//! `scripts/coop-qa.sh`). "M<n>" below names the milestone in that table
//! that showed the condition. `coop.rs` uses them: `p2_should_exist` and
//! `p2_anchor_ok` in the update slice, `p2_lost` in the physics slice.

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
    /// Stage exit (teleport out after a boss / the Vile scene).
    pub const STAGE_EXIT: u8 = 0x1A;
    /// "Hold still": the generic cutscene action, set by `$84:9FE6` (34
    /// callers: boss intros, doors, ...) and ended by `$84:9FFE` (action 0).
    /// Its handler (`$81:8B4D`) lets an airborne X fall, lands it and stands.
    pub const HOLD: u8 = 0x1E;
    /// Vile's capture (`$84:A04D` / `$A061` / `$A070`: grabbed, held, down).
    pub const CAPTURED: u8 = 0x2E;
    pub const CAPTURED_HELD: u8 = 0x30;
    pub const CAPTURED_DOWN: u8 = 0x32;
}

/// `$1F23`: the stage is cleared (the boss died, `$84:AA28`): X's action
/// dispatcher (`$81:8258`) runs the victory/teleport code instead of X's
/// action, then `$D3 = 8`.
const STAGE_CLEAR: u32 = 0x7E1F23;

/// The ROM has taken X's control away: a scripted sequence (boss intro and
/// health-bar fill, boss door, Vile's capture and the scene with the NPC
/// Zero, stage clear / teleport out). X's input is still remapped from the
/// pad in all of these; it is the action that ignores it. Every action an
/// object forces on X through the setters at `$84:9F85..$84:A07B` is
/// listed except `$18` / `$1C` (P1's own health / tank pickups, where the
/// world freezes but P2 plays on, docs/coop.md "Items").
pub fn p1_scripted(w: &Wram) -> bool {
    b(w, STAGE_CLEAR) != 0
        || matches!(
            b(w, ram::X_ACTION),
            act::STAGE_EXIT | act::HOLD | act::CAPTURED | act::CAPTURED_HELD | act::CAPTURED_DOWN | 0x34..=0x40 | 0x46
        )
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

/// P2 fell into a pit: the ROM's own pit rule for X (camera code
/// `$80:E14A`): `Y - $20 >= cam Y + $E0`, signed like its `CMP`/`BMI`; X
/// moves several px a frame, so P1 is seen dying at `y - cam_y = 257`.
/// The caller only counts it while P2 is airborne: a grounded P2 down there
/// was left behind by a scroll (M5, Y 575 with the camera at 256), not dead.
pub fn p2_lost(p2_y: u16, cam_y: u16) -> bool {
    (p2_y.wrapping_sub(0x20).wrapping_sub(cam_y.wrapping_add(0xE0)) as i16) >= 0
}

/// The level's side walls: the camera's follow mode (`$80:E0DE` Rev 1,
/// after the limit clamps) keeps X within 8 px of the camera limits
/// (`left` = `$1E56`, `right` = `$1E58`, both camera X), i.e. in
/// `[left + 8, right + 248]`; P1 only. Signed compares, as the ROM's `BPL`.
pub fn p2_level_clamp(x: u16, left: u16, right: u16) -> u16 {
    let x = if (x.wrapping_add(8).wrapping_sub(right.wrapping_add(0x100)) as i16) >= 0 { right.wrapping_add(0xF8) } else { x };
    if (x.wrapping_sub(8).wrapping_sub(left) as i16) < 0 {
        left.wrapping_add(8)
    } else {
        x
    }
}

/// P2's anchor is inside the 256x224 window.
pub fn p2_on_screen(x: u16, y: u16, cam_x: u16, cam_y: u16) -> bool {
    x.wrapping_sub(cam_x) < 256 && in_view_y(y, cam_y)
}

/// P2 is a whole screen or more outside the window: warp it now.
pub fn p2_far(x: u16, y: u16, cam_x: u16, cam_y: u16) -> bool {
    let dx = x.wrapping_sub(cam_x) as i16;
    let dy = y.wrapping_sub(cam_y) as i16;
    !(-256..512).contains(&dx) || !(-224..448).contains(&dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_clamp() {
        assert_eq!(p2_level_clamp(0xFFF0, 0, 0x1000), 8); // highway-start, walking left
        assert_eq!(p2_level_clamp(8, 0, 0x1000), 8);
        assert_eq!(p2_level_clamp(300, 0, 0x1000), 300);
        assert_eq!(p2_level_clamp(0x1100, 0, 0x1000), 0x10F8);
    }

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
    fn coop_stages_scripted() {
        let free = [act::STAND, act::READY, act::WALK, act::JUMP, act::FALL, act::LAND, act::HURT, act::WALL_KICK, act::WALL_SLIDE, 0x18, 0x1C, 0x24];
        for a in free {
            let mut w = gameplay();
            set(&mut w, ram::X_ACTION, a);
            assert!(!p1_scripted(&w), "act {a:02X}");
            set(&mut w, STAGE_CLEAR, 1);
            assert!(p1_scripted(&w), "act {a:02X}, stage clear");
        }
        for a in [act::STAGE_EXIT, act::HOLD, act::CAPTURED, act::CAPTURED_HELD, act::CAPTURED_DOWN, 0x34, 0x40, 0x46] {
            let mut w = gameplay();
            set(&mut w, ram::X_ACTION, a);
            assert!(p1_scripted(&w), "act {a:02X}");
        }
    }

    #[test]
    fn coop_stages_lost() {
        assert!(!p2_lost(367, 256)); // on the road
        assert!(p2_lost(895, 256)); // M0: sank through the pit
        assert!(p2_lost(513, 256)); // where P1's pit death was seen
        assert!(p2_lost(512, 256) && !p2_lost(511, 256)); // the ROM's threshold
        assert!(!p2_lost(0x10, 0)); // signed: near the top of the level
        assert!(p2_lost(575, 256) && !p2_lost(575, 479)); // M5: stranded below after the climb
        assert!(in_view_y(256, 256) && !in_view_y(480, 256));
    }

    #[test]
    fn coop_stages_follow() {
        assert!(p2_on_screen(100, 367, 0, 256) && !p2_on_screen(256, 367, 0, 256));
        assert!(!p2_on_screen(3413, 431, 3416, 256)); // hw-e: left of the window
        assert!(!p2_on_screen(100, 575, 0, 256));
        assert!(!p2_far(3413, 431, 3416, 256) && !p2_far(2648, 575, 2520, 256));
        assert!(p2_far(1000, 367, 1300, 256) && p2_far(1812, 367, 1300, 256) && !p2_far(1811, 367, 1300, 256));
        assert!(p2_far(100, 704, 0, 256) && !p2_far(100, 703, 0, 256) && p2_far(100, 31, 0, 256));
        assert!(!p2_far(10, 367, 0xFFF0, 256)); // wraps like the ROM's words
    }
}
