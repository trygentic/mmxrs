//! Independent power-on acceptance test: real four-player ROM simulation,
//! source-movie controller and the actual HD pack, with no survival assist.
use mmx_core::{ram::{self, wram_index as ix}, Character, Game, Pads};

const ROSTER: [Character; 4] = [Character::X, Character::Zero, Character::Sigma, Character::Vile];
// base, state1, state2, subtype (+$0B), x, y, optional native contact box
type HealthPickup = (usize, u8, u8, u8, u16, u16, Option<[i32; 4]>);

// Preserve the objects from the frame that actually caused the failure.
// Native death teardown can clear the pools before the post-frame audit.
struct PreFrame {
    actors: [Option<[u16; 5]>; 4],
    native_damage: u8,
    gameplay: bool,
    health_pickups: Vec<HealthPickup>,
    // pool, base, state1, state2, id, hp, damage, x, y, optional native box
    objects: Vec<(&'static str, usize, u8, u8, u8, u8, u8, u16, u16, Option<[i32; 4]>)>,
}

impl PreFrame {
    fn capture(game: &Game) -> Self {
        let w = game.wram();
        let ambient_base = ix(ram::PICKUPS_BASE) + ram::ITEM_COUNT * ram::ITEM_STRIDE;
        let ambient_stride = 0x30;
        let ambient_count = (ix(ram::EFFECTS_BASE) - ambient_base) / ambient_stride;
        let pools = [
            ("enemies", ix(ram::ENEMIES_BASE), ram::ENEMY_COUNT, ram::ENEMY_STRIDE, true),
            ("enemy_shots", ix(ram::PICKUPS_BASE), ram::ITEM_COUNT, ram::ITEM_STRIDE, true),
            ("ambient_pickups_road", ambient_base, ambient_count, ambient_stride, false),
        ];
        let mut objects = Vec::new();
        let mut health_pickups = Vec::new();
        for (pool, first, count, stride, actor_layout) in pools {
            for slot in 0..count {
                let at = first + slot * stride;
                if w[at] == 0 { continue; }
                let word = |offset| u16::from_le_bytes([w[at + offset], w[at + offset + 1]]);
                let pointer = word(0x20);
                let valid_box = actor_layout && pointer != 0 && (pointer < 0x2000 || pointer >= 0x8000);
                if !actor_layout && w[at + 0x0a] == 2 {
                    // Native health pickups carry the common contact-box
                    // pointer even though their pool stride is only $30.
                    let bounds = (pointer != 0 && (pointer < 0x2000 || pointer >= 0x8000))
                        .then(|| game.object_contact_bounds(at as u16)).flatten();
                    health_pickups.push((at, w[at + 1], w[at + 2], w[at + 0x0b], word(5), word(8), bounds));
                }
                objects.push((pool, at, w[at + 1], w[at + 2], w[at + 0x0a],
                    w[at + 0x27] & 0x7f, w[at + 0x26], word(5), word(8),
                    valid_box.then(|| game.object_contact_bounds(at as u16)).flatten()));
            }
        }
        Self {
            actors: std::array::from_fn(|slot| game.player_physics_state(slot)),
            native_damage: w[ix(ram::X_BASE) + 0x26],
            gameplay: w[ix(ram::MODE_D2)] == ram::D2_IN_STAGE
                && w[ix(ram::STAGE_PHASE)] == ram::PHASE_GAMEPLAY
                && !mmx_core::coop_stages::p1_scripted(w),
            health_pickups,
            objects,
        }
    }

