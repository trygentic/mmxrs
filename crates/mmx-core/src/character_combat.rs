//! Vile's shoulder cannon uses the ROM's full-charge shot and enemy collision,
//! with its own launch point and yellow native projectile palette. Ownership already lives
//! in rollback-safe co-op shot masks; no separate projectile simulation.
use crate::{ram, Character, Game};
use crate::ram::wram_index as ix;

const SHOTS: u32 = 0x7E1228;
const STRIDE: usize = 0x40;
const LARGE_CHARGE_ID: u8 = 3;
const LARGE_CHARGE_SET: u8 = 0x9e;
const SHOULDER: (i16, i16) = (20, -19);

pub(crate) fn shoulder_muzzle(game: &mut Game) {
    let k = game.regs().d.wrapping_sub(SHOTS as u16) / STRIDE as u16;
    if k >= 8 { return; }
    let at = ix(SHOTS) + usize::from(k) * STRIDE;
    let w = game.wram();
    if w[at] == 0 || (!game.active_shot_initial && !(w[at + 0x0a] == LARGE_CHARGE_ID && w[at + 2] == 0)) { return; }
    let b = ix(ram::X_BASE);
    let direction = if w[at + 0x11] & 0x40 != 0 { 1 } else { -1 };
    let px = i16::from_le_bytes([w[b + 5], w[b + 6]]);
    let py = i16::from_le_bytes([w[b + 8], w[b + 9]]);
    let (x, y) = shoulder_position((px, py), direction);
    let w = game.wram_mut();
    w[at + 5..at + 7].copy_from_slice(&x.to_le_bytes());
    w[at + 8..at + 10].copy_from_slice(&y.to_le_bytes());
    w[at + 0x11] = (w[at + 0x11] & !0x0e) | 6; // Native charged-shot palette 3.
}

// Initial placement only; flying projectiles keep their independent position.
fn shoulder_position((px, py): (i16, i16), direction: i16) -> (i16, i16) {
    (px.wrapping_add(direction * SHOULDER.0), py.wrapping_add(SHOULDER.1))
}

fn owner(game: &Game, slot: usize) -> usize {
    if game.coop.p2_shots >> slot & 1 != 0 { return 1; }
    for i in 0..game.local_players.saturating_sub(2).min(2) {
        if game.extra_coop[i].p2_shots >> slot & 1 != 0 { return i + 2; }
    }
    0
}

/// Upgrade before the first ROM shot update. ID 3 is the observed natural full
/// 32-pixel blue/white animation, velocity, hitbox and enemy damage table; it is not a
/// stretched lemon. The owner's charged-shot counter follows the new type.
pub(crate) fn prepare_shoulder_blast(game: &mut Game) {
    if !game.active_shot_initial { return; }
    // A normal charged fire also loads palette ID $20. Upgrading a lemon
    // skips that player routine, so run the same resolved ROM loader here.
    let regs = game.regs();
    let mut palette_regs = regs;
    palette_regs.y = 0x40;
    game.set_regs(palette_regs);
    game.call_isolated(game.hook(crate::hooks::HookId::PaletteLoad));
    game.set_regs(regs);
    let at = usize::from(game.regs().d);
    let w = game.wram_mut();
    if w[at + 0x0a] != LARGE_CHARGE_ID {
        let was_lemon = w[at + 0x0a] == 0;
        w[at + 0x0a] = LARGE_CHARGE_ID;
        w[at + 4] = 0x40; // Charged spawn flag observed in natural X release.
        if was_lemon {
            let charged = ix(ram::X_BASE) + 0x63;
            w[charged] = w[charged].saturating_add(1);
        }
    }
}

#[derive(Clone, Copy)]
struct Piece { x: i32, y: i32, tile: u16, size: i32, flips: u8 }

#[derive(Clone)]
struct Blast {
    owner: usize,
    slot: usize,
    pieces: Vec<Piece>,
}

fn matching_blast(blasts: &[Blast], actual: Piece) -> Option<&Blast> {
    let distance = |b: &Blast| b.pieces.iter().filter(|p| p.tile == actual.tile && p.size == actual.size && p.flips == actual.flips)
        .map(|p| (p.x - actual.x).pow(2) + (p.y - actual.y).pow(2)).min().unwrap_or(i32::MAX);
    let nearest = blasts.iter().min_by_key(|b| (distance(b), b.slot))?;
    (distance(nearest) <= 16).then_some(nearest)
}

