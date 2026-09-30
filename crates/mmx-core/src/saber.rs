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
/// Hit box `+20` (bank `$86`): `$BF56` = `00 00 14 14`, 40 x 40 px around
/// the shot's position.
pub const BOX: u16 = 0xBF56;
/// Box centre relative to P2's `+05/+08`: in front of Zero, head high (the
/// arc of frames 88/97 on the sheet).
pub const REACH_X: i16 = 16;
pub const REACH_Y: i16 = -16;
pub const FRAMES_PER_POSE: u8 = 4;
const GROUND: [usize; 5] = [86, 87, 88, 89, 90];
const AIR: [usize; 4] = [95, 96, 97, 98];
/// Pose index of the arc (88, 97).
const ARC: usize = 2;

/// Swing state (in `CoopState`, so rollback carries it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, bincode::Encode, bincode::Decode)]
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
        p[(usize::from(s.t - 1) / usize::from(FRAMES_PER_POSE)).min(p.len() - 1)]
    })
}

/// Called with P2's pad before its update slice: start / advance / end the
/// swing, return the pad the ROM gets. Pure but for `s`.
pub fn update(s: &mut Saber, action: u8, pad: u16) -> u16 {
    let x = pad & BTN_X != 0;
    let pressed = x && !s.btn;
    s.btn = x;
    let pad = pad & !BTN_X;
    if s.t > 0 {
        s.t += 1;
        if action == 0x0E || action == 0x0C || usize::from(s.t) > poses(s.air).len() * usize::from(FRAMES_PER_POSE) {
            s.t = 0; // hurt / dead / done
        }
    } else if pressed {
        // stand, ready, walk, land, dash / jump, fall. Not on walls, not hurt.
        let (ground, air) = (matches!(action, 0x00 | 0x02 | 0x04 | 0x0A | 0x14), matches!(action, 0x06 | 0x08));
        if ground || air {
            *s = Saber { t: 1, air, hit: false, btn: true, slot: 0xFF };
        }
    }
    match s.t {
        0 => pad,
        _ if s.air => pad & (BTN_LEFT | BTN_RIGHT),
        _ => 0,
    }
}

/// The swing is on its arc pose and has not hit yet.
fn arc(s: &Saber) -> bool {
    s.t > 0 && !s.hit && usize::from(s.t - 1) / usize::from(FRAMES_PER_POSE) == ARC
}

/// The shot the arc puts into the table (`+05/+08` = the box centre). Pure.
pub fn shot_bytes(p2: &[u8]) -> [u8; SHOT_LEN] {
    let w = |o: usize| i16::from_le_bytes([p2[o], p2[o + 1]]);
    let right = p2[0x11] & 0x40 != 0;
    let x = w(OFF_POS_X).wrapping_add(if right { REACH_X } else { -REACH_X });
    let y = w(OFF_POS_Y).wrapping_add(REACH_Y);
    let mut s = [0u8; SHOT_LEN];
    s[0x00] = 1;
    s[0x0A] = DAMAGE_ID;
    s[0x05..0x07].copy_from_slice(&x.to_le_bytes());
    s[0x08..0x0A].copy_from_slice(&y.to_le_bytes());
    s[0x11] = p2[0x11];
    s[0x20..0x22].copy_from_slice(&BOX.to_le_bytes());
    s
}

/// [`HOOK_SHOTS_DONE`]: put the arc's shot into a free slot for the enemy loop.
pub fn shots_done(game: &mut Game) {
    let c = &game.coop;
    if !c.enabled || c.p2_hidden || game.zero_set.is_none() || !arc(&c.saber) {
        return;
    }
    let shot = shot_bytes(&c.p2);
    let w = game.wram_mut();
    let Some(k) = (0..8).find(|&k| w[ix(SHOTS) + k * SHOT_LEN] == 0) else { return };
    w[ix(SHOTS) + k * SHOT_LEN..][..SHOT_LEN].copy_from_slice(&shot);
    game.coop.saber.slot = k as u8;
}

/// Physics hook (after the enemy loop): take the shot back out; a hit
/// (`+30` bumped by `$84:9E15`) ends the swing's damage.
pub fn take_back(game: &mut Game) {
    let k = game.coop.saber.slot;
    if k == 0xFF {
        return;
    }
    game.coop.saber.slot = 0xFF;
    let o = ix(SHOTS) + usize::from(k) * SHOT_LEN;
    let w = game.wram_mut();
    if w[o + 0x30] != 0 {
        game.coop.saber.hit = true;
    }
    game.wram_mut()[o..o + SHOT_LEN].fill(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saber_swing_ground_and_air() {
        let mut s = Saber { slot: 0xFF, ..Default::default() };
        assert_eq!(update(&mut s, 0x00, 0x0100), 0x0100);
        assert_eq!(frame(&s), None);
        assert_eq!(update(&mut s, 0x04, BTN_X | 0x0100), 0); // starts, pad masked
        assert_eq!(frame(&s), Some(86));
        let mut seen = vec![86];
        for _ in 0..30 {
            update(&mut s, 0x00, BTN_X); // held: no restart
            seen.extend(frame(&s));
        }
        assert_eq!(seen.len(), 20);
        assert_eq!(seen[8..12], [88; 4]);
        assert_eq!(s.t, 0);
        // Air: left/right pass.
        update(&mut s, 0x08, 0);
        assert_eq!(update(&mut s, 0x08, BTN_X | 0x0100 | 0x8000), 0x0100);
        assert_eq!(frame(&s), Some(95));
        // Hurt cancels; walls don't start.
        update(&mut s, 0x0E, 0);
        assert_eq!(s.t, 0);
        update(&mut s, 0x12, 0);
        update(&mut s, 0x12, BTN_X);
        assert_eq!(s.t, 0);
    }

    #[test]
    fn saber_shot_in_front() {
        let mut p2 = [0u8; 0x90];
        p2[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&500u16.to_le_bytes());
        p2[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&367u16.to_le_bytes());
        p2[0x11] = 0x62; // facing right
        let s = shot_bytes(&p2);
        assert_eq!((u16::from_le_bytes([s[5], s[6]]), u16::from_le_bytes([s[8], s[9]])), (516, 351));
        assert_eq!((s[0], s[0x0A], s[0x30]), (1, DAMAGE_ID, 0));
        p2[0x11] = 0x22;
        assert_eq!(u16::from_le_bytes([shot_bytes(&p2)[5], shot_bytes(&p2)[6]]), 484);
    }

    /// hw-spiky (docs/coop.md): P2 walks right 20 frames, X at 22; the
    /// Spiky (enemy slot 0, 2 HP) dies on the arc. Needs both ROMs and the state.
    #[test]
    fn saber_kills_spiky() {
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
        assert!(seen[28..33].iter().any(|&(zf, hp)| zf == 88 && hp == 0), "{seen:?}");
        assert_eq!(g.wram()[0x0BDD], 0, "P1's shot count untouched");
    }
}

