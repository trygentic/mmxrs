//! Margin sprites (docs/widescreen.md "Objects in the margins"): with a
//! widescreen margin, objects the game culled from OAM because they sit
//! past the 256-px picture are drawn in the margins. Display only.
//!
//! The OAM build's per-object emitter (listing `$80:D67C`, `X` = the object)
//! drops each piece with `x + 16 >= $10F` or `y + 15 >= $EF` (camera-
//! relative, unsigned), i.e. keeps X `-16..=254`. After the build (the display
//! hook `$80:9A86`, after P2's slice) every live object near a margin goes
//! through the same emitter once more, isolated, with the camera X moved by
//! 256 towards it, so its margin pieces land inside the emitter's window;
//! the pieces it writes are read back, moved by 256 and handed to the PPU's
//! margin sprite list (`set_margin_sprites`), which is not OAM and goes live
//! with the next OAM upload. Then WRAM `$0000..$1FFF` (direct page, stack,
//! the OAM buffer, the camera) is put back byte for byte, so the simulation
//! never sees the pass.

use crate::ram::{self, wram_index as ix};
use crate::Game;
use snes_core::api::MarginSprite;

/// `(first, count, stride, unqueued)`: enemies / shots / items / more
/// objects (`$0E68..$1928`, 43 x `$40`), effects (`$1928`, 31 x `$20`),
/// `$1D08`. `unqueued`: also drawn when the game stopped queueing it (see
/// [`MarginState`]); effects are not: off the window some stop moving (the
/// Sky Claw's cable links hang where they were) and they die soon anyway.
const TABLES: [(u16, u16, u16, bool); 3] = [(0x0E68, 43, 0x40, true), (0x1928, 31, 0x20, false), (0x1D08, 16, 0x20, false)];
const SLOTS: usize = 43;
/// Camera-relative X the emitter keeps (`x + 16 < $10F`).
const KEPT: std::ops::RangeInclusive<i16> = -16..=254;
const SHIFT: i16 = 256;
const OAM_CURSOR: usize = 0xE4;
const OAM_HI_INDEX: usize = 0xE5;
const OAM_HI_ACC: usize = 0xE6;
const OAM_HI: usize = 0x0900;
/// The overlay's own cap (the PPU has no per-line limit for it).
const MAX_SPRITES: usize = 128;

/// Per object slot, display only: `id + 1` of the object last seen queued
/// for drawing (`+$0E != 0`), so a live object the game stopped queueing
/// because it left the window (`$82:80B4`: X outside `-32..288`) is still
/// drawn, and helper objects that never draw are not; until a tile it uses
/// is written after the game last queued it (then its slot holds someone
/// else's graphics).
#[derive(Debug, Clone, Default)]
pub struct MarginState {
    seen: Vec<u8>,
    /// Per slot: the frame it was last queued.
    queued_at: Vec<u64>,
    /// OBJ VRAM at the last pass and, per tile, the frame it last changed.
    vram: Vec<u16>,
    changed: Vec<u64>,
    /// What the last pass handed the PPU (tests, `xtask`).
    pub last: Vec<MarginSprite>,
    /// P2's HP bar slots this step (coop_hud), for widescreen::update_margin_tiles.
    pub(crate) p2_bar: u128,
    /// Debug check (`MMX_MARGIN_CHECK`, or [`MarginState::enable_check`]).
    pub check: Option<Box<Check>>,
}

impl MarginState {
    /// Turn the residency check on (tests, `xtask`); `MMX_MARGIN_CHECK=1` does it too.
    pub fn enable_check(&mut self) {
        self.check.get_or_insert_with(Default::default);
    }
}

