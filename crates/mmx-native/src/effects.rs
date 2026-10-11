//! Optional presentation-only lights and charge particles. Reads the current
//! game snapshot; never changes emulation or rollback state.

use mmx_core::{character::Character, ram, Game};

#[derive(Clone, Copy)]
struct Light {
    x: i32,
    y: i32,
    radius: i32,
    color: [i32; 3],
}

#[derive(Clone, Copy)]
struct Charger {
    x: i32,
    y: i32,
    zero: bool,
    level: u8,
}

fn word(bytes: &[u8], at: usize) -> i32 {
    i32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn add_light(
    lights: &mut Vec<Light>,
    x: i32,
    y: i32,
    radius: i32,
    color: [i32; 3],
    w: usize,
    h: usize,
) {
    if x + radius >= 0 && y + radius >= 0 && x - radius < w as i32 && y - radius < h as i32 {
        lights.push(Light {
            x,
            y,
            radius,
            color,
        });
    }
}

/// The ROM uses `$1228..$1428` for eight moving shots from both players and
/// enemies. `$0C98` is the sparkle/armour object table, not projectiles.
fn projectile_lights(
    lights: &mut Vec<Light>,
    ram_bytes: &[u8],
    source: &[u8],
    w: usize,
    h: usize,
    camera_x: i32,
    camera_y: i32,
    margin: i32,
) {
    for slot in 0..ram::ITEM_COUNT {
        let at = ram::wram_index(ram::ITEMS_BASE) + slot * ram::ITEM_STRIDE;
        let obj = &ram_bytes[at..at + ram::ITEM_STRIDE];
        if obj[0] == 0 {
            continue;
        }
        let x = word(obj, 5) - camera_x + margin;
        let y = word(obj, 8) - camera_y;
        // Use the visible shot's colour so X, Zero and enemy projectiles
        // cast different coloured light without assuming an owner or weapon.
        let color = projectile_color(source, w, h, x, y);
        let radius = if obj[0x0a] == 2 { 35 } else { 25 };
        add_light(lights, x, y, radius, color, w, h);
    }
}

fn projectile_color(source: &[u8], w: usize, h: usize, x: i32, y: i32) -> [i32; 3] {
    let mut picked = [100, 145, 190];
    let mut best = 0;
    for dy in [-3, 0, 3] {
        for dx in [-3, 0, 3] {
            let (sx, sy) = (x + dx, y + dy);
            if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 {
                continue;
            }
            let at = (sy as usize * w + sx as usize) * 4;
            let rgb = [
                i32::from(source[at]),
                i32::from(source[at + 1]),
                i32::from(source[at + 2]),
            ];
            let (lo, hi) = (*rgb.iter().min().unwrap(), *rgb.iter().max().unwrap());
            let score = hi - lo + hi / 5;
            if score > best {
                best = score;
                picked = rgb;
            }
        }
    }
    // Baseline light makes a dim or white projectile visible as well.
    [45 + picked[0] / 4, 45 + picked[1] / 4, 45 + picked[2] / 4]
}

/// Apply an ambient RGB multiply map and radial lights, then draw additive
/// charge halos and moving sparks into an RGBA frame at native resolution.
pub fn apply(
    frame: &mut [u8],
    w: usize,
    h: usize,
    game: &Game,
    dynamic_lighting: bool,
    charge_effects: bool,
) {
    if (!dynamic_lighting && !charge_effects) || frame.len() < w.saturating_mul(h).saturating_mul(4)
    {
        return;
    }
    let ram = game.wram();
    if ram[ram::wram_index(ram::MODE_D2)] != ram::D2_IN_STAGE
        || ram[ram::wram_index(ram::STAGE_PHASE)] != ram::PHASE_GAMEPLAY
    {
        return;
    }
    let camera_x = word(ram, ram::wram_index(ram::CAMERA_X));
    let camera_y = word(ram, ram::wram_index(ram::CAMERA_Y));
    let margin = (w as i32 - 256) / 2;
    let mut lights = Vec::with_capacity(18);
    let mut chargers = Vec::with_capacity(4);
    let mut player = |obj: &[u8], character: Character| {
        // The ROM's live player object has zero in byte 0; HP indicates
        // whether this participant can display effects.
        if obj[0x27] & 0x7f == 0 {
            return;
        }
        let x = word(obj, 5) - camera_x + margin;
        let y = word(obj, 8) - camera_y - 13;
        let zero = character == Character::Zero;
        if dynamic_lighting {
            add_light(
                &mut lights,
                x,
                y,
                56,
                if zero { [52, 32, 42] } else { [30, 52, 72] },
                w,
                h,
            );
        }
        if charge_effects && matches!(character, Character::X | Character::Zero) && obj[0x58] == 4 {
            chargers.push(Charger {
                x,
                y,
                zero,
                level: obj[0x5b],
            });
        }
    };
    let x = ram::wram_index(ram::X_BASE);
    player(
        &ram[x..x + ram::X_BLOCK_LEN],
        game.player_character(0).unwrap_or(Character::X),
    );
    if let Some(p2) = game.visible_p2_object() {
        player(p2, game.player_character(1).unwrap_or(Character::Zero));
    }
    for extra in 0..2 {
        if let Some(obj) = game.visible_extra_player_object(extra) {
            player(
                obj,
                game.player_character(extra + 2).unwrap_or(Character::X),
            );
        }
    }

    if dynamic_lighting {
        projectile_lights(&mut lights, ram, frame, w, h, camera_x, camera_y, margin);
        for (i, pixel) in frame.chunks_exact_mut(4).take(w * h).enumerate() {
            let x = (i % w) as i32;
            let y = (i / w) as i32;
            let mut mul = [188, 195, 211];
            for light in &lights {
                let dx = x - light.x;
                let dy = y - light.y;
                let r2 = light.radius * light.radius;
                let d2 = dx * dx + dy * dy;
                if d2 < r2 {
                    let falloff = (r2 - d2) * 256 / r2;
                    for c in 0..3 {
                        mul[c] += light.color[c] * falloff / 256;
                    }
                }
            }
            for c in 0..3 {
                pixel[c] = ((i32::from(pixel[c]) * mul[c].clamp(0, 255)) / 255) as u8;
            }
        }
    }
    if charge_effects {
        for charger in chargers {
            draw_charge(frame, w, h, charger, game.frame_count());
        }
    }
}

fn glow(
    frame: &mut [u8],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    radius: i32,
    color: [u8; 3],
    strength: i32,
) {
    let left = (x - radius).max(0);
    let right = (x + radius).min(w as i32 - 1);
    let top = (y - radius).max(0);
    let bottom = (y + radius).min(h as i32 - 1);
    let r2 = radius * radius;
    for py in top..=bottom {
        for px in left..=right {
            let d2 = (px - x).pow(2) + (py - y).pow(2);
            if d2 >= r2 {
                continue;
            }
            let amount = (r2 - d2) * strength / r2;
            let at = (py as usize * w + px as usize) * 4;
            for c in 0..3 {
                let added = amount * i32::from(color[c]) / 255;
                frame[at + c] = frame[at + c].saturating_add(added.clamp(0, 255) as u8);
            }
        }
    }
}

fn draw_charge(frame: &mut [u8], w: usize, h: usize, charger: Charger, tick: u64) {
    let [x, y] = [charger.x, charger.y];
    let color = if charger.zero {
        [255, 105, 220]
    } else {
        [75, 205, 255]
    };
    let pulse = (tick % 20) as i32;
    let strength = 46
        + if pulse < 10 {
            pulse * 2
        } else {
            (20 - pulse) * 2
        };
    glow(
        frame,
        w,
        h,
        x,
        y,
        30 + i32::from(charger.level > 0) * 7,
        color,
        strength,
    );
    // Deterministic orbiting particles: they track the emulated frame, so
    // pauses, frame stepping and rollback never accumulate stray particles.
    for n in 0..12_u64 {
        let phase = ((tick * 3 + n * 47) % 256) as f32 * std::f32::consts::TAU / 256.0;
        let radius = 8.0 + ((tick + n * 17) % 21) as f32;
        let sx = x + (phase.cos() * radius) as i32;
        let sy = y + (phase.sin() * radius * 0.75) as i32;
        glow(frame, w, h, sx, sy, 3, color, 105);
        if sx >= 0 && sy >= 0 && sx < w as i32 && sy < h as i32 {
            let at = (sy as usize * w + sx as usize) * 4;
            frame[at..at + 3].copy_from_slice(&[240, 245, 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_real_projectile_table() {
        let mut ram_bytes = vec![0; 0x20000];
        // A charge sparkle at $0C98 is not a projectile.
        ram_bytes[ram::wram_index(ram::SHOTS_BASE)] = 1;
        let shot = ram::wram_index(ram::ITEMS_BASE);
        ram_bytes[shot] = 1;
        ram_bytes[shot + 5..shot + 7].copy_from_slice(&120_u16.to_le_bytes());
        ram_bytes[shot + 8..shot + 10].copy_from_slice(&80_u16.to_le_bytes());
        let mut lights = Vec::new();
        let frame = vec![0; 256 * 224 * 4];
        projectile_lights(&mut lights, &ram_bytes, &frame, 256, 224, 20, 10, 0);
        assert_eq!(lights.len(), 1);
        assert_eq!((lights[0].x, lights[0].y), (100, 70));
        ram_bytes[shot] = 0;
        lights.clear();
        projectile_lights(&mut lights, &ram_bytes, &frame, 256, 224, 20, 10, 0);
        assert!(lights.is_empty());
    }
}
