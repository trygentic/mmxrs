//! P2's Z-saber (only with the X3 Zero set loaded). See docs/zero.md
//! "Playing as Zero".
//!
//! Swing: pad 2's X (`BTN_X`, unused by X1's default config) starts it when
//! P2 stands, walks, lands or dashes (ground: X3 frames 86-90) or is in the
//! air (95-98), [`FRAMES_PER_POSE`] frames per pose. P2's normal update keeps
//! running; for the swing its pad is masked (ground: nothing, Zero stops;
//! air: left/right only, the jump carries on), and the display shows the
//! saber frame instead of the ROM's.
//!
//! Damage: for the arc pose (88 / 97) a shot object is put into a free slot
//! of X's shot table `$1228` *after* the shot loop has run
//! ([`HOOK_SHOTS_DONE`]) and taken out again at the physics hook (after the
//! enemy loop). Enemies test it in their own AI through the ROM's
//! enemy-vs-shot routine `$84:9B43` (shot `+0` active, `+30 = 0`, box `+20`
//! against the enemy's) and damage themselves through `$84:9E15` (damage =
//! the enemy's table `$86:EEDB[+28]` at the shot id `+0A`). The shot's own
//! update never runs, it is never drawn and never counted in `$0BDD`, so no
//! shot bookkeeping (docs/coop.md "Shots (B3)") is involved. One hit per
//! swing: the damage routine does `INC +30` on the shot, which ends it.
//!
//! Wave: the arc also throws X3's green wave ([`Wave`], data
//! `zero_data::x3_wave_step`), which flies on for [`WAVE_FRAMES`] frames
//! after the swing; its own shot follows it (same one hit per swing).

use crate::coop::{OFF_POS_X, OFF_POS_Y};
use crate::ram::wram_index as ix;
use crate::{Game, BTN_LEFT, BTN_RIGHT, BTN_X};

/// The instruction after the object master's `JSR $80:D3B2` (the shot loop,
/// at `$80:D1FC`); the enemy loops follow. Confirmed by `trace --calls`.
pub const HOOK_SHOTS_DONE: u32 = 0x80D1FF;
const SHOTS: u32 = 0x7E1228;
const SHOT_LEN: usize = 0x40;
/// Shot id `+0A` whose damage the saber deals: 2 = X's fully charged buster
/// shot (`$86:EEDB` row 3, the Highway enemies and Bee Blader: lemon 1,
/// half charge 2, full charge 4).
pub const DAMAGE_ID: u8 = 2;
/// Hit box `+20` (bank `$86`, `x, y, half w, half h`; the enemy-vs-shot
/// test `$84:9C0E` mirrors `x` by the shot's facing): `$BFC4` = `00 01 13 0A`,
/// 38 x 20 px, centred 1 px below the shot's position.
pub const BOX: u16 = 0xBFC4;
/// Shot position relative to P2's `+05/+08` (x toward Zero's facing), so the
/// box covers the blade as drawn: frame 88 on screen (`hw-spiky`, docs/zero.md)
/// runs from 13 px behind to 26 px in front, 29 to 8 px above `+08`; frame
/// 97's sits 3 px higher.
pub const REACH_X: i16 = 7;
pub const REACH_Y: i16 = -19;
pub const REACH_Y_AIR: i16 = -22;
pub const FRAMES_PER_POSE: u8 = 4;
/// The arc pose is held longer than X3 does (4 frames, ~67 ms, half of it
/// lost while blinking): playtesters could not see the blade at all.
pub const ARC_FRAMES: u8 = 10;
const GROUND: [usize; 5] = [86, 87, 88, 89, 90];
const AIR: [usize; 4] = [95, 96, 97, 98];
/// Pose index of the arc (88, 97).
const ARC: usize = 2;

fn pose_len(i: usize) -> u8 {
    if i == ARC { ARC_FRAMES } else { FRAMES_PER_POSE }
}