    fn trace_native_healing(&self, game: &Game, seen: [bool; 4]) {
        let w = game.wram();
        if !self.gameplay || w[ix(ram::MODE_D2)] != ram::D2_IN_STAGE
            || w[ix(ram::STAGE_PHASE)] != ram::PHASE_GAMEPLAY
            || mmx_core::coop_stages::p1_scripted(w) { return; }
        let post_actors: [Option<[u16; 5]>; 4] = std::array::from_fn(|slot| game.player_physics_state(slot));
        let healed: Vec<_> = (0..4).filter_map(|slot| {
            let (before, after) = (self.actors[slot]?, post_actors[slot]?);
            (seen[slot] && before[2] > 0 && after[2] > before[2])
                .then_some((slot + 1, before[2], after[2]))
        }).collect();
        if healed.is_empty() { return; }
        let post = Self::capture(game);
        eprintln!("HEAL_NATIVE frame{} increases[P,before_hp,after_hp]={healed:?} actors_pre={:?} actors_post={post_actors:?} PRE_active_health_ID2[base,state1,state2,subtype_0B,x,y,bounds]={:?} POST_active_health_ID2={:?}",
            game.frame_count(), self.actors, self.health_pickups, post.health_pickups);
    }
}

fn compose_recording_frame(game: &Game) -> Vec<u8> {
    let (pixels, width, height) = game.frame();
    let w = game.wram();
    if w[ix(ram::MODE_D2)] == ram::D2_IN_STAGE && w[ix(ram::STAGE_PHASE)] == ram::PHASE_GAMEPLAY {
        assert_eq!((width, height, game.frame_scale()), (1600, 896, 4), "gameplay must use the actual new HD pack");
    } else {
        assert!(matches!((width,height,game.frame_scale()), (1600,896,4)|(400,224,1)), "unexpected native presentation size");
    }
    let mut rgba: Vec<u8> = pixels.iter().take(width * height).flat_map(|p| [
        (p >> 16) as u8, (p >> 8) as u8, *p as u8, 255,
    ]).collect();
    crate::effects::apply(&mut rgba, width, height, game, true, true);
    game.draw_ui_rgba(&mut rgba, width, height);
    if (width,height)==(1600,896) { return rgba; }
    // Inactive scenes deliberately retain the native surface. Normalize the
    // composed image with exact nearest4x sampling; keep gameplay strict.
    let mut normalized=vec![0u8;1600*896*4];
    for y in 0..896 {
        for x in 0..1600 {
            let source=((y/4)*width+x/4)*4;
            let target=(y*1600+x)*4;
            normalized[target..target+4].copy_from_slice(&rgba[source..source+4]);
        }
    }
    normalized
}

// Optional export consumes the same audited simulation, not a separate replay.
struct CaptureExport {
    directory: std::path::PathBuf,
    fps: f64,
    child: Option<std::process::Child>,
    stdin: Option<std::process::ChildStdin>,
    audio: Option<std::io::BufWriter<std::fs::File>>,
    first_frame: Option<u64>,
    last_frame: Option<u64>,
    video_frames: u64,
    audio_samples: u64,
    finished: bool,
    clear_state_saved: bool,
}

impl CaptureExport {
    fn new(directory: std::path::PathBuf, fps: f64) -> std::io::Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let audio = std::io::BufWriter::new(std::fs::File::create(directory.join("game-audio.f32le"))?);
        Ok(Self { directory, fps, child: None, stdin: None, audio: Some(audio),
            first_frame: None, last_frame: None, video_frames: 0, audio_samples: 0,
            finished: false, clear_state_saved: false })
    }

