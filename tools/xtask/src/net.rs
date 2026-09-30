//! `xtask netsim` (two session games over an impaired loopback link, virtual
//! clock) and `xtask netreplay` (a recorded session log against a plain run).
//! See docs/netplay.md "Testing the netcode".

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::rc::Rc;

use mmx_core::netgame::{net_checksum, session_game, trapset_id, Mode, NetConfig, NetPlay, Simulated};
use mmx_core::netlog::{self, Event, Log, Recorder};
use mmx_core::zero_data::ZeroSet;
use mmx_core::{ram, Game, Pads, Timing};
use mmx_net::{hash_state, loopback_pair, LoopbackConfig, LoopbackTransport, COOP_TWO_PLAYERS};

/// Rebuild a session game the way both peers (and the replay) do: power-on
/// with co-op on, the X3 set and P1-as-Zero flag, then the save state.
pub fn sim_game(rom: &[u8], timing: Timing, x3: Option<&ZeroSet>, zero: bool, load: Option<&[u8]>) -> Result<Game, String> {
    let mut g = session_game(rom, COOP_TWO_PLAYERS, timing).map_err(|e| e.to_string())?;
    g.set_zero_set(x3.cloned());
    g.set_p1_zero(zero)?;
    if let Some(s) = load {
        g.load_state(s)?;
    }
    Ok(g)
}

// ---------------------------------------------------------------- netsim

pub struct SimOpts {
    pub mode: Mode,
    pub delay: u8,
    pub prediction: u8,
    pub rtt_ms: u32,
    pub jitter_ms: u32,
    pub loss_pct: f64,
    pub frames: u32,
    pub seed: u64,
    pub timing: Timing,
    pub x3: Option<ZeroSet>,
    pub zero: bool,
    /// `(name for the log header, save state)` loaded on both peers at frame 0.
    pub load: Option<(String, Vec<u8>)>,
    /// Record both peers' session logs into the report.
    pub record: bool,
    /// Test hook: from this frame on the guest's WRAM `$1F00` is overwritten
    /// every tick (a forced desync).
    pub corrupt_guest_at: Option<u32>,
}