/// Pose index for swing frame `t` (1-based), `None` past the end.
fn pose_index(t: u8, air: bool) -> Option<usize> {
    pose_at(t, air).map(|p| p.0)
}

/// (pose index, frame within the pose from 0) for swing frame `t`.
fn pose_at(t: u8, air: bool) -> Option<(usize, u8)> {
    let mut left = t.checked_sub(1)?;
    for i in 0..poses(air).len() {
        if left < pose_len(i) {
            return Some((i, left));
        }
        left -= pose_len(i);
    }
    None
}

/// The green slash arc's frame (`zero_data::X3_SLASH_SET`; air + 6) while
/// swinging: the arc pose shows the rising arc (2) for 2 frames, then the
/// full crescent (3); the pose after it the tail (4, 5; 2 frames each).
/// Zero's own swing has no arc in X3, so this timing is ours. Pure.
pub fn slash_frame(s: &Saber) -> Option<usize> {
    let (i, k) = pose_at(s.t, s.air)?;
    let f = match (i, k) {
        (ARC, 0 | 1) => 2,
        (ARC, _) => 3,
        (3, 0 | 1) => 4,
        (3, 2 | 3) => 5,
        _ => return None,
    };
    Some(if s.air { f + 6 } else { f })
}

/// The wave's life in frames: X3's died after 31 (both runs, about 197 px
/// out, just off screen).
pub const WAVE_FRAMES: u8 = 31;
/// Wave hit boxes in X1's bank `$86` matching X3's (`x y half-w half-h`):
/// `$BDC1` = `00 00 04 04` (the spawn frame, X3 `$B7FB`), `$D381` =
/// `00 00 10 09` (32 x 18 px, X3 `$B807`/`$B80F`; X3's x offset goes into
/// the shot's position).
pub const WAVE_BOX_SPAWN: u16 = 0xBDC1;
pub const WAVE_BOX: u16 = 0xD381;

/// A flying wave (in [`Saber`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct Wave {
    /// Frames since the spawn + 1, 0 = none.
    pub t: u8,
    pub air: bool,
    pub right: bool,
    /// Zero's drawn origin at the spawn, world coordinates (`+05`, `+08 +
    /// (i8)+19 + X3_Y_OFFSET`).
    pub x: u16,
    pub y: u16,
}

impl Wave {
    /// `(world x, world y, wave frame, box)` now, `None` when there is none.
    /// Pure.
    pub fn at(&self) -> Option<(u16, u16, usize, crate::zero_data::HitBox)> {
        let (dx, dy, f, b) = crate::zero_data::x3_wave_step(self.t.checked_sub(1)?, self.air);
        let dx = if self.right { dx } else { -dx };
        Some((self.x.wrapping_add(dx as u16), self.y.wrapping_add(dy as u16), f, b))
    }
}

/// Swing state (in `CoopState`, so rollback carries it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct Saber {
    /// Frames into the swing, 0 = none.
    pub t: u8,
    pub air: bool,
    /// The swing has hit something.
    pub hit: bool,
    /// Pad 2's X last frame (edge detect).
    pub btn: bool,
    /// Shot slot (0-7) in use this frame, `0xFF` = none.
    pub slot: u8,
    pub wave: Wave,
    /// The wave's shot slot this frame, `0xFF` = none.
    pub wave_slot: u8,
}

impl Default for Saber {
    fn default() -> Self {
        Self { t: 0, air: false, hit: false, btn: false, slot: 0xFF, wave: Wave::default(), wave_slot: 0xFF }
    }
}

fn poses(air: bool) -> &'static [usize] {
    if air {
        &AIR
    } else {
        &GROUND
    }
}

/// The X3 frame to show while swinging. Pure.
pub fn frame(s: &Saber) -> Option<usize> {
    (s.t > 0).then(|| {
        let p = poses(s.air);
        p[pose_index(s.t, s.air).unwrap_or(p.len() - 1)]
    })
}