    fn start_video(&mut self) -> std::io::Result<()> {
        let log = std::fs::File::create(self.directory.join("ffmpeg-video.log"))?;
        let mut child = std::process::Command::new("/opt/homebrew/bin/ffmpeg")
            .args(["-hide_banner", "-loglevel", "warning", "-nostdin", "-y",
                "-f", "rawvideo", "-pixel_format", "rgba", "-video_size", "1600x896", "-framerate"])
            .arg(format!("{:.12}", self.fps))
            .args(["-i", "pipe:0", "-an", "-vf",
                "scale=1920:1076:flags=neighbor,pad=1920:1080:0:2:black",
                "-r", "60", "-c:v", "h264_videotoolbox", "-allow_sw", "1",
                "-b:v", "16M", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(self.directory.join("game-video.mp4"))
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(log)).spawn()?;
        self.stdin = child.stdin.take();
        self.child = Some(child);
        Ok(())
    }

    fn frame(&mut self, game: &Game, samples: &[(f32, f32)], audit: &IntroAudit) -> std::io::Result<()> {
        use std::io::Write;
        if self.finished { return Ok(()); }
        let w = game.wram();
        if self.first_frame.is_none() {
            if w[ix(ram::MODE_D2)] != ram::D2_IN_STAGE || w[ix(ram::STAGE_PHASE)] != ram::PHASE_GAMEPLAY {
                return Ok(());
            }
            self.start_video()?;
            self.first_frame = Some(game.frame_count());
            eprintln!("CAPTURE_START frame{} native_fps={:.12} audio=48000 stereo game-only", game.frame_count(), self.fps);
        }
        let rgba = compose_recording_frame(game);
        self.stdin.as_mut().unwrap().write_all(&rgba)?;
        let mut pcm = Vec::with_capacity(samples.len() * 8);
        for &(left, right) in samples {
            pcm.extend_from_slice(&left.to_le_bytes());
            pcm.extend_from_slice(&right.to_le_bytes());
        }
        self.audio.as_mut().unwrap().write_all(&pcm)?;
        self.video_frames += 1;
        self.audio_samples += samples.len() as u64;
        self.last_frame = Some(game.frame_count());
        if !self.clear_state_saved && w[ix(ram::MODE_D2)] == ram::D2_IN_STAGE && w[ix(ram::STAGE_PHASE)] == 8 {
            std::fs::write("/tmp/mmx_verified_clear.state", game.save_state())?;
            self.clear_state_saved = true;
            eprintln!("CAPTURE_CLEAR_STATE frame{} /tmp/mmx_verified_clear.state", game.frame_count());
        }
        // Include beam-out and the fully presented native stage-select grid.
        if audit.stage_select_frames >= 90 {
            self.finish(audit, true)?;
        }
        Ok(())
    }

    fn metadata(&self, audit: &IntroAudit, stage_select: bool) -> std::io::Result<()> {
        let value = serde_json::json!({
            "video": "full-coop.mp4", "source": "actual audited Game frames and SPC audio",
            "roster": ["X", "Zero", "Sigma", "Vile"], "lighting": true, "charge_effects": true,
            "system_microphone": false, "audio_sample_rate": 48000, "audio_channels": 2,
            "source_fps": self.fps, "output_fps": 60, "output_size": [1920,1080],
            "first_native_frame": self.first_frame, "last_native_frame": self.last_frame,
            "source_video_frames": self.video_frames, "audio_sample_frames": self.audio_samples,
            "video_duration_seconds": self.video_frames as f64/self.fps,
            "audio_duration_seconds": self.audio_samples as f64/48000.0,
            "native_capture_seen": audit.capture_seen, "native_clear_seen": audit.clear_seen,
            "hd_gameplay_verified": audit.saw_hd_gameplay,
            "stage_select_verified": stage_select, "audit_deaths": [0,0,0,0], "audit_warps": [0,0,0,0],
            "minimum_hp": audit.minimum_hp, "furthest_x": audit.furthest_x,
            "clear_state": "/tmp/mmx_verified_clear.state"
        });
        std::fs::write(self.directory.join("capture-metadata.json"), serde_json::to_vec_pretty(&value)?)
    }

    fn finish(&mut self, audit: &IntroAudit, stage_select: bool) -> std::io::Result<()> {
        use std::io::Write;
        if self.finished { return self.metadata(audit, stage_select); }
        // EOF flushes the encoder; close PCM before the mux process reads it.
        self.stdin.take();
        if let Some(mut audio) = self.audio.take() { audio.flush()?; }
        let status = self.child.as_mut().unwrap().wait()?;
        self.child.take();
        if !status.success() { return Err(std::io::Error::other(format!("video encoder failed: {status}"))); }
        let log = std::fs::File::create(self.directory.join("ffmpeg-mux.log"))?;
        let status = std::process::Command::new("/opt/homebrew/bin/ffmpeg")
            .args(["-hide_banner", "-loglevel", "warning", "-nostdin", "-y", "-i"])
            .arg(self.directory.join("game-video.mp4"))
            .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(self.directory.join("game-audio.f32le"))
            .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart"])
            .arg(self.directory.join("full-coop.mp4"))
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::from(log)).status()?;
        if !status.success() { return Err(std::io::Error::other(format!("audio mux failed: {status}"))); }
        self.finished = true;
        self.metadata(audit, stage_select)?;
        eprintln!("CAPTURE_FINISHED frames={} native_seconds={:.3} audio_seconds={:.3} file={}",
            self.video_frames, self.video_frames as f64/self.fps, self.audio_samples as f64/48000.0,
            self.directory.join("full-coop.mp4").display());
        Ok(())
    }
}