/// Run from the display hook (`coop::p2_display`'s trap) every gameplay frame.
pub(crate) fn observe(game: &mut Game) {
    let m = game.wide_margin() as i16;
    if m == 0 || game.isolated_depth > 0 || game.hooks().get(crate::coop::PLAYER_OAM).is_none() {
        return;
    }
    let saved_wram: Box<[u8]> = game.wram()[..0x2000].into();
    let saved_cpu = game.cpu_snapshot();
    let saved_contact = game.coop.in_contact;
    // coop_objects::effect_oam traps the emitter; keep it out of this pass.
    game.coop.in_contact = true;
    let emitter = crate::coop_objects::emitter(game);
    let mut out = Vec::new();
    let mut seen = std::mem::take(&mut game.margin.seen);
    seen.resize(SLOTS, 0);
    let frame = game.frame_count();
    let mut queued_at = std::mem::take(&mut game.margin.queued_at);
    queued_at.resize(SLOTS, 0);
    track_vram(game, frame);
    let cam = word(&saved_wram, ram::CAMERA_X);
    let mut slot = 0;
    let mut srcs = Vec::new();
    for (first, n, stride, unqueued) in TABLES {
        for k in 0..n {
            let base = first + k * stride;
            let o = &saved_wram[usize::from(base)..usize::from(base + stride)];
            let id = o[0x0A].wrapping_add(1);
            let mut draw = match (o[0], o[0x0E] & 0x80) {
                (0, _) => 0,
                (_, 0) if unqueued && seen[slot] == id => id,
                (_, 0) => 0,
                _ => id,
            };
            let queued = o[0x0E] & 0x80 != 0;
            if draw != 0 {
                let x = u16::from_le_bytes([o[5], o[6]]).wrapping_sub(cam) as i16;
                let at = out.len();
                emit(game, emitter, base, x, m, queued, None, &mut out);
                // Unqueued, its tiles are only its own while nothing has
                // been written over them since the game last drew it (a
                // later enemy of the same set streams into the same slot,
                // or a set loads there): from then on, not drawn. The NMI
                // right after its last queued frame is its own upload.
                if !queued && out[at..].iter().any(|s| tiles(s).iter().any(|&t| game.margin.changed[t] > queued_at[slot] + 1)) {
                    out.truncate(at);
                    draw = 0;
                }
                srcs.resize(out.len(), Src { base, id: o[0x0A], queued, gfx: [o[0x16], o[0x17], o[0x18]] });
            }
            if unqueued {
                seen[slot] = draw;
                if queued {
                    queued_at[slot] = frame;
                }
                slot += 1;
            }
        }
    }
    game.margin.seen = seen;
    game.margin.queued_at = queued_at;
    p2(game, emitter, m, &saved_wram, &mut out);
    out.truncate(MAX_SPRITES);
    if game.margin.check.is_none() && check_env() {
        game.margin.enable_check();
    }
    if let Some(mut c) = game.margin.check.take() {
        srcs.resize(out.len(), Src { base: ram::X_BASE as u16, id: 0xFF, queued: true, gfx: [0; 3] });
        c.pass(game, &saved_wram, &out, &srcs);
        game.margin.check = Some(c);
    }
    game.wram_mut()[..0x2000].copy_from_slice(&saved_wram);
    game.restore_cpu_snapshot(saved_cpu);
    game.coop.in_contact = saved_contact;
    game.margin.last.clone_from(&out);
    game.set_margin_sprites(out);
}

/// Diff OBJ VRAM against the last pass: `changed[tile] = frame` for every
/// tile written since (by the NMI's DMA).
fn track_vram(game: &mut Game, frame: u64) {
    let mut st = std::mem::take(&mut game.margin);
    let v = &game.vram()[OBJ_VRAM..OBJ_VRAM + OBJ_TILES * 16];
    if st.vram.len() == v.len() {
        for (t, (a, b)) in v.chunks_exact(16).zip(st.vram.chunks_exact_mut(16)).enumerate() {
            if a != b {
                b.copy_from_slice(a);
                st.changed[t] = frame;
            }
        }
    } else {
        st.vram = v.to_vec();
        st.changed = vec![0; OBJ_TILES];
    }
    game.margin = st;
}

fn check_env() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MMX_MARGIN_CHECK").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Where an overlay sprite came from (the check).
#[derive(Debug, Clone, Copy)]
struct Src {
    base: u16,
    /// `+$0A`; `$FF` = P2.
    id: u8,
    queued: bool,
    /// `+$16` set, `+$17` frame, `+$18` VRAM slot.
    gfx: [u8; 3],
}

/// OBJ tiles at VRAM word `$6000` (OBSEL, every stage seen), 512 tiles.
const OBJ_VRAM: usize = 0x6000;
const OBJ_TILES: usize = 512;

