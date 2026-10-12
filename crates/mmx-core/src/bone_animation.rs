//! Optional animation sheets decoded by the frontend.
//!
//! The core has no filesystem or PNG dependency. Frontends decode `bae-sheet`
//! pages and pass cropped RGBA frames here. Movement uses ROM physics; saber
//! contact profiles follow the visible HD weapon frames.

use crate::character::Character;
use crate::ram::{self, wram_index as ix};
use crate::Game;
use jgenesis_common::frontend::EmulatorTrait;

#[derive(Debug, Clone)]
pub struct BoneFrame {
    pub width: u16,
    pub height: u16,
    /// Pixel coordinate of the game object's ROM sprite origin in this crop.
    pub pivot: [i16; 2],
    /// Cropped row-major RGBA8 pixels.
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct BoneClip {
    pub name: String,
    pub fps: f32,
    /// Loop locomotion; one-shot clips hold their final exported frame.
    pub looped: bool,
    pub frames: Vec<BoneFrame>,
}

#[derive(Debug, Clone)]
pub struct BoneSheet {
    pub clips: Vec<BoneClip>,
    /// Source sprite pixels per ROM/game pixel. Does not change physics.
    pub render_scale: u8,
    /// None retains the source direction used by legacy character sheets.
    pub source_faces_right: Option<bool>,
}

impl Default for BoneSheet {
    fn default() -> Self {
        Self { clips: Vec::new(), render_scale: 1, source_faces_right: None }
    }
}

/// A small, owned idle pose for character selection screens.
pub fn selection_portrait(game: &Game, character: Character) -> Option<BoneFrame> {
    game.bone_sheets.get(&character)
        .and_then(|sheet| sheet.clips.iter().find(|clip| clip.name == "idle"))
        .and_then(|clip| clip.frames.first())
        .cloned()
        .or_else(|| game.static_sprites.get(&character).map(|frame| frame.as_ref().clone()))
}

impl BoneSheet {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.render_scale, 1 | 2 | 4 | 8) {
            return Err("render_scale must be 1, 2, 4 or 8 source pixels per game pixel".into());
        }
        if !self.clips.iter().any(|clip| clip.name == "idle" && !clip.frames.is_empty()) {
            return Err("bone sheet needs a nonempty idle animation".into());
        }
        let mut names = std::collections::HashSet::new();
        for clip in &self.clips {
            if clip.name.is_empty() || !names.insert(clip.name.as_str()) {
                return Err(format!("empty or duplicate animation name: {}", clip.name));
            }
            if !clip.fps.is_finite() || clip.fps <= 0.0 || clip.fps > 240.0 {
                return Err(format!("invalid fps for {}", clip.name));
            }
            for frame in &clip.frames {
                validate_frame(frame).map_err(|e| format!("{e} for {}", clip.name))?;
            }
        }
        Ok(())
    }

    fn clip(&self, name: &str) -> &BoneClip {
        let alias = match name {
            "shoot" => "attack",
            "climb" => "ladder",
            "wall_jump" => "wall_kick",
            _ => name,
        };
        self.exact_clip(name)
            .or_else(|| self.exact_clip(alias))
            .unwrap_or_else(|| self.clips.iter().find(|c| c.name == "idle").expect("validated idle"))
    }

    fn exact_clip(&self, name: &str) -> Option<&BoneClip> {
        self.clips.iter().find(|c| c.name == name && !c.frames.is_empty())
    }

    /// Extended poses are opt-in: old packs keep their existing selection.
    fn clip_for_object(&self, obj: &[u8], character: Character, attacking: bool) -> &BoneClip {
        let legacy = clip_for_character(obj, character);
        if matches!(legacy, "hurt" | "death") {
            return self.clip(legacy);
        }
        if attacking {
            if let Some(clip) = self.exact_clip("attack") { return clip; }
        }
        if let Some(clip) = self.exact_clip(&format!("action_{:02x}", obj[0x02])) {
            return clip;
        }
        let locomotion = locomotion_for_object(obj);
        if shooting_frame(obj) {
            if let Some(clip) = self.exact_clip(&format!("{locomotion}_shoot")) {
                return clip;
            }
            // Explicitly oriented/high-resolution packs use the closest
            // locomotion if a combined pose is absent. Legacy sheets retain
            // their former stand-shoot fallback in the same situation.
            if (self.render_scale > 1 || self.source_faces_right.is_some())
                && !matches!(locomotion, "idle" | "ready") {
                if let Some(clip) = self.exact_clip(locomotion) { return clip; }
            }
        }
        if obj[0x58] == 4 {
            if let Some(clip) = self.exact_clip(&format!("{locomotion}_charge")) {
                return clip;
            }
        }
        // The legacy path intentionally keeps run while firing. A new pack
        // supplies the combined variant instead; absent variants keep that
        // path, rather than stopping a running character to show `shoot`.
        if !shooting_frame(obj) || legacy != "shoot" {
            if let Some(clip) = self.exact_clip(locomotion) { return clip; }
        }
        self.clip(legacy)
    }
}

