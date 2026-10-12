//! mmx-bridge: drive one `mmx_core::Game` from another process, one frame at a
//! time, over newline-delimited JSON on stdin/stdout. Lockstep by design: the
//! game never advances unless the driver asks, so the driver owns the clock,
//! the input for every frame is explicit, and a replay is the same request
//! stream again. Useful for scripted playtests and replays.
//!
//!   mmx-bridge --rom PATH [--overclock off|1.5|2] [--timing auto|ntsc|pal]
//!
//! Requests (one per line; `id` is echoed; unknown fields ignored):
//!   {"id":1,"op":"hello"}
//!   {"id":2,"op":"load_state","path":"states/usa11-highway-start.state"}
//!   {"id":3,"op":"save_state","path":"/tmp/x.state"}
//!   {"id":4,"op":"step","p1":256,"p2":0,"frames":1,"hash":false}
//!   {"id":5,"op":"observe"}
//! Every reply is `{"id":..,"ok":true,...}` or `{"id":..,"ok":false,"error":".."}`.
//! `step` applies the same pad word to each of `frames` frames and replies
//! with the observation after the last one. Overclock is a launch setting
//! because `load_state` keeps the running game's overclock (it is not stored in
//! a state), so one bridge process pins one configuration.

use mmx_core::{ram, Game, Overclock, Pads, Timing};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::time::Instant;

#[derive(Deserialize)]
struct Req {
    id: Option<serde_json::Value>,
    op: String,
    path: Option<String>,
    #[serde(default)]
    p1: u16,
    #[serde(default)]
    p2: u16,
    frames: Option<u32>,
    #[serde(default)]
    hash: bool,
}

#[derive(Serialize)]
struct Enemy {
    slot: usize,
    id: u8,
    x: u16,
    y: u16,
    hp: u8,
    contact: u8,
}

/// Typed observation, schema `mmx-obs/0`. Raw units: pixels (u16 positions),
/// signed 1/256-px-per-frame velocities as the game stores them, HP points.
#[derive(Serialize)]
struct Obs {
    schema: &'static str,
    generation: u32,
    frame: u64,
    mode: [u8; 4],
    pause: u8,
    level: u8,
    lives: u8,
    rng: u8,
    camera: [u16; 2],
    x: Player,
    enemies: Vec<Enemy>,
    terrain: Terrain,
    pad1_held: u16,
    checksum: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    wram_sha256: Option<String>,
}

/// Terrain around X, decoded from the live screen map (`$7E:2000`), the
/// screen table (`$7E:E800`) and the stage's per-block collision bytes in ROM
/// (long pointer at `$0B92`), the same lookup the ROM's terrain phase uses
/// (`$84:90B3`/`$84:916A`; see docs/coop.md "Walls"). Objects (cars, moving
/// platforms, collapsing chunks already fallen) are not map cells; `on_ground`
/// includes them via the game's own flags. Distances are from X's leading edge
/// (anchor + 8 px) in the facing direction; `null` means none within `scan`.
#[derive(Serialize)]
struct Terrain {
    on_ground: bool,
    wall_left: bool,
    wall_right: bool,
    /// Floor-top y under X's feet (first floor cell at or below anchor y + 17), or null.
    floor_top: Option<i32>,
    /// Pixels ahead to the first column with no floor down to the death line.
    pit_ahead_px: Option<i32>,
    /// Width of that pit in px (columns until floor returns), when one is ahead.
    pit_width_px: Option<i32>,
    /// Topmost floor y in the far-side column (landing height; may be above X's feet), when a pit is ahead.
    pit_far_floor_top: Option<i32>,
    /// Pixels ahead to the first solid cell at body height.
    wall_ahead_px: Option<i32>,
    scan_px: i32,
}