/// Residency check (debug, display only): for every overlay sprite, its
/// object, tiles and page, and whether those tiles were what the game
/// meant when the PPU drew them. The overlay of pass `p` is drawn with
/// VRAM as it is at pass `p + 1` (it goes live with NMI `p`'s OAM upload,
/// after that NMI's DMAs). Findings go to `reports` (and stderr):
/// - `late`: a tile changed in NMI `p + 1`, right after the overlay drew it.
///   Informational: a streamed object (`+$17` bit 7, X-style) changes its
///   tiles whenever it animates, and for a queued object the overlay is
///   what the ROM's own OAM would show in a wider window.
/// - `stale`: an object drawn while unqueued whose tiles changed since the
///   game last queued it (its slot got someone else's graphics);
/// - `page`: a shadow page on a non-player sprite;
/// - `palette`: an unqueued object with a palette it never had queued;
/// - `slot`: an enemy whose VRAM slot `+$18` is not its graphics set's
///   (`$82:827D` copied it before the set was loaded).
///
/// Everything but `late` is garbage ([`Check::garbage`]).
#[derive(Debug, Clone, Default)]
pub struct Check {
    vram: Vec<u16>,
    /// Per tile: the frame of the last change seen.
    changed: Vec<u64>,
    /// The last two passes' overlays (`(frame, sprite, src)`).
    pending: [Vec<(u64, MarginSprite, Src)>; 2],
    /// Per object base: `(id, frame last queued, palettes seen queued)`.
    queued: std::collections::HashMap<u16, (u8, u64, u8)>,
    pub reports: Vec<String>,
    /// Overlay sprites checked.
    pub sprites: u64,
}

fn tiles(s: &MarginSprite) -> Vec<usize> {
    let t = usize::from(s.tile) | usize::from(s.attr & 1) << 8;
    let at = |dx: usize, dy: usize| (t & 0x100) | (t & 0xF0).wrapping_add(dy << 4) & 0xF0 | (t + dx) & 0xF;
    if s.large { vec![at(0, 0), at(1, 0), at(0, 1), at(1, 1)] } else { vec![t] }
}

impl Check {
    fn pass(&mut self, game: &Game, saved: &[u8], out: &[MarginSprite], srcs: &[Src]) {
        let f = game.frame_count();
        let v = &game.vram()[OBJ_VRAM..OBJ_VRAM + OBJ_TILES * 16];
        self.changed.resize(OBJ_TILES, 0);
        let mut now = vec![false; OBJ_TILES];
        if self.vram.len() == v.len() {
            for (t, n) in now.iter_mut().enumerate() {
                if v[t * 16..t * 16 + 16] != self.vram[t * 16..t * 16 + 16] {
                    *n = true;
                    self.changed[t] = f;
                }
            }
        }
        self.vram = v.to_vec();
        // Pass p = f - 2 was drawn with the VRAM before this NMI's uploads.
        for (pf, s, src) in std::mem::take(&mut self.pending[0]) {
            let late: Vec<usize> = tiles(&s).into_iter().filter(|&t| now[t]).collect();
            if !late.is_empty() && src.id != 0xFF {
                self.report(format!("late f={pf} obj={:04X} id={:02X} queued={} gfx={:02X?} x={} tiles={late:03X?} attr={:02X}", src.base, src.id, src.queued, src.gfx, s.x, s.attr));
            }
        }
        self.pending.swap(0, 1);
        for (s, src) in out.iter().zip(srcs) {
            self.sprites += 1;
            if src.id != 0xFF && s.page.is_some() {
                self.report(format!("page f={f} obj={:04X} id={:02X} page={:?}", src.base, src.id, s.page));
            }
            if src.id != 0xFF && !src.queued {
                match self.queued.get(&src.base) {
                    Some(&(id, qf, pals)) if id == src.id => {
                        let stale: Vec<usize> = tiles(s).into_iter().filter(|&t| self.changed[t] > qf + 1).collect();
                        if !stale.is_empty() {
                            self.report(format!("stale f={f} obj={:04X} id={:02X} queued_at={qf} x={} tiles={stale:03X?}", src.base, src.id, s.x));
                        }
                        if pals & 1 << (s.attr >> 1 & 7) == 0 {
                            self.report(format!("palette f={f} obj={:04X} id={:02X} attr={:02X} seen={pals:08b}", src.base, src.id, s.attr));
                        }
                    }
                    _ => self.report(format!("unseen f={f} obj={:04X} id={:02X} x={}", src.base, src.id, s.x)),
                }
            }
            self.pending[1].push((f, *s, *src));
        }
        // Enemies whose VRAM slot is not their set's ($82:827D copied it
        // before the set was loaded, wide gameplay before the fix).
        for k in 0..15u16 {
            let base = 0x0E68 + k * 0x40;
            let o = &saved[usize::from(base)..];
            if o[0] != 0 && o[0x0E] & 0x80 != 0 && srcs.iter().any(|s| s.base == base) {
                let set = usize::from(crate::wide_gameplay::gfx_set(game, o[0x0A]));
                let (slot, pal) = (game.wram()[0x1_8200 + set], game.wram()[0x1_8300 + set]);
                if pal != 0 && slot != o[0x18] {
                    self.report(format!("slot f={f} obj={base:04X} id={:02X} +18={:02X} set {set:02X} slot={slot:02X} pal={pal:02X} +11={:02X}", o[0x0A], o[0x18], o[0x11]));
                }
            }
        }
        // Remember what the game queues (with the palettes the emitter gives it).
        for (first, n, stride, _) in TABLES {
            for k in 0..n {
                let base = first + k * stride;
                let o = &saved[usize::from(base)..];
                if o[0] != 0 && o[0x0E] & 0x80 != 0 {
                    let e = self.queued.entry(base).or_insert((o[0x0A], f, 0));
                    if e.0 != o[0x0A] {
                        *e = (o[0x0A], f, 0);
                    }
                    e.1 = f;
                    let pal = out.iter().zip(srcs).filter(|(_, s)| s.base == base).fold(0u8, |a, (s, _)| a | 1 << (s.attr >> 1 & 7));
                    e.2 |= pal | 1 << (o[0x11] >> 1 & 7);
                } else if o[0] == 0 {
                    self.queued.remove(&base);
                }
            }
        }
    }