/// Called with the player's block and pad before its update: start /
/// advance / end the swing and its wave, return the pad the ROM gets. Pure
/// but for `s`.
pub fn update(s: &mut Saber, obj: &[u8], pad: u16) -> u16 {
    let action = obj[0x02];
    if s.wave.t > 0 {
        s.wave.t = if s.wave.t >= WAVE_FRAMES { 0 } else { s.wave.t + 1 };
    }
    let x = pad & BTN_X != 0;
    let pressed = x && !s.btn;
    s.btn = x;
    let pad = pad & !BTN_X;
    if s.t > 0 {
        s.t += 1;
        if action == 0x0E || action == 0x0C || pose_index(s.t, s.air).is_none() {
            s.t = 0; // hurt / dead / done
        }
    } else if pressed {
        // stand, ready, walk, land, dash / jump, fall. Not on walls, not hurt.
        let (ground, air) = (matches!(action, 0x00 | 0x02 | 0x04 | 0x0A | 0x14), matches!(action, 0x06 | 0x08));
        if ground || air {
            *s = Saber { t: 1, air, btn: true, wave: s.wave, ..Saber::default() };
        }
    }
    // The arc's first frame throws the wave (replacing an older one).
    if s.t > 1 && pose_index(s.t, s.air) == Some(ARC) && pose_index(s.t - 1, s.air) != Some(ARC) {
        let w = |o: usize| u16::from_le_bytes([obj[o], obj[o + 1]]);
        let y = w(OFF_POS_Y).wrapping_add(obj[0x19] as i8 as u16).wrapping_add(crate::zero::X3_Y_OFFSET as u16);
        s.wave = Wave { t: 1, air: s.air, right: obj[0x11] & 0x40 != 0, x: w(OFF_POS_X), y };
    }
    match s.t {
        0 => pad,
        _ if s.air => pad & (BTN_LEFT | BTN_RIGHT),
        _ => 0,
    }
}

/// The swing is on its arc pose and has not hit yet.
fn arc(s: &Saber) -> bool {
    s.t > 0 && !s.hit && pose_index(s.t, s.air) == Some(ARC)
}

/// The shot the arc puts into the table (`+05/+08` = the box centre). Pure.
pub fn shot_bytes(p2: &[u8], air: bool) -> [u8; SHOT_LEN] {
    let w = |o: usize| i16::from_le_bytes([p2[o], p2[o + 1]]);
    let right = p2[0x11] & 0x40 != 0;
    let x = w(OFF_POS_X).wrapping_add(if right { REACH_X } else { -REACH_X });
    let y = w(OFF_POS_Y).wrapping_add(if air { REACH_Y_AIR } else { REACH_Y });
    let mut s = [0u8; SHOT_LEN];
    s[0x00] = 1;
    s[0x0A] = DAMAGE_ID;
    s[0x05..0x07].copy_from_slice(&x.to_le_bytes());
    s[0x08..0x0A].copy_from_slice(&y.to_le_bytes());
    s[0x11] = p2[0x11];
    s[0x20..0x22].copy_from_slice(&BOX.to_le_bytes());
    s
}

/// The wave's shot (`+05/+08` = its box centre), `None` without a wave or
/// after the swing's hit. Pure.
pub fn wave_shot_bytes(s: &Saber) -> Option<[u8; SHOT_LEN]> {
    let (x, y, _, (bx, by, _, _)) = s.wave.at().filter(|_| !s.hit)?;
    let bx = i16::from(bx);
    let x = x.wrapping_add(if s.wave.right { bx } else { -bx } as u16);
    let mut b = [0u8; SHOT_LEN];
    b[0x00] = 1;
    b[0x0A] = DAMAGE_ID;
    b[0x05..0x07].copy_from_slice(&x.to_le_bytes());
    b[0x08..0x0A].copy_from_slice(&y.wrapping_add(i16::from(by) as u16).to_le_bytes());
    b[0x11] = if s.wave.right { 0x40 } else { 0 };
    let bp = if s.wave.t == 1 { WAVE_BOX_SPAWN } else { WAVE_BOX };
    b[0x20..0x22].copy_from_slice(&bp.to_le_bytes());
    Some(b)
}

