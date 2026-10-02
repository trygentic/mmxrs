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
//! Damage: while the slash arc is drawn (the arc pose 88 / 97 and the tail
//! pose after it) a shot object is put into a free slot
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
//! The green slash arc (`zero_data::X3_SLASH_SET`, [`slash_frame`]) is
//! drawn on Zero by `zero.rs`; the damage box ([`BOX_GROUND`], [`BOX_AIR`],
//! carried in the shot itself) covers the blade and the arc's drawn extent.

use crate::coop::{OFF_POS_X, OFF_POS_Y};
use crate::ram::wram_index as ix;
use crate::{Game, BTN_LEFT, BTN_RIGHT, BTN_X};

/// The instruction after the object master's `JSR $80:D3B2` (the shot loop,
/// at `$80:D1FC`); the enemy loops follow. Confirmed by `trace --calls`.
pub const HOOK_SHOTS_DONE: crate::hooks::HookId = crate::hooks::HookId::ShotsDone;
const SHOTS: u32 = 0x7E1228;
const SHOT_LEN: usize = 0x40;
/// Shot id `+0A` whose damage the saber deals: 2 = X's fully charged buster
/// shot (`$86:EEDB` row 3, the Highway enemies and Bee Blader: lemon 1,
/// half charge 2, full charge 4).
pub const DAMAGE_ID: u8 = 2;
/// The damage box, against Zero's drawn origin (`+05`, `+08 + (i8)+19 +
/// X3_Y_OFFSET`): centre `(x toward his facing, y)` and half sizes. It is
/// the union of every opaque pixel of the body poses (blade included) and
/// the slash arc while the arc is drawn (test `saber_box_covers_swing`,
/// from the X3 ROM's frames), plus [`MARGIN`] in front, behind and above;
/// below it stops at his feet. Ground (86-89, arc 2-5): pixels 48 in front
/// to 21 behind, 39 above to 23 below (the feet) -> x -29..57, y -47..23.
/// Air (97-98, arc 8-11): 53 / 23 / 39 / 27 -> x -31..61, y -47..27.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitBox {
    pub x: i16,
    pub y: i16,
    pub half_w: u8,
    pub half_h: u8,
}
pub const MARGIN: i16 = 8;
pub const BOX_GROUND: HitBox = HitBox { x: 14, y: -12, half_w: 43, half_h: 35 };
pub const BOX_AIR: HitBox = HitBox { x: 15, y: -10, half_w: 46, half_h: 37 };
/// No box in X1's bank `$86` has these sizes on every dump (the bank moves
/// between releases), so the shot carries its own: the enemy-vs-shot test
/// `$84:9C0E` reads the box `x, y, half w, half h` at `DB = $86` through
/// the 16-bit pointer `+20`, and `$86:0000-$1FFF` is low WRAM, so `+20`
/// points at bytes `+BOX_AT..+BOX_AT+3` of the shot's own slot (unused by
/// the enemy code; the slot is cleared again after the enemy loop).
pub const BOX_AT: usize = 0x38;
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
}

impl Default for Saber {
    fn default() -> Self {
        Self { t: 0, air: false, hit: false, btn: false, slot: 0xFF }
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
/// advance / end the swing, return the pad the ROM gets. Pure but for `s`.
pub fn update(s: &mut Saber, obj: &[u8], pad: u16) -> u16 {
    let action = obj[0x02];
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
            *s = Saber { t: 1, air, btn: true, ..Saber::default() };
        }
    }
    match s.t {
        0 => pad,
        _ if s.air => pad & (BTN_LEFT | BTN_RIGHT),
        _ => 0,
    }
}

/// First frame of the arc pose: where X3's whoosh plays (`sfx.rs`; X3 plays
/// it 10 frames into Zero's swing, with the ball he throws after the arc).
pub fn whoosh(s: &Saber) -> bool {
    pose_at(s.t, s.air) == Some((ARC, 0))
}

/// The swing deals damage (the slash arc is drawn: the arc pose and the
/// tail after it) and has not hit yet.
fn arc(s: &Saber) -> bool {
    !s.hit && slash_frame(s).is_some()
}

