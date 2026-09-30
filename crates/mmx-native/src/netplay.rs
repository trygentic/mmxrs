//! Online co-op for the desktop app: open the WebRTC transport and wrap
//! `mmx_core::netgame::NetPlay` (the per-tick rollback or lockstep driver
//! shared with the web build). The window loop calls [`Net::tick_audio`] while the audio queue is
//! below target ([`Net::tick`] at `Game::fps` without an audio device).

use mmx_core::netgame::{Mode, NetConfig, NetPlay};
use mmx_core::Game;
use mmx_net::ConnectStage;

#[cfg(feature = "netplay")]
pub type NetTransport = mmx_net::MatchboxTransport;
// ponytail: without the transport nothing ever constructs a session; the
// loopback type only keeps the code below compiling.
#[cfg(not(feature = "netplay"))]
pub type NetTransport = mmx_net::LoopbackTransport;

/// Keep pumping this long after a quit so the Bye leaves the machine.
const GOODBYE_FLUSH_MS: u64 = 200;

pub struct Net {
    pub play: NetPlay<NetTransport>,
    reported: Option<ConnectStage>,
    reported_end: bool,
    said_running: bool,
    epoch: std::time::Instant,
    /// `--net-record`: the log file and when it was last appended to.
    record: Option<(std::fs::File, u64)>,
}

/// `--coop-host/--coop-join ROOM [--signal] [--ice] [--mode] [--delay] [--prediction] [--net-record]` -> a connecting session.
#[allow(clippy::too_many_arguments)]
pub fn connect(
    rom: &[u8],
    host: bool,
    room: &str,
    signal: &str,
    ice: Option<&str>,
    mode: Mode,
    delay: u8,
    prediction: Option<u8>,
    x3: Option<&mmx_core::zero_data::ZeroSet>,
    zero: bool,
    timing: mmx_core::Timing,
    record: Option<&str>,
) -> Result<Net, String> {
    let mut cfg = NetPlay::<NetTransport>::config(rom, host, mode, delay, x3, zero, timing)?;
    if let (NetConfig::Rollback(c), Some(p)) = (&mut cfg, prediction) {
        c.max_prediction = p; // local policy, not negotiated
    }
    let url = mmx_net::room_url(signal, room).map_err(|e| e.to_string())?;
    let ice = ice
        .map(mmx_net::IceConfig::parse)
        .transpose()
        .map_err(|e| format!("--ice: {e}"))?;
    let record = record
        .map(|p| std::fs::File::create(p).map(|f| (f, 0)).map_err(|e| format!("--net-record {p}: {e}")))
        .transpose()?;
    let t = open(&url, ice)?;
    let mut play = NetPlay::new(cfg, rom.to_vec(), t, room)?;
    if record.is_some() {
        play.record = Some(mmx_core::netlog::Recorder::new());
    }
    Ok(Net {
        play,
        reported: None,
        reported_end: false,
        said_running: false,
        epoch: std::time::Instant::now(),
        record,
    })
}

#[cfg(feature = "netplay")]
fn open(url: &str, ice: Option<mmx_net::IceConfig>) -> Result<NetTransport, String> {
    Ok(mmx_net::MatchboxTransport::connect(url, ice))
}

#[cfg(not(feature = "netplay"))]
fn open(_: &str, _: Option<mmx_net::IceConfig>) -> Result<NetTransport, String> {
    Err("this build has no netplay transport: rebuild with the default `netplay` feature".into())
}

impl Net {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// One tick; returns frames simulated. Logs each connect stage once.
    pub fn tick(&mut self, game: &mut Game, pad: u16) -> u32 {
        let n = self.play.tick(game, pad, self.now_ms());
        self.report();
        n
    }

    /// [`NetPlay::tick_audio`]: frames simulated and one frame of audio.
    pub fn tick_audio(&mut self, game: &mut Game, pad: u16) -> (u32, Vec<(f32, f32)>) {
        let r = self.play.tick_audio(game, pad, self.now_ms());
        self.report();
        r
    }

    /// Append the session log's new lines (every second, and when done).
    fn flush_record(&mut self, force: bool) {
        use std::io::Write;
        let now = self.now_ms();
        let (Some((file, last)), Some(rec)) = (self.record.as_mut(), self.play.record.as_mut()) else {
            return;
        };
        if force || now >= *last + 1000 {
            *last = now;
            if let Err(e) = file.write_all(rec.drain().as_bytes()) {
                eprintln!("netplay: --net-record: {e}");
                self.record = None;
            }
        }
    }

    fn report(&mut self) {
        self.flush_record(false);
        let stage = mmx_net::Transport::stage(self.play.transport());
        if self.reported != Some(stage) {
            self.reported = Some(stage);
            if stage != ConnectStage::Ended {
                eprintln!("netplay: {}", stage.describe());
            }
        }
        if self.play.live() && self.reported != Some(ConnectStage::Ended) && !self.said_running {
            self.said_running = true;
            eprintln!("netplay: session running, restarted from power-on");
        }
        if !self.reported_end && self.play.closed() {
            self.reported_end = true;
            eprintln!("netplay: {}", self.play.status());
            self.flush_record(true);
        }
    }

    /// Window-title suffix.
    pub fn title(&self) -> String {
        format!(" [NET {}]", self.play.status())
    }