impl Drop for CaptureExport {
    fn drop(&mut self) {
        self.stdin.take();
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Default)]
struct IntroAudit {
    seen: [bool; 4],
    previous: [Option<[u16; 2]>; 4],
    minimum_hp: [Option<u16>; 4],
    furthest_x: [u16; 4],
    capture_seen: bool,
    clear_seen: bool,
    saw_hd_gameplay: bool,
    stage_select_frames: usize,
    last_mode: Option<(u8, u8)>,
    level: Option<u8>,
}

impl IntroAudit {
    fn observe(&mut self, game: &Game) -> Result<bool, String> {
        let w = game.wram();
        let (mode, phase) = (w[ix(ram::MODE_D2)], w[ix(ram::STAGE_PHASE)]);
        let states: [Option<[u16; 5]>; 4] = std::array::from_fn(|slot| game.player_physics_state(slot));
        let capture = mode == ram::D2_IN_STAGE && phase == ram::PHASE_GAMEPLAY
            && states[0].is_some_and(|s| matches!(s[3], 0x2e | 0x30 | 0x32));
        self.capture_seen |= capture;
        if mode == ram::D2_IN_STAGE && self.seen[0]
            && self.level.is_some_and(|level| level != w[ix(ram::LEVEL_ID)]) {
            return Err(format!("changed stage before intro completion: {:?}->{}, frame{}", self.level, w[ix(ram::LEVEL_ID)], game.frame_count()));
        }
        if self.last_mode != Some((mode, phase)) {
            eprintln!("full-intro frame{} mode={mode:02x} phase={phase:02x} actors={states:?}", game.frame_count());
            self.last_mode = Some((mode, phase));
        }
        if mode == 0x0a || (mode == ram::D2_IN_STAGE && phase == 6) {
            return Err(format!("native death/game-over at frame{}: mode={mode:02x}, phase={phase:02x}, actors={states:?}", game.frame_count()));
        }
        if mode == ram::D2_IN_STAGE && phase == ram::PHASE_GAMEPLAY {
            let (_,width,height)=game.frame();
            if (width,height,game.frame_scale())!=(1600,896,4) {
                return Err(format!("gameplay lost the HD pack at frame{}: size({width},{height}) scale{}",game.frame_count(),game.frame_scale()));
            }
            self.saw_hd_gameplay=true;
            let visible = [true, game.visible_p2_object().is_some(),
                game.visible_extra_player_object(0).is_some(), game.visible_extra_player_object(1).is_some()];
            let scripted = mmx_core::coop_stages::p1_scripted(w);
            for slot in 0..4 {
                let Some(s) = states[slot] else { continue; };
                // At power-on the player blocks are not initialized yet.
                if !self.seen[slot] {
                    if !visible[slot] || s[2] == 0 { continue; }
                    if slot == 0 && !mmx_core::coop_stages::p2_anchor_ok(w) { continue; }
                    self.seen[slot] = true;
                    if slot == 0 { self.level = Some(w[ix(ram::LEVEL_ID)]); }
                }
                // Vile's required story capture is not a player death. Its
                // capture action plus gameplay phase distinguishes it from
                // HP0/action$0C/pit-death and the respawn phase.
                if s[3] == 0x0c || (s[2] == 0 && !(slot == 0 && self.capture_seen && scripted)) {
                    return Err(format!("P{} died at frame{}: raw={s:?}", slot + 1, game.frame_count()));
                }
                if slot > 0 && !visible[slot] && !scripted {
                    return Err(format!("P{} disappeared/downed at frame{}: raw={s:?}", slot + 1, game.frame_count()));
                }
                if visible[slot] && (slot > 0 || !scripted) {
                    let at = [s[0], s[1]];
                    if let Some(before) = self.previous[slot] {
                        if before[0].abs_diff(at[0]) > 64 || before[1].abs_diff(at[1]) > 64 {
                            return Err(format!("P{} warped at frame{}: {before:?}->{at:?}", slot + 1, game.frame_count()));
                        }
                    }
                    self.previous[slot] = Some(at);
                } else if slot == 0 {
                    self.previous[slot] = None; // Native capture/beam-out motion.
                }
                if s[2] > 0 {
                    self.minimum_hp[slot] = Some(self.minimum_hp[slot].map_or(s[2], |hp| hp.min(s[2])));
                }
                self.furthest_x[slot] = self.furthest_x[slot].max(s[0]);
            }
        }
        self.clear_seen |= mode == ram::D2_IN_STAGE && phase == 8 && self.capture_seen;
        // Rev 1 $80:BCBB is the stage-select grid in MODE0/phase1.
        // MODE2 is the following stage load after a Y/B confirmation.
        if mode == 0 && phase == 1 && self.clear_seen {
            self.stage_select_frames += 1;
        } else { self.stage_select_frames = 0; }
        if self.stage_select_frames >= 90 {
            if !self.seen.iter().all(|seen| *seen) {
                return Err(format!("stage select reached without all four players: {:?}", self.seen));
            }
            if self.furthest_x.iter().any(|x| *x < 7000) {
                return Err(format!("stage select reached with a player missing the final road: {:?}", self.furthest_x));
            }
            return Ok(true);
        }
        Ok(false)
    }
}