fn native_pieces(game: &Game, set: u8, frame: u8, origins: &[[i32; 2]], facing_right: bool, offset: u8) -> Vec<Piece> {
    let long = |a| u32::from(game.rom_read(a)) | u32::from(game.rom_read(a + 1)) << 8 | u32::from(game.rom_read(a + 2)) << 16;
    let table = long(crate::zero_data::SET_TABLE + u32::from(set) * 3);
    let frame = frame & 0x7f;
    if set == LARGE_CHARGE_SET && frame > 9 { return Vec::new(); }
    let mut frames = vec![frame];
    if frame > 0 { frames.push(frame - 1); }
    if set == LARGE_CHARGE_SET && frame == 2 { frames.push(4); } // native right flight loop wraps 4 -> 2
    if set == LARGE_CHARGE_SET && frame == 7 { frames.push(9); } // left loop wraps 9 -> 7
    let mut out = Vec::new();
    for f in frames {
        let layout = long(table + u32::from(f) * 3);
        let count = game.rom_read(layout);
        if count > 32 { continue; }
        for i in 0..count {
            let a = layout + 1 + u32::from(i) * 4;
            let flags = game.rom_read(a + 3);
            let size = if flags & 0x20 != 0 { 16 } else { 8 };
            let dx = i32::from(game.rom_read(a) as i8);
            let dx = if facing_right { -dx - size } else { dx };
            let dy = i32::from(game.rom_read(a + 1) as i8);
            let tile = u16::from(game.rom_read(a + 2).wrapping_add(offset));
            let flips = (flags & 0xc0) ^ if facing_right { 0x40 } else { 0 };
            for origin in origins { out.push(Piece { x: origin[0] + dx, y: origin[1] + dy, tile, size, flips }); }
        }
    }
    out
}