impl BoneClip {
    pub(crate) fn frame_index(&self, elapsed: u64, game_fps: f64) -> usize {
        let index = (elapsed as f64 * f64::from(self.fps) / game_fps) as usize;
        if self.looped { index % self.frames.len() } else { index.min(self.frames.len() - 1) }
    }
}

pub fn validate_frame(frame: &BoneFrame) -> Result<(), String> {
    if frame.width == 0 || frame.height == 0
        || frame.rgba.len() != usize::from(frame.width) * usize::from(frame.height) * 4 {
        return Err("invalid RGBA crop".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Draw {
    pub character: Character,
    pub clip: usize,
    pub frame: usize,
    pub origin: [i16; 2],
    pub mirror: bool,
    pub static_art: bool,
}

/// X1's action and frame IDs were measured from real gameplay. The frame ID
/// distinguishes shooting from ordinary movement because firing often keeps
/// the same action. `climb` is retained for compatible sheets/ROM variants.
pub fn clip_for_player(obj: &[u8]) -> &'static str {
    let action = obj[0x02];
    let frame = obj[0x17];
    match action {
        0x0C => "death",
        0x0E => "hurt",
        0x10 => "wall_jump",
        0x12 => "wall_slide",
        0x14 | 0x20 => "dash",
        0x16 => "climb",
        0x06 => if (32..=34).contains(&frame) { "shoot" } else { "jump" },
        0x08 => if (34..=36).contains(&frame) { "shoot" } else { "fall" },
        0x04 => if (13..=22).contains(&frame) { "shoot" } else { "run" },
        0x00 | 0x02 | 0x0A => if (49..=50).contains(&frame) { "shoot" } else { "idle" },
        _ => "idle",
    }
}

/// Firing can leave X's object in an idle or shooting frame even while its
/// physics advances horizontally. Preserve the locomotion silhouette for
/// every playable character while their grounded action is moving.
fn clip_for_character(obj: &[u8], _character: Character) -> &'static str {
    let action = obj[0x02];
    let velocity = i16::from_le_bytes([obj[0x1A], obj[0x1B]]);
    if velocity != 0 && matches!(action, 0x00 | 0x02 | 0x04 | 0x0A)
    {
        return "run";
    }
    clip_for_player(obj)
}

/// X1 pose IDs shared with the X3 pose table (zero_data.rs). The high bit is
/// a ROM animation flag, not part of the pose number.
fn shooting_frame(obj: &[u8]) -> bool {
    let frame = obj[0x17] & 0x7F;
    match obj[0x02] {
        0x00 | 0x02 => (49..=50).contains(&frame),
        0x04 => (13..=22).contains(&frame),
        0x06 => (32..=34).contains(&frame),
        0x08 => (34..=36).contains(&frame),
        0x0A => (37..=38).contains(&frame),
        0x10 => (47..=48).contains(&frame),
        0x12 => (44..=46).contains(&frame),
        0x14 | 0x20 => (58..=59).contains(&frame),
        0x16 => frame == 74,
        _ => false,
    }
}

fn locomotion_for_object(obj: &[u8]) -> &'static str {
    let velocity = i16::from_le_bytes([obj[0x1A], obj[0x1B]]);
    if velocity != 0 && matches!(obj[0x02], 0x00 | 0x02 | 0x04) {
        return "run";
    }
    match obj[0x02] {
        0x02 => "ready", 0x04 => "run", 0x06 => "jump", 0x08 => "fall",
        0x0A => "land", 0x0C => "death", 0x0E => "hurt", 0x10 => "wall_jump",
        0x12 => "wall_slide", 0x14 | 0x20 => "dash", 0x16 => "climb",
        0x18 | 0x1C => "item_get", 0x1A => "beam_out", 0x1E => "hold",
        0x22 => "spawn", 0x2E | 0x30 => "captured", 0x32 => "captured_down",
        _ => "idle",
    }
}