#[test]
fn real_rom_four_player_power_on_intro_finishes_without_death_or_warp() {
    let Some(rom_path) = std::env::var_os("MMX_ROM").map(std::path::PathBuf::from) else {
        return eprintln!("MMX_ROM not set; skipping full intro acceptance test");
    };
    let root = rom_path.parent().unwrap().parent().unwrap();
    let movie_path = std::env::var_os("MMX_MOVIE").map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("movies/hetfield90v3-megamanx-100p.bk2"));
    let pack_path = std::env::var_os("MMX_BONE_SHEETS").map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("roms/derived/bone_sheets"));
    let args = crate::cli::parse([
        "--rom".to_owned(), rom_path.to_string_lossy().into_owned(),
        "--movie".into(), movie_path.to_string_lossy().into_owned(),
        "--movie-smart".into(), "--movie-coop-delay".into(), "8".into(),
        "--widescreen".into(), "16:9".into(),
        "--dynamic-lighting".into(), "--charge-effects".into(),
        "--p1-character".into(), "x".into(), "--p2-character".into(), "zero".into(),
        "--p3-character".into(), "sigma".into(), "--p4-character".into(), "vile".into(),
    ], None).unwrap();
    assert!(!args.demo_survival, "acceptance run must not heal or accelerate revival");
    assert!(!args.wide_gameplay, "recording uses presentation-only widescreen");
    let mut controller = crate::movie_route::RoutePlayback::load(&args).unwrap();
    let bytes = std::fs::read(&rom_path).unwrap();
    let mut game = Game::from_rom_timing(bytes.clone(), args.timing).unwrap();
    game.coop.enabled = true;
    game.set_local_players(4);
    mmx_core::coop::register_coop_traps(&mut game).unwrap();
    for (slot, character) in ROSTER.into_iter().enumerate() {
        game.set_player_character(slot, character);
        if character != Character::X {
            game.set_boss_set(character, mmx_core::boss_data::extract_x1(&bytes, character).unwrap());
        }
    }
    crate::bone_sheets::install(&mut game, &pack_path, &ROSTER).unwrap();
    game.set_allow_opposing_directions(true);
    game.set_widescreen(args.widescreen.initial());
    game.set_overclock(args.overclock());
    let mut capture = std::env::var_os("MMX_CAPTURE_DIR").map(|dir| {
        game.set_audio_rate(48000);
        CaptureExport::new(std::path::PathBuf::from(dir), game.fps()).unwrap()
    });
    let mut audit = IntroAudit::default();
    let max_frames = std::env::var("MMX_ACCEPTANCE_MAX_FRAMES").ok()
        .and_then(|v| v.parse::<usize>().ok()).unwrap_or(36_000);
    for frame in 0..max_frames {
        let inputs = controller.next(&game);
        game.set_extra_pads([inputs[2], inputs[3]]);
        let pre_frame = PreFrame::capture(&game);
        if std::env::var_os("MMX_NPC_TRACE").is_some() && frame % 4 == 0 {
            let w = game.wram();
            if pre_frame.actors[0].is_some_and(|p| p[0] >= 7200)
                && !pre_frame.objects.iter().any(|o| o.0 == "enemies" && o.4 == 0x32) {
                eprintln!("ENDING_TRACE frame{} pads={inputs:x?} actors={:?} objects={:?}",
                    game.frame_count(), pre_frame.actors, pre_frame.objects);
                eprintln!("ENDING_FACING {:?}", [Some(w[ix(ram::X_FACING)]), game.visible_p2_object().map(|o| o[0x11]), game.visible_extra_player_object(0).map(|o| o[0x11]), game.visible_extra_player_object(1).map(|o| o[0x11])]);
            }
            for npc in pre_frame.objects.iter().filter(|o| o.0 == "enemies" && o.4 == 0x32) {
                let base = npc.1;
                eprintln!("NPC_TRACE frame{} pads={inputs:x?} actors={:?} npc={npc:?} states={:?} anim={:02x} facing={:02x} capture_flags={:02x}",
                    game.frame_count(), pre_frame.actors, &w[base + 1..base + 4],
                    w[base + 0x0f], w[base + 0x11], w[0x0c2f]);
            }
        }
        game.step(Pads { p1: inputs[0], p2: inputs[1] });
        let audio = game.take_audio();
        pre_frame.trace_native_healing(&game, audit.seen);
        let complete = audit.observe(&game).unwrap_or_else(|error| {
            eprintln!("full-intro pre-frame actors={:?} native_damage[$0BCE]={} active_objects[pool,base,state1,state2,id,hp,damage,x,y,bounds]={:?}", pre_frame.actors, pre_frame.native_damage, pre_frame.objects);
            let w = game.wram();
            let actors: Vec<_> = (0..4).map(|slot| game.player_physics_state(slot)).collect();
            let enemies: Vec<_> = (0..ram::ENEMY_COUNT).filter_map(|slot| {
                let at = ix(ram::ENEMIES_BASE) + slot * ram::ENEMY_STRIDE;
                (w[at] != 0).then(|| (at, w[at + ram::ENEMY_ID], w[at + ram::ENEMY_HP] & 0x7f,
                    w[at + ram::ENEMY_CONTACT_DAMAGE], game.object_contact_bounds(at as u16)))
            }).collect();
            eprintln!("full-intro failure pads={inputs:x?} actors={actors:?} enemies[base,id,hp,contact,bounds]={enemies:?}");
            panic!("{error}")
        });
        if let Some(export) = &mut capture {
            export.frame(&game, &audio, &audit).unwrap();
        }
        if complete {
            assert_eq!(audit.level, Some(0), "completed a different stage than the canonical intro");
            let (deaths, warps, _) = controller.outcomes();
            assert_eq!(deaths, [0; 4], "controller detected deaths missed by independent audit");
            assert_eq!(warps, [0; 4], "controller detected warps missed by independent audit");
            assert!(audit.saw_hd_gameplay, "test never verified actual HD gameplay");
            compose_recording_frame(&game);
            if let Some(export) = &mut capture {
                assert!(export.finished, "native clear never reached the capture's MODE0 boundary");
                export.finish(&audit, true).unwrap();
            }
            if std::env::var_os("MMX_NPC_TRACE").is_some() {
                let (pixels, width, height) = game.frame();
                crate::headless::write_png("/tmp/mmx_verified_stage_select.png", pixels, width, height).unwrap();
            }
            eprintln!("FULL_INTRO_PASS frames={} level={:?} deaths=[0,0,0,0] warps=[0,0,0,0] min_hp={:?} furthest_x={:?} capture={} clear={} stage_select=true", game.frame_count(), audit.level, audit.minimum_hp, audit.furthest_x, audit.capture_seen, audit.clear_seen);
            return;
        }
        if frame == 15000 && std::env::var_os("MMX_NPC_TRACE").is_some() {
            let (pixels, width, height) = game.frame();
            crate::headless::write_png("/tmp/mmx_ending_check.png", pixels, width, height).unwrap();
        }
        if frame % 1800 == 0 {
            compose_recording_frame(&game);
            eprintln!("full-intro progress frame{} furthest={:?} min_hp={:?}", game.frame_count(), audit.furthest_x, audit.minimum_hp);
            if game.player_physics_state(0).is_some_and(|p| p[0] >= 6300) {
                let w = game.wram();
                let word = |at: usize| u16::from_le_bytes([w[at], w[at+1]]);
                let weak: Vec<_> = pre_frame.objects.iter().filter(|o| o.0 == "enemies" && o.4 == 0x11)
                    .map(|o| (o.1, word(o.1+0x38), game.object_contact_bounds_with_box(o.1 as u16, word(o.1+0x38))))
                    .collect();
                let p = game.player_physics_state(0).unwrap();
                let tiles: Vec<_> = (-32..=128).step_by(16).map(|dx| {
                    let x = i32::from(p[0])+dx;
                    (x, [400, 416, 432, 448, 464].map(|y| game.terrain_collision(x,y)))
                }).collect();
                eprintln!("LATE_WORLD frame{} pads={inputs:x?} actors={:?} objects={:?} driver_weak={weak:?} camera_limits={:?} terrain_columns_y400_416_432_448_464={tiles:?}",
                    game.frame_count(), pre_frame.actors, pre_frame.objects,
                    (0x1e54..=0x1e60).step_by(2).map(word).collect::<Vec<_>>());
            }
        }
    }
    panic!("whole-level timeout at frame{}: min_hp={:?}, furthest={:?}, capture={}, clear={}, actors={:?}",
        game.frame_count(), audit.minimum_hp, audit.furthest_x, audit.capture_seen, audit.clear_seen,
        (0..4).map(|slot| game.player_physics_state(slot)).collect::<Vec<_>>());
}

