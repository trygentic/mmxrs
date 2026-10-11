//! Optional, display-only animation sheets decoded by the frontend.
//!
//! The core has no filesystem or PNG dependency. Frontends decode `bae-sheet`
//! pages and pass cropped RGBA frames here; ROM physics and combat stay intact.

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
    pub frames: Vec<BoneFrame>,
}

#[derive(Debug, Clone)]
pub struct BoneSheet {
    pub clips: Vec<BoneClip>,
}

impl BoneSheet {
    pub fn validate(&self) -> Result<(), String> {
        if !self.clips.iter().any(|clip| clip.name == "idle" && !clip.frames.is_empty()) {
            return Err("bone sheet needs a nonempty idle animation".into());
        }
        for clip in &self.clips {
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
        self.clips.iter().find(|c| c.name == name && !c.frames.is_empty())
            .or_else(|| self.clips.iter().find(|c| c.name == alias && !c.frames.is_empty()))
            .unwrap_or_else(|| self.clips.iter().find(|c| c.name == "idle").expect("validated idle"))
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
            let clip = sheet.clip(clip_for_character(obj, character));
            let clip_index = sheet.clips.iter().position(|c| std::ptr::eq(c, clip)).unwrap();
            let previous = game.bone_states[player];
            let started = if previous.clip == Some(clip_index) { previous.started } else { game.frame_count };
            let elapsed = game.frame_count.saturating_sub(started);
            let frame = ((elapsed as f64 * f64::from(clip.fps) / game.emu.target_fps()) as usize) % clip.frames.len();
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
        let mirror = should_mirror(character, facing_right);
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
    let draws = std::mem::take(&mut game.bone_draws);
    for pose in draws.into_iter().flatten() {
        let frame = if pose.static_art {
            let Some(frame) = game.static_sprites.get(&pose.character) else { continue };
            frame.as_ref()
        } else {
            let Some(sheet) = game.bone_sheets.get(&pose.character) else { continue };
            &sheet.clips[pose.clip].frames[pose.frame]
        };
        let fw = usize::from(frame.width);
        let fh = usize::from(frame.height);
        let left = if pose.mirror { pose.origin[0] + frame.pivot[0] - frame.width as i16 }
            else { pose.origin[0] - frame.pivot[0] };
        let top = pose.origin[1] - frame.pivot[1];
        for sy in 0..fh {
            let dy = top + sy as i16;
            if dy < 0 || usize::try_from(dy).unwrap_or(usize::MAX) >= game.video.height { continue }
            for sx in 0..fw {
                let dx = left + sx as i16;
                if dx < 0 || usize::try_from(dx).unwrap_or(usize::MAX) >= game.video.width { continue }
                let source_x = if pose.mirror { fw - 1 - sx } else { sx };
                let p = (sy * fw + source_x) * 4;
                let alpha = u32::from(frame.rgba[p + 3]);
                if alpha == 0 { continue }
                let at = dy as usize * game.video.width + dx as usize;
                let src = (u32::from(frame.rgba[p]) << 16)
                    | (u32::from(frame.rgba[p + 1]) << 8) | u32::from(frame.rgba[p + 2]);
                if alpha == 255 { game.video.pixels[at] = src; continue }
                let dst = game.video.pixels[at];
                let blend = |shift| (((src >> shift & 255u32) * alpha + (dst >> shift & 255u32) * (255 - alpha) + 127u32) / 255u32) << shift;
                game.video.pixels[at] = blend(16) | blend(8) | blend(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pads, BTN_LEFT, BTN_RIGHT, BTN_START};

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
            name: "idle".into(), fps: 60.0988,
            frames: vec![BoneFrame { width: 16, height: 16, pivot: [8, 8], rgba: purple.repeat(256) }],
        }] }).unwrap();
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
                name: "idle".into(), fps: 60.0988,
                frames: vec![frame],
            }] }).unwrap(); }
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