/// The shot the swing puts into the table (`+05/+08` = the box centre, the
/// box itself at [`BOX_AT`]; `+20` is set by the slot it goes to). Pure.
pub fn shot_bytes(p2: &[u8], air: bool) -> [u8; SHOT_LEN] {
    let w = |o: usize| i16::from_le_bytes([p2[o], p2[o + 1]]);
    let right = p2[0x11] & 0x40 != 0;
    let b = if air { BOX_AIR } else { BOX_GROUND };
    let x = w(OFF_POS_X).wrapping_add(if right { b.x } else { -b.x });
    let y = w(OFF_POS_Y).wrapping_add(i16::from(p2[0x19] as i8)).wrapping_add(crate::zero::X3_Y_OFFSET).wrapping_add(b.y);
    let mut s = [0u8; SHOT_LEN];
    s[0x00] = 1;
    s[0x0A] = DAMAGE_ID;
    s[0x05..0x07].copy_from_slice(&x.to_le_bytes());
    s[0x08..0x0A].copy_from_slice(&y.to_le_bytes());
    s[0x11] = p2[0x11];
    s[BOX_AT..BOX_AT + 4].copy_from_slice(&[0, 0, b.half_w, b.half_h]);
    s
}

/// [`HOOK_SHOTS_DONE`]: put each swing's arc shot (a Zero P1's, P2's) into
/// a free slot for the enemy loop.
pub fn shots_done(game: &mut Game) {
    if game.zero_set.is_none() {
        return;
    }
    if game.coop.p1.on && arc(&game.coop.p1.saber) {
        let b = ix(crate::ram::X_BASE);
        let shot = shot_bytes(&game.wram()[b..b + crate::ram::X_BLOCK_LEN], game.coop.p1.saber.air);
        game.coop.p1.saber.slot = put_shot(game, &shot);
    }
    let c = &game.coop;
    if c.enabled && !c.p2_hidden && arc(&c.saber) {
        let shot = shot_bytes(&c.p2, c.saber.air);
        game.coop.saber.slot = put_shot(game, &shot);
    }
}

/// Into the first free slot; the slot, or `0xFF` when the table is full.
fn put_shot(game: &mut Game, shot: &[u8; SHOT_LEN]) -> u8 {
    let w = game.wram_mut();
    let Some(k) = (0..8).find(|&k| w[ix(SHOTS) + k * SHOT_LEN] == 0) else { return 0xFF };
    let o = ix(SHOTS) + k * SHOT_LEN;
    w[o..o + SHOT_LEN].copy_from_slice(shot);
    // `+20` -> the box in this slot (bank `$86` mirrors low WRAM).
    let at = (SHOTS as usize + k * SHOT_LEN + BOX_AT) as u16;
    w[o + 0x20..o + 0x22].copy_from_slice(&at.to_le_bytes());
    k as u8
}