/// Recolor only opaque pixels of native palette-3 OAM tiles belonging to
/// Vile's charged blast. The source shape, frame timing and transparency all
/// come from the ROM. Blue scenery in transparent sprite areas is untouched.
pub(crate) fn recolor_shoulder_blasts(game: &mut Game, scale: u8) {
    let word = |w: &[u8], o: usize| i32::from(u16::from_le_bytes([w[o], w[o + 1]]));
    let w = game.wram();
    let camera = (word(w, ix(ram::CAMERA_X)), word(w, ix(ram::CAMERA_Y)));
    let blasts: Vec<_> = (0..8).filter_map(|slot| {
        let at = ix(SHOTS) + slot * STRIDE;
        let player = owner(game, slot);
        (w[at] != 0 && matches!(w[at + 0x0a], 2 | LARGE_CHARGE_ID) && player < game.local_players
            && w[at + 0x20..at + 0x22] != ((SHOTS as usize + slot * STRIDE + crate::saber::BOX_AT) as u16).to_le_bytes())
            .then(|| {
                // The visible OAM belongs to the preceding native movement
                // sample. Account for one tick of signed 8.8 shot velocity.
                let vx = i32::from(i16::from_le_bytes([w[at + 0x1a], w[at + 0x1b]])) / 256;
                let vy = i32::from(i16::from_le_bytes([w[at + 0x1c], w[at + 0x1d]])) / 256;
                let drawn = [word(w, at + 5) - camera.0 - vx, word(w, at + 8) - camera.1 - vy];
                let current = [word(w, at + 5) - camera.0, word(w, at + 8) - camera.1];
                let previous = if w[at + 0x0a] == LARGE_CHARGE_ID {
                    [word(w, at + 0x22) - camera.0, word(w, at + 0x24) - camera.1]
                } else { drawn };
                let pieces = native_pieces(game, w[at + 0x16], w[at + 0x17], &[previous, drawn, current], w[at + 0x11] & 0x40 != 0, w[at + 0x18]);
                Blast { owner: player, slot, pieces }
            })
    }).collect();
    if !blasts.iter().any(|b| game.player_characters[b.owner] == Character::Vile) { return; }
    let oam = game.oam();
    let vram = game.vram().to_vec();
    let palette = game.cgram();
    let front = game.emu.obj_front().map(|p| p.to_vec());
    let brightness = game.emu.frame_inidisp().0;
    let (base, gap) = game.obsel();
    let margin = i32::from(game.wide_margin());
    let scale = i32::from(scale);
    for slot in 0..128 {
        let at = slot * 4;
        let attr = oam[at + 3];
        if attr >> 1 & 7 != 3 { continue; }
        let high = oam[512 + slot / 4] >> (2 * (slot % 4)) & 3;
        let x = i32::from(oam[at]) - if high & 1 != 0 { 256 } else { 0 };
        let y = i32::from(oam[at + 1]);
        let size = if high & 2 != 0 { 16 } else { 8 };
        let tile = u16::from(oam[at + 2]);
        let Some(blast) = matching_blast(&blasts, Piece { x, y, tile, size, flips: attr & 0xc0 }) else { continue };
        let tint = game.player_characters[blast.owner] == Character::Vile;
        let name = if attr & 1 != 0 { 0x1000 + gap } else { 0 };
        for py in 0..size { for px in 0..size {
            let sx = if attr & 0x40 != 0 { size - 1 - px } else { px };
            let sy = if attr & 0x80 != 0 { size - 1 - py } else { py };
            let index = obj_pixel(&vram, base.wrapping_add(name), tile, sx, sy);
            if index == 0 { continue; }
            let native = palette[128 + 3 * 16 + usize::from(index)];
            let (r, g, b) = (native & 31, native >> 5 & 31, native >> 10 & 31);
            if r.max(g).max(b) - r.min(g).min(b) < 3 { continue; }
            for yy in (y + py) * scale..(y + py + 1) * scale {
                for xx in (x + px + margin) * scale..(x + px + margin + 1) * scale {
                    if xx < 0 || yy < 0 || xx >= game.video.width as i32 || yy >= game.video.height as i32 { continue; }
                    let native_x = x + px;
                    let native_y = y + py;
                    if (0..256).contains(&native_x) && (0..239).contains(&native_y) {
                        if front.as_ref().is_some_and(|p| p[native_y as usize * 256 + native_x as usize] != slot as u8) { continue; }
                    }
                    let at = yy as usize * game.video.width + xx as usize;
                    let color = game.video.pixels[at];
                    if let Some(yellow) = yellow_pixel(native, color, brightness) {
                        if tint {
                            game.video.pixels[at] = yellow;
                            #[cfg(test)] RECOLORED_PIXELS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        } else {
                            #[cfg(test)] if blast.owner == 0 { ORIGINAL_X_PIXELS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
                        }
                    }
                }
            }
        }}
    }
}

fn obj_pixel(vram: &[u16], base: u16, tile: u16, x: i32, y: i32) -> u16 {
    let t = (tile & 0xf0).wrapping_add(((tile & 15) + (x / 8) as u16) & 15)
        .wrapping_add((y / 8) as u16 * 16);
    let addr = usize::from(base.wrapping_add(t * 16)) & 0x7fff;
    let low = vram[(addr + (y % 8) as usize) & 0x7fff];
    let high = vram[(addr + 8 + (y % 8) as usize) & 0x7fff];
    let bit = 7 - x % 8;
    ((low >> bit) & 1) | ((low >> (bit + 8)) & 1) << 1
        | ((high >> bit) & 1) << 2 | ((high >> (bit + 8)) & 1) << 3
}

fn yellow_pixel(native: u16, rendered: u32, brightness: u8) -> Option<u32> {
    let (r, g, b) = (native & 31, native >> 5 & 31, native >> 10 & 31);
    if r.max(g).max(b) - r.min(g).min(b) < 3 { return None; }
    let (cr, cg, cb) = (rendered >> 16 & 255, rendered >> 8 & 255, rendered & 255);
    let matches = |c: u32, p: u16| {
        let expected = (f64::from(p) * f64::from(brightness) * 255.0 / (31.0 * 15.0)).round() as i32;
        (c as i32 - expected).abs() <= 1
    };
    if !matches(cr, r) || !matches(cg, g) || !matches(cb, b) { return None; }
    let high = cr.max(cg).max(cb);
    let low = cr.min(cg).min(cb);
    Some((high << 16) | ((high * 4 / 5).max(low) << 8) | low)
}