#[test]
fn real_rom_solo_source_reaches_native_intro_completion() {
    let Some(rom_path) = std::env::var_os("MMX_ROM").map(std::path::PathBuf::from) else { return; };
    let root = rom_path.parent().unwrap().parent().unwrap();
    let path = std::env::var_os("MMX_MOVIE").map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("movies/hetfield90v3-megamanx-100p.bk2"));
    let movie = mmx_verify::movie_bk2::parse_bk2(&std::fs::read(path).unwrap()).unwrap();
    let mut game = Game::from_rom(std::fs::read(rom_path).unwrap()).unwrap();
    game.set_allow_opposing_directions(true);
    game.set_overclock(mmx_core::Overclock::Off);
    let mut level = None;
    let mut capture = false;
    let mut clear = false;
    let mut furthest = 0;
    let mut previous = None;
    for pads in movie.pads.into_iter().take(12_000) {
        game.step(pads);
        game.take_audio();
        let w = game.wram();
        let mode = w[ix(ram::MODE_D2)];
        let phase = w[ix(ram::STAGE_PHASE)];
        let state = game.player_physics_state(0).unwrap();
        if mode == 4 && phase == 4 {
            level.get_or_insert(w[ix(ram::LEVEL_ID)]);
            furthest = furthest.max(state[0]);
            if matches!(state[3], 0x2e | 0x30 | 0x32) { capture = true; }
        }
        clear |= mode == 4 && phase == 8;
        let tuple = (mode, phase, w[ix(ram::LEVEL_ID)], state[3], state[2]);
        if previous != Some(tuple) && (capture || clear || phase == 6) {
            eprintln!("solo-source completion frame{} [mode,phase,level,action,hp]={tuple:?}, pos=({},{}), furthest={furthest}", game.frame_count(), state[0], state[1]);
        }
        previous = Some(tuple);
        assert!(mode != 0x0a && !(mode == 4 && phase == 6), "canonical solo source died");
        if mode == 0 && phase == 1 && clear {
            assert!(capture, "solo intro clear did not include Vile's native capture");
            assert_eq!(level, Some(0));
            assert!(furthest >= 7000, "source never reached the final highway road");
            eprintln!("SOLO_INTRO_PASS frames={} level={level:?} furthest={furthest} capture={capture} clear={clear} stage_select=true", game.frame_count());
            return;
        }
    }
    panic!("canonical source did not complete intro: level={level:?} capture={capture} clear={clear} furthest={furthest}");
}