/// Physics hook (after the enemy loop): take the shots back out; a hit
/// (`+30` bumped by `$84:9E15`) ends the swing's damage.
pub fn take_back(game: &mut Game) {
    let mut p1 = game.coop.p1.saber;
    let mut p2 = game.coop.saber;
    for s in [&mut p1, &mut p2] {
        let k = std::mem::replace(&mut s.slot, 0xFF);
        if k != 0xFF {
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

    /// Opaque pixels of `sprites` drawn from `page` (16-wide, 32 B slots),
    /// facing left, as `(front, back, up, down)` against the origin: `front`
    /// = the furthest pixel toward the facing (`-x`), `up` = above (`-y`).
    fn extent(page: &[u8], sprites: &[crate::zero_data::Sprite]) -> Option<(i32, i32, i32, i32)> {
        let mut e: Option<(i32, i32, i32, i32)> = None;
        for s in sprites {
            let w = if s.big { 16 } else { 8 };
            for py in 0..w {
                for px in 0..w {
                    let (tx, ty) = (if s.hflip { w - 1 - px } else { px }, if s.vflip { w - 1 - py } else { py });
                    let slot = usize::from(s.tile) + (ty / 8) * 16 + tx / 8;
                    let Some(t) = page.get(slot * 32..slot * 32 + 32) else { continue };
                    let (r, bit) = (ty % 8, 7 - tx % 8);
                    let v = (t[2 * r] >> bit & 1) | (t[2 * r + 1] >> bit & 1) | (t[16 + 2 * r] >> bit & 1) | (t[17 + 2 * r] >> bit & 1);
                    if v != 0 {
                        let (x, y) = (i32::from(s.x) + px as i32, i32::from(s.y) + py as i32);
                        let n = (-x, -x, -y, y);
                        e = Some(e.map_or(n, |o| (o.0.max(n.0), o.1.min(n.1), o.2.max(n.2), o.3.max(n.3))));
                    }
                }
            }
        }
        e
    }

    /// The damage box covers every opaque pixel of the blade (body poses)
    /// and the slash arc while the arc is drawn, plus [`MARGIN`], ground and
    /// air; measured from the X3 ROM's frames. Prints the extents.
    #[test]
    fn saber_box_covers_swing() {
        let Some(x3) = std::env::var_os("MMX_X3_ROM") else {
            eprintln!("MMX_X3_ROM not set; skipping");
            return;
        };
        let rom = std::fs::read(x3).unwrap();
        let rom = if rom.len() % 0x8000 == 512 { &rom[512..] } else { &rom[..] };
        let set = crate::zero_data::extract_x3(rom).unwrap();
        let sl = set.slash.as_ref().unwrap();
        for air in [false, true] {
            let mut all: Option<(i32, i32, i32, i32)> = None;
            let mut add = |e: Option<(i32, i32, i32, i32)>| {
                if let Some(n) = e {
                    all = Some(all.map_or(n, |o| (o.0.max(n.0), o.1.min(n.1), o.2.max(n.2), o.3.max(n.3))));
                }
            };
            for t in 1..=40 {
                let s = Saber { t, air, ..Saber::default() };
                let Some(f) = slash_frame(&s) else { continue };
                let body = frame(&s).unwrap();
                let (eb, ea) = (extent(&set.page(body), &set.frames[body].sprites), extent(&sl.page(f, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]), &sl.frames[f].sprites));
                eprintln!("air {air} t {t} body {body} {eb:?} arc {f} {ea:?}");
                add(eb);
                add(ea);
            }
            let (front, back, up, down) = all.unwrap();
            eprintln!("air {air}: front {front} back {back} up {up} down {down}");
            // `back` is negative: pixels behind the origin.
            let b = if air { BOX_AIR } else { BOX_GROUND };
            let (bx, by, hw, hh) = (i32::from(b.x), i32::from(b.y), i32::from(b.half_w), i32::from(b.half_h));
            let m = i32::from(MARGIN);
            assert!(bx + hw >= front + m && bx - hw <= back - m, "air {air}: x {}..{} vs {back}..{front} + {m}", bx - hw, bx + hw);
            assert!(by - hh <= -(up + m) && by + hh >= down, "air {air}: y {}..{} vs -{up}..{down}", by - hh, by + hh);
            // and not much more than that (2 px of rounding)
            assert!(bx + hw <= front + m + 2 && bx - hw >= back - m - 2 && by - hh >= -(up + m) - 2 && by + hh <= down + 2);
        }
    }

    #[test]
    fn saber_shot_in_front() {
        let mut p2 = [0u8; 0x90];
        p2[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&500u16.to_le_bytes());
        p2[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&367u16.to_le_bytes());
        p2[0x11] = 0x62; // facing right
        let s = shot_bytes(&p2, false);
        assert_eq!((u16::from_le_bytes([s[5], s[6]]), u16::from_le_bytes([s[8], s[9]])), (514, 347));
        assert_eq!((s[0], s[0x0A], s[0x30]), (1, DAMAGE_ID, 0));
        assert_eq!(s[BOX_AT..BOX_AT + 4], [0, 0, 43, 35]);
        p2[0x11] = 0x22;
        let s = shot_bytes(&p2, true);
        assert_eq!((u16::from_le_bytes([s[5], s[6]]), u16::from_le_bytes([s[8], s[9]])), (485, 349));
        assert_eq!(s[BOX_AT..BOX_AT + 4], [0, 0, 46, 37]);
    }

    /// hw-spiky (docs/coop.md): P2 walks right 20 frames, X at 22; the
    /// Spiky (enemy slot 0, 2 HP, 1 per hit of id 2) is hit on the arc's
    /// first frame, its centre 55 px in front of Zero, once.
    /// Needs both ROMs and the state.
    #[test]
    fn saber_hits_spiky() {
        let Some(mut g) = spiky(false) else { return };
        let mut seen = Vec::new();
        for f in 1..=45 {
            let p2 = match f {
                ..=20 => BTN_RIGHT,
                22 | 23 => BTN_X,
                _ => 0,
            };
            g.step(crate::Pads { p1: 0, p2 });
            let gap = i32::from(u16::from_le_bytes([g.wram()[0x0E68 + 5], g.wram()[0x0E68 + 6]]))
                - i32::from(u16::from_le_bytes([g.coop.p2[OFF_POS_X], g.coop.p2[OFF_POS_X + 1]]));
            seen.push((g.coop.zero_frame, g.wram()[0x0E68 + 0x27] & 0x7F, gap));
        }
        let hit = seen.iter().position(|s| s.1 == 1).expect("hit");
        assert_eq!(seen[hit].0, 88, "on the arc: {seen:?}");
        assert!((40..=60).contains(&seen[hit].2), "{seen:?}"); // the first damage frame (the box reaches 57 px out)
        assert!(seen[hit..].iter().all(|s| s.1 == 1), "one hit per swing: {seen:?}");
        assert_eq!(g.wram()[0x0BDD], 0, "P1's shot count untouched");
    }

    fn spiky(p1_zero: bool) -> Option<crate::Game> {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return None;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return None;
        };
        let mut g = crate::Game::from_rom(std::fs::read(x1).unwrap()).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        g.load_state(&state).unwrap();
        if p1_zero {
            g.set_p1_zero(true).unwrap();
        } else {
            g.coop.enabled = true;
            crate::coop::register_coop_traps(&mut g).unwrap();
        }
        Some(g)
    }

    /// hw-spiky, P2 swings where it stands while the Spiky walks in:
    /// `(zero_frame, Spiky HP, Spiky x - P2 x)` per frame.
    fn swing_at_approaching_spiky(at: usize) -> Vec<(u16, u8, i32)> {
        let Some(mut g) = spiky(false) else { return Vec::new() };
        let mut seen = Vec::new();
        for f in 1..=45 {
            g.step(crate::Pads { p1: 0, p2: if f == at { BTN_X } else { 0 } });
            let gap = i32::from(u16::from_le_bytes([g.wram()[0x0E68 + 5], g.wram()[0x0E68 + 6]]))
                - i32::from(u16::from_le_bytes([g.coop.p2[OFF_POS_X], g.coop.p2[OFF_POS_X + 1]]));
            seen.push((g.coop.zero_frame, g.wram()[0x0E68 + 0x27] & 0x7F, gap));
        }
        seen
    }

    /// Swing at 2: the Spiky is 115..94 px away while the arc is drawn
    /// (frames 9-22 of the swing): out of reach.
    #[test]
    fn saber_misses_spiky_from_afar() {
        let seen = swing_at_approaching_spiky(2);
        if seen.is_empty() {
            return;
        }
        assert!(seen.iter().any(|s| s.0 == 88), "{seen:?}");
        let last = seen.iter().rposition(|s| s.0 == 89).expect("tail");
        assert!(seen[last].2 >= 90, "{seen:?}");
        assert!(seen.iter().all(|&(_, hp, _)| hp == 2), "{seen:?}");
    }

    /// Swing at 19: on the arc pose (88) the Spiky is 88..74 px away; it
    /// walks into the box's front edge (57 px, plus its own half width) on
    /// the tail pose (89), at 68 px: hit once. The old box (the crescent's
    /// centre 23 px out, 24 half wide, arc pose only) never reached it.
    #[test]
    fn saber_reaches_spiky_on_the_tail() {
        let seen = swing_at_approaching_spiky(19);
        if seen.is_empty() {
            return;
        }
        let hit = seen.iter().position(|s| s.1 == 1).expect("hit");
        assert_eq!(seen[hit].0, 89, "{seen:?}");
        assert!((60..=75).contains(&seen[hit].2), "{seen:?}");
        assert!(seen[hit..].iter().all(|s| s.1 == 1), "one hit per swing: {seen:?}");
    }

    /// `hw-a`: P2 walks left into the dip under the Sky Claws, turns right,
    /// jumps and swings at the apex. The Claw (enemy slot 0, 4 HP, box `00
    /// F8 0F 0D`: 30 x 26 centred 8 above it) is behind him and above his
    /// head when it dies; the old box (the crescent's centre, 28 px in
    /// front and 19 up, 48 x 46) stopped short of it in both axes.
    /// Needs both ROMs and `states/hw-a.state`.
    #[test]
    fn saber_hits_sky_claw_overhead() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-a.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let mut g = crate::Game::from_rom(std::fs::read(x1).unwrap()).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        g.load_state(&state).unwrap();
        g.coop.enabled = true;
        crate::coop::register_coop_traps(&mut g).unwrap();
        let w16 = |g: &crate::Game, a: usize| i32::from(u16::from_le_bytes([g.wram()[a], g.wram()[a + 1]]));
        let p16 = |g: &crate::Game, o: usize| i32::from(u16::from_le_bytes([g.coop.p2[o], g.coop.p2[o + 1]]));
        let mut seen = Vec::new();
        for f in 1..=160 {
            let p2 = match f {
                1..=96 => BTN_LEFT,
                118 => BTN_RIGHT,
                124..=153 => crate::BTN_B | if f == 136 { BTN_X } else { 0 },
                _ => 0,
            };
            g.step(crate::Pads { p1: 0, p2 });
            // (swing frame, P2 x, drawn origin y, facing right, Claw x, Claw y, Claw HP)
            let oy = p16(&g, OFF_POS_Y) + i32::from(crate::zero::X3_Y_OFFSET);
            seen.push((g.coop.saber.t, p16(&g, OFF_POS_X), oy, g.coop.p2[0x11] & 0x40 != 0, w16(&g, 0x0E68 + 5), w16(&g, 0x0E68 + 8), g.wram()[0x0E68 + 0x27] & 0x7F));
        }
        let hit = seen.iter().position(|s| s.6 < 4).unwrap_or_else(|| panic!("no hit: {seen:?}"));
        let (t, x, oy, right, cx, cy, _) = seen[hit];
        assert!(t > 0 && seen[..hit].iter().all(|s| s.6 == 4), "{seen:?}");
        assert!(right && cx + 15 < x, "behind him: {:?}", seen[hit]);
        assert!(cy - 8 + 13 < oy - 16, "above his head: {:?}", seen[hit]);
        // The old box: centre (x + 28, oy - 19), half 24 x 23.
        assert!(cx + 15 < x + 28 - 24 && cy - 8 + 13 < oy - 19 - 23, "{:?}", seen[hit]);
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
    /// same pictures and plays the same audio (whoosh included) as the
    /// straight run (page, mask, `zero_frame` and `sfx` travel together in
    /// `net_save`).
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
            crate::coop::register_coop_traps(&mut g).unwrap();
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
                    b.take_audio(); // replayed frames' audio is dropped (netgame)
                    b.step(pad(r));
                }
            }
            assert_eq!(a.frame().0, b.frame().0, "frame {f} (zf {})", a.coop.zero_frame);
            assert_eq!(a.take_audio(), b.take_audio(), "frame {f}: audio (whoosh {:?})", a.coop.sfx);
        }
    }
}