fn coll_class(game: &Game, x: i32, y: i32) -> Option<u8> {
    if !(0..8192).contains(&x) || !(0..8192).contains(&y) {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    let w = game.wram();
    let screen = w[0xE800 + (y >> 8) * 32 + (x >> 8)] as usize;
    let o = 0x2000 + screen * 0x200 + ((y >> 4) & 15) * 32 + ((x >> 4) & 15) * 2;
    let map = u16::from_le_bytes([w[o], w[o + 1]]) as u32;
    let p = u32::from_le_bytes([w[0x0B92], w[0x0B93], w[0x0B94], 0]);
    Some(game.rom_read(p + map) & 0x3F)
}
fn is_solid(c: u8) -> bool { c >= 0x33 }
fn is_floor(c: u8) -> bool { is_solid(c) || (0x01..=0x0E).contains(&c) || c == 0x11 || c == 0x13 }

/// First floor-top y at column `x` from `y0` down to `y_max` (16-px rows), or None.
fn floor_top_at(game: &Game, x: i32, y0: i32, y_max: i32) -> Option<i32> {
    let mut y = y0;
    while y <= y_max {
        if coll_class(game, x, y).is_some_and(is_floor) {
            return Some(y & !15);
        }
        y += 16;
    }
    None
}

/// `ahead` is toward the stage goal (+x on the Highway), not X's facing: after a
/// hit X faces the attacker, and a predicate that followed the facing reported a
/// pit behind him as "ahead".
fn terrain(game: &Game, px: i32, py: i32, _facing_right: bool, cam_y: i32) -> Terrain {
    let w = game.wram();
    let flags = w[ram::wram_index(0x7E0BD3)] | w[ram::wram_index(0x7E0BD4)];
    let dir = 1;
    let feet = py + 17;
    // The ROM kills X once Y >= camY + 256; floor below that line cannot save it.
    let death = cam_y + 256 + 16;
    let scan = 128;
    let edge = px + dir * 8;
    let mut pit_ahead = None;
    let mut pit_width = None;
    let mut far_top = None;
    let mut wall_ahead = None;
    let mut d = 0;
    while d <= scan {
        let x = edge + dir * d;
        if wall_ahead.is_none() && d > 0 && [-8, 0, 8].iter().any(|dy| coll_class(game, x, py + dy).is_some_and(is_solid)) {
            wall_ahead = Some(d);
        }
        let top = floor_top_at(game, x, feet, death);
        if pit_ahead.is_none() && top.is_none() {
            pit_ahead = Some(d);
        } else if pit_ahead.is_some() && pit_width.is_none() && top.is_some() {
            pit_width = Some(d - pit_ahead.unwrap());
            // The far side may be HIGHER than X's feet (a ledge): report the topmost
            // floor in that column from well above the head down, not the first below the feet.
            far_top = floor_top_at(game, x, feet - 96, death).or(top);
        }
        d += 4;
    }
    Terrain {
        on_ground: flags & 4 != 0,
        wall_left: flags & 2 != 0,
        wall_right: flags & 1 != 0,
        floor_top: floor_top_at(game, px, feet, death),
        pit_ahead_px: pit_ahead,
        pit_width_px: pit_width,
        pit_far_floor_top: far_top,
        wall_ahead_px: wall_ahead,
        scan_px: scan,
    }
}

#[derive(Serialize)]
struct Player {
    x: u16,
    y: u16,
    vx: i16,
    vy: i16,
    hp: u8,
    max_hp: u8,
    action: u8,
    facing_right: bool,
    iframes: u8,
    weapon: u8,
    charge: u8,
}

fn u8_at(w: &[u8], a: u32) -> u8 {
    w[ram::wram_index(a)]
}
fn u16_at(w: &[u8], a: u32) -> u16 {
    u16::from_le_bytes([w[ram::wram_index(a)], w[ram::wram_index(a) + 1]])
}

fn observe(game: &Game, generation: u32, hash: bool) -> Obs {
    let w = game.wram();
    let mut enemies = Vec::new();
    for slot in 0..ram::ENEMY_COUNT {
        let base = ram::wram_index(ram::ENEMIES_BASE) + slot * ram::ENEMY_STRIDE;
        if w[base + ram::ENEMY_ACTIVE] == 0 {
            continue;
        }
        enemies.push(Enemy {
            slot,
            id: w[base + ram::ENEMY_ID],
            x: u16::from_le_bytes([w[base + ram::ENEMY_POS_X], w[base + ram::ENEMY_POS_X + 1]]),
            y: u16::from_le_bytes([w[base + ram::ENEMY_POS_Y], w[base + ram::ENEMY_POS_Y + 1]]),
            hp: w[base + ram::ENEMY_HP],
            contact: w[base + ram::ENEMY_CONTACT_DAMAGE],
        });
    }
    Obs {
        schema: "mmx-obs/2",
        generation,
        frame: game.frame_count(),
        mode: [u8_at(w, ram::MODE_D1), u8_at(w, ram::MODE_D2), u8_at(w, ram::STAGE_PHASE), u8_at(w, ram::MODE_D4)],
        pause: u8_at(w, ram::PAUSE),
        level: u8_at(w, ram::LEVEL_ID),
        lives: u8_at(w, ram::LIVES),
        rng: u8_at(w, ram::RNG),
        camera: [u16_at(w, ram::CAMERA_X), u16_at(w, ram::CAMERA_Y)],
        x: Player {
            x: u16_at(w, ram::X_POS_X),
            y: u16_at(w, ram::X_POS_Y),
            vx: u16_at(w, ram::X_VEL_X) as i16,
            vy: u16_at(w, ram::X_VEL_Y) as i16,
            hp: u8_at(w, ram::X_HP) & 0x7F,
            max_hp: u8_at(w, ram::MAX_HP),
            action: u8_at(w, ram::X_ACTION),
            facing_right: u8_at(w, ram::X_FACING) & 0x40 != 0,
            iframes: u8_at(w, ram::X_IFRAMES),
            weapon: u8_at(w, ram::X_WEAPON),
            // Charge level lives at X block +$58 (4 = full); see coop.rs.
            charge: w[ram::wram_index(ram::X_BASE) + 0x58],
        },
        enemies,
        terrain: terrain(game, u16_at(w, ram::X_POS_X) as i32, u16_at(w, ram::X_POS_Y) as i32, u8_at(w, ram::X_FACING) & 0x40 != 0, u16_at(w, ram::CAMERA_Y) as i32),
        pad1_held: u16_at(w, ram::PAD1_HELD),
        checksum: format!("{:016x}", mmx_core::netgame::net_checksum(game)),
        wram_sha256: hash.then(|| format!("{:x}", Sha256::digest(&w[..]))),
    }
}

fn write_png(path: &std::path::Path, px: &[u32], w: usize, h: usize) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in &px[..w * h] {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    writer.write_image_data(&rgb)?;
    Ok(())
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = arg(&args, "--rom")
        .or_else(|| std::env::var("MMX_ROM").ok())
        .unwrap_or_else(|| {
            eprintln!("mmx-bridge: --rom PATH or $MMX_ROM required");
            std::process::exit(2)
        });
    let overclock: Overclock = arg(&args, "--overclock").unwrap_or_else(|| "off".into()).parse().unwrap_or_else(|e| {
        eprintln!("mmx-bridge: --overclock: {e}");
        std::process::exit(2)
    });
    let timing = Timing::parse(&arg(&args, "--timing").unwrap_or_else(|| "auto".into())).unwrap_or_else(|| {
        eprintln!("mmx-bridge: --timing auto|ntsc|pal");
        std::process::exit(2)
    });
    // Development assist: keep X's HP at max after every frame so recording runs
    // reach later sections despite enemies. Reported in `hello` and recorded by the
    // harness manifest as an assist; never used for playtest scores.
    let dev_refill_hp = args.iter().any(|a| a == "--dev-refill-hp");
    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("mmx-bridge: {rom_path}: {e}");
        std::process::exit(2)
    });
    let mut game = Game::from_rom_timing(rom, timing).unwrap_or_else(|e| {
        eprintln!("mmx-bridge: {rom_path}: {e}");
        std::process::exit(2)
    });
    game.set_overclock(overclock);
    // Single player, co-op off, no health assist: ordinary playtest conditions.
    let mut generation: u32 = 0;

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Req = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = writeln!(out, "{}", serde_json::json!({"id": null, "ok": false, "error": format!("bad request: {e}")}));
                let _ = out.flush();
                continue;
            }
        };
        let id = req.id.clone().unwrap_or(serde_json::Value::Null);
        let reply = match req.op.as_str() {
            "hello" => {
                let info = game.rom_info();
                Ok(serde_json::json!({
                    "bridge": "mmx-bridge/0",
                    "obs_schema": "mmx-obs/2",
                    "game": {
                        "title": info.title, "region": info.region_name(), "version": info.version,
                        "sha256": info.sha256.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                        "timing": format!("{:?}", game.timing()), "fps": game.fps(),
                        "overclock": format!("{overclock:?}"), "players": 1, "coop": false,
                    },
                    "assists": {"dev_refill_hp": dev_refill_hp},
                    // What this bridge actually supports. `deterministic_replay` is a
                    // property of the core (no clock, no entropy) checked by the harness.
                    "capabilities": ["observe", "input", "step", "snapshot", "restore", "deterministic_replay"],
                    "buttons": {"B": 1<<15, "Y": 1<<14, "SELECT": 1<<13, "START": 1<<12, "UP": 1<<11, "DOWN": 1<<10, "LEFT": 1<<9, "RIGHT": 1<<8, "A": 1<<7, "X": 1<<6, "L": 1<<5, "R": 1<<4},
                }))
            }
            "load_state" => match req.path.as_deref().ok_or("path required").and_then(|p| std::fs::read(p).map_err(|e| { let _ = e; "read failed" })) {
                Ok(bytes) => game.load_state(&bytes).map(|()| {
                    generation += 1;
                    game.set_overclock(overclock);
                    serde_json::json!({"obs": observe(&game, generation, true)})
                }),
                Err(e) => Err(e.to_string()),
            },
            "save_state" => match req.path.as_deref() {
                Some(p) => std::fs::write(p, game.save_state()).map(|()| serde_json::json!({"frame": game.frame_count()})).map_err(|e| e.to_string()),
                None => Err("path required".into()),
            },
            "step" => {
                let n = req.frames.unwrap_or(1).max(1);
                let t = Instant::now();
                for _ in 0..n {
                    game.step(Pads { p1: req.p1, p2: req.p2 });
                    game.take_audio();
                    if dev_refill_hp {
                        let max = game.wram()[ram::wram_index(ram::MAX_HP)];
                        let hp = &mut game.wram_mut()[ram::wram_index(ram::X_HP)];
                        if *hp & 0x7F < max && *hp & 0x7F > 0 { *hp = max; }
                    }
                }
                let step_us = t.elapsed().as_micros() as u64;
                Ok(serde_json::json!({"applied": {"p1": req.p1, "p2": req.p2, "frames": n}, "step_us": step_us, "obs": observe(&game, generation, req.hash)}))
            }
            "observe" => Ok(serde_json::json!({"obs": observe(&game, generation, req.hash)})),
            // Write the current framebuffer as a PNG (video renders of recorded runs).
            "frame_png" => match req.path.as_deref() {
                Some(p) => {
                    let (px, w, h) = game.frame();
                    write_png(std::path::Path::new(p), px, w, h).map(|()| serde_json::json!({"frame": game.frame_count(), "w": w, "h": h})).map_err(|e| e.to_string())
                }
                None => Err("path required".into()),
            },
            // Debug: collision classes over a rectangle, one row per 16 px, one entry per 8 px.
            "terrain_dump" => {
                let w = game.wram();
                let x0 = req.p1 as i32; let x1 = req.p2 as i32; let y0 = req.frames.unwrap_or(0) as i32;
                let rows: Vec<serde_json::Value> = (0..12).map(|r| {
                    let y = y0 + r * 16;
                    serde_json::json!({"y": y, "classes": (x0..=x1).step_by(8).map(|x| coll_class(&game, x, y).map(|c| format!("{c:02x}")).unwrap_or("--".into())).collect::<Vec<_>>()})
                }).collect();
                let cam_y = u16_at(w, ram::CAMERA_Y);
                let ptr = u32::from_le_bytes([w[0x0B92], w[0x0B93], w[0x0B94], 0]);
                Ok(serde_json::json!({"x0": x0, "x1": x1, "cam_y": cam_y, "coll_ptr": format!("{ptr:06x}"), "rows": rows}))
            }
            other => Err(format!("unknown op {other}")),
        };
        let mut v = match reply {
            Ok(v) => { let mut v = v; v["ok"] = true.into(); v }
            Err(e) => serde_json::json!({"ok": false, "error": e}),
        };
        v["id"] = id;
        if writeln!(out, "{v}").is_err() || out.flush().is_err() {
            break;
        }
    }
}