    /// Findings that are garbage on screen (all but `late`).
    pub fn garbage(&self) -> usize {
        self.reports.iter().filter(|r| !r.starts_with("late") && !r.starts_with("unseen")).count()
    }

    fn report(&mut self, r: String) {
        eprintln!("margin-check {r}");
        self.reports.push(r);
    }
}

fn word(w: &[u8], a: u32) -> u16 {
    u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]])
}

/// P2 off the window (co-op, not hidden): X's emitter on P2's block, or
/// the X3 Zero emitter. `queued`: P2's slice drew its window pieces already.
fn p2(game: &mut Game, emitter: u32, m: i16, saved: &[u8], out: &mut Vec<MarginSprite>) {
    if !game.coop.enabled || game.coop.p2_hidden {
        return;
    }
    let p2 = game.coop.p2;
    let x = u16::from_le_bytes([p2[5], p2[6]]).wrapping_sub(word(saved, ram::CAMERA_X)) as i16;
    if (64..192).contains(&x) {
        return;
    }
    let pal = game.coop.zero_palette;
    let start = out.len();
    if let Some(z) = game.zero_set.clone() {
        let shift = if x < 128 { -SHIFT } else { SHIFT };
        set_camera(game, saved, shift);
        for (e, hi, page) in crate::zero::margin_entries(game, &z) {
            if let Some(s) = margin_sprite(e, hi, shift, m, true) {
                out.push(MarginSprite { page: Some(page), ..s });
            }
        }
        return;
    }
    game.wram_mut()[ix(ram::X_BASE)..][..p2.len()].copy_from_slice(&p2);
    emit(game, emitter, ram::X_BASE as u16, x, m, true, Some(pal), out);
    let base = u16::from(p2[crate::p2tiles::OFF_TILE_BASE]);
    for s in &mut out[start..] {
        if crate::p2tiles::in_window(u16::from(s.tile) | u16::from(s.attr & 1) << 8, base) {
            s.page = Some(0);
        }
    }
}

/// Camera X = the saved one + `shift`.
fn set_camera(game: &mut Game, saved: &[u8], shift: i16) {
    let c = word(saved, ram::CAMERA_X).wrapping_add(shift as u16);
    game.wram_mut()[ix(ram::CAMERA_X)..][..2].copy_from_slice(&c.to_le_bytes());
}