/// [`HOOK_SHOTS_DONE`]: put each swing's arc shot and wave shot (a Zero
/// P1's, P2's) into free slots for the enemy loop.
pub fn shots_done(game: &mut Game) {
    if game.zero_set.is_none() {
        return;
    }
    if game.coop.p1.on {
        if arc(&game.coop.p1.saber) {
            let b = ix(crate::ram::X_BASE);
            let shot = shot_bytes(&game.wram()[b..b + crate::ram::X_BLOCK_LEN], game.coop.p1.saber.air);
            game.coop.p1.saber.slot = put_shot(game, &shot);
        }
        if let Some(shot) = wave_shot_bytes(&game.coop.p1.saber) {
            game.coop.p1.saber.wave_slot = put_shot(game, &shot);
        }
    }
    let c = &game.coop;
    if c.enabled && !c.p2_hidden {
        if arc(&c.saber) {
            let shot = shot_bytes(&c.p2, c.saber.air);
            game.coop.saber.slot = put_shot(game, &shot);
        }
        if let Some(shot) = wave_shot_bytes(&game.coop.saber) {
            game.coop.saber.wave_slot = put_shot(game, &shot);
        }
    }
}

/// Into the first free slot; the slot, or `0xFF` when the table is full.
fn put_shot(game: &mut Game, shot: &[u8; SHOT_LEN]) -> u8 {
    let w = game.wram_mut();
    let Some(k) = (0..8).find(|&k| w[ix(SHOTS) + k * SHOT_LEN] == 0) else { return 0xFF };
    w[ix(SHOTS) + k * SHOT_LEN..][..SHOT_LEN].copy_from_slice(shot);
    k as u8
}