impl SimOpts {
    pub fn new(mode: Mode) -> SimOpts {
        SimOpts {
            mode,
            delay: mode.default_delay(),
            prediction: mmx_net::DEFAULT_MAX_PREDICTION,
            rtt_ms: 300,
            jitter_ms: 40,
            loss_pct: 3.0,
            frames: 3600,
            seed: 1,
            timing: Timing::Auto,
            x3: None,
            zero: false,
            load: None,
            record: false,
            corrupt_guest_at: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct PeerReport {
    pub role: &'static str,
    /// Frames simulated for the first time / settled (final on both peers).
    pub frames: u32,
    pub settled: u32,
    /// Frames per second of virtual time once live, and the game's rate.
    pub fps: f64,
    pub game_fps: f64,
    /// Live ticks that simulated no new frame (lockstep wait, full
    /// prediction window, wait recommendation), percent of live ticks.
    pub stall_pct: f64,
    pub rollbacks: u64,
    pub max_rollback_depth: u32,
    pub replayed_frames: u64,
    /// Advances refused because the prediction window was full.
    pub saturated: u64,
    pub wait_recommendations: u32,
    pub checks_ok: u32,
    /// Other player's first-drawn vs settled screen position, px (max of |dx|, |dy|).
    pub remote_err_max: u32,
    pub remote_err_mean: f64,
    pub remote_err_over4_pct: f64,
    pub audio_queue_max_ms: u64,
    pub audio_dropped_ms: u64,
    pub audio_starved_ms: u64,
    pub log: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct SimReport {
    pub mode: &'static str,
    pub delay: u8,
    pub prediction: u8,
    pub rtt_ms: u32,
    pub jitter_ms: u32,
    pub loss_pct: f64,
    pub frames: u32,
    pub virtual_secs: f64,
    /// `desync at frame N` (or another close reason) when a peer closed.
    pub desync: Option<String>,
    pub desync_frame: Option<u32>,
    pub closed: Option<String>,
    /// First frame where a peer's settled state differs from the plain run.
    pub plain_mismatch: Option<(u32, &'static str)>,
    /// Frames compared against the plain run.
    pub plain_frames: u32,
    pub packets_dropped: u64,
    pub peers: [PeerReport; 2],
    pub recommendation: String,
}

impl SimReport {
    pub fn ok(&self) -> bool {
        self.desync.is_none() && self.plain_mismatch.is_none() && self.closed.is_none() && self.plain_frames >= self.frames
    }
}

/// A frame's last simulation: remote player's screen position, pads, state hash.
type Settled = ((i32, i32), (u16, u16), u64);

/// Per-peer trace from `NetPlay::observe`.
#[derive(Default)]
struct Track {
    /// Remote player's screen position when the frame was first simulated.
    shown: Vec<Option<(i32, i32)>>,
    settled: Vec<Option<Settled>>,
}

fn word(w: &[u8], a: usize) -> i32 {
    i32::from(u16::from_le_bytes([w[a], w[a + 1]]))
}

/// The other player's position on screen: P2 on the host, P1 on the guest.
fn remote_pos(g: &Game, host: bool) -> (i32, i32) {
    let w = g.wram();
    let cam = (word(w, ram::wram_index(ram::CAMERA_X)), word(w, ram::wram_index(ram::CAMERA_Y)));
    let (x, y) = if host {
        (word(&g.coop.p2, 5), word(&g.coop.p2, 8))
    } else {
        (word(w, ram::wram_index(ram::X_POS_X)), word(w, ram::wram_index(ram::X_POS_Y)))
    };
    (x - cam.0, y - cam.1)
}

/// What a settled frame must equal: WRAM, P2's block and the OAM.
fn state_hash(g: &Game) -> u64 {
    hash_state(&[g.wram(), &g.coop.p2, &g.oam()])
}

struct Dev {
    hz: u64,
    queue: u64,
    max: u64,
    dropped: u64,
    starved: u64,
}

/// Two session games through `NetPlay` over `loopback_pair`, 1 ms of virtual
/// time per loopback tick, each paced by a fake audio device like the
/// frontends (top the queue up to 50 ms with `tick_audio`, at most 4 ticks
/// per 2 ms wake-up; 200 ms cap, overflow dropped). `script(frame, pads)`
/// gives the pads for the tick where `frame` (1-based) is next: the host
/// plays its `.p1`, the guest its `.p2`. Runs until both peers settled
/// `frames` frames, then checks every settled frame against a plain
/// single-`Game` run fed the host's settled pads.
pub fn netsim(rom: &[u8], o: &SimOpts, script: &dyn Fn(usize, Pads) -> Pads) -> Result<SimReport, String> {
    let link_cfg = LoopbackConfig {
        latency_ticks: o.rtt_ms / 2,
        jitter_ticks: o.jitter_ms,
        loss_per_mille: (o.loss_pct * 10.0).round() as u16,
        connect_after_ticks: 50,
    };
    let (link, ta, tb) = loopback_pair(link_cfg, o.seed);
    let x3 = o.x3.as_ref();
    let load = o.load.as_ref().map(|(_, s)| s.as_slice());
    let mk = |host: bool, t: LoopbackTransport, hz: u32| -> Result<(NetPlay<LoopbackTransport>, Rc<RefCell<Track>>), String> {
        let mut cfg = NetPlay::<LoopbackTransport>::config(rom, host, o.mode, o.delay, x3, o.zero, o.timing)?;
        if let NetConfig::Rollback(c) = &mut cfg {
            c.max_prediction = o.prediction;
        }
        let mut p = NetPlay::new(cfg, rom.to_vec(), t, "netsim")?;
        p.audio_rate = Some(hz);
        let track = Rc::new(RefCell::new(Track::default()));
        let tr = Rc::clone(&track);
        p.observe = Some(Box::new(move |g: &Game, s: Simulated| {
            let mut t = tr.borrow_mut();
            let f = s.frame as usize;
            if t.settled.len() <= f {
                t.settled.resize(f + 64, None);
                t.shown.resize(f + 64, None);
            }
            let pos = remote_pos(g, host);
            if !s.replay {
                t.shown[f] = Some(pos);
            }
            t.settled[f] = Some((pos, (s.pad1, s.pad2), state_hash(g)));
        }));
        if o.record {
            let mut rec = Recorder::new();
            rec.load = o.load.as_ref().map(|(n, _)| n.clone());
            p.record = Some(rec);
        }
        Ok((p, track))
    };
    let (h, ht) = mk(true, ta, 48_000)?;
    let (g, gt) = mk(false, tb, 44_100)?;
    let mut nets = [h, g];
    let tracks = [ht, gt];
    let mut games = [sim_game(rom, o.timing, x3, o.zero, None)?, sim_game(rom, o.timing, x3, o.zero, None)?];
    let mut devs = [48_000u64, 44_100].map(|hz| Dev { hz, queue: 0, max: 0, dropped: 0, starved: 0 });
    let mut live_at: [Option<(u64, u32)>; 2] = [None; 2];
    let (mut live_ticks, mut stall_ticks) = ([0u64; 2], [0u64; 2]);
    let game_fps = games[0].fps();
    let limit_ms = (f64::from(o.frames) / game_fps * 1000.0) as u64 * 20 + 120_000;
    let mut ms = 0u64;
    loop {
        let done = nets.iter().all(|n| n.settled_frames() >= o.frames);
        if done || nets.iter().any(|n| n.closed()) || ms > limit_ms {
            break;
        }
        link.advance(1);
        for i in 0..2 {
            let (net, game, dev) = (&mut nets[i], &mut games[i], &mut devs[i]);
            if live_at[i].is_some() {
                let want = dev.hz * (ms + 1) / 1000 - dev.hz * ms / 1000;
                let got = want.min(dev.queue);
                dev.queue -= got;
                dev.starved += want - got;
            }
            if !ms.is_multiple_of(2) {
                continue;
            }
            if !net.live() {
                net.tick(game, 0, ms);
                if net.live() {
                    live_at[i] = Some((ms, net.frame()));
                    if let Some(s) = load {
                        game.load_state(s)?;
                        // the state carries the rate it was saved at
                        game.set_audio_rate(dev.hz as u32);
                    }
                }
                continue;
            }
            let mut k = 0;
            while k < 4 && dev.queue < dev.hz / 20 && net.live() {
                let f0 = net.frame();
                if let Some(c) = o.corrupt_guest_at.filter(|&c| i == 1 && f0 >= c) {
                    game.wram_mut()[0x1F00] = 0xA5 ^ c as u8;
                }
                let pads = script(f0 as usize + 1, Pads::default());
                let (_, pcm) = net.tick_audio(game, if i == 0 { pads.p1 } else { pads.p2 }, ms);
                dev.queue += pcm.len() as u64;
                if dev.queue > dev.hz / 5 {
                    dev.dropped += dev.queue - dev.hz / 5;
                    dev.queue = dev.hz / 5;
                }
                dev.max = dev.max.max(dev.queue);
                live_ticks[i] += 1;
                stall_ticks[i] += u64::from(net.frame() == f0);
                k += 1;
            }
        }
        ms += 1;
    }
    // let a last Bye / checksum land, as a frontend would
    for _ in 0..400 {
        link.advance(1);
        ms += 1;
        for n in nets.iter_mut() {
            n.pump(ms);
        }
    }

    let mut r = SimReport {
        mode: o.mode.name(),
        delay: o.delay,
        prediction: if o.mode == Mode::Rollback { o.prediction } else { 0 },
        rtt_ms: o.rtt_ms,
        jitter_ms: o.jitter_ms,
        loss_pct: o.loss_pct,
        frames: o.frames,
        virtual_secs: ms as f64 / 1000.0,
        packets_dropped: link.dropped(),
        ..SimReport::default()
    };
    for (i, net) in nets.iter_mut().enumerate() {
        let (t0, f0) = live_at[i].unwrap_or((ms, 0));
        let secs = (ms.saturating_sub(t0)) as f64 / 1000.0;
        let t = tracks[i].borrow();
        let n = (net.settled_frames().min(o.frames) as usize).min(t.settled.len());
        let errs: Vec<u32> = (0..n)
            .filter_map(|f| {
                let (a, (b, ..)) = (t.shown[f]?, t.settled[f]?);
                Some((a.0 - b.0).unsigned_abs().max((a.1 - b.1).unsigned_abs()))
            })
            .collect();
        let dev = &devs[i];
        let ms_of = |s: u64| s * 1000 / dev.hz;
        let mut p = PeerReport {
            role: if i == 0 { "host" } else { "guest" },
            frames: net.frame(),
            settled: net.settled_frames(),
            fps: if secs > 0.0 { f64::from(net.frame() - f0) / secs } else { 0.0 },
            game_fps,
            stall_pct: 100.0 * stall_ticks[i] as f64 / live_ticks[i].max(1) as f64,
            checks_ok: net.checks_ok(),
            remote_err_max: errs.iter().copied().max().unwrap_or(0),
            remote_err_mean: errs.iter().map(|&e| f64::from(e)).sum::<f64>() / errs.len().max(1) as f64,
            remote_err_over4_pct: 100.0 * errs.iter().filter(|&&e| e > 4).count() as f64 / errs.len().max(1) as f64,
            audio_queue_max_ms: ms_of(dev.max),
            audio_dropped_ms: ms_of(dev.dropped),
            audio_starved_ms: ms_of(dev.starved),
            ..PeerReport::default()
        };
        if let Some(s) = net.rollback().map(|s| s.stats()) {
            p.rollbacks = s.rollbacks;
            p.max_rollback_depth = s.max_rollback_depth;
            p.replayed_frames = s.rollback_frames;
            p.saturated = s.stalled_advances;
            p.wait_recommendations = s.wait_recommendations;
        }
        if let Some(rec) = net.record.as_mut() {
            rec.finish(ms.saturating_sub(t0));
            p.log = Some(rec.snapshot());
        }
        if net.closed() {
            let why = net.last_error.clone().unwrap_or_else(|| net.status());
            if let Some(f) = why.strip_prefix("desync at frame ") {
                r.desync_frame = f.trim().parse().ok();
                r.desync = Some(why);
            } else if r.desync.is_none() {
                r.closed.get_or_insert(why);
            }
        }
        r.peers[i] = p;
    }
    if r.desync.is_some() {
        r.closed = None; // the other peer's "peer quit" is the same event
    }

    // Plain run: one Game, the host's settled pads.
    let (ht, gt) = (tracks[0].borrow(), tracks[1].borrow());
    let n = nets.iter().map(|n| n.settled_frames()).min().unwrap_or(0).min(o.frames);
    let mut plain = sim_game(rom, o.timing, x3, o.zero, load)?;
    for f in 0..n as usize {
        let (Some((_, pads, hh)), Some((_, gpads, gh))) = (ht.settled.get(f).copied().flatten(), gt.settled.get(f).copied().flatten()) else {
            r.plain_mismatch = Some((f as u32, "missing frame"));
            break;
        };
        plain.step(Pads { p1: pads.0, p2: pads.1 });
        let want = state_hash(&plain);
        if gpads != pads {
            r.plain_mismatch = Some((f as u32, "guest pads"));
        } else if hh != want {
            r.plain_mismatch = Some((f as u32, "host"));
        } else if gh != want {
            r.plain_mismatch = Some((f as u32, "guest"));
        }
        if r.plain_mismatch.is_some() {
            break;
        }
        r.plain_frames = f as u32 + 1;
    }
    r.recommendation = recommend(o, game_fps, &r);
    Ok(r)
}

/// Lockstep: both peers wait on each other, so frame `f` needs the peer's
/// frame `f - delay`, which needed ours at `f - 2 * delay`: at most
/// `2 * delay` frames per round trip (measured: delay 3 at 300 ms RTT runs
/// 20 fps). Full speed needs `delay >= one-way trip + jitter` in frames.
/// Rollback: the prediction window must cover how far the remote pad lags,
/// up to a round trip plus jitter (the peers drift apart until wait
/// recommendations pull them back), minus the input delay.
fn recommend(o: &SimOpts, fps: f64, r: &SimReport) -> String {
    let frames = |ms: f64| (ms * fps / 1000.0).ceil() as u32;
    let ls_delay = frames(f64::from(o.rtt_ms) / 2.0 + f64::from(o.jitter_ms)) + 1;
    let rb_delay = u32::from(if o.mode == Mode::Rollback { o.delay } else { mmx_net::DEFAULT_ROLLBACK_DELAY });
    let pred = (frames(f64::from(o.rtt_ms + o.jitter_ms)) + 1).saturating_sub(rb_delay).max(8);
    let stall = r.peers.iter().map(|p| p.stall_pct).fold(0.0, f64::max);
    let now = match o.mode {
        Mode::Lockstep if stall > 2.0 => format!("this run stalled {stall:.0}% of ticks; "),
        Mode::Rollback if r.peers.iter().any(|p| p.saturated > 0) && stall > 2.0 => {
            format!("the prediction window ran full ({stall:.0}% of ticks stalled); ")
        }
        _ => String::new(),
    };
    if ls_delay <= u32::from(mmx_net::MAX_DELAY) {
        format!("{now}at {} ms RTT (+{} ms jitter) lockstep needs delay >= {ls_delay} (max {}): lockstep --delay {ls_delay} is smooth; rollback works with --prediction >= {pred}", o.rtt_ms, o.jitter_ms, mmx_net::MAX_DELAY)
    } else {
        format!("{now}at {} ms RTT (+{} ms jitter) lockstep needs delay >= {ls_delay} (max {}): use rollback with --prediction >= {pred}", o.rtt_ms, o.jitter_ms, mmx_net::MAX_DELAY)
    }
}

pub fn sim_text(r: &SimReport) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "netsim: {} delay {} prediction {}, RTT {} ms, jitter {} ms, loss {}%, {} frames, {:.1} s virtual, {} packets dropped",
        r.mode, r.delay, r.prediction, r.rtt_ms, r.jitter_ms, r.loss_pct, r.frames, r.virtual_secs, r.packets_dropped
    );
    let desync = match (&r.desync, &r.closed) {
        (Some(d), _) => format!("YES ({d})"),
        (None, Some(c)) => format!("no, but the session closed: {c}"),
        _ => "no".into(),
    };
    let _ = writeln!(s, "desync: {desync}");
    for p in &r.peers {
        let _ = writeln!(
            s,
            "{:5}: {} frames, {} settled, {:.2} fps of {:.2} ({:.1}% ticks stalled), {} checks ok",
            p.role, p.frames, p.settled, p.fps, p.game_fps, p.stall_pct, p.checks_ok
        );
        if r.mode == "rollback" {
            let _ = writeln!(
                s,
                "       rollbacks {} (max depth {}, {} frames replayed), prediction window full {} times, {} wait recommendations",
                p.rollbacks, p.max_rollback_depth, p.replayed_frames, p.saturated, p.wait_recommendations
            );
        }
        let _ = writeln!(
            s,
            "       remote player first drawn vs settled: max {} px, mean {:.2} px, {:.1}% of frames > 4 px",
            p.remote_err_max, p.remote_err_mean, p.remote_err_over4_pct
        );
        let _ = writeln!(
            s,
            "       audio: queue max {} ms, dropped {} ms, starved {} ms",
            p.audio_queue_max_ms, p.audio_dropped_ms, p.audio_starved_ms
        );
    }
    let plain = match r.plain_mismatch {
        Some((f, who)) => format!("MISMATCH at frame {f} ({who})"),
        None if r.plain_frames < r.frames => format!("only {} of {} frames settled", r.plain_frames, r.frames),
        None => format!("identical for all {} settled frames on both peers", r.plain_frames),
    };
    let _ = writeln!(s, "plain single-Game run: {plain}");
    let _ = writeln!(s, "recommendation: {}", r.recommendation);
    s
}

pub fn sim_json(r: &SimReport) -> String {
    let q = |v: &str| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""));
    let opt = |v: &Option<String>| v.as_deref().map_or("null".into(), q);
    let peers: Vec<String> = r
        .peers
        .iter()
        .map(|p| {
            format!(
                "{{\"role\":\"{}\",\"frames\":{},\"settled\":{},\"fps\":{:.3},\"game_fps\":{:.3},\"stall_pct\":{:.2},\"checks_ok\":{},\
                 \"rollbacks\":{},\"max_rollback_depth\":{},\"replayed_frames\":{},\"prediction_full\":{},\"wait_recommendations\":{},\
                 \"remote_err_max_px\":{},\"remote_err_mean_px\":{:.3},\"remote_err_over4_pct\":{:.2},\
                 \"audio_queue_max_ms\":{},\"audio_dropped_ms\":{},\"audio_starved_ms\":{}}}",
                p.role, p.frames, p.settled, p.fps, p.game_fps, p.stall_pct, p.checks_ok, p.rollbacks, p.max_rollback_depth,
                p.replayed_frames, p.saturated, p.wait_recommendations, p.remote_err_max, p.remote_err_mean,
                p.remote_err_over4_pct, p.audio_queue_max_ms, p.audio_dropped_ms, p.audio_starved_ms
            )
        })
        .collect();
    format!(
        "{{\"mode\":\"{}\",\"delay\":{},\"prediction\":{},\"rtt_ms\":{},\"jitter_ms\":{},\"loss_pct\":{},\"frames\":{},\
         \"virtual_secs\":{:.3},\"packets_dropped\":{},\"desync\":{},\"desync_frame\":{},\"closed\":{},\"plain_frames\":{},\
         \"plain_mismatch_frame\":{},\"ok\":{},\"peers\":[{}],\"recommendation\":{}}}\n",
        r.mode, r.delay, r.prediction, r.rtt_ms, r.jitter_ms, r.loss_pct, r.frames, r.virtual_secs, r.packets_dropped,
        opt(&r.desync), r.desync_frame.map_or("null".into(), |f| f.to_string()), opt(&r.closed), r.plain_frames,
        r.plain_mismatch.map_or("null".into(), |(f, _)| f.to_string()), r.ok(), peers.join(","), q(&r.recommendation)
    )
}

// ---------------------------------------------------------------- netreplay

#[derive(Debug, Default)]
pub struct Replay {
    pub frames: usize,
    /// Checks inside the pad track, compared against the replay.
    pub checks: usize,
    /// First check where the log's local or remote checksum differs from
    /// the replay: `(frames, local, remote, replay)`.
    pub first_bad: Option<(u32, u64, u64, u64)>,
}

/// Replay the confirmed pad track through a session game and compare the
/// log's checksums (state after N frames) with the replay's.
pub fn replay(rom: &[u8], log: &Log, x3: Option<&ZeroSet>, load: Option<&[u8]>) -> Result<Replay, String> {
    let h = &log.header;
    if h.x3 && x3.is_none() {
        return Err("the session had the X3 Zero set loaded: pass --x3 PATH (or MMX_X3_ROM)".into());
    }
    if h.load.is_some() && load.is_none() {
        return Err(format!("the session loaded {} at frame 0: pass --load STATE", h.load.as_deref().unwrap_or("")));
    }
    let mut g = sim_game(rom, h.timing, if h.x3 { x3 } else { None }, h.zero, load)?;
    if g.rom_info().crc32 != h.rom_crc {
        return Err(format!("ROM CRC {:08x} but the log was recorded with {:08x}", g.rom_info().crc32, h.rom_crc));
    }
    if h.trapset != 0 && trapset_id(&g) != h.trapset {
        return Err(format!(
            "trap set {:016x} but the log's is {:016x}: a different build (NET_REVISION / trap addresses) or X3/Zero flags",
            trapset_id(&g),
            h.trapset
        ));
    }
    let checks: BTreeMap<u32, (u64, u64)> = log
        .events
        .iter()
        .filter_map(|e| match *e {
            Event::Check { frames, local, remote, .. } | Event::Desync { frames, local, remote, .. } => Some((frames, (local, remote))),
            _ => None,
        })
        .collect();
    let mut r = Replay { frames: log.pads.len(), ..Replay::default() };
    for n in 0..=log.pads.len() {
        if let Some(&(local, remote)) = checks.get(&(n as u32)) {
            r.checks += 1;
            let sum = net_checksum(&g);
            if local != sum || remote != sum {
                r.first_bad = Some((n as u32, local, remote, sum));
                break;
            }
        }
        if let Some(&(p1, p2)) = log.pads.get(n) {
            g.step(Pads { p1, p2 });
        }
    }
    Ok(r)
}

/// Stall time, rollback histogram, waits, interruptions, how it ended.
pub fn summary(log: &Log) -> String {
    let mut s = String::new();
    let h = &log.header;
    let secs = log.events.iter().filter_map(|e| if let Event::Time { ms, .. } = e { Some(*ms) } else { None }).max().unwrap_or(0);
    let _ = writeln!(
        s,
        "{} {}: delay {}, prediction {}, check every {} frames, timing {}, x3 {}, zero {}; {} confirmed frames over {}:{:02}",
        h.mode.name(), h.role(), h.delay, h.prediction, h.interval, h.timing.name(), u8::from(h.x3), u8::from(h.zero),
        log.pads.len(), secs / 60_000, secs / 1000 % 60
    );
    let stalls: Vec<(u64, u32, u64)> =
        log.events.iter().filter_map(|e| if let Event::Stall { ms, frame, dur_ms } = *e { Some((ms, frame, dur_ms)) } else { None }).collect();
    let total: u64 = stalls.iter().map(|s| s.2).sum();
    if let Some(&(ms, f, d)) = stalls.iter().max_by_key(|s| s.2) {
        let _ = writeln!(s, "stalls: {} totalling {} ms; longest {d} ms at {}:{:02} (frame {f})", stalls.len(), total, ms / 60_000, ms / 1000 % 60);
    } else {
        let _ = writeln!(s, "stalls: none");
    }
    let mut hist: BTreeMap<u32, u32> = BTreeMap::new();
    for e in &log.events {
        if let Event::Rollback { depth, .. } = e {
            let b = match depth {
                0..=2 => 2,
                3..=4 => 4,
                5..=8 => 8,
                9..=16 => 16,
                _ => 99,
            };
            *hist.entry(b).or_default() += 1;
        }
    }
    if !hist.is_empty() {
        let label = |b: u32| match b {
            2 => "1-2",
            4 => "3-4",
            8 => "5-8",
            16 => "9-16",
            _ => "17+",
        };
        let parts: Vec<String> = hist.iter().map(|(b, n)| format!("{}: {n}", label(*b))).collect();
        let _ = writeln!(s, "rollbacks by depth: {}", parts.join(", "));
    }
    let count = |f: fn(&Event) -> bool| log.events.iter().filter(|e| f(e)).count();
    let waits = count(|e| matches!(e, Event::Wait { .. }));
    let ints = count(|e| matches!(e, Event::Interrupted { .. }));
    let checks = count(|e| matches!(e, Event::Check { local, remote, .. } if local == remote));
    let _ = writeln!(s, "{checks} checksum matches, {waits} wait recommendations, {ints} network interruptions");
    for e in &log.events {
        match e {
            Event::Desync { ms, frames, local, remote } => {
                let _ = writeln!(s, "desync detected at {}:{:02} on the state after frame {frames}: {local:016x} vs {remote:016x}", ms / 60_000, ms / 1000 % 60);
            }
            Event::Closed { ms, reason } => {
                let _ = writeln!(s, "closed at {}:{:02}: {reason}", ms / 60_000, ms / 1000 % 60);
            }
            _ => {}
        }
    }
    s
}

/// One log against the replay: who was right.
pub fn verdict(log: &Log, r: &Replay) -> String {
    let (me, peer) = if log.header.host { ("host", "guest") } else { ("guest", "host") };
    match r.first_bad {
        None => format!("replay of {} frames matches all {} checksums in the log", r.frames, r.checks),
        Some((n, local, remote, sum)) => {
            let who = match (local == sum, remote == sum) {
                (true, _) => me,
                (_, true) => peer,
                _ => "neither",
            };
            if local == remote {
                format!("both peers agreed at frame {n} ({local:016x}) but the replay has {sum:016x}: the replay is not the session's game (build, ROM or flags differ)")
            } else {
                format!("host and guest disagreed at frame {n} (state after {n} frames); the replay matches {who}")
            }
        }
    }
}

/// Two logs of the same session from both peers.
pub fn compare(a: &Log, b: &Log) -> String {
    let mut s = String::new();
    if a.header.host == b.header.host {
        let _ = writeln!(s, "warning: both logs are from the {}", a.header.role());
    }
    let n = a.pads.len().min(b.pads.len());
    match (0..n).find(|&f| a.pads[f] != b.pads[f]) {
        Some(f) => {
            let _ = writeln!(s, "input tracks differ at frame {f}: {:04x?} vs {:04x?} (a netcode bug: confirmed pads must agree)", a.pads[f], b.pads[f]);
        }
        None => {
            let _ = writeln!(s, "input tracks identical for {n} frames ({} / {} recorded)", a.pads.len(), b.pads.len());
        }
    }
    let local = |l: &Log| -> BTreeMap<u32, u64> {
        l.events.iter().filter_map(|e| match *e {
            Event::Check { frames, local, .. } | Event::Desync { frames, local, .. } => Some((frames, local)),
            _ => None,
        }).collect()
    };
    let (la, lb) = (local(a), local(b));
    let both: Vec<u32> = la.keys().filter(|k| lb.contains_key(k)).copied().collect();
    let (ha, hb) = if a.header.host { ("host", "guest") } else { ("guest", "host") };
    match both.iter().find(|k| la[k] != lb[k]) {
        Some(k) => {
            let _ = writeln!(s, "checksums differ at frame {k}: {ha} {:016x}, {hb} {:016x} ({} earlier intervals agree)", la[k], lb[k], both.iter().filter(|x| *x < k).count());
        }
        None => {
            let _ = writeln!(s, "checksums agree at all {} common intervals", both.len());
        }
    }
    s
}

pub fn parse_log(path: &str) -> Result<Log, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    netlog::parse(&text).map_err(|e| format!("{path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmx_core::netgame::synthetic_rom;

    fn script(f: usize, mut p: Pads) -> Pads {
        let f = f as u32;
        p.p1 = ((f / 9).wrapping_mul(2_654_435_761) >> 11) as u16 & 0xFFF0;
        p.p2 = ((f / 7).wrapping_mul(2_654_435_761).wrapping_add(0x7777) >> 11) as u16 & 0xFFF0;
        p
    }

    fn opts(mode: Mode) -> SimOpts {
        SimOpts { frames: 300, record: true, ..SimOpts::new(mode) }
    }

    #[test]
    fn netsim_synthetic_reports() {
        let rom = synthetic_rom();
        for mode in [Mode::Rollback, Mode::Lockstep] {
            let mut o = opts(mode);
            o.prediction = 20;
            let r = netsim(&rom, &o, &script).unwrap();
            let text = sim_text(&r);
            eprintln!("{text}");
            assert!(r.ok(), "{text}");
            assert_eq!(r.plain_frames, 300);
            for p in &r.peers {
                assert!(p.settled >= 300 && p.fps > 0.0 && p.checks_ok >= 4, "{text}");
                assert_eq!(p.audio_dropped_ms, 0, "{text}");
            }
            if mode == Mode::Rollback {
                assert!(r.peers.iter().all(|p| p.rollbacks > 0), "{text}");
            } else {
                assert!(r.peers[0].stall_pct > 10.0, "300 ms RTT lockstep at delay 3 must stall: {text}");
            }
            let json = sim_json(&r);
            for k in ["\"desync\":null", "\"stall_pct\"", "\"remote_err_max_px\"", "\"audio_queue_max_ms\"", "\"prediction_full\"", "\"ok\":true"] {
                assert!(json.contains(k), "{k} in {json}");
            }
            assert!(text.contains("at 300 ms RTT (+40 ms jitter) lockstep needs delay >= 13 (max 8): use rollback with --prediction >= 20"), "{text}");
        }
    }

    #[test]
    fn netsim_detects_forced_desync() {
        for mode in [Mode::Rollback, Mode::Lockstep] {
            let mut o = opts(mode);
            o.corrupt_guest_at = Some(150);
            let r = netsim(&synthetic_rom(), &o, &script).unwrap();
            let text = sim_text(&r);
            assert!(!r.ok() && r.desync.is_some(), "{text}");
            let f = r.desync_frame.unwrap();
            assert!((150..=300).contains(&f), "{text}");
            assert!(text.contains("desync: YES"), "{text}");
        }
    }

    /// netsim --record -> both logs -> netreplay: the replay matches every
    /// checksum; the logs agree with each other; a tampered checksum is
    /// pinned to its frame and the peer that was right.
    #[test]
    fn netreplay_on_netsim_logs() {
        let rom = synthetic_rom();
        for mode in [Mode::Rollback, Mode::Lockstep] {
            let r = netsim(&rom, &opts(mode), &script).unwrap();
            let logs: Vec<Log> = r.peers.iter().map(|p| netlog::parse(p.log.as_deref().unwrap()).unwrap()).collect();
            for l in &logs {
                assert!(l.pads.len() >= 300, "{}", l.pads.len());
                let rep = replay(&rom, l, None, None).unwrap();
                assert!(rep.first_bad.is_none() && rep.checks >= 4, "{rep:?}");
                eprintln!("{}{}", summary(l), verdict(l, &rep));
            }
            let c = compare(&logs[0], &logs[1]);
            assert!(c.contains("input tracks identical") && c.contains("checksums agree"), "{c}");
            // the guest's own checksum at the 3rd check was wrong
            let mut bad = logs[1].clone();
            let k = bad.events.iter().enumerate().filter(|(_, e)| matches!(e, Event::Check { .. })).nth(2).unwrap().0;
            let Event::Check { frames, local, .. } = &mut bad.events[k] else { unreachable!() };
            *local ^= 1;
            let n = *frames;
            let rep = replay(&rom, &bad, None, None).unwrap();
            assert_eq!(rep.first_bad.map(|b| b.0), Some(n));
            let v = verdict(&bad, &rep);
            assert!(v.contains(&format!("disagreed at frame {n}")) && v.ends_with("matches host"), "{v}");
            assert!(compare(&logs[0], &bad).contains(&format!("checksums differ at frame {n}")));
        }
    }
}
