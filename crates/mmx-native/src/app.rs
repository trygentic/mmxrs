//! Windowed loop: winit 0.30 ApplicationHandler + pixels 0.15, paced by the
//! cpal ring depth (~50 ms), or by a `Game::fps` timer without an audio device.
//! Online too: the audio clock paces `Net::tick_audio` (one frame of audio per
//! tick, silence when the session held the frame back).

use crate::audio::Audio;
use crate::cli::Args;
use crate::keys::{self, Hotkey, KeyAction};
use crate::netplay::Net;
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
    window: Option<Arc<Window>>,
    pixels: Option<pixels::Pixels<'static>>,
    tex: (usize, usize),
    audio: Option<Audio>,
    gilrs: Option<gilrs::Gilrs>,
    /// macOS GameController pads seen last frame (see `gc_pad`).
    #[cfg(target_os = "macos")]
    gc: Vec<crate::gc_pad::GcPad>,
    pad1: Option<PadId>,
    pad2: Option<PadId>,
    /// `--pads` / `MMX_PAD_DEBUG=1`: log every pad bit change.
    pad_debug: bool,
    /// Last bits logged per pad (`pad_debug` only).
    pad_seen: Vec<(PadId, u16)>,
    kb: [u16; 2],
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
    /// Online co-op session (`--coop-host/--coop-join`).
    net: Option<Net>,
}

pub fn run(mut game: Game, args: &Args, mut net: Option<Net>) -> Result<(), String> {
    let audio = Audio::open();
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
    crate::gc_pad::monitor_background(pad_debug);
    let gilrs = gilrs::Gilrs::new()
        .map_err(|e| eprintln!("gamepads: {e}"))
        .ok();
    if let Some(g) = &gilrs {
        for (_, gp) in g.gamepads() {
            eprintln!("gamepad connected (gilrs): {}", gp.name());
        }
    }
    let mut app = App {
        game,
        scale: args.scale,
        start_fullscreen: args.fullscreen,
        coop: args.coop_local,
        window: None,
        pixels: None,
        tex: (256, 224),
        audio,
        gilrs,
        #[cfg(target_os = "macos")]
        gc: Vec::new(),
        pad1: None,
        pad2: None,
        pad_debug,
        pad_seen: Vec::new(),
        kb: [0; 2],
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
        net,
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
        let buttons = keys::PAD_BUTTONS
            .iter()
            .filter(|(b, _)| gp.is_pressed(*b))
            .fold(0, |a, (_, bit)| a | bit);
        buttons
            | keys::stick_bits(
                gp.value(gilrs::Axis::LeftStickX),
                gp.value(gilrs::Axis::LeftStickY),
            )
    }

    /// z2-native's rule: the pad that last pressed a button is P1; in co-op a
    /// different pad that presses becomes P2.
    fn claim(&mut self, id: PadId) {
        if !self.coop {
            self.pad1 = Some(id);
        } else if self.pad2 != Some(id) {
            if self.pad1.is_none() || self.pad1 == Some(id) {
                self.pad1 = Some(id);
            } else {
                self.pad2 = Some(id);
            }
        }
    }

    fn forget(&mut self, id: PadId) {
        if self.pad1 == Some(id) {
            self.pad1 = None;
        }
        if self.pad2 == Some(id) {
            self.pad2 = None;
        }
        self.pad_seen.retain(|(p, _)| *p != id);
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
        let now = crate::gc_pad::snapshot();
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
            let slot = if self.pad1 == Some(id) {
                "P1"
            } else if self.pad2 == Some(id) {
                "P2"
            } else {
                "--"
            };
            eprintln!("pad {slot} {name}: {bits:#06x} {}", keys::bit_names(bits));
        }
    }

    fn step(&mut self) {
        let pads = Pads {
            p1: self.kb[0] | self.pad_bits(self.pad1),
            p2: self.kb[1] | self.pad_bits(self.pad2),
        };
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
        let pad = self.kb[0] | self.pad_bits(self.pad1);
        let Some(net) = self.net.as_mut() else { return 0 };
        let n = net.tick(&mut self.game, pad);
        self.push_audio();
        n
    }

    /// Online tick paced by the audio device: always queues one frame of audio.
    fn net_tick_audio(&mut self) -> u32 {
        let pad = self.kb[0] | self.pad_bits(self.pad1);
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
        // Pause, fast-forward and save states would desync an online session.
        if self.net.is_some() && !matches!(h, Hotkey::Quit | Hotkey::Fullscreen) {
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
            Hotkey::Quit => self.quit(event_loop),
            Hotkey::Pause => {
                self.paused = !self.paused;
                if let Some(a) = &self.audio {
                    a.ring.lock().unwrap().clear();
                }
            }
            Hotkey::FrameStep => self.frame_step = self.paused,
        }
    }

    fn update_title(&mut self) {
        let mut t = format!("mmxrs - {:.0} fps", self.fps);
        if let Some(net) = &self.net {
            t += &net.title();
        } else if self.coop {
            t += " [coop]";
        }
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

    fn draw(&mut self) {
        let Some(p) = self.pixels.as_mut() else {
            return;
        };
        let (px, w, h) = self.game.frame();
        if (w, h) != self.tex {
            // 256/512 wide, 224/239 (x2 interlaced) tall
            if let Err(e) = p.resize_buffer(w as u32, h as u32) {
                eprintln!("resize buffer: {e}");
                return;
            }
            self.tex = (w, h);
        }
        for (dst, &c) in p.frame_mut().chunks_exact_mut(4).zip(&px[..w * h]) {
            dst.copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 0xFF]);
        }
        if let Err(e) = p.render() {
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
            Ok(p) => self.pixels = Some(p),
            Err(e) => {
                eprintln!("create pixels: {e}");
                return event_loop.exit();
            }
        }
        self.window = Some(window);
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
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                match keys::map_key(code, self.coop) {
                    Some(KeyAction::P1(b)) => set_bit(&mut self.kb[0], b, pressed),
                    Some(KeyAction::P2(b)) => set_bit(&mut self.kb[1], b, pressed),
                    Some(KeyAction::Hot(h)) if !event.repeat || h == Hotkey::FrameStep => {
                        self.hotkey(h, pressed, event_loop)
                    }
                    _ => {}
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
        if self.net.is_some() && !was_live {
            self.net_tick(); // connecting / closed: network only
        }
        let live = self.net.as_ref().is_some_and(|n| n.play.live());
        if live {
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
