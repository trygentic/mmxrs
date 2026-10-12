//! Windowed loop: winit 0.30 ApplicationHandler + pixels 0.15, paced by the
//! cpal ring depth (~50 ms), or by a `Game::fps` timer without an audio device.
//! Online too: the audio clock paces `Net::tick_audio` (one frame of audio per
//! tick, silence when the session held the frame back).

use crate::audio::Audio;
use crate::cli::{Args, WideSetting};
use crate::demo_bots::DemoBots;
use crate::keys::{self, Hotkey, KeyAction};
use crate::netplay::Net;
use crate::video::{self, FitRenderer};
use crate::vs_select::VsSelect;
use mmx_core::{Game, Pads};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Fullscreen, Window, WindowId};

const AUDIO_TARGET_SECS: f64 = 0.050;
const MAX_CATCHUP: u32 = 4;
const FAST_FORWARD_STEPS: u32 = 8;

/// A gamepad from either source. GameController keys are the controller
/// object's address (`gc_pad::GcPad::key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PadId {
    Gilrs(gilrs::GamepadId),
    #[cfg(target_os = "macos")]
    Gc(usize),
}

struct App {
    game: Game,
    scale: u32,
    start_fullscreen: bool,
    coop: bool,
    players: usize,
    demo_bots: Option<DemoBots>,
    vs_bots: Option<crate::vs_bots::VsBots>,
    vs_bot_round_hold: u16,
    demo_survival: bool,
    movie: Option<crate::movie::Playback>,
    movie_route: Option<crate::movie_route::RoutePlayback>,
    showcase_characters: bool,
    window: Option<Arc<Window>>,
    pixels: Option<pixels::Pixels<'static>>,
    /// Scales the frame to fit the window (`video`).
    fit: Option<FitRenderer>,
    /// Surface size in physical pixels.
    surface: (u32, u32),
    /// `--widescreen` / F10; `Auto` re-picks the margin on every resize.
    wide: WideSetting,
    /// `MMX_VIDEO_DEBUG=1`: log window, margin, frame and picture rectangle.
    video_debug: bool,
    dynamic_lighting: bool,
    charge_effects: bool,
    tex: (usize, usize),
    audio: Option<Audio>,
    gilrs: Option<gilrs::Gilrs>,
    pad_map: keys::PadMap,
    /// macOS GameController pads seen last frame (see `gc_pad`).
    #[cfg(target_os = "macos")]
    gc: Vec<crate::gc_pad::GcPad>,
    /// The pad driving each local player (`pad_slots`).
    slots: crate::pad_slots::PadSlots<PadId>,
    /// `--pads` / `MMX_PAD_DEBUG=1`: log every pad bit change.
    pad_debug: bool,
    /// Last bits logged per pad (`pad_debug` only).
    pad_seen: Vec<(PadId, u16)>,
    /// Keyboard pad bits per local player (`keys::PLAYER_KEYS`).
    kb: [u16; 4],
    fast: bool,
    paused: bool,
    frame_step: bool,
    slot: u8,
    last: Instant,
    acc: f64,
    fps_t: Instant,
    fps_frames: u32,
    fps: f64,
    title: String,
    audio_stats: bool,
    /// `--show-fps` / F2: `osd` speed readout.
    show_fps: bool,
    /// `osd` notice and when it goes away (no audio device).
    notice: Option<(&'static str, Instant)>,
    /// Online co-op session (`--coop-host/--coop-join`).
    net: Option<Net>,
    /// Local fighter and arena selection, shown before the first ROM frame.
    vs_select: Option<VsSelect>,
    vs_prev_inputs: [u16; 4],
}

pub fn run(mut game: Game, args: &Args, mut net: Option<Net>) -> Result<(), String> {
    let audio = Audio::open();
    let no_audio = audio.is_none();
    match &audio {
        Some(a) => game.set_audio_rate(a.rate),
        None => eprintln!("audio: no output device, pacing by timer"),
    }
    if let (Some(n), Some(a)) = (net.as_mut(), &audio) {
        n.play.audio_rate = Some(a.rate);
    }
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let pad_debug = args.pads || std::env::var_os("MMX_PAD_DEBUG").is_some_and(|v| v == "1");
    #[cfg(target_os = "macos")]
    // Always: a game spawned by the launcher is often not macOS's active app,
    // and GameController delivers nothing to inactive apps otherwise.
    crate::gc_pad::monitor_background(true);
    let _ = pad_debug;
    let pad_map = keys::PadMap::parse(args.pad_map.as_deref())?;
    let gilrs = gilrs::Gilrs::new()
        .map_err(|e| eprintln!("gamepads: {e}"))
        .ok();
    if let Some(g) = &gilrs {
        for (_, gp) in g.gamepads() {
            eprintln!("gamepad connected (gilrs): {}", gp.name());
        }
    }
    let vs_select = (args.vs_mode && !args.vs_bots).then(|| {
        let mut select = VsSelect::new(
            usize::from(args.players),
            args.characters,
            mmx_core::vs_stages::ARENAS.iter().map(|arena| arena.name.to_string()).collect(),
            args.vs_stage.as_deref().and_then(|id|
                mmx_core::vs_stages::ARENAS.iter().position(|arena| arena.id == id)).unwrap_or(0),
        );
        select.set_portraits(mmx_core::Character::ALL.iter().map(|&character|
            mmx_core::bone_animation::selection_portrait(&game, character)).collect());
        select
    });
    let mut app = App {
        game,
        scale: args.scale,
        start_fullscreen: args.fullscreen,
        coop: args.vs_mode || args.players > 1 || args.coop_local,
        players: usize::from(args.players.max(if args.coop_local { 2 } else { 1 })),
        demo_bots: args.demo_bots.then(DemoBots::default),
        vs_bots: args.vs_bots.then(crate::vs_bots::VsBots::default),
        vs_bot_round_hold: 0,
        demo_survival: args.demo_survival,
        movie: if args.movie_route { None } else { crate::movie::Playback::load(args)? },
        movie_route: if args.movie_route { Some(crate::movie_route::RoutePlayback::load(args)?) } else { None },
        showcase_characters: args.showcase_characters,
        window: None,
        pixels: None,
        fit: None,
        surface: (0, 0),
        wide: args.widescreen,
        video_debug: std::env::var_os("MMX_VIDEO_DEBUG").is_some_and(|v| v == "1"),
        dynamic_lighting: args.dynamic_lighting,
        charge_effects: args.charge_effects,
        tex: (args.widescreen.initial().width(), 224),
        audio,
        gilrs,
        pad_map,
        #[cfg(target_os = "macos")]
        gc: Vec::new(),
        slots: crate::pad_slots::PadSlots::new(
            usize::from(args.players.max(if args.coop_local { 2 } else { 1 })),
            args.vs_mode || args.players > 1 || args.coop_local,
        ),
        pad_debug,
        pad_seen: Vec::new(),
        kb: [0; 4],
        fast: false,
        paused: false,
        frame_step: false,
        slot: 1,
        last: Instant::now(),
        acc: 0.0,
        fps_t: Instant::now(),
        fps_frames: 0,
        fps: 0.0,
        title: String::new(),
        audio_stats: std::env::var_os("MMX_AUDIO_STATS").is_some_and(|v| v == "1"),
        show_fps: args.show_fps,
        notice: no_audio.then(|| ("NO AUDIO DEVICE - SOUND OFF", Instant::now() + Duration::from_secs(6))),
        net,
        vs_select,
        vs_prev_inputs: [0; 4],
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

/// `$HOME/.mmxrs/slotN.state` (or `./.mmxrs/` without HOME).
// ponytail: no per-ROM subdir; there is one ROM.
fn state_path(slot: u8) -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    base.join(".mmxrs").join(format!("slot{slot}.state"))
}

impl App {
    /// True while GameController lists any pad: gilrs input is then ignored,
    /// since GC sees every pad gilrs could (and gilrs may see the same pad
    /// again, or see it but read nothing).
    fn gc_active(&self) -> bool {
        #[cfg(target_os = "macos")]
        return !self.gc.is_empty();
        #[cfg(not(target_os = "macos"))]
        false
    }

    fn pad_bits(&self, id: Option<PadId>) -> u16 {
        match id {
            Some(PadId::Gilrs(id)) if !self.gc_active() => self.gilrs_bits(id),
            #[cfg(target_os = "macos")]
            Some(PadId::Gc(k)) => self.gc.iter().find(|p| p.key == k).map_or(0, |p| p.bits),
            _ => 0,
        }
    }

    fn gilrs_bits(&self, id: gilrs::GamepadId) -> u16 {
        let Some(gp) = self.gilrs.as_ref().and_then(|g| g.connected_gamepad(id)) else {
            return 0;
        };
        let buttons = self
            .pad_map
            .gilrs
            .iter()
            .filter(|(b, _)| gp.is_pressed(*b))
            .fold(0, |a, (_, bit)| a | bit);
        buttons
            | keys::stick_bits(
                gp.value(gilrs::Axis::LeftStickX),
                gp.value(gilrs::Axis::LeftStickY),
            )
            // Pads without a d-pad mapping report it as a hat axis.
            | keys::stick_bits(gp.value(gilrs::Axis::DPadX), gp.value(gilrs::Axis::DPadY))
    }

    /// Give a newly used controller the first open local player slot
    /// (`pad_slots`). Single player keeps the last-pressed controller.
    fn claim(&mut self, id: PadId) {
        if let Some(slot) = self.slots.claim(id) {
            if self.coop {
                eprintln!("gamepad joined as P{}", slot + 1);
            }
        }
    }

    fn forget(&mut self, id: PadId) {
        self.slots.forget(id);
        self.pad_seen.retain(|(p, _)| *p != id);
    }

    /// Local players whose keyboard table is live.
    fn key_players(&self) -> usize {
        if self.coop { self.players.max(2) } else { 1 }
    }

    /// Each local player's keyboard and pad bits.
    fn local_inputs(&self) -> [u16; 4] {
        std::array::from_fn(|i| self.kb[i] | self.pad_bits(self.slots.pads[i]))
    }

    fn poll_pads(&mut self) {
        #[cfg(target_os = "macos")]
        self.poll_gc();
        let gc_active = self.gc_active();
        let mut events = Vec::new();
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(gilrs::Event { id, event, .. }) = g.next_event() {
                events.push((id, event, g.gamepad(id).name().to_string()));
            }
        }
        for (id, event, name) in events {
            match event {
                gilrs::EventType::ButtonPressed(..) if !gc_active => self.claim(PadId::Gilrs(id)),
                // Sticks and hat-style d-pads move a player too, so they join one.
                gilrs::EventType::AxisChanged(axis, value, _)
                    if !gc_active && keys::MOVE_AXES.contains(&axis) && value.abs() > 0.5 =>
                {
                    self.claim(PadId::Gilrs(id))
                }
                gilrs::EventType::Connected => eprintln!("gamepad connected (gilrs): {name}"),
                gilrs::EventType::Disconnected => {
                    eprintln!("gamepad disconnected (gilrs): {name}");
                    self.forget(PadId::Gilrs(id));
                }
                _ => {}
            }
        }
        if self.pad_debug {
            self.log_pad_changes();
        }
    }

    /// Diff this frame's GameController pads against last frame's: log
    /// connects/disconnects, and a newly pressed bit counts as a press.
    #[cfg(target_os = "macos")]
    fn poll_gc(&mut self) {
        let now = crate::gc_pad::snapshot(&self.pad_map.gc);
        let old = std::mem::replace(&mut self.gc, now.clone());
        for o in &old {
            match now.iter().find(|p| p.key == o.key) {
                Some(p) if p.bits & !o.bits != 0 => self.claim(PadId::Gc(p.key)),
                Some(_) => {}
                None => {
                    eprintln!("gamepad disconnected (GameController): {}", o.name);
                    self.forget(PadId::Gc(o.key));
                }
            }
        }
        for p in &now {
            if !old.iter().any(|o| o.key == p.key) {
                eprintln!("gamepad connected (GameController): {}", p.name);
                if p.bits != 0 {
                    self.claim(PadId::Gc(p.key));
                }
            }
        }
    }

    /// `--pads`: one stderr line per pad whose bits changed this frame.
    fn log_pad_changes(&mut self) {
        let mut pads: Vec<(PadId, String, u16)> = Vec::new();
        #[cfg(target_os = "macos")]
        pads.extend(self.gc.iter().map(|p| (PadId::Gc(p.key), format!("{} [GC]", p.name), p.bits)));
        if let Some(g) = &self.gilrs {
            let ignored = if self.gc_active() { ", ignored: GC active" } else { "" };
            for (id, gp) in g.gamepads() {
                pads.push((PadId::Gilrs(id), format!("{} [gilrs{ignored}]", gp.name()), self.gilrs_bits(id)));
            }
        }
        for (id, name, bits) in pads {
            let last = self.pad_seen.iter_mut().find(|(p, _)| *p == id);
            let prev = last.as_ref().map_or(0, |(_, b)| *b);
            if bits == prev {
                continue;
            }
            match last {
                Some(l) => l.1 = bits,
                None => self.pad_seen.push((id, bits)),
            }
            let slot = self.slots.pads.iter().position(|p| *p == Some(id))
                .map_or_else(|| "--".to_string(), |i| format!("P{}", i + 1));
            eprintln!("pad {slot} {name}: {bits:#06x} {}", keys::bit_names(bits));
        }
    }

    fn step(&mut self) {
        if self.vs_select.is_some() {
            let inputs = self.local_inputs();
            let choice = self.vs_select.as_mut().and_then(|select| select.step(inputs));
            if let Some(choice) = choice {
                for (slot, character) in choice.characters.into_iter().enumerate() {
                    self.game.set_player_character(slot, character);
                }
                if let Err(e) = self.game.set_p1_zero(
                    choice.characters[0] == mmx_core::Character::Zero && self.game.zero_set().is_some()
                ) {
                    eprintln!("VS fighter: {e}");
                    if let Some(select) = &mut self.vs_select { select.retry_arena(e); }
                    return;
                }
                let arena = &mmx_core::vs_stages::ARENAS[choice.stage_index];
                if let Err(e) = self.game.enable_vs(self.players) {
                    eprintln!("VS arena: {e}");
                    if let Some(select) = &mut self.vs_select { select.retry_arena(e); }
                    return;
                }
                self.game.set_vs_arena(*arena);
                if let Err(e) = mmx_core::vs_stages::start_stage(&mut self.game, arena) {
                    eprintln!("VS arena: {e}");
                    if let Some(select) = &mut self.vs_select { select.retry_arena(e); }
                    return;
                }
                // The ROM boot is fast-forwarded with no audio output. Discard
                // its title and loading music before live arena pacing starts.
                let _ = self.game.take_audio();
                self.vs_select = None;
                self.acc = 0.0;
                eprintln!("VS: {} players, {}", self.players, arena.name);
            }
            self.fps_frames += 1;
            return;
        }
        if self.demo_survival { self.game.maintain_demo_survival(); }
        if let Some(bots) = self.demo_bots.as_ref() {
            bots.maintain_party(&mut self.game);
        }
        let inputs = if let Some(bots) = self.vs_bots.as_mut() {
            bots.pads(&self.game)
        } else if let Some(route) = self.movie_route.as_mut() {
            route.next(&self.game)
        } else if let Some(movie) = self.movie.as_mut() {
            movie.next(&self.game)
        } else { match self.demo_bots.as_mut() {
            Some(bots) => bots.pads(&self.game),
            None => self.local_inputs(),
        }};
        if self.game.vs_status().is_some_and(|status| status.round_over) {
            if self.vs_bots.is_some() {
                self.vs_bot_round_hold += 1;
                // Game::step freezes the completed round, so supply one frame
                // of silence to keep audio-led pacing at the normal rate.
                if !self.fast {
                    if let Some(audio) = &self.audio {
                        let frames = (f64::from(audio.rate) / self.game.fps()).ceil() as usize;
                        audio.push(&vec![(0.0, 0.0); frames]);
                    }
                }
                if self.vs_bot_round_hold >= 90 {
                    self.game.restart_vs_round();
                    self.vs_bot_round_hold = 0;
                }
                self.fps_frames += 1;
                return;
            }
            let start_pressed = (0..self.players).any(|i|
                inputs[i] & mmx_core::BTN_START != 0
                    && self.vs_prev_inputs[i] & mmx_core::BTN_START == 0);
            self.vs_prev_inputs = inputs;
            if start_pressed { self.game.restart_vs_round(); }
            self.fps_frames += 1;
            return;
        }
        self.vs_bot_round_hold = 0;
        self.vs_prev_inputs = inputs;
        if self.showcase_characters {
            if let Some((group, characters)) = self.demo_bots.as_ref().and_then(DemoBots::showcase_selection) {
                for (slot, character) in characters.into_iter().enumerate() {
                    self.game.set_player_character(slot, character);
                }
                eprintln!("showcase group {}: {}", group + 1,
                    characters.map(|character| character.label()).join(", "));
            }
        }
        let pads = Pads { p1: inputs[0], p2: inputs[1] };
        let extra = [inputs[2], inputs[3]];
        self.game.set_extra_pads(extra);
        self.game.step(pads);
        self.push_audio();
        self.fps_frames += 1;
    }

    fn push_audio(&mut self) {
        let samples = self.game.take_audio();
        if let (Some(a), false) = (&self.audio, self.fast) {
            a.push(&samples);
        }
    }

    /// One online tick (this peer's pad is always the P1 keys / first pad).
    fn net_tick(&mut self) -> u32 {
        let pad = self.kb[0] | self.pad_bits(self.slots.pads[0]);
        let Some(net) = self.net.as_mut() else { return 0 };
        let n = net.tick(&mut self.game, pad);
        self.push_audio();
        n
    }

    /// Online tick paced by the audio device: always queues one frame of audio.
    fn net_tick_audio(&mut self) -> u32 {
        let pad = self.kb[0] | self.pad_bits(self.slots.pads[0]);
        let (Some(net), Some(a)) = (self.net.as_mut(), &self.audio) else {
            return 0;
        };
        let (n, pcm) = net.tick_audio(&mut self.game, pad);
        a.push(&pcm);
        n
    }

    /// Ring depth to keep queued, in samples (`None`: no audio device).
    fn audio_target(&self) -> Option<usize> {
        self.audio
            .as_ref()
            .map(|a| (f64::from(a.rate) * AUDIO_TARGET_SECS) as usize)
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(net) = self.net.as_mut() {
            net.close_and_flush();
        }
        event_loop.exit();
    }

    fn hotkey(&mut self, h: Hotkey, pressed: bool, event_loop: &ActiveEventLoop) {
        if self.vs_select.is_some() && !matches!(h, Hotkey::Quit | Hotkey::Fullscreen | Hotkey::Widescreen | Hotkey::ShowFps) {
            return;
        }
        // Pause, fast-forward and save states would desync an online session.
        if self.net.is_some() && !matches!(h, Hotkey::Quit | Hotkey::Fullscreen | Hotkey::Widescreen | Hotkey::ShowFps) {
            return;
        }
        // Game states do not contain the movie cursor. Loading one would
        // resume the wrong inputs, and saving one implies it can be resumed.
        if (self.movie.is_some() || self.movie_route.is_some()) && matches!(h, Hotkey::SaveState | Hotkey::LoadState) {
            if pressed { eprintln!("movie: save/load states are disabled during playback"); }
            return;
        }
        if h == Hotkey::FastForward {
            self.fast = pressed;
            return;
        }
        if !pressed {
            return;
        }
        match h {
            Hotkey::FastForward => {}
            Hotkey::Slot(n) => self.slot = n,
            Hotkey::SaveState => {
                let p = state_path(self.slot);
                let r = p
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|_| std::fs::write(&p, self.game.save_state()));
                match r {
                    Ok(()) => eprintln!("saved {}", p.display()),
                    Err(e) => eprintln!("save {}: {e}", p.display()),
                }
            }
            Hotkey::LoadState => {
                let p = state_path(self.slot);
                match std::fs::read(&p)
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.game.load_state(&s))
                {
                    Ok(()) => eprintln!("loaded {}", p.display()),
                    Err(e) => eprintln!("load {}: {e}", p.display()),
                }
            }
            Hotkey::Fullscreen => {
                if let Some(w) = &self.window {
                    let fs = w
                        .fullscreen()
                        .is_none()
                        .then_some(Fullscreen::Borderless(None));
                    w.set_fullscreen(fs);
                }
            }
            Hotkey::Widescreen => {
                // Display only, fine online. A preset in a plain window
                // reshapes the window to its aspect (the buffer resizes with
                // the next frame, `draw`); auto fits the window instead.
                self.wide = self.wide.next();
                match self.wide {
                    WideSetting::Fixed(ws) => {
                        self.game.set_widescreen(ws);
                        eprintln!("widescreen: {}", ws.name());
                        let plain = |w: &&Arc<Window>| w.fullscreen().is_none() && !w.is_maximized();
                        if let Some(w) = self.window.as_ref().filter(plain) {
                            let h = w.inner_size().height.max(224);
                            let _ = w.request_inner_size(winit::dpi::PhysicalSize::new(h * ws.width() as u32 / 224, h));
                        }
                    }
                    WideSetting::Auto => eprintln!("widescreen: auto"),
                }
                self.relayout("widescreen");
            }
            Hotkey::Quit => self.quit(event_loop),
            Hotkey::Pause => {
                self.paused = !self.paused;
                if let Some(a) = &self.audio {
                    a.ring.lock().unwrap().clear();
                }
            }
            Hotkey::FrameStep => self.frame_step = self.paused,
            Hotkey::ShowFps => self.show_fps = !self.show_fps,
        }
    }

    fn update_title(&mut self) {
        let mut t = format!("mmxrs - {:.0} fps", self.fps);
        if let Some(net) = &self.net {
            t += &net.title();
        } else if self.vs_select.is_some() || self.game.vs_status().is_some() {
            t += " [VS]";
        } else if self.coop {
            t += " [coop]";
        }
        if self.demo_bots.is_some() {
            t += " [DEMO BOTS]";
        }
        if self.movie.is_some() { t += " [MOVIE]"; }
        if self.movie_route.is_some() { t += " [MOVIE ROUTE]"; }
        t += &format!(" [SLOT {}]", self.slot);
        if self.paused {
            t += " [PAUSED]";
        }
        if self.fast {
            t += " [FF]";
        }
        if t != self.title {
            if let Some(w) = &self.window {
                w.set_title(&t);
            }
            self.title = t;
        }
    }

    /// The picture's rectangle in the window for the current frame size.
    fn dest(&self) -> video::Rect {
        let tol = if self.wide == WideSetting::Auto { video::AUTO_FILL_TOLERANCE } else { 0.0 };
        video::fit_rect(self.surface, (self.tex.0 as u32, self.tex.1 as u32), tol)
    }

    /// After a resize (maximize, fullscreen) or F10: auto re-picks the
    /// margin for the window's aspect. Logs with `MMX_VIDEO_DEBUG=1`.
    fn relayout(&mut self, why: &str) {
        let Some(w) = &self.window else { return };
        let size = w.inner_size();
        self.surface = (size.width, size.height);
        let logical_height = self.game.frame().2 as u32 / u32::from(self.game.frame_scale());
        if self.wide == WideSetting::Auto && size.width > 0 && size.height > 0 {
            let ws = video::auto_margin(self.surface, logical_height);
            if ws != self.game.widescreen() {
                self.game.set_widescreen(ws);
            }
        }
        if self.video_debug {
            let ws = self.game.widescreen();
            // The frame takes the new width with the next frame drawn.
            let frame = (ws.width() as u32, logical_height);
            let tol = if self.wide == WideSetting::Auto { video::AUTO_FILL_TOLERANCE } else { 0.0 };
            let d = video::fit_rect(self.surface, frame, tol);
            let mode = if self.wide == WideSetting::Auto { "auto" } else { "fixed" };
            eprintln!(
                "video: {why}: window {}x{} (scale factor {}), widescreen {mode} margin {} tiles, frame {}x{}, dest {}x{} at ({},{}), scale {:.3}x{:.3}{}",
                size.width,
                size.height,
                w.scale_factor(),
                ws.0,
                frame.0,
                frame.1,
                d.w,
                d.h,
                d.x,
                d.y,
                f64::from(d.w) / f64::from(frame.0),
                f64::from(d.h) / f64::from(frame.1),
                if (d.w, d.h) == self.surface { " (fills window)" } else { "" },
            );
        }
    }

    fn draw(&mut self) {
        let select_screen = self.vs_select.is_some();
        let (px, w, h) = if select_screen { (&[][..], self.tex.0, self.tex.1) } else { self.game.frame() };
        let Some(p) = self.pixels.as_mut() else {
            return;
        };
        if (w, h) != self.tex {
            // 256-640 wide (widescreen), 224/239 (x2 interlaced) tall
            if let Err(e) = p.resize_buffer(w as u32, h as u32) {
                eprintln!("resize buffer: {e}");
                return;
            }
            self.tex = (w, h);
            if self.video_debug {
                let d = self.dest();
                eprintln!("video: frame now {w}x{h}, dest {}x{} at ({},{})", d.w, d.h, d.x, d.y);
            }
        }
        let p = self.pixels.as_mut().expect("pixels");
        if let Some(select) = &self.vs_select {
            select.draw(p.frame_mut(), w, h);
        } else {
            for (dst, &c) in p.frame_mut().chunks_exact_mut(4).zip(&px[..w * h]) {
                dst.copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 0xFF]);
            }
            crate::effects::apply(p.frame_mut(), w, h, &self.game, self.dynamic_lighting, self.charge_effects);
            self.game.draw_ui_rgba(p.frame_mut(), w, h);
            self.notice = self.notice.filter(|(_, until)| Instant::now() < *until);
            let speed = self.show_fps.then(|| crate::osd::speed_line(self.fps, self.game.fps()));
            let lines: Vec<&str> = speed.as_deref().into_iter().chain(self.notice.map(|(n, _)| n)).collect();
            if !lines.is_empty() {
                crate::osd::draw(p.frame_mut(), w, h, &lines, usize::from(self.game.frame_scale()));
            }
            if let Some(status) = self.game.vs_status().filter(|status| status.round_over) {
                crate::vs_select::draw_result_scaled(p.frame_mut(), w, h, status, self.game.frame_scale());
            }
        }
        let dest = self.dest();
        let (Some(p), Some(fit)) = (self.pixels.as_ref(), self.fit.as_mut()) else {
            return;
        };
        let r = p.render_with(|encoder, target, ctx| {
            fit.render(encoder, target, ctx, dest);
            Ok(())
        });
        if let Err(e) = r {
            eprintln!("render: {e}");
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = (
            self.tex.0 as u32 * self.scale,
            self.tex.1 as u32 * self.scale,
        );
        let attrs = Window::default_attributes()
            .with_title("mmxrs")
            .with_inner_size(winit::dpi::LogicalSize::new(w, h))
            .with_fullscreen(
                self.start_fullscreen
                    .then_some(Fullscreen::Borderless(None)),
            );
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("create window: {e}");
                return event_loop.exit();
            }
        };
        let size = window.inner_size();
        let surface = pixels::SurfaceTexture::new(size.width, size.height, window.clone());
        match pixels::Pixels::new(self.tex.0 as u32, self.tex.1 as u32, surface) {
            Ok(p) => {
                self.fit = Some(FitRenderer::new(&p));
                self.pixels = Some(p);
            }
            Err(e) => {
                eprintln!("create pixels: {e}");
                return event_loop.exit();
            }
        }
        self.window = Some(window);
        self.relayout("start");
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.quit(event_loop),
            WindowEvent::Resized(s) => {
                if let Some(p) = self.pixels.as_mut() {
                    if s.width > 0 && s.height > 0 {
                        let _ = p.resize_surface(s.width, s.height);
                    }
                }
                self.relayout("resize");
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                let (slot, b) = match keys::map_key(code, self.key_players()) {
                    Some(KeyAction::P1(b)) => (0, b),
                    Some(KeyAction::P2(b)) => (1, b),
                    Some(KeyAction::P3(b)) => (2, b),
                    Some(KeyAction::P4(b)) => (3, b),
                    Some(KeyAction::Hot(h)) if !event.repeat || h == Hotkey::FrameStep => {
                        return self.hotkey(h, pressed, event_loop);
                    }
                    _ => return,
                };
                set_bit(&mut self.kb[slot], b, pressed);
                if pressed {
                    if let Some((from, to)) = self.slots.keyboard(slot, b) {
                        eprintln!("keyboard took P{}: its gamepad moved to P{}", from + 1, to + 1);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }
        self.poll_pads();
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;

        let mut stepped = 0;
        // Online: pump the network every wake-up; once the session is live it
        // drives the game, one tick per frame of audio the device plays (or
        // per `Game::fps` tick without audio). A hard-coded 60 Hz timer
        // overran the ring on the PAL ROM (50 fps): dropped audio, fast music.
        let was_live = self.net.as_ref().is_some_and(|n| n.play.live());
        if self.vs_select.is_some() {
            // The menu has no ROM audio, so use a real 60 Hz clock instead of
            // repeatedly filling the empty audio ring.
            self.acc = (self.acc + dt * 60.0).min(f64::from(MAX_CATCHUP));
            while self.acc >= 1.0 {
                self.acc -= 1.0;
                self.step();
                stepped += 1;
            }
        } else if self.net.is_some() && !was_live {
            self.net_tick(); // connecting / closed: network only
        }
        let live = self.net.as_ref().is_some_and(|n| n.play.live());
        if self.vs_select.is_some() {
            // Selection was paced above and must not enter the ROM audio loop.
        } else if live {
            if let (true, Some(target)) = (was_live, self.audio_target()) {
                let mut ticks = 0;
                while ticks < MAX_CATCHUP && self.audio.as_ref().is_some_and(|a| a.depth() < target)
                {
                    let n = self.net_tick_audio();
                    ticks += 1;
                    stepped += n;
                    self.fps_frames += n.min(1);
                }
            } else if was_live {
                self.acc = (self.acc + dt * self.game.fps()).min(f64::from(MAX_CATCHUP));
                while self.acc >= 1.0 {
                    self.acc -= 1.0;
                    let n = self.net_tick();
                    stepped += n;
                    self.fps_frames += n.min(1);
                }
            } else {
                self.acc = 0.0; // `Running` just rebuilt the game: start clean
                stepped = 1;
            }
        } else if self.paused {
            if std::mem::take(&mut self.frame_step) {
                self.step();
                stepped = 1;
            }
        } else if self.fast {
            for _ in 0..FAST_FORWARD_STEPS {
                self.step();
            }
            stepped = FAST_FORWARD_STEPS;
        } else if let Some(target) = self.audio_target() {
            // audio clock is the master: top the ring up to ~50 ms
            while stepped < MAX_CATCHUP && self.audio.as_ref().is_some_and(|a| a.depth() < target) {
                self.step();
                stepped += 1;
            }
        } else {
            self.acc = (self.acc + dt * self.game.fps()).min(f64::from(MAX_CATCHUP));
            while self.acc >= 1.0 {
                self.acc -= 1.0;
                self.step();
                stepped += 1;
            }
        }

        if now.duration_since(self.fps_t) >= Duration::from_secs(1) {
            let secs = now.duration_since(self.fps_t).as_secs_f64();
            self.fps = f64::from(self.fps_frames) / secs;
            if let (Some(a), true) = (&self.audio, self.audio_stats) {
                eprintln!(
                    "audio: {:.0} samples/s (device {} Hz), ring {} ms, dropped {}, {:.2} fps",
                    a.pushed.take() as f64 / secs,
                    a.rate,
                    a.depth() as u64 * 1000 / u64::from(a.rate),
                    a.dropped.take(),
                    self.fps
                );
            }
            self.fps_frames = 0;
            self.fps_t = now;
        }
        self.update_title();
        if stepped > 0 {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        // ponytail: 2 ms wake-ups instead of a precise frame scheduler.
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(2)));
    }
}

fn set_bit(v: &mut u16, bit: u16, on: bool) {
    if on {
        *v |= bit;
    } else {
        *v &= !bit;
    }
}
