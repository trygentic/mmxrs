//! Zero and Sigma's saber combat, independent of the selected sprite pack. See docs/zero.md
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
//!
//! Wave (the Hadouken's counterpart, docs/zero-upgrades.md "The wave"): a
//! swing started with a full charge (`+58 = 4`, `+5B` 2 or 1) while the
//! Hadouken is owned (`$1F7E` bit 7) consumes the charge the ROM's way and
//! throws X3's green wave ([`Wave`], `zero_data::x3_wave_step`) on the arc
//! pose's first frame; it flies [`WAVE_FRAMES`] frames as its own shot of
//! the Hadouken's id ([`WAVE_DAMAGE_ID`]), and like the Hadouken's shot it
//! goes through what it hits (put in fresh every frame).

use crate::coop::{OFF_POS_X, OFF_POS_Y};
use crate::ram::wram_index as ix;
use crate::{Game, BTN_LEFT, BTN_RIGHT, BTN_X};

/// The Hadouken's shot id (`+0A` of the shot it spawns, measured: `4`;
/// `$86:EEDB` row 3, the Highway enemies: 32 = one hit).
pub const WAVE_DAMAGE_ID: u8 = 4;
/// The wave's life in frames: X3's died after 31 (about 197 px out).
pub const WAVE_FRAMES: u8 = 31;

/// A thrown wave (`Game::waves`, one per player, saved in the state's `WAVE`
/// tail, so rollback carries it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub struct Wave {
    /// Frames since the throw + 1, 0 = none.
    pub t: u8,
    pub air: bool,
    pub right: bool,
    /// Zero's drawn origin at the throw, world coordinates (`+05`, `+08 +
    /// (i8)+19 + X3_Y_OFFSET`).
    pub x: u16,
    pub y: u16,
    /// The swing that started with a full charge: throw on its arc.
    pub armed: bool,
    /// Shot slot (0-7) this frame, `0xFF` = none.
    pub slot: u8,
}