/// Run the emitter on the object at `base` (camera-relative X `x`) with the
/// camera moved 256 px towards its side and keep what lands in the margins.
/// `queued`: the game drew the window pieces itself (skip those).
#[allow(clippy::too_many_arguments)]
fn emit(game: &mut Game, emitter: u32, base: u16, x: i16, m: i16, queued: bool, pal: Option<u8>, out: &mut Vec<MarginSprite>) {
    // Objects are at most ~64 px wide around their origin.
    if (64..192).contains(&x) || !(-m - 128..256 + m + 128).contains(&x) {
        return;
    }
    let shift = if x < 128 { -SHIFT } else { SHIFT };
    let saved = game.wram()[ix(ram::CAMERA_X)..][..2].to_vec();
    let cam = u16::from_le_bytes([saved[0], saved[1]]).wrapping_add(shift as u16);
    let w = game.wram_mut();
    w[ix(ram::CAMERA_X)..][..2].copy_from_slice(&cam.to_le_bytes());
    w[OAM_CURSOR] = 0;
    w[OAM_HI_INDEX] = 0;
    w[OAM_HI_ACC] = 0;
    // What the build gives the emitter: M=1, X=0, D=0, DB=$8D (as P2's slice).
    let mut r = game.regs();
    r.d = 0;
    r.dbr = 0x8D;
    r.p = (r.p | 0x20) & !0x18;
    r.x = base;
    game.set_regs(r);
    game.call_isolated_ret(emitter, crate::TrapReturn::Rts);
    let w = game.wram_mut();
    w[ix(ram::CAMERA_X)..][..2].copy_from_slice(&saved);
    let n = usize::from(w[OAM_CURSOR] & 0x7F);
    for k in 0..n {
        let mut e: [u8; 4] = w[ix(ram::OAM_BUF) + 4 * k..][..4].try_into().unwrap();
        // High-table bits: whole bytes at $0900, the last partial group in
        // the accumulator's top bits.
        let hi = if k / 4 < n / 4 { w[OAM_HI + k / 4] >> (2 * (k % 4)) } else { w[OAM_HI_ACC] >> (2 * (4 - n % 4 + k % 4)) };
        if let Some(p) = pal {
            crate::coop::set_oam_palette(&mut e, p);
        }
        if let Some(s) = margin_sprite(e, hi & 3, shift, m, queued) {
            out.push(s);
        }
    }
}

/// An emitted entry (camera shifted by `shift`) as a margin sprite at its
/// real X, if any of it is in the margins and the game did not draw it.
fn margin_sprite(e: [u8; 4], hi: u8, shift: i16, m: i16, queued: bool) -> Option<MarginSprite> {
    let x9 = u16::from(e[0]) | u16::from(hi & 1) << 8;
    let x = if x9 >= 256 { x9 as i16 - 512 } else { x9 as i16 } + shift;
    let w = if hi & 2 != 0 { 16 } else { 8 };
    let in_margin = (x < 0 && x + w > -m) || (x + w > 256 && x < 256 + m);
    (in_margin && !(queued && KEPT.contains(&x)))
        .then_some(MarginSprite { x, y: e[1], tile: e[2], attr: e[3], large: hi & 2 != 0, page: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_sprite_kept_and_dropped() {
        let e = |x: u8| [x, 100, 0x20, 0x31];
        // Right pass (camera + 256): emitted X 4 = real 260.
        assert_eq!(margin_sprite(e(4), 0, SHIFT, 136, true).map(|s| s.x), Some(260));
        // Real 250: the game drew it (queued), not again; unqueued, its spill.
        assert_eq!(margin_sprite(e(250), 1, SHIFT, 136, true), None);
        assert_eq!(margin_sprite([0xFA, 100, 0, 0], 1 | 2, SHIFT, 136, false).map(|s| s.x), Some(250));
        // Left pass (camera - 256): emitted 200 = real -56; past the margin: none.
        assert_eq!(margin_sprite(e(200), 0, -SHIFT, 136, false).map(|s| s.x), Some(-56));
        assert_eq!(margin_sprite(e(100), 0, -SHIFT, 136, false), None);
    }
}