    /// Send Bye and pump for [`GOODBYE_FLUSH_MS`] (blocks that long).
    pub fn close_and_flush(&mut self) {
        self.play.close();
        let until = self.now_ms() + GOODBYE_FLUSH_MS;
        while self.now_ms() < until {
            let now = self.now_ms();
            self.play.pump(now);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.flush_record(true);
    }
}

/// Desktop end to end: in-process mmx-signal, two `NetPlay<MatchboxTransport>`
/// over 127.0.0.1 (ICE none), 300 settled frames without a desync, in each mode.
#[cfg(all(test, feature = "netplay"))]
mod tests {
    use super::*;
    use mmx_core::netgame::{session_game, synthetic_rom};
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    fn start_signal() -> SocketAddr {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let mut server = mmx_signal::build_server(SocketAddr::from(([127, 0, 0, 1], 0)), false);
                let addr = server.bind().expect("bind signalling server");
                tx.send(addr).expect("report address");
                let _ = server.serve().await;
            });
        });
        rx.recv_timeout(Duration::from_secs(10)).expect("signalling server did not start")
    }

    fn run_pair(rom: &[u8], room: &str, frames: u32, mode: Mode) {
        let url = format!("ws://{}", start_signal());
        // Rollback at delay 0: every remote press lands after its frame ran,
        // so pad changes are corrected by real rollbacks. Lockstep: default 3.
        let delay = match mode {
            Mode::Rollback => 0,
            Mode::Lockstep => mode.default_delay(),
        };
        // Both peers record (--net-record): the logs must agree.
        let log = |host: bool| std::env::temp_dir().join(format!("mmxrs-{room}-{}-{}.log", std::process::id(), if host { "host" } else { "guest" }));
        let mk = |host: bool| {
            let path = log(host);
            connect(rom, host, room, &url, Some("none"), mode, delay, None, None, false, mmx_core::Timing::Auto, path.to_str()).expect("connect")
        };
        let mut peers = [
            (mk(true), session_game(rom, 0, mmx_core::Timing::Auto).unwrap(), 0x1111u32),
            (mk(false), session_game(rom, 0, mmx_core::Timing::Auto).unwrap(), 0x7777u32),
        ];
        let clock = Instant::now();
        let mut next = Duration::ZERO;
        while peers.iter().any(|(n, ..)| n.play.settled_frames() < frames) {
            assert!(
                clock.elapsed() < Duration::from_secs(120),
                "timeout: {} / {}",
                peers[0].0.play.status(),
                peers[1].0.play.status()
            );
            if clock.elapsed() < next {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            next += Duration::from_millis(16);
            for (net, game, seed) in peers.iter_mut() {
                let f = net.play.frame();
                let pad = ((f / 12).wrapping_mul(2_654_435_761).wrapping_add(*seed) >> 13) as u16 & 0xFFF0;
                net.tick(game, pad);
                assert!(net.play.last_error.is_none(), "{}", net.play.status());
            }
        }
        for (net, ..) in &peers {
            eprintln!("{}: {} checks ok", net.play.status(), net.play.checks_ok());
            assert!(net.play.checks_ok() >= frames / 60 - 1, "{}", net.play.status());
        }
        peers[0].0.close_and_flush();
        let quit_by = Instant::now();
        while peers[1].0.play.live() {
            assert!(quit_by.elapsed() < Duration::from_secs(20), "guest never saw the quit");
            let (net, game, _) = &mut peers[1];
            net.tick(game, 0);
            std::thread::sleep(Duration::from_millis(10));
        }
        let logs: Vec<mmx_core::netlog::Log> = [true, false]
            .map(|h| {
                let text = std::fs::read_to_string(log(h)).expect("session log written");
                let _ = std::fs::remove_file(log(h));
                mmx_core::netlog::parse(&text).expect("session log parses")
            })
            .to_vec();
        let n = logs[0].pads.len().min(logs[1].pads.len());
        assert!(n as u32 >= frames - 10, "{n} confirmed frames logged");
        assert_eq!(logs[0].pads[..n], logs[1].pads[..n]);
        assert!(logs[0].header.host && !logs[1].header.host && logs[0].header.mode == mode);
        let checks = |l: &mmx_core::netlog::Log| -> Vec<(u32, u64)> {
            l.events.iter().filter_map(|e| match *e {
                mmx_core::netlog::Event::Check { frames, local, remote, .. } => {
                    assert_eq!(local, remote);
                    Some((frames, local))
                }
                _ => None,
            }).collect()
        };
        let (a, b) = (checks(&logs[0]), checks(&logs[1]));
        let want = (frames / 60 - 1) as usize;
        assert!(a.len() >= want && a.iter().filter(|c| b.contains(c)).count() >= want, "{a:?} / {b:?}");
        assert!(logs.iter().all(|l| l.events.iter().any(|e| matches!(e, mmx_core::netlog::Event::Closed { .. }))));
    }

    #[test]
    fn rollback_pair_over_localhost_matchbox_synthetic() {
        run_pair(&synthetic_rom(), "native-e2e", 300, Mode::Rollback);
    }

    #[test]
    fn lockstep_pair_over_localhost_matchbox_synthetic() {
        run_pair(&synthetic_rom(), "native-e2e-ls", 300, Mode::Lockstep);
    }

    /// Same on the real ROM with co-op on; skipped unless `MMX_ROM` is set.
    #[test]
    fn rollback_pair_over_localhost_matchbox_rom() {
        let Some(path) = std::env::var_os("MMX_ROM").filter(|p| !p.is_empty()) else {
            eprintln!("skipped: MMX_ROM not set");
            return;
        };
        let rom = std::fs::read(path).expect("read MMX_ROM");
        let frames = if cfg!(debug_assertions) { 120 } else { 600 };
        run_pair(&rom, "native-e2e-rom", frames, Mode::Rollback);
        run_pair(&rom, "native-e2e-rom-ls", frames, Mode::Lockstep);
    }
}