fn obj_for(game: &Game, player: usize) -> Option<&[u8]> {
    match player {
        0 => {
            let b = ix(ram::X_BASE);
            Some(&game.wram()[b..b + ram::X_BLOCK_LEN])
        }
        1 if game.coop.enabled && !game.coop.p2_hidden => Some(&game.coop.p2),
        2 | 3 if game.local_players > player => {
            let state = &game.extra_coop[player - 2];
            (state.enabled && !state.p2_hidden).then_some(&state.p2[..])
        }
        _ => None,
    }
}

/// Called after all local player OAM emitters and before the PPU uploads OAM.
pub(crate) fn prepare(game: &mut Game) {
    game.bone_draws = [None; 4];
    if !crate::coop_stages::stage_actor_scene(game.wram()) { return; }
    let cam = |w: &[u8; 0x20000], a: u32| u16::from_le_bytes([w[ix(a)], w[ix(a) + 1]]);
    let camera = (cam(game.wram(), ram::CAMERA_X), cam(game.wram(), ram::CAMERA_Y));
    for player in 0..game.local_players {
        let character = game.player_characters[player];
        let sheet = game.bone_sheets.get(&character);
        let static_art = sheet.is_none() && game.static_sprites.contains_key(&character);
        if sheet.is_none() && !static_art { continue }
        let Some(obj) = obj_for(game, player) else { continue };
        if obj[0x0E] == 0 { continue }
        let (clip_index, frame, started) = if let Some(sheet) = sheet {
            let saber = match player {
                0 => &game.coop.p1.saber,
                1 => &game.coop.saber,
                _ => &game.extra_coop[player - 2].saber,
            };
            let clip = sheet.clip_for_object(obj, character, crate::saber::uses_saber(character) && saber.t > 0);
            let clip_index = sheet.clips.iter().position(|c| std::ptr::eq(c, clip)).unwrap();
            let previous = game.bone_states[player];
            let started = if crate::saber::uses_saber(character) && saber.t > 0 {
                game.frame_count.saturating_sub(u64::from(saber.t - 1))
            } else if previous.clip == Some(clip_index) && previous.started <= game.frame_count {
                previous.started
            } else { game.frame_count };
            let elapsed = game.frame_count.saturating_sub(started);
            let frame = clip.frame_index(elapsed, game.emu.target_fps());
            (clip_index, frame, started)
        } else { (0, 0, game.frame_count) };
        let (x, y) = crate::zero::screen_origin(obj, camera.0, camera.1);
        // bae-sheet pivots are at the feet. X1's object origin is 16 px
        // above the standing floor; boss `y_offset` is only for their ROM
        // OAM layouts, whose own feet have different relative coordinates.
        const STANDING_FOOT_Y: i16 = 16;
        // X1's facing flag is set when the player faces right. X's ROM run
        // poses face left; the supplied Zero/Vile art and Sigma's ROM poses
        // face right. Mirror relative to each source pose,
        // rather than treating the ROM flag itself as a mirror flag.
        let facing_right = obj[0x11] & 0x40 != 0;
        let mirror = sheet.and_then(|sheet| sheet.source_faces_right)
            .map_or_else(|| should_mirror(character, facing_right), |source_right| facing_right != source_right);
        let mask = match player {
            0 => game.oam_slots[0],
            1 => crate::display::slots(game.coop.p2_oam_base, game.coop.p2_oam_base + game.coop.p2_oam_count),
            2 | 3 => {
                let state = &game.extra_coop[player - 2];
                crate::display::slots(state.p2_oam_base, state.p2_oam_base + state.p2_oam_count)
            }
            _ => 0,
        };
        // If the ROM/coop emitter had no room for this player, still show the
        // bone frame. Conversely never hide a slot belonging to another player.
        let w = game.wram_mut();
        for slot in 0..128 {
            if mask >> slot & 1 != 0 {
                w[ix(ram::OAM_BUF) + slot * 4 + 1] = 0xE0;
            }
        }
        game.bone_states[player] = State { clip: Some(clip_index), started };
        game.bone_draws[player] = Some(Draw { character, clip: clip_index, frame,
            origin: [x + game.wide_margin() as i16, y + STANDING_FOOT_Y], mirror, static_art });
    }
}