/// Physics hook (after the enemy loop): take the shots back out; a hit
/// (`+30` bumped by `$84:9E15`) ends the swing's damage.
pub fn take_back(game: &mut Game) {
    let mut p1 = game.coop.p1.saber;
    let mut p2 = game.coop.saber;
    for s in [&mut p1, &mut p2] {
        for k in [std::mem::replace(&mut s.slot, 0xFF), std::mem::replace(&mut s.wave_slot, 0xFF)] {
            if k == 0xFF {
                continue;
            }
            let o = ix(SHOTS) + usize::from(k) * SHOT_LEN;
            let w = game.wram_mut();
            s.hit |= w[o + 0x30] != 0 || w[o + 0x01] != 0; // +30 bumped, or +01 = 6 (the hit state)
            w[o..o + SHOT_LEN].fill(0);
        }
    }
    game.coop.p1.saber = p1;
    game.coop.saber = p2;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saber_swing_ground_and_air() {
        let blk = |a: u8| {
            let mut b = [0u8; 0x90];
            b[2] = a;
            b
        };
        let mut s = Saber::default();
        assert_eq!(update(&mut s, &blk(0x00), 0x0100), 0x0100);
        assert_eq!(frame(&s), None);
        assert_eq!(update(&mut s, &blk(0x04), BTN_X | 0x0100), 0); // starts, pad masked
        assert_eq!(frame(&s), Some(86));
        let mut seen = vec![86];
        for _ in 0..30 {
            update(&mut s, &blk(0x00), BTN_X); // held: no restart
            seen.extend(frame(&s));
        }
        // 4 + 4 + ARC_FRAMES + 4 + 4 poses
        assert_eq!(seen.len(), 16 + usize::from(ARC_FRAMES));
        assert_eq!(seen[8..8 + usize::from(ARC_FRAMES)], vec![88; usize::from(ARC_FRAMES)][..]);
        assert_eq!(s.t, 0);
        // Air: left/right pass.
        update(&mut s, &blk(0x08), 0);
        assert_eq!(update(&mut s, &blk(0x08), BTN_X | 0x0100 | 0x8000), 0x0100);
        assert_eq!(frame(&s), Some(95));
        // Hurt cancels; walls don't start.
        update(&mut s, &blk(0x0E), 0);
        assert_eq!(s.t, 0);
        update(&mut s, &blk(0x12), 0);
        update(&mut s, &blk(0x12), BTN_X);
        assert_eq!(s.t, 0);
    }

    #[test]
    fn saber_shot_in_front() {
        let mut p2 = [0u8; 0x90];
        p2[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&500u16.to_le_bytes());
        p2[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&367u16.to_le_bytes());
        p2[0x11] = 0x62; // facing right
        let s = shot_bytes(&p2, false);
        assert_eq!((u16::from_le_bytes([s[5], s[6]]), u16::from_le_bytes([s[8], s[9]])), (507, 348));
        assert_eq!((s[0], s[0x0A], s[0x30]), (1, DAMAGE_ID, 0));
        p2[0x11] = 0x22;
        let s = shot_bytes(&p2, true);
        assert_eq!((u16::from_le_bytes([s[5], s[6]]), u16::from_le_bytes([s[8], s[9]])), (493, 345));
    }

    /// hw-spiky (docs/coop.md): P2 walks right 20 frames, X at 22; the
    /// Spiky (enemy slot 0, 2 HP, 1 per hit of id 2) is hit on the arc, at the
    /// blade's tip (its box edge 26 px in front). Needs both ROMs and the state.
    #[test]
    fn saber_hits_spiky() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let mut g = crate::Game::from_rom(std::fs::read(x1).unwrap()).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        g.load_state(&state).unwrap();
        g.coop.enabled = true;
        crate::coop::register_coop_traps(&mut g);
        let hp = |g: &crate::Game| g.wram()[0x0E68 + 0x27] & 0x7F;
        let mut seen = Vec::new();
        for f in 1..=40 {
            let p2 = match f {
                ..=20 => BTN_RIGHT,
                22 | 23 => BTN_X,
                _ => 0,
            };
            g.step(crate::Pads { p1: 0, p2 });
            seen.push((g.coop.zero_frame, hp(&g)));
        }
        assert_eq!(seen[25].1, 2, "alive before the arc: {seen:?}");
        assert!(seen[28..33].iter().any(|&(zf, hp)| zf == 88 && hp == 1), "{seen:?}");
        assert_eq!(g.wram()[0x0BDD], 0, "P1's shot count untouched");
    }

    #[test]
    fn saber_wave_spawns_on_arc_and_flies() {
        let mut blk = [0u8; 0x90];
        blk[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&500u16.to_le_bytes());
        blk[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&367u16.to_le_bytes());
        blk[0x11] = 0x62; // facing right
        let mut s = Saber::default();
        update(&mut s, &blk, BTN_X);
        let mut t = 1;
        while s.wave.t == 0 {
            update(&mut s, &blk, 0);
            t += 1;
        }
        assert_eq!(frame(&s), Some(88), "spawns on the arc's first frame (swing frame {t})");
        assert_eq!((s.wave.x, s.wave.y), (500, 367 - 8));
        let shot = wave_shot_bytes(&s).unwrap();
        assert_eq!(u16::from_le_bytes([shot[5], shot[6]]), 508); // spawn: 8 in front, 4x4 box
        for _ in 0..4 {
            update(&mut s, &blk, 0);
        }
        // t = 5 (step 4): 29 + 6 in front, box 15 further, 1 below the origin
        let shot = wave_shot_bytes(&s).unwrap();
        assert_eq!((u16::from_le_bytes([shot[5], shot[6]]), u16::from_le_bytes([shot[8], shot[9]])), (550, 360));
        assert_eq!(u16::from_le_bytes([shot[0x20], shot[0x21]]), WAVE_BOX);
        s.hit = true;
        assert_eq!(wave_shot_bytes(&s), None, "one hit per swing");
        for _ in 0..usize::from(WAVE_FRAMES) {
            update(&mut s, &blk, 0);
        }
        assert_eq!(s.wave.t, 0);
    }

    /// hw-spiky: P2 swings where it stands, 88 px from the Spiky: the blade
    /// cannot reach, the wave takes it 2 -> 1, once.
    #[test]
    fn saber_wave_hits_spiky_from_afar() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let mut g = crate::Game::from_rom(std::fs::read(x1).unwrap()).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        g.load_state(&state).unwrap();
        g.coop.enabled = true;
        crate::coop::register_coop_traps(&mut g);
        let hp = |g: &crate::Game| g.wram()[0x0E68 + 0x27] & 0x7F;
        let mut seen = Vec::new();
        for f in 1..=60 {
            g.step(crate::Pads { p1: 0, p2: if f == 2 { BTN_X } else { 0 } });
            let gap = u16::from_le_bytes([g.wram()[0x0E68 + 5], g.wram()[0x0E68 + 6]]) as i32
                - i32::from(u16::from_le_bytes([g.coop.p2[OFF_POS_X], g.coop.p2[OFF_POS_X + 1]]));
            seen.push((hp(&g), gap));
        }
        let hit = seen.iter().position(|&(hp, _)| hp == 1).expect("the wave hits");
        assert!(seen[hit].1 > 60, "out of the blade's reach: {seen:?}");
        assert!(seen[hit..].iter().all(|&(hp, _)| hp == 1), "one hit per swing: {seen:?}");
    }

    /// Single player, P1 as Zero (`hw-spiky`): walk right 26 frames, X at 28;
    /// the Spiky loses 1 HP on the arc pose, P1's buster count stays 0.
    #[test]
    fn saber_p1_hits_spiky() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let mut g = crate::Game::from_rom(std::fs::read(x1).unwrap()).unwrap();
        assert!(g.set_p1_zero(true).is_err(), "needs the X3 set");
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        g.load_state(&state).unwrap();
        g.set_p1_zero(true).unwrap();
        let hp = |g: &crate::Game| g.wram()[0x0E68 + 0x27] & 0x7F;
        let mut seen = Vec::new();
        for f in 1..=44 {
            let p1 = match f {
                ..=26 => BTN_RIGHT,
                28 | 29 => BTN_X,
                _ => 0,
            };
            g.step(crate::Pads { p1, p2: 0 });
            seen.push((g.coop.p1.frame, hp(&g)));
        }
        assert_eq!(seen[30].1, 2, "alive before the arc: {seen:?}");
        assert!(seen[31..44].iter().any(|&(zf, hp)| zf == 88 && hp == 1), "{seen:?}");
        assert_eq!(g.wram()[0x0BDD], 0);
    }

    /// Netplay: a 2-frame rollback every frame through the swing draws the
    /// same pictures as the straight run (page, mask and `zero_frame` travel
    /// together in `net_save`).
    #[test]
    fn saber_frames_survive_rollback() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let new = || {
            let mut g = crate::Game::from_rom(std::fs::read(&x1).unwrap()).unwrap();
            g.set_x3_rom(&std::fs::read(&x3).unwrap()).unwrap();
            g.load_state(&state).unwrap();
            g.coop.enabled = true;
            crate::coop::register_coop_traps(&mut g);
            g.set_p1_zero(true).unwrap(); // both Zeros: shadow pages 0 and 1
            g
        };
        let pad = |f: usize| crate::Pads {
            p1: if f == 12 { BTN_X } else { 0 },
            p2: if f <= 20 { BTN_RIGHT } else if f == 22 { BTN_X } else { 0 },
        };
        let (mut a, mut b) = (new(), new());
        let mut saves = Vec::new();
        for f in 1..=40 {
            a.step(pad(f));
            saves.push(crate::netgame::net_save(&b));
            b.step(pad(f));
            if f > 2 {
                crate::netgame::net_load(&mut b, &saves[f - 3]).unwrap();
                for r in f - 2..=f {
                    b.step(pad(r));
                }
            }
            assert_eq!(a.frame().0, b.frame().0, "frame {f} (zf {})", a.coop.zero_frame);
        }
    }
}