impl Default for Wave {
    fn default() -> Self {
        Self { t: 0, air: false, right: false, x: 0, y: 0, armed: false, slot: 0xFF }
    }
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

/// After [`update`], with the same block (mutable: the charge is consumed),
/// the swing and `hadouken` (`$1F7E` bit 7): arm on a swing's first frame
/// with a full charge, throw on the arc's first frame, fly. Returns true
/// when the charge sound must be stopped (`$17`, as `$81:97F6` does). Pure
/// but for `w` and `obj`.
pub fn wave_step(w: &mut Wave, s: &Saber, obj: &mut [u8], hadouken: bool) -> bool {
    if w.t > 0 {
        w.t = if w.t >= WAVE_FRAMES { 0 } else { w.t + 1 };
    }
    let mut stop = false;
    if s.t == 1 {
        w.armed = hadouken && obj[0x58] == 4 && matches!(obj[0x5B], 1 | 2);
        if w.armed {
            // The ROM's cancel (`$81:97EE`): no charged shot on the masked release.
            stop = obj[0x87] & 0x40 != 0;
            obj[0x87] &= !0x40;
            obj[0x58] = 0;
            obj[0x5A] = 0;
            obj[0x5B] = 0;
        }
    }
    if s.t == 0 {
        w.armed = false;
    }
    if w.armed && whoosh(s) {
        let o = |k: usize| u16::from_le_bytes([obj[k], obj[k + 1]]);
        let y = o(OFF_POS_Y).wrapping_add(obj[0x19] as i8 as u16).wrapping_add(crate::zero::X3_Y_OFFSET as u16);
        *w = Wave { t: 1, air: s.air, right: obj[0x11] & 0x40 != 0, x: o(OFF_POS_X), y, ..Wave::default() };
    }
    stop
}

/// The wave's shot (`+05/+08` = its box centre, the box at [`BOX_AT`]),
/// `None` without a wave. Pure.
pub fn wave_shot_bytes(w: &Wave) -> Option<[u8; SHOT_LEN]> {
    let (x, y, _, (bx, by, hw, hh)) = w.at()?;
    let bx = i16::from(bx);
    let x = x.wrapping_add(if w.right { bx } else { -bx } as u16);
    let mut b = [0u8; SHOT_LEN];
    b[0x00] = 1;
    b[0x0A] = WAVE_DAMAGE_ID;
    b[0x05..0x07].copy_from_slice(&x.to_le_bytes());
    b[0x08..0x0A].copy_from_slice(&y.wrapping_add(i16::from(by) as u16).to_le_bytes());
    b[0x11] = if w.right { 0x40 } else { 0 };
    b[BOX_AT..BOX_AT + 4].copy_from_slice(&[0, 0, hw, hh]);
    Some(b)
}

/// Queue sound `cmd` on the game's ring (`$0B72`, as `$80:886F` does).
fn queue_sound(game: &mut Game, cmd: u8) {
    let w = game.wram_mut();
    let k = usize::from(w[ix(crate::ram::SND_WRITE)] & 0x1E);
    w[ix(crate::ram::SOUND_QUEUE) + k] = cmd;
    w[ix(crate::ram::SOUND_QUEUE) + k + 1] = 0;
    w[ix(crate::ram::SND_WRITE)] = (k as u8 + 2) & 0x1E;
}

/// [`wave_step`] for player `k` (0: P1, the live block; 1-3: the shadow
/// player whose slice runs, `game.coop`), after its [`update`].
/// ponytail: players 3 and 4 throw no wave (shadow page 3 holds two; their
/// sabers are drawn but not yet inserted as shots either).
pub fn wave_for(game: &mut Game, k: usize) {
    if k > 1 {
        return;
    }
    let had = game.wram()[ix(crate::ram::HADOUKEN)] & crate::ram::HADOUKEN_BIT != 0;
    let (s, mut w) = if k == 0 { (game.coop.p1.saber, game.waves[0]) } else { (game.coop.saber, game.waves[1]) };
    let stop = if k == 0 {
        let b = ix(crate::ram::X_BASE);
        wave_step(&mut w, &s, &mut game.wram_mut()[b..b + crate::ram::X_BLOCK_LEN], had)
    } else {
        wave_step(&mut w, &s, &mut game.coop.p2, had)
    };
    game.waves[k] = w;
    if stop {
        queue_sound(game, 0x17);
    }
}

/// The instruction after the object master's `JSR $80:D3B2` (the shot loop,
/// at `$80:D1FC`); the enemy loops follow. Confirmed by `trace --calls`.
pub const HOOK_SHOTS_DONE: crate::hooks::HookId = crate::hooks::HookId::ShotsDone;
const SHOTS: u32 = 0x7E1228;
const SHOT_LEN: usize = 0x40;
/// Shot id `+0A` whose damage the saber deals: X's charged-buster table
/// entry 2 (`$86:EEDB` row 3, the Highway enemies and Bee Blader: lemon 1,
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
impl HitBox {
    /// Logical game-pixel rectangle relative to the actor object origin when
    /// facing right. Bounds are inclusive left/top, exclusive right/bottom.
    pub fn from_bounds([left, top, right, bottom]: [i16; 4]) -> Option<Self> {
        if right <= left || bottom <= top { return None; }
        let width = i32::from(right) - i32::from(left);
        let height = i32::from(bottom) - i32::from(top);
        let half_w = (width + 1) / 2;
        let half_h = (height + 1) / 2;
        if half_w > 255 || half_h > 255 { return None; }
        Some(Self { x: (i32::from(left) + half_w) as i16, y: (i32::from(top) + half_h) as i16,
            half_w: half_w as u8, half_h: half_h as u8 })
    }
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

/// Per-frame contacts derived from the loaded green saber pixels, cached
/// alongside the static art (never serialized into runtime save states).
#[derive(Debug, Clone)]
pub(crate) struct Profile {
    pub fps: f32,
    pub boxes: Vec<Option<HitBox>>,
}

impl Profile {
    pub fn duration(&self, game_fps: f64) -> u8 {
        (self.boxes.len() as f64 * game_fps / f64::from(self.fps)).ceil().clamp(1.0, 254.0) as u8
    }
    fn contact(&self, t: u8, game_fps: f64) -> Option<HitBox> {
        if t == 0 { return None; }
        let frame = (f64::from(t - 1) * f64::from(self.fps) / game_fps) as usize;
        self.boxes.get(frame).copied().flatten()
    }
}

pub(crate) fn profile(sheet: &crate::bone_animation::BoneSheet) -> Option<Profile> {
    let clip = sheet.clips.iter().find(|c| c.name == "attack")?;
    let right = sheet.source_faces_right.unwrap_or(false);
    Some(Profile { fps: clip.fps, boxes: clip.frames.iter().map(|f| blade_box(f, sheet.render_scale, right)).collect() })
}

/// A connected emerald blade is much longer than the green chest details.
/// Only that component contributes contact; cape, boots and the actor body
/// never expand the damage rectangle. No detected blade means no HD contact.
fn blade_box(frame: &crate::bone_animation::BoneFrame, scale: u8, source_right: bool) -> Option<HitBox> {
    let width = usize::from(frame.width);
    let height = usize::from(frame.height);
    let green = |at: usize| {
        let p = &frame.rgba[at * 4..at * 4 + 4];
        p[3] >= 128 && p[1] >= 90 && u16::from(p[0]) * 2 < u16::from(p[1])
            && u16::from(p[2]) * 5 < u16::from(p[1]) * 4
    };
    let mut visited = vec![false; width * height];
    let mut best = None;
    let mut best_count = 0;
    for at in 0..width * height {
        if visited[at] || !green(at) { continue; }
        visited[at] = true;
        let mut queue = vec![at];
        let (mut minx, mut maxx, mut miny, mut maxy, mut count) = (width, 0, height, 0, 0);
        while let Some(at) = queue.pop() {
            let (x, y) = (at % width, at / width);
            minx = minx.min(x); maxx = maxx.max(x); miny = miny.min(y); maxy = maxy.max(y); count += 1;
            for next in [x.checked_sub(1).map(|x| y * width + x), (x + 1 < width).then_some(at + 1),
                         y.checked_sub(1).map(|y| y * width + x), (y + 1 < height).then_some(at + width)].into_iter().flatten() {
                if !visited[next] && green(next) { visited[next] = true; queue.push(next); }
            }
        }
        if count > best_count && (maxx - minx).max(maxy - miny) >= usize::from(scale) * 20 {
            best_count = count; best = Some((minx, maxx, miny, maxy));
        }
    }
    let (minx, maxx, miny, maxy) = best?;
    let scale = i32::from(scale);
    let mut left = (minx as i32 - i32::from(frame.pivot[0])).div_euclid(scale);
    let mut right = (maxx as i32 + 1 - i32::from(frame.pivot[0]) + scale - 1).div_euclid(scale);
    if !source_right { (left, right) = (-right, -left); }
    let top = (miny as i32 - i32::from(frame.pivot[1])).div_euclid(scale) + 16;
    let bottom = (maxy as i32 + 1 - i32::from(frame.pivot[1]) + scale - 1).div_euclid(scale) + 16;
    let half_w = ((right - left + 1) / 2).clamp(1, 255) as u8;
    let half_h = ((bottom - top + 1) / 2).clamp(1, 255) as u8;
    Some(HitBox { x: ((left + right) / 2) as i16, y: ((top + bottom) / 2) as i16, half_w, half_h })
}

pub(crate) fn contact(game: &Game, player: usize, s: &Saber) -> Option<HitBox> {
    if s.hit { return None; }
    let character = game.player_characters[player];
    if let Some(profile) = game.saber_profiles.get(&character) { return profile.contact(s.t, game.fps()); }
    arc(s).then_some(if s.air { BOX_AIR } else { BOX_GROUND })
}

pub(crate) fn duration(game: &Game, player: usize) -> Option<u8> {
    game.saber_profiles.get(&game.player_characters[player]).map(|p| p.duration(game.fps()))
}

/// These characters use the same melee lifecycle and ROM enemy collision.
pub const fn uses_saber(character: crate::Character) -> bool {
    matches!(character, crate::Character::Zero | crate::Character::Sigma)
}

/// A grounded attack masks locomotion, but its initial direction request
/// still chooses which way the blade swings. Keep ambiguous opposite inputs
/// and active swings unchanged, and leave ordinary movement to the ROM.
pub(crate) fn face_attack_start(s: &Saber, obj: &mut [u8], pad: u16) {
    if s.t != 0 || s.btn || pad & BTN_X == 0 || !matches!(obj[2], 0x00 | 0x02 | 0x04 | 0x0A | 0x14) { return; }
    match pad & (BTN_LEFT | BTN_RIGHT) {
        BTN_LEFT => obj[0x11] &= !0x40,
        BTN_RIGHT => obj[0x11] |= 0x40,
        _ => {},
    }
}

/// May `player` start a swing now. Zero's saber is the arm part
/// (docs/zero-upgrades.md; `$1F99` is shared by every player), whatever
/// art he wears (the slash arc and sounds are X3's; X1 art and sprite
/// packs swing without them). Sigma's attack is his own.
pub(crate) fn armed(game: &Game, player: usize) -> bool {
    game.player_characters[player] != crate::Character::Zero || game.wram()[ix(crate::ram::ARMOR)] & crate::ram::ARMOR_ARM != 0
}

/// Called with the player's block, pad and the armour byte `$1F99` before
/// its update: start / advance / end the swing, return the pad the ROM
/// gets. A swing starts only while `parts` has [`crate::ram::ARMOR_ARM`]
/// (the saber is the arm part); without it X starts nothing (and still
/// never reaches the ROM). Pure but for `s`.
pub fn update(s: &mut Saber, obj: &[u8], pad: u16, parts: u8) -> u16 {
    update_timed(s, obj, pad, None, parts & crate::ram::ARMOR_ARM != 0)
}

/// [`update`] with a bone sheet's attack length (`duration`, else the X3
/// poses') and whether a swing may start ([`armed`]).
pub(crate) fn update_timed(s: &mut Saber, obj: &[u8], pad: u16, duration: Option<u8>, armed: bool) -> u16 {
    let action = obj[0x02];
    let x = pad & BTN_X != 0;
    let pressed = x && !s.btn;
    s.btn = x;
    let pad = pad & !BTN_X;
    if s.t > 0 && (!s.air || matches!(action, 0x00 | 0x02 | 0x04 | 0x0A | 0x12 | 0x14)) && pad & crate::BTN_B != 0 {
        // Ground jumps and wall kicks have priority over the attack. The original player
        // routine performs the jump and terrain collision normally.
        s.t = 0;
        return pad;
    }
    if s.t > 0 {
        s.t += 1;
        if action == 0x0E || action == 0x0C || duration.map_or_else(|| pose_index(s.t, s.air).is_none(), |length| s.t > length) {
            s.t = 0; // hurt / dead / done
        }
    } else if pressed && armed {
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
    let b = if air { BOX_AIR } else { BOX_GROUND };
    shot_with_box(p2, b, crate::zero::X3_Y_OFFSET)
}

fn shot_with_box(p2: &[u8], b: HitBox, y_offset: i16) -> [u8; SHOT_LEN] {
    let w = |o: usize| i16::from_le_bytes([p2[o], p2[o + 1]]);
    let right = p2[0x11] & 0x40 != 0;
    let x = w(OFF_POS_X).wrapping_add(if right { b.x } else { -b.x });
    let y = w(OFF_POS_Y).wrapping_add(i16::from(p2[0x19] as i8)).wrapping_add(y_offset).wrapping_add(b.y);
    let mut s = [0u8; SHOT_LEN];
    s[0x00] = 1;
    s[0x0A] = DAMAGE_ID;
    s[0x05..0x07].copy_from_slice(&x.to_le_bytes());
    s[0x08..0x0A].copy_from_slice(&y.to_le_bytes());
    s[0x11] = p2[0x11];
    s[BOX_AT..BOX_AT + 4].copy_from_slice(&[0, 0, b.half_w, b.half_h]);
    s
}

pub(crate) fn player_shot(game: &Game, player: usize, obj: &[u8], s: &Saber) -> Option<[u8; SHOT_LEN]> {
    let box_ = contact(game, player, s)?;
    let offset = if game.saber_profiles.contains_key(&game.player_characters[player]) { 0 } else { crate::zero::X3_Y_OFFSET };
    Some(shot_with_box(obj, box_, offset))
}

/// [`HOOK_SHOTS_DONE`]: put each swing's arc shot (a Zero P1's, P2's) into
/// a free slot for the enemy loop.
pub fn shots_done(game: &mut Game) {
    if !crate::coop_stages::stage_actor_scene(game.wram()) { return; }
    if uses_saber(game.player_characters[0]) && contact(game, 0, &game.coop.p1.saber).is_some() {
        let b = ix(crate::ram::X_BASE);
        let box_ = contact(game, 0, &game.coop.p1.saber).unwrap();
        let offset = if game.saber_profiles.contains_key(&game.player_characters[0]) { 0 } else { crate::zero::X3_Y_OFFSET };
        let shot = shot_with_box(&game.wram()[b..b + crate::ram::X_BLOCK_LEN], box_, offset);
        game.coop.p1.saber.slot = put_shot(game, &shot);
    }
    let c = &game.coop;
    if c.enabled && !c.p2_hidden && uses_saber(game.player_characters[1]) && contact(game, 1, &c.saber).is_some() {
        let box_ = contact(game, 1, &c.saber).unwrap();
        let offset = if game.saber_profiles.contains_key(&game.player_characters[1]) { 0 } else { crate::zero::X3_Y_OFFSET };
        let shot = shot_with_box(&c.p2, box_, offset);
        game.coop.saber.slot = put_shot(game, &shot);
    }
    for index in 0..game.local_players.saturating_sub(2).min(2) {
        let c = &game.extra_coop[index];
        if c.enabled && !c.p2_hidden && uses_saber(game.player_characters[index + 2]) && contact(game, index + 2, &c.saber).is_some() {
            let box_ = contact(game, index + 2, &c.saber).unwrap();
            let offset = if game.saber_profiles.contains_key(&game.player_characters[index + 2]) { 0 } else { crate::zero::X3_Y_OFFSET };
            let shot = shot_with_box(&c.p2, box_, offset);
            game.extra_coop[index].saber.slot = put_shot(game, &shot);
        }
    }
    for k in 0..2 {
        if let Some(shot) = wave_shot_bytes(&game.waves[k]) {
            game.waves[k].slot = put_shot(game, &shot);
        }
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
    let mut p3 = game.extra_coop[0].saber;
    let mut p4 = game.extra_coop[1].saber;
    for s in [&mut p1, &mut p2, &mut p3, &mut p4] {
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
    game.extra_coop[0].saber = p3;
    game.extra_coop[1].saber = p4;
    // The wave pierces, like the Hadouken's shot: no hit bookkeeping.
    for k in 0..2 {
        let s = std::mem::replace(&mut game.waves[k].slot, 0xFF);
        if s != 0xFF {
            let o = ix(SHOTS) + usize::from(s) * SHOT_LEN;
            game.wram_mut()[o..o + SHOT_LEN].fill(0);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn ground_saber_start_faces_requested_direction_without_moving() {
        let mut obj=[0;0x90];obj[0x11]=0x62;
        let mut s=Saber::default();
        face_attack_start(&s,&mut obj,BTN_LEFT|BTN_X);
        assert_eq!(obj[0x11],0x22);
        assert_eq!(update_timed(&mut s,&obj,BTN_LEFT|BTN_X,Some(49),true),0);
        face_attack_start(&s,&mut obj,BTN_RIGHT|BTN_X);
        assert_eq!(obj[0x11],0x22,"active swing direction was changed");
    }

    #[test]
    fn ground_saber_jump_cancel_preserves_jump_direction_and_air_rules() {
        let mut obj = [0; 0x90];
        obj[2] = 0;
        let mut ground = Saber { t: 20, air: false, ..Saber::default() };
        let jump = crate::BTN_B | BTN_RIGHT;
        assert_eq!(update_timed(&mut ground, &obj, jump, Some(49), true), jump);
        assert_eq!(ground.t, 0);
        let mut air = Saber { t: 20, air: true, ..Saber::default() };
        obj[2] = 0x06;
        assert_eq!(update_timed(&mut air, &obj, jump, Some(49), true), BTN_RIGHT);
        assert_eq!(air.t, 21, "air swing should retain its existing input mask");
        obj[2] = 0; // A landed air swing is grounded again.
        assert_eq!(update_timed(&mut air, &obj, jump, Some(49), true), jump);
        assert_eq!(air.t, 0);
        air = Saber { t: 20, air: true, ..Saber::default() };
        obj[2] = 0x12; // A real wall slide must permit a fresh wall-kick input.
        assert_eq!(update_timed(&mut air, &obj, jump, Some(49), true), jump);
        assert_eq!(air.t, 0);
    }

    #[test]
    fn hd_blade_contact_excludes_body_and_tracks_direction_and_pixel_scale() {
        use crate::bone_animation::BoneFrame;
        let mut rgba = vec![0; 100 * 64 * 4];
        for y in 5..60 { for x in 20..46 {
            let at = (y * 100 + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[110, 110, 110, 255]);
        }}
        // Short green chest details must not become an attack rectangle.
        for y in 10..18 { for x in 25..30 {
            let at = (y * 100 + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[0, 240, 80, 255]);
        }}
        let neutral = BoneFrame { width: 100, height: 64, pivot: [40, 60], rgba: rgba.clone() };
        assert_eq!(blade_box(&neutral, 1, true), None);
        for y in 20..24 { for x in 50..82 {
            let at = (y * 100 + x) * 4;
            rgba[at..at + 4].copy_from_slice(&[0, 240, 80, 255]);
        }}
        let blade = BoneFrame { rgba, ..neutral.clone() };
        let box_ = blade_box(&blade, 1, true).unwrap();
        assert!(box_.x - i16::from(box_.half_w) > 0, "cape/body enabled a rear hit");
        assert!(box_.y + i16::from(box_.half_h) < 0, "legs enlarged the sword contact");
        let mirrored = blade_box(&blade, 1, false).unwrap();
        assert_eq!(mirrored.x, -box_.x);
        assert_eq!((mirrored.y, mirrored.half_w, mirrored.half_h), (box_.y, box_.half_w, box_.half_h));
        let mut hd = BoneFrame { width: 400, height: 256, pivot: [160, 240], rgba: vec![0; 400 * 256 * 4] };
        for y in 0..256 { for x in 0..400 {
            let src = (y / 4 * 100 + x / 4) * 4;
            let dst = (y * 400 + x) * 4;
            hd.rgba[dst..dst + 4].copy_from_slice(&blade.rgba[src..src + 4]);
        }}
        assert_eq!(blade_box(&hd, 4, true), Some(box_));
        let profile = Profile { fps: 60.0, boxes: vec![None, Some(box_), None] };
        assert_eq!(profile.contact(1, 60.0), None);
        assert_eq!(profile.contact(2, 60.0), Some(box_));
        assert_eq!(profile.contact(3, 60.0), None);
    }

    #[test]
    fn saber_swing_ground_and_air() {
        let blk = |a: u8| {
            let mut b = [0u8; 0x90];
            b[2] = a;
            b
        };
        let mut s = Saber::default();
        assert_eq!(update(&mut s, &blk(0x00), 0x0100, 0x02), 0x0100);
        assert_eq!(frame(&s), None);
        assert_eq!(update(&mut s, &blk(0x04), BTN_X | 0x0100, 0x02), 0); // starts, pad masked
        assert_eq!(frame(&s), Some(86));
        let mut seen = vec![86];
        for _ in 0..30 {
            update(&mut s, &blk(0x00), BTN_X, 0x02); // held: no restart
            seen.extend(frame(&s));
        }
        // 4 + 4 + ARC_FRAMES + 4 + 4 poses
        assert_eq!(seen.len(), 16 + usize::from(ARC_FRAMES));
        assert_eq!(seen[8..8 + usize::from(ARC_FRAMES)], vec![88; usize::from(ARC_FRAMES)][..]);
        assert_eq!(s.t, 0);
        // Air: left/right pass.
        update(&mut s, &blk(0x08), 0, 0x02);
        assert_eq!(update(&mut s, &blk(0x08), BTN_X | 0x0100 | 0x8000, 0x02), 0x0100);
        assert_eq!(frame(&s), Some(95));
        // Hurt cancels; walls don't start.
        update(&mut s, &blk(0x0E), 0, 0x02);
        assert_eq!(s.t, 0);
        update(&mut s, &blk(0x12), 0, 0x02);
        update(&mut s, &blk(0x12), BTN_X, 0x02);
        assert_eq!(s.t, 0);
    }

    /// The saber is the arm part: refused at `$1F99 = 0` (and with the
    /// other three parts), taken at `$02`. X never reaches the ROM.
    #[test]
    fn saber_needs_arm_part() {
        let stand = [0u8; 0x90];
        for parts in [0x00, 0x01, 0x04, 0x08, 0x0D] {
            let mut s = Saber::default();
            assert_eq!(update(&mut s, &stand, BTN_X | 0x0100, parts), 0x0100, "$1F99 = {parts:02X}");
            assert_eq!(s.t, 0, "$1F99 = {parts:02X}");
        }
        let mut s = Saber::default();
        assert_eq!(update(&mut s, &stand, BTN_X | 0x0100, crate::ram::ARMOR_ARM), 0);
        assert_eq!(frame(&s), Some(86));
    }

    /// A swing with a full charge and the Hadouken arms the wave (charge
    /// consumed, the charge sound stopped), throws it on the arc's first
    /// frame from Zero's drawn origin, and it flies `WAVE_FRAMES`.
    #[test]
    fn saber_wave_spawns_on_arc_and_flies() {
        let mut blk = [0u8; 0x90];
        blk[OFF_POS_X..OFF_POS_X + 2].copy_from_slice(&500u16.to_le_bytes());
        blk[OFF_POS_Y..OFF_POS_Y + 2].copy_from_slice(&367u16.to_le_bytes());
        blk[0x11] = 0x62; // facing right
        let charged = |b: &mut [u8; 0x90]| {
            b[0x58] = 4;
            b[0x5B] = 2;
            b[0x87] = 0x40;
        };
        // No Hadouken: nothing armed, the charge stays for the ROM.
        let (mut s, mut w, mut b) = (Saber::default(), Wave::default(), blk);
        charged(&mut b);
        update(&mut s, &b, BTN_X, 0x02);
        assert!(!wave_step(&mut w, &s, &mut b, false));
        assert!(!w.armed && b[0x5B] == 2);
        // Hadouken, no charge: nothing.
        let (mut s, mut w, mut b) = (Saber::default(), Wave::default(), blk);
        update(&mut s, &b, BTN_X, 0x02);
        wave_step(&mut w, &s, &mut b, true);
        assert!(!w.armed);
        // Both: armed, charge consumed, then thrown on the arc.
        let (mut s, mut w, mut b) = (Saber::default(), Wave::default(), blk);
        charged(&mut b);
        update(&mut s, &b, BTN_X, 0x02);
        assert!(wave_step(&mut w, &s, &mut b, true), "stop the charge sound");
        assert!(w.armed && b[0x58] == 0 && b[0x5B] == 0 && b[0x87] == 0);
        while w.t == 0 {
            update(&mut s, &b, 0, 0x02);
            wave_step(&mut w, &s, &mut b, true);
        }
        assert_eq!(frame(&s), Some(88), "thrown on the arc's first frame");
        assert_eq!((w.x, w.y), (500, 367 - 8));
        let shot = wave_shot_bytes(&w).unwrap();
        assert_eq!((shot[0x0A], u16::from_le_bytes([shot[5], shot[6]])), (WAVE_DAMAGE_ID, 508)); // spawn: 8 in front
        for _ in 0..WAVE_FRAMES {
            update(&mut s, &b, 0, 0x02);
            wave_step(&mut w, &s, &mut b, true);
        }
        assert_eq!(w.t, 0);
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
        give_arm(&mut g);
        if p1_zero {
            g.set_p1_zero(true).unwrap();
        } else {
            g.coop.enabled = true;
            crate::coop::register_coop_traps(&mut g).unwrap();
        }
        Some(g)
    }

    /// The saber is the arm part; the test states have none.
    pub(crate) fn give_arm(g: &mut crate::Game) {
        g.wram_mut()[ix(crate::ram::ARMOR)] |= crate::ram::ARMOR_ARM;
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

    /// hw-spiky, P2 swings where it stands with a full charge at frame 2:
    /// `(Spiky HP, Spiky x - P2 x, wave frames)` per frame.
    fn charged_swing(hadouken: bool) -> Vec<(u8, i32, u8)> {
        let Some(mut g) = spiky(false) else { return Vec::new() };
        if hadouken {
            g.wram_mut()[ix(crate::ram::HADOUKEN)] |= crate::ram::HADOUKEN_BIT;
        }
        let mut seen = Vec::new();
        for f in 1..=60 {
            if f == 2 {
                g.coop.p2[0x58] = 4; // charging, full
                g.coop.p2[0x5B] = 2;
            }
            g.step(crate::Pads { p1: 0, p2: if f == 2 { BTN_X } else { 0 } });
            let gap = i32::from(u16::from_le_bytes([g.wram()[0x0E68 + 5], g.wram()[0x0E68 + 6]]))
                - i32::from(u16::from_le_bytes([g.coop.p2[OFF_POS_X], g.coop.p2[OFF_POS_X + 1]]));
            seen.push((g.wram()[0x0E68 + 0x27] & 0x7F, gap, g.waves[1].t));
        }
        seen
    }

    /// With the Hadouken and a full charge the swing throws the wave: the
    /// Spiky, out of the blade's reach (test `saber_misses_spiky_from_afar`),
    /// dies to it as to X's Hadouken (measured: 2 -> 1 -> dead on two
    /// frames running; the shot id 4 is the Hadouken's, it pierces).
    #[test]
    fn saber_wave_hits_spiky_from_afar() {
        let seen = charged_swing(true);
        if seen.is_empty() {
            return;
        }
        let hit = seen.iter().position(|s| s.0 < 2).unwrap_or_else(|| panic!("the wave hits: {seen:?}"));
        assert!(seen[hit].1 > 60 && seen[hit].2 > 0, "out of the blade's reach, by the wave: {seen:?}");
        assert_eq!(seen[hit + 1].0, 0, "dead the next frame, as to the Hadouken: {seen:?}");
    }

    /// Save states carry each player's wave in their `WAVE` tail; a state
    /// saved before it (hw-spiky) loads with none.
    #[test]
    fn saber_wave_in_save_state() {
        let Some(mut g) = spiky(false) else { return };
        assert_eq!(g.waves, [Wave::default(); 4]);
        g.waves[1] = Wave { t: 5, air: true, right: true, x: 1234, y: 345, ..Wave::default() };
        let s = g.save_state();
        let mut h = spiky(false).unwrap();
        h.load_state(&s).unwrap();
        assert_eq!(h.waves, g.waves);
    }

    /// P1 as Zero (`--zero`): the same charged swing from the live block
    /// throws P1's own wave (`waves[0]`), with the charge consumed.
    #[test]
    fn saber_p1_throws_the_wave() {
        let Some(mut g) = spiky(true) else { return };
        g.wram_mut()[ix(crate::ram::HADOUKEN)] |= crate::ram::HADOUKEN_BIT;
        let b = ix(crate::ram::X_BASE);
        let mut thrown = false;
        for f in 1..=30 {
            if f == 2 {
                g.wram_mut()[b + 0x58] = 4;
                g.wram_mut()[b + 0x5B] = 2;
            }
            g.step(crate::Pads { p1: if f == 2 { BTN_X } else { 0 }, p2: 0 });
            thrown |= g.waves[0].t > 0;
            assert_eq!(g.waves[1].t, 0, "P2 has none");
        }
        assert!(thrown && g.wram()[b + 0x5B] == 0);
    }

    /// No Hadouken: the same charged swing throws nothing.
    #[test]
    fn saber_no_wave_without_hadouken() {
        let seen = charged_swing(false);
        assert!(seen.iter().all(|s| s.2 == 0), "{seen:?}");
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
        give_arm(&mut g);
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
        give_arm(&mut g);
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
            give_arm(&mut g);
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