#[cfg(test)]
static ORIGINAL_X_PIXELS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
static RECOLORED_PIXELS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
mod tests {
    use super::*;
    static MID_SHOT: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);
    fn capture_mid_shot(game: &mut Game) {
        if game.active_shot_initial && game.active_shot_extra == Some(1) {
            *MID_SHOT.lock().unwrap() = Some(game.save_before_current_trap());
        }
        crate::coop::local_shot_slot_done(game);
    }
    #[test]
    fn demo_survival_uses_ordinary_revive_without_healing_dead_or_scripted_players() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else { return; };
        let mut g = Game::from_rom(rom).unwrap();
        g.coop.enabled = true;
        g.set_local_players(4);
        g.wram_mut()[ix(ram::MODE_D2)] = crate::coop_stages::MODE_IN_STAGE;
        g.wram_mut()[ix(ram::STAGE_PHASE)] = crate::coop_stages::PHASE_GAMEPLAY;
        g.wram_mut()[ix(ram::MAX_HP)] = 16;
        g.wram_mut()[ix(ram::X_HP)] = 8;
        g.wram_mut()[ix(ram::X_ACTION)] = 0;
        g.coop.p2_hidden = false;
        g.coop.p2[crate::coop::OFF_HP] = 8;
        g.extra_coop[0].p2_hidden = true;
        g.extra_coop[0].down_frames = 600;
        g.extra_coop[0].revive = crate::coop_hud::Revive { spot: Some((200, 300)), touch: 9, half_hp: true };
        g.extra_coop[1].p2_hidden = true;
        g.extra_coop[1].down_frames = 20;
        let dead_block = g.extra_coop[0].p2;
        let revive = g.extra_coop[0].revive;
        g.maintain_demo_survival();
        assert_eq!(g.wram()[ix(ram::X_HP)] & 0x7f, 16);
        assert_eq!(g.coop.p2[crate::coop::OFF_HP] & 0x7f, 16);
        assert_eq!(g.extra_coop[0].down_frames, 30);
        assert_eq!(g.extra_coop[1].down_frames, 20, "shorter timer was reset");
        assert_eq!(g.extra_coop[0].p2, dead_block, "dead HP or position was fabricated");
        assert_eq!(g.extra_coop[0].revive, revive);
        g.wram_mut()[ix(ram::X_ACTION)] = 0x2e; // Vile's native captured sequence.
        g.wram_mut()[ix(ram::X_HP)] = 1;
        g.extra_coop[0].down_frames = 600;
        g.maintain_demo_survival();
        assert_eq!(g.wram()[ix(ram::X_HP)], 1);
        assert_eq!(g.extra_coop[0].down_frames, 600);
        g.wram_mut()[ix(ram::X_ACTION)] = 0x0c;
        g.wram_mut()[ix(ram::X_HP)] = 0;
        g.maintain_demo_survival();
        assert_eq!(g.wram()[ix(ram::X_HP)], 0, "P1's normal death flow was bypassed");
    }

    #[test]
    fn natural_maximum_x_charge_uses_large_blue_variant() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else { return; };
        let mut g = Game::from_rom(rom).unwrap();
        let b = ix(ram::X_BASE);
        for f in 0..2400 {
            g.step(crate::Pads { p1: if f % 90 == 0 { crate::BTN_START } else { 0 }, p2: 0 });
            if crate::coop_stages::p2_anchor_ok(g.wram()) && g.wram()[b + 1] == 2 && g.wram()[b + 2] == 0 { break; }
        }
        // Let the original player charge and release naturally. This guards
        // against mistaking a damage-table index for the real large variant.
        for _ in 0..220 { g.step(crate::Pads { p1: crate::BTN_Y, p2: 0 }); }
        assert_eq!(g.wram()[b + 0x58], 4, "X did not reach natural maximum charge");
        let mut large_flight = false;
        for frame in 0..14 {
            g.step(crate::Pads::default());
            for slot in 0..8 {
                let a=ix(SHOTS)+slot*STRIDE;
                if g.wram()[a] == 0 { continue; }
                assert_eq!(g.wram()[a + 1], 2);
                assert_eq!(g.wram()[a + 0x0a], LARGE_CHARGE_ID);
                assert_eq!(g.wram()[a + 0x16], LARGE_CHARGE_SET);
                if g.wram()[a + 2] == 2 {
                    let box_pointer = u16::from_le_bytes([g.wram()[a + 0x20],g.wram()[a + 0x21]]);
                    let half_w=g.rom_read(0x860000|u32::from(box_pointer.wrapping_add(2)));
                    let half_h=g.rom_read(0x860000|u32::from(box_pointer.wrapping_add(3)));
                    large_flight |= half_w >= 18 && half_h >= 17;
                }
            }
            if frame == 6 {
                let (pixels, width, height)=g.frame();
                let mut ppm=format!("P6\n{width} {height}\n255\n").into_bytes();
                for &color in pixels {ppm.extend([(color>>16)as u8,(color>>8)as u8,color as u8]);}
                std::fs::write("/tmp/mmx_natural_charge.ppm",ppm).unwrap();
            }
        }
        assert!(large_flight, "natural charged projectile did not grow to the large native collider");
    }

    #[test]
    fn native_piece_attribution_considers_x_and_vile_with_oam_order_ties() {
        let piece = |x| Piece { x, y: 80, tile: 32, size: 16, flips: 0x40 };
        let blasts = [Blast { owner: 0, slot: 1, pieces: vec![piece(100)] }, Blast { owner: 3, slot: 0, pieces: vec![piece(104)] }];
        assert_eq!(matching_blast(&blasts, piece(100)).unwrap().owner, 0);
        assert_eq!(matching_blast(&blasts, piece(104)).unwrap().owner, 3);
        assert_eq!(matching_blast(&blasts, piece(102)).unwrap().owner, 3, "pool order follows front native OAM on identical distances");
        assert!(matching_blast(&blasts, piece(200)).is_none());
        let wrong_tile = Piece { tile: 34, ..piece(100) };
        assert!(matching_blast(&blasts, wrong_tile).is_none(), "other sprite pieces were attributed by proximity alone");
    }

    #[test]
    fn native_tile_mask_preserves_transparency_and_foreground_occlusion() {
        let mut vram = vec![0; 0x8000];
        vram[0] = 0x8000; // opaque palette index 2 at first pixel only.
        assert_eq!(obj_pixel(&vram, 0, 0, 0, 0), 2);
        assert_eq!(obj_pixel(&vram, 0, 0, 1, 0), 0);
        let cyan = (20 << 5) | (31 << 10);
        assert_eq!(yellow_pixel(cyan, 0x00a5ff, 15), Some(0xffcc00));
        assert_eq!(yellow_pixel(cyan, 0x884422, 15), None, "occluding foreground was repainted");
        // Transparent pixels never call the recolor path, even when scenery
        // happens to have exactly the charged shot's cyan RGB value.
        let backdrop = 0x00a5ff;
        let result = if obj_pixel(&vram, 0, 0, 1, 0) == 0 { backdrop }
            else { yellow_pixel(cyan, backdrop, 15).unwrap_or(backdrop) };
        assert_eq!(result, backdrop);
    }

    #[test]
    fn real_rom_four_player_sabers_and_shoulder_charged_blast() {
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM not set; skipping combat integration test");
        };
        ORIGINAL_X_PIXELS.store(0, std::sync::atomic::Ordering::Relaxed);
        RECOLORED_PIXELS.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut game = Game::from_rom(rom).unwrap();
        game.coop.enabled = true;
        game.set_local_players(4);
        crate::coop::register_coop_traps(&mut game).unwrap();
        for (i, character) in [Character::X, Character::Zero, Character::Sigma, Character::Vile].into_iter().enumerate() {
            game.set_player_character(i, character);
        }
        for f in 0..2400 {
            game.step(crate::Pads { p1: if f % 90 == 0 { crate::BTN_START } else { 0 }, p2: 0 });
            if game.visible_extra_player_object(1).is_some() && game.coop.p2[2] == 0 { break; }
        }
        assert!(game.visible_extra_player_object(1).is_some(), "four players never spawned");
        // Test fixture places the followers in a legal standing action; input
        // still runs through the actual ROM player and projectile routines.
        for block in std::iter::once(&mut game.coop.p2).chain(game.extra_coop.iter_mut().map(|c| &mut c.p2)) {
            block[2] = 0;
            block[0x11] |= 0x40;
        }
        // Roundtrip new pending state and reset it when loading older tails.
        game.active_shot_initial = true;
        game.active_shot_extra = Some(1);
        let pending = game.save_state();
        game.active_shot_initial = false;
        game.active_shot_extra = None;
        game.load_state(&pending).unwrap();
        assert!(game.active_shot_initial);
        assert_eq!(game.active_shot_extra, Some(1));
        let old = &pending[..pending.len() - 7]; // SHP1 + bool + Some discriminator + index.
        game.load_state(old).unwrap();
        assert!(!game.active_shot_initial);
        assert_eq!(game.active_shot_extra, None);
        game.traps.register(game.hook(crate::coop::HOOK_SHOT_SLOT_DONE), "test_mid_shot", crate::TrapReturn::Observe, capture_mid_shot);
        game.set_extra_pads([crate::BTN_X, crate::BTN_Y]);
        game.step(crate::Pads { p1: 0, p2: crate::BTN_X });
        let mut mid_shot_checked = false;
        let snapshot = MID_SHOT.lock().unwrap().take();
        if let Some(mid) = snapshot {
            let expected = game.save_state();
            let expected_pixels = game.frame().0.to_vec();
            let expected_wram = game.wram().to_vec();
            game.load_state(&mid).unwrap();
            assert!(game.active_shot_initial);
            assert_eq!(game.active_shot_extra, Some(1));
            game.step(crate::Pads { p1: 0, p2: crate::BTN_X });
            let actual = game.save_state();
            let first_diff = actual.iter().zip(&expected).position(|(a,b)| a != b);
            let ram_diff = game.wram().iter().zip(&expected_wram).position(|(a,b)| a != b);
            assert!(actual == expected, "mid-shot rollback changed emulation: diff {first_diff:?} RAM {ram_diff:?}, lengths {} {}", actual.len(), expected.len());
            assert!(game.frame().0 == expected_pixels, "mid-shot rollback changed rocket rendering");
            MID_SHOT.lock().unwrap().take();
            mid_shot_checked = true;
        }
        assert_eq!(game.coop.saber.t, 1, "Zero never swung without X3");
        assert_eq!(game.extra_coop[0].saber.t, 1, "Sigma never swung");
        assert_ne!(game.extra_coop[1].p2_shots, 0, "Vile never launched a projectile");
        // Create a nearby simultaneous X full-charge projectile from the
        // exact initialized native object. Fixture-only placement avoids a
        // long charge wait; both then run through the ROM shot/OAM routines.
        let vile_slot = (0..8).find(|&i| game.extra_coop[1].p2_shots >> i & 1 != 0).unwrap();
        let x_slot = (0..8).find(|&i| game.wram()[ix(SHOTS) + i * STRIDE] == 0).unwrap();
        let from = ix(SHOTS) + vile_slot * STRIDE;
        let to = ix(SHOTS) + x_slot * STRIDE;
        let native: Vec<_> = game.wram()[from..from + STRIDE].to_vec();
        game.wram_mut()[to..to + STRIDE].copy_from_slice(&native);
        let x = i16::from_le_bytes([native[5], native[6]]).wrapping_add(14);
        game.wram_mut()[to + 5..to + 7].copy_from_slice(&x.to_le_bytes());
        game.wram_mut()[ix(ram::X_BASE) + crate::coop::OFF_SHOTS] += 1;
        game.wram_mut()[ix(ram::X_BASE) + 0x63] += 1;
        game.set_extra_pads([0, 0]);
        let mut blast_seen = false;
        let mut damage_slots_seen = false;
        let mut shoulder_checked = false;
        for _ in 0..12 {
            game.step(crate::Pads::default());
            assert_eq!(game.extra_coop[1].p2_shots.count_ones(), 1, "one Vile pulse spawned a flurry");
            for slot in 0..8 {
                let at = ix(SHOTS) + slot * STRIDE;
                if game.extra_coop[1].p2_shots >> slot & 1 == 0 || game.wram()[at] == 0 { continue; }
                assert_eq!(game.wram()[at + 0x0a], LARGE_CHARGE_ID, "Vile did not use the natural maximum charged shot");
                assert_eq!(game.wram()[at + 0x16], LARGE_CHARGE_SET, "Vile did not initialize the natural blue32px graphic set");
                let box_pointer = u16::from_le_bytes([game.wram()[at + 0x20], game.wram()[at + 0x21]]);
                let half_w = game.rom_read(0x860000 | u32::from(box_pointer.wrapping_add(2)));
                let half_h = game.rom_read(0x860000 | u32::from(box_pointer.wrapping_add(3)));
                assert!(half_w >= 9 && half_h >= 11, "native full-charge blast retained pellet collision: {half_w} {half_h}");
                let word = |a| i32::from(u16::from_le_bytes([game.wram()[a], game.wram()[a + 1]]));
                let x = word(at + 5) - word(ix(ram::CAMERA_X));
                let y = word(at + 8) - word(ix(ram::CAMERA_Y));
                let (pixels, width, height) = game.frame();
                for sy in (y - 20).max(0)..(y + 20).min(height as i32) {
                    for sx in (x - 32).max(0)..(x + 32).min(width as i32) {
                        let c = pixels[sy as usize * width + sx as usize];
                        let (r,g,b) = (c >> 16 & 255, c >> 8 & 255, c & 255);
                        blast_seen |= r > 160 && g > 100 && b < 80;
                    }
                }
            }
            if !shoulder_checked {
                for slot in 0..8 {
                    let at = ix(SHOTS) + slot * STRIDE;
                    if game.extra_coop[1].p2_shots >> slot & 1 == 0 || game.wram()[at] == 0 { continue; }
                    let w = game.wram();
                    let sy = i16::from_le_bytes([w[at + 8], w[at + 9]]);
                    let block = &game.extra_coop[1].p2;
                    let py = i16::from_le_bytes([block[8], block[9]]);
                    if sy == 0 { continue; } // ROM hasn't initialized its first update yet.
                    assert!(sy.wrapping_sub(py) <= -10, "rocket did not launch above arm cannon: y delta {}", sy.wrapping_sub(py));
                    shoulder_checked = true;
                }
            }
            if crate::saber::slash_frame(&game.extra_coop[0].saber).is_some() {
                // The physics hook removes melee objects after enemies have
                // processed them; exercise both occupied damage slots here.
                if game.coop.saber.slot == 0xff && game.extra_coop[0].saber.slot == 0xff {
                    crate::saber::shots_done(&mut game);
                }
                assert_ne!(game.coop.saber.slot, 0xff);
                assert_ne!(game.extra_coop[0].saber.slot, 0xff);
                assert_ne!(game.coop.saber.slot, game.extra_coop[0].saber.slot);
                // Simulate the ROM enemy's consumed-shot signal, then prove
                // each player's swing receives its own hit and clears its slot.
                let a = ix(SHOTS) + usize::from(game.coop.saber.slot) * STRIDE;
                let b = ix(SHOTS) + usize::from(game.extra_coop[0].saber.slot) * STRIDE;
                game.wram_mut()[a + 0x30] = 1;
                game.wram_mut()[b + 0x30] = 1;
                crate::saber::take_back(&mut game);
                assert!(game.coop.saber.hit && game.extra_coop[0].saber.hit);
                assert_eq!(game.wram()[a], 0);
                assert_eq!(game.wram()[b], 0);
                damage_slots_seen = true;
                break;
            }
        }
        let (pixels, width, height)=game.frame();
        let mut ppm=format!("P6\n{width} {height}\n255\n").into_bytes();
        for &color in pixels {ppm.extend([(color>>16)as u8,(color>>8)as u8,color as u8]);}
        std::fs::write("/tmp/mmx_vile_large_comparison.ppm",ppm).unwrap();
        assert!(ORIGINAL_X_PIXELS.load(std::sync::atomic::Ordering::Relaxed) > 0, "nearby X charged pixels were not preserved in their native color");
        eprintln!("native charged pixels: {} Vile yellow, {} nearby X unchanged", RECOLORED_PIXELS.load(std::sync::atomic::Ordering::Relaxed), ORIGINAL_X_PIXELS.load(std::sync::atomic::Ordering::Relaxed));
        assert!(RECOLORED_PIXELS.load(std::sync::atomic::Ordering::Relaxed) > 0, "native Vile blast pixels were never recolored");
        assert!(blast_seen, "Vile yellow full-charge blast never rendered");
        assert!(mid_shot_checked, "no initialized Vile shot reached the callback snapshot");
        assert!(shoulder_checked, "Vile rocket never reached its shoulder launch position");
        assert!(damage_slots_seen, "Zero/Sigma never reached their active blade frames");
    }

}