fn source_faces_right(character: Character) -> bool {
    matches!(character,
        Character::Vile | Character::Sigma | Character::DRex
        | Character::ProtoMan)
}

fn should_mirror(character: Character, facing_right: bool) -> bool {
    facing_right != source_faces_right(character)
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct State {
    pub clip: Option<usize>,
    pub started: u64,
}

/// Composite decoded RGBA over the completed PPU frame, with integer pixel
/// placement and SNES-style left/right facing. This is display-only.
pub(crate) fn draw(game: &mut Game) {
    if !crate::coop_stages::stage_actor_scene(game.wram()) {
        game.bone_draws = [None;4];
        return;
    }
    // The larger framebuffer preserves sprite detail. Background/OAM art
    // stays on its original grid, replicated with nearest-neighbour pixels.
    let scale = game.player_characters[..game.local_players].iter()
        .filter_map(|character| game.bone_sheets.get(character).map(|s| s.render_scale))
        .max().unwrap_or(1).max(game.video.scale);
    game.video.upscale(scale);
    crate::character_combat::recolor_shoulder_blasts(game, scale);
    let draws = std::mem::take(&mut game.bone_draws);
    for pose in draws.into_iter().flatten() {
        let frame = if pose.static_art {
            let Some(frame) = game.static_sprites.get(&pose.character) else { continue };
            frame.as_ref()
        } else {
            let Some(sheet) = game.bone_sheets.get(&pose.character) else { continue };
            &sheet.clips[pose.clip].frames[pose.frame]
        };
        let source_scale = if pose.static_art { 1 } else { game.bone_sheets[&pose.character].render_scale };
        composite(&mut game.video.pixels, [game.video.width, game.video.height],
            frame, pose.origin, pose.mirror, source_scale, scale);
    }
}

/// Pure pixel composition shared by normal and high-resolution sheets.
fn composite(pixels: &mut [u32], [width, height]: [usize; 2], frame: &BoneFrame,
             origin: [i16; 2], mirror: bool, source_scale: u8, output_scale: u8) {
    let factor = i32::from(output_scale / source_scale);
    let fw = i32::from(frame.width) * factor;
    let fh = i32::from(frame.height) * factor;
    let ox = i32::from(origin[0]) * i32::from(output_scale);
    let oy = i32::from(origin[1]) * i32::from(output_scale);
    let pivot_x = i32::from(frame.pivot[0]) * factor;
    let left = if mirror { ox + pivot_x - fw } else { ox - pivot_x };
    let top = oy - i32::from(frame.pivot[1]) * factor;
    for sy in (-top).max(0)..fh.min(height as i32 - top) {
        let dy = top + sy;
        for sx in (-left).max(0)..fw.min(width as i32 - left) {
            let dx = left + sx;
            let source_x = if mirror { i32::from(frame.width) - 1 - sx / factor } else { sx / factor };
            let p = ((sy / factor) * i32::from(frame.width) + source_x) as usize * 4;
            let alpha = u32::from(frame.rgba[p + 3]);
            if alpha == 0 { continue }
            let at = dy as usize * width + dx as usize;
            let src = (u32::from(frame.rgba[p]) << 16)
                | (u32::from(frame.rgba[p + 1]) << 8) | u32::from(frame.rgba[p + 2]);
            if alpha == 255 { pixels[at] = src; continue }
            let dst = pixels[at];
            let blend = |shift| (((src >> shift & 255u32) * alpha + (dst >> shift & 255u32) * (255 - alpha) + 127u32) / 255u32) << shift;
            pixels[at] = blend(16) | blend(8) | blend(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pads, BTN_LEFT, BTN_RIGHT, BTN_START};

    fn test_clip(name: &str, count: usize, looped: bool) -> BoneClip {
        BoneClip { name: name.into(), fps: 60.0, looped,
            frames: vec![BoneFrame { width: 1, height: 1, pivot: [0, 1], rgba: vec![255, 0, 0, 255] }; count] }
    }

    #[test]
    fn pacing_uses_emulated_time_and_one_shots_hold_the_endpoint() {
        let looped = test_clip("run", 12, true);
        let held = test_clip("death", 12, false);
        for tick in 0..60 {
            assert_eq!(looped.frame_index(tick, 60.0), tick as usize % 12);
            assert_eq!(held.frame_index(tick, 60.0), (tick as usize).min(11));
        }
        // A 60 Hz export follows elapsed emulation time on the slightly
        // faster NTSC clock. It also works correctly on a 50 Hz PAL clock.
        assert_eq!(held.frame_index(10, 60.0988), 9);
        assert_eq!(looped.frame_index(50, 50.0), 0);
        assert_eq!(looped.frame_index(0, 60.0), 0);
    }

    #[test]
    fn combined_shots_and_scripted_states_are_selected_without_legacy_regressions() {
        let names = ["idle", "run", "jump", "fall", "dash", "climb", "wall_jump", "wall_slide",
                     "shoot", "attack", "hurt", "death", "run_shoot", "jump_shoot", "fall_shoot",
                     "dash_shoot", "climb_shoot", "wall_slide_shoot", "wall_jump_shoot",
                     "land", "land_shoot", "spawn", "beam_out", "captured", "captured_down", "hold"];
        let mut sheet = BoneSheet { clips: names.into_iter().map(|n| test_clip(n, 2, true)).collect(),
            render_scale: 4, source_faces_right: Some(true) };
        sheet.validate().unwrap();
        let mut obj = [0; 0x90];
        for (action, frame, expected) in [
            (0x04, 13, "run_shoot"), (0x06, 32, "jump_shoot"), (0x08, 35, "fall_shoot"),
            (0x14, 58, "dash_shoot"), (0x16, 74, "climb_shoot"), (0x12, 44, "wall_slide_shoot"),
            (0x10, 47, "wall_jump_shoot"), (0x0A, 37, "land_shoot"), (0x0A, 28, "land"),
            (0x22, 60, "spawn"), (0x1A, 60, "beam_out"), (0x1E, 0, "hold"),
            (0x2E, 0, "captured"), (0x30, 0, "captured"), (0x32, 0, "captured_down"),
            (0x0C, 49, "death"), (0x0E, 49, "hurt"),
        ] {
            obj[0x02] = action; obj[0x17] = frame | 0x80;
            assert_eq!(sheet.clip_for_object(&obj, Character::X, false).name, expected);
        }
        obj[0x02] = 0x00; obj[0x17] = 0;
        for character in [Character::Zero, Character::Sigma] {
            assert_eq!(sheet.clip_for_object(&obj, character, crate::saber::uses_saber(character)).name, "attack");
        }
        obj[0x02] = 0x0E;
        assert_eq!(sheet.clip_for_object(&obj, Character::Zero, true).name, "hurt");
        obj[0x02] = 0x0A; obj[0x17] = 28;
        obj[0x1A..0x1C].copy_from_slice(&0x200i16.to_le_bytes());
        assert_eq!(sheet.clip_for_object(&obj, Character::X, false).name, "land", "moving landing retains its authored landing pose");
        obj[0x1A..0x1C].fill(0);

        sheet.clips.retain(|c| !c.name.ends_with("_shoot"));
        obj[0x02] = 0x06; obj[0x17] = 32;
        assert_eq!(sheet.clip_for_object(&obj, Character::X, false).name, "jump");
        sheet.render_scale = 1; sheet.source_faces_right = None;
        assert_eq!(sheet.clip_for_object(&obj, Character::X, false).name, "shoot");
        obj[0x02] = 0x04; obj[0x17] = 13;
        obj[0x1A..0x1C].copy_from_slice(&0x200i16.to_le_bytes());
        assert_eq!(sheet.clip_for_object(&obj, Character::X, false).name, "run");
    }

    #[test]
    fn high_resolution_pixels_and_trimmed_pivots_keep_world_anchor_when_mirrored() {
        let rgba: Vec<u8> = (0..32).flat_map(|n| [n, 100, 0, 255]).collect();
        let frame = BoneFrame { width: 8, height: 4, pivot: [4, 4], rgba };
        for mirror in [false, true] {
            let mut output = vec![0; 24 * 24];
            composite(&mut output, [24, 24], &frame, [3, 3], mirror, 4, 4);
            for y in 0..4 {
                for x in 0..8 {
                    let sx = if mirror { 7 - x } else { x };
                    assert_eq!(output[(y + 8) * 24 + x + 8], ((y * 8 + sx) as u32) << 16 | 100 << 8);
                }
            }
            assert!(output[12 * 24..].iter().all(|&p| p == 0), "pixels below fixed feet");
        }
        // Crop coordinates can put the anchor outside the frame. Cropping
        // must produce identical placement, including the mirrored case.
        let mut untrimmed = BoneFrame { width: 6, height: 4, pivot: [4, 4], rgba: vec![0; 6 * 4 * 4] };
        untrimmed.rgba[(1 * 6 + 1) * 4..][..4].copy_from_slice(&[1, 2, 3, 255]);
        let trimmed = BoneFrame { width: 1, height: 1, pivot: [3, 3], rgba: vec![1, 2, 3, 255] };
        for mirror in [false, true] {
            let (mut a, mut b) = (vec![0; 24 * 24], vec![0; 24 * 24]);
            composite(&mut a, [24, 24], &untrimmed, [3, 3], mirror, 4, 4);
            composite(&mut b, [24, 24], &trimmed, [3, 3], mirror, 4, 4);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn legacy_sprites_repeat_nearest_into_the_high_resolution_buffer() {
        let frame = BoneFrame { width: 2, height: 1, pivot: [1, 1], rgba: vec![10, 20, 30, 255, 40, 50, 60, 255] };
        for mirror in [false, true] {
            let (mut native, mut hd) = (vec![0; 6 * 6], vec![0; 24 * 24]);
            composite(&mut native, [6, 6], &frame, [3, 3], mirror, 1, 1);
            composite(&mut hd, [24, 24], &frame, [3, 3], mirror, 1, 4);
            for y in 0..24 { for x in 0..24 { assert_eq!(hd[y * 24 + x], native[y / 4 * 6 + x / 4]); } }
        }
    }

    #[test]
    fn real_rom_high_resolution_preserves_physics_and_displays_subpixel_detail() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM not set; skipping high-resolution renderer test");
        };
        let mut base = Game::from_rom(rom.clone()).unwrap();
        let sheet = |scale: u8| {
            let edge = 16 * u16::from(scale);
            let mut rgba = Vec::new();
            for y in 0..edge { for x in 0..edge {
                let detail = scale > 1 && x % 2 == 1 && y % 2 == 0;
                rgba.extend_from_slice(if detail { &[1, 253, 249, 255] } else { &[253, 1, 249, 255] });
            } }
            BoneSheet { clips: vec![BoneClip { name: "idle".into(), fps: 60.0,
                looped: true, frames: vec![BoneFrame { width: edge, height: edge,
                    pivot: [(edge / 2) as i16, edge as i16], rgba }] }],
                render_scale: scale, source_faces_right: Some(true) }
        };
        base.set_bone_sheet(Character::X, sheet(1)).unwrap();
        let mut visible = false;
        for tick in 0..2400 {
            base.step(Pads { p1: if tick % 90 == 0 { BTN_START } else { 0 }, p2: 0 });
            base.take_audio();
            if base.frame().0.contains(&0xFD01F9) { visible = true; break; }
        }
        assert!(visible, "no real player renderer reached");
        let state = base.save_state();
        let mut hd = Game::from_rom(rom).unwrap();
        hd.set_bone_sheet(Character::X, sheet(4)).unwrap();
        hd.load_state(&state).unwrap();
        for _ in 0..16 {
            let pads = Pads { p1: BTN_RIGHT, p2: 0 };
            base.step(pads); hd.step(pads);
            base.take_audio(); hd.take_audio();
            assert_eq!(base.wram(), hd.wram(), "presentation scale changed emulated RAM");
            let (b, bw, bh) = base.frame(); let (h, hw, hh) = hd.frame();
            assert_eq!((hw, hh, hd.frame_scale()), (bw * 4, bh * 4, 4));
            assert!(h.contains(&0x01FDF9), "high-resolution-only texel detail was reduced away");
            // Everything outside the replaced sprite remains exact nearest
            // replication of the ROM background and non-target OAM art.
            for y in 0..hh { for x in 0..hw {
                let pixel = h[y * hw + x];
                let original = b[y / 4 * bw + x / 4];
                if pixel != 0xFD01F9 && pixel != 0x01FDF9 && original != 0xFD01F9 {
                    assert_eq!(pixel, original);
                }
            } }
        }
    }

    #[test]
    fn mirror_tracks_rom_facing_and_source_art() {
        for character in [Character::Vile, Character::Sigma,
                          Character::DRex, Character::ProtoMan] {
            assert!(!should_mirror(character, true), "{} should face right", character.id());
            assert!(should_mirror(character, false), "{} should face left", character.id());
        }
        for character in [Character::X, Character::Zero, Character::ChillPenguin, Character::SparkMandrill,
                          Character::StormEagle, Character::Velguarder, Character::Bass] {
            assert!(should_mirror(character, true), "{} needs a right-facing flip", character.id());
            assert!(!should_mirror(character, false), "{} should keep its left source", character.id());
        }
    }

    #[test]
    fn action_mapping_covers_gameplay() {
        let mut obj = [0; 0x90];
        for (action, frame, expected) in [
            (0x04, 3, "run"), (0x04, 13, "shoot"), (0x14, 54, "dash"),
            (0x06, 23, "jump"), (0x08, 26, "fall"), (0x10, 42, "wall_jump"),
            (0x12, 39, "wall_slide"), (0x16, 67, "climb"), (0x0E, 51, "hurt"),
        ] {
            obj[0x02] = action;
            obj[0x17] = frame;
            assert_eq!(clip_for_player(&obj), expected);
        }
    }

    #[test]
    fn moving_shots_keep_all_characters_in_their_run_clip() {
        let mut obj = [0; 0x90];
        obj[0x02] = 0x04;
        obj[0x17] = 13; // The ROM's shooting frame inside its run action.
        obj[0x1A..0x1C].copy_from_slice(&0x0200i16.to_le_bytes());
        for character in [Character::X, Character::Zero, Character::Sigma, Character::Vile] {
            assert_eq!(clip_for_character(&obj, character), "run");
        }
        obj[0x1A..0x1C].fill(0);
        assert_eq!(clip_for_character(&obj, Character::Vile), "shoot");
        obj[0x02] = 0x06;
        obj[0x1A] = 1;
        assert_eq!(clip_for_character(&obj, Character::Sigma), "jump");
    }

    #[test]
    fn real_rom_sheet_replaces_player_pixels() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM not set; skipping sheet renderer test");
        };
        let mut game = Game::from_rom(rom).unwrap();
        let purple = [252, 3, 251, 255];
        game.set_bone_sheet(Character::X, BoneSheet { clips: vec![BoneClip {
            name: "idle".into(), fps: 60.0988, looped: true,
            frames: vec![BoneFrame { width: 16, height: 16, pivot: [8, 8], rgba: purple.repeat(256) }],
        }], ..BoneSheet::default() }).unwrap();
        let mut saw_purple = false;
        for frame in 0..2400 {
            game.step(Pads { p1: if frame % 90 == 0 { BTN_START } else { 0 }, p2: 0 });
            saw_purple |= game.frame().0.contains(&0xFC03FB);
            if saw_purple { break }
        }
        assert!(saw_purple, "decoded sheet never appeared in actual ROM rendering");
        assert_eq!(crate::display::visible(game.wram(), game.oam_slots[0]), 0,
            "the original X OAM sprite remained visible under the sheet");
    }

    #[test]
    fn real_rom_four_players_draw_bone_and_static_art_without_oam_budget() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM not set; skipping four-sheet renderer test");
        };
        let mut game = Game::from_rom(rom).unwrap();
        game.coop.enabled = true;
        game.set_local_players(4);
        for (player, character, color) in [
            (0, Character::X, [251, 1, 3, 255]),
            (1, Character::Zero, [1, 251, 3, 255]),
            (2, Character::Sigma, [3, 1, 251, 255]),
            (3, Character::Vile, [251, 1, 249, 255]),
        ] {
            game.set_player_character(player, character);
            let frame = BoneFrame { width: 16, height: 16, pivot: [8, 16], rgba: color.repeat(256) };
            if character == Character::Vile {
                game.set_static_sprite(character, frame).unwrap();
            } else { game.set_bone_sheet(character, BoneSheet { clips: vec![BoneClip {
                name: "idle".into(), fps: 60.0988, looped: true,
                frames: vec![frame],
            }], ..BoneSheet::default() }).unwrap(); }
        }
        let mut seen = [false; 4];
        let colors = [0xFB0103, 0x01FB03, 0x0301FB, 0xFB01F9];
        let mut spawned = false;
        for frame in 0..2400 {
            if spawned { game.set_extra_pads([BTN_LEFT, 0]); }
            game.step(Pads { p1: if frame % 90 == 0 { BTN_START } else { 0 }, p2: if spawned { BTN_RIGHT } else { 0 } });
            spawned |= game.visible_extra_player_object(0).is_some() && game.visible_extra_player_object(1).is_some();
            for (found, color) in seen.iter_mut().zip(colors) {
                *found |= game.frame().0.contains(&color);
            }
            if seen.iter().all(|&found| found) { break }
        }
        assert!(seen.iter().all(|&found| found), "missing per-player sheet pixels: {seen:?}");
    }
}
