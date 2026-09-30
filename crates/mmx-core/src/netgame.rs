//! Netplay glue shared by `mmx-native` and `mmx-web`: [`Game`] as an
//! [`mmx_net::RollbackGame`] and [`NetPlay`], the per-tick driver both
//! frontends call, in either [`Mode`]: rollback (z2rs's `RollbackLink` +
//! `rollback_tick`, shrunk) or input-delay lockstep (z2rs's `step_session`).
//! See `docs/netplay.md` "Frontend wiring".
//!
//! Save states for rollback are [`Game::save_state`] plus the co-op state
//! ([`coop::CoopState`], P2's shadow block), which `save_state` leaves out:
//! without it a rollback would rewind P1 and the world but not P2.

use mmx_net::{
    hash_state, CloseReason, ConnectStage, Role, RollbackConfig, RollbackError, RollbackEvent,
    RollbackGame, RollbackRequest, RollbackSession, RollbackState, Session, SessionConfig,
    SessionEvent, SessionState, StateRing, Transport, COOP_TWO_PLAYERS, DEFAULT_ROLLBACK_DELAY,
    MAX_DELAY, MAX_ROLLBACK_DELAY,
};

use crate::netlog::{Event, Header, Recorder};
use crate::zero_data::ZeroSet;
use crate::{Timing, coop, Game, Pads, RomError};

/// Bump when anything that changes simulation between two builds changes
/// outside the trap addresses (mixed into [`trapset_id`]).
const NET_REVISION: u64 = 2;

/// Rollback save state: `u32` LE length of the encoded [`coop::CoopState`],
/// that encoding, then [`Game::save_state`].
pub fn net_save(game: &Game) -> Vec<u8> {
    let cfg = bincode::config::standard();
    let coop = bincode::encode_to_vec(&game.coop, cfg).expect("coop state encodes");
    let mut v = Vec::with_capacity(4 + coop.len() + 512 * 1024);
    v.extend_from_slice(&(coop.len() as u32).to_le_bytes());
    v.extend_from_slice(&coop);
    v.extend_from_slice(&game.save_state());
    v
}

/// Inverse of [`net_save`].
pub fn net_load(game: &mut Game, s: &[u8]) -> Result<(), String> {
    let n = s
        .get(..4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or("net state too short")?;
    let coop_bytes = s.get(4..4 + n).ok_or("net state truncated")?;
    let (coop, _): (coop::CoopState, _) =
        bincode::decode_from_slice(coop_bytes, bincode::config::standard())
            .map_err(|e| e.to_string())?;
    game.load_state(&s[4 + n..])?;
    game.coop = coop;
    Ok(())
}

/// Desync checksum: FNV-1a 64 over WRAM `$7E:0000..$2000` and P2's shadow block.
pub fn net_checksum(game: &Game) -> u64 {
    hash_state(&[&game.wram()[..0x2000], &game.coop.p2])
}

/// Handshake trap-set id: FNV over the sorted trap addresses,
/// [`NET_REVISION`], whether the X3 Zero set is loaded (`Game::set_zero_set`:
/// it changes P2's OAM, so both peers must agree) and whether P1 plays as
/// Zero (`Game::set_p1_zero`). Peers with different ids refuse to start.
pub fn trapset_id(game: &Game) -> u64 {
    let mut addrs: Vec<u32> = game.traps.addrs().collect();
    addrs.sort_unstable();
    let bytes: Vec<u8> = addrs.iter().flat_map(|a| a.to_le_bytes()).collect();
    hash_state(&[
        &NET_REVISION.to_le_bytes(),
        &bytes,
        &[u8::from(game.zero_set().is_some()), u8::from(game.p1_zero()), game.timing() as u8],
    ])
}

/// The game a session plays: power-on, co-op traps registered, co-op on when
/// the session's flags say so. Both peers build exactly this on `Running`.
pub fn session_game(rom: &[u8], coop_flags: u32, timing: Timing) -> Result<Game, RomError> {
    let mut g = Game::from_rom_timing(rom.to_vec(), timing)?;
    g.coop.enabled = coop_flags & COOP_TWO_PLAYERS != 0;
    coop::register_coop_traps(&mut g);
    Ok(g)
}

impl RollbackGame for Game {
    type State = Vec<u8>;
    fn save(&self) -> Vec<u8> {
        net_save(self)
    }
    fn load(&mut self, state: &Vec<u8>) {
        net_load(self, state).expect("own rollback state loads");
    }
    fn advance(&mut self, pad1: u16, pad2: u16) {
        self.step(Pads { p1: pad1, p2: pad2 });
    }
    fn checksum(&self) -> u64 {
        net_checksum(self)
    }
}

/// Status-line text for a connect stage (out of four, as in docs/netplay.md).
pub fn stage_label(stage: ConnectStage, room: &str) -> String {
    match stage {
        ConnectStage::Signalling => "1/4 connecting to the signal server".into(),
        ConnectStage::WaitingForPeer => format!("2/4 waiting for the other player in {room}"),
        ConnectStage::EstablishingLink => "3/4 establishing the peer link".into(),
        ConnectStage::Connected | ConnectStage::Ended => "4/4 connected, starting".into(),
    }
}

/// How a [`NetPlay`] session keeps the peers in step.
///
/// The mode is not negotiated: both peers must pick the same one. The two
/// sessions announce different protocol versions in their Hello, so a
/// rollback peer meeting a lockstep peer refuses to start with
/// `mismatch: NetMode` (docs/netplay.md "Frontend wiring").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// [`RollbackSession`]: the local player never waits; a mispredicted
    /// remote pad is corrected by re-simulating, so the remote player can
    /// visibly snap. Input delay `0..=MAX_ROLLBACK_DELAY`.
    #[default]
    Rollback,
    /// Input-delay lockstep ([`Session`]): a frame steps only once both pads
    /// are known. Smooth, but the game waits when the remote pad is late.
    /// Input delay `0..=MAX_DELAY`.
    Lockstep,
}

impl Mode {
    /// `"rollback"` / `"lockstep"`.
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim() {
            "rollback" => Some(Mode::Rollback),
            "lockstep" => Some(Mode::Lockstep),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Rollback => "rollback",
            Mode::Lockstep => "lockstep",
        }
    }

    /// Input delay when none is given: 2 frames for rollback, 3 (50 ms) for lockstep.
    pub fn default_delay(self) -> u8 {
        match self {
            Mode::Rollback => DEFAULT_ROLLBACK_DELAY,
            Mode::Lockstep => 3,
        }
    }

    /// Largest accepted input delay.
    pub fn max_delay(self) -> u8 {
        match self {
            Mode::Rollback => MAX_ROLLBACK_DELAY,
            Mode::Lockstep => MAX_DELAY,
        }
    }
}

/// Handshake config for either mode (see [`NetPlay::config`]).
#[derive(Debug, Clone)]
pub enum NetConfig {
    Rollback(RollbackConfig),
    Lockstep(SessionConfig),
}

impl NetConfig {
    pub fn mode(&self) -> Mode {
        match self {
            NetConfig::Rollback(_) => Mode::Rollback,
            NetConfig::Lockstep(_) => Mode::Lockstep,
        }
    }

    /// Desync-check spacing in frames (rollback `check_interval`, lockstep
    /// `hash_interval`; the host's wins).
    pub fn set_check_interval(&mut self, frames: u16) {
        match self {
            NetConfig::Rollback(c) => c.check_interval = frames,
            NetConfig::Lockstep(c) => c.hash_interval = frames,
        }
    }
}

// One per session, never moved around: boxing buys nothing.
#[allow(clippy::large_enum_variant)]
enum Engine<T: Transport> {
    Rollback {
        session: RollbackSession<T>,
        ring: StateRing<Vec<u8>>,
        skip: u32,
        interrupted: bool,
    },
    Lockstep {
        session: Session,
        transport: T,
    },
}

/// One simulated frame, for [`NetPlay::observe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Simulated {
    pub frame: u32,
    pub pad1: u16,
    pub pad2: u16,
    /// Re-simulation after a rollback (the frame was shown before).
    pub replay: bool,
}

/// Callback run after every simulated frame (see [`NetPlay::observe`]).
pub type Observer = Box<dyn FnMut(&Game, Simulated)>;

/// A session in either [`Mode`] plus everything a frontend tick needs: the
/// ROM (to rebuild the game when the session starts), rollback's state ring
/// and skip/interrupt bookkeeping.
pub struct NetPlay<T: Transport> {
    engine: Engine<T>,
    rom: Vec<u8>,
    pub room: String,
    /// The session started and the game was rebuilt from power-on.
    pub started: bool,
    /// Why the session ended (desync, peer left, ...), for the status line.
    pub last_error: Option<String>,
    /// Applied to the rebuilt game (the frontend's audio device rate).
    pub audio_rate: Option<u32>,
    /// Called after every simulated frame, rollback replays included
    /// (`xtask netsim` measures the remote player's snapping with it).
    pub observe: Option<Observer>,
    /// Session log (`--net-record`, docs/netplay.md "Session logs"): set
    /// before the session starts; written from `Running` on, times in ms
    /// since then.
    pub record: Option<Recorder>,
    /// Rollback prediction window (0 in lockstep), for the log header.
    prediction: u8,
    /// `now_ms` at `Running`, and at the latest tick.
    t0_ms: u64,
    last_ms: u64,
}

impl<T: Transport> NetPlay<T> {
    /// Handshake config for `rom`: ROM CRC32, trap-set id of a session game
    /// (with the X3 Zero set when `x3`, P1 as Zero when `p1_zero`), `COOP_TWO_PLAYERS`, input delay
    /// `delay` (host's wins; range per [`Mode::max_delay`]). The session game
    /// takes the set from the frontend's game on start, so load it before
    /// connecting.
    pub fn config(
        rom: &[u8],
        host: bool,
        mode: Mode,
        delay: u8,
        x3: Option<&ZeroSet>,
        p1_zero: bool,
        timing: Timing,
    ) -> Result<NetConfig, String> {
        if delay > mode.max_delay() {
            return Err(format!("{} input delay must be 0-{}", mode.name(), mode.max_delay()));
        }
        let mut g = session_game(rom, COOP_TWO_PLAYERS, timing).map_err(|e| e.to_string())?;
        g.set_zero_set(x3.cloned());
        g.set_p1_zero(p1_zero)?;
        let (crc, traps) = (g.rom_info().crc32, trapset_id(&g));
        Ok(match mode {
            Mode::Rollback => {
                let mut cfg = if host {
                    RollbackConfig::host(crc, traps, COOP_TWO_PLAYERS)
                } else {
                    RollbackConfig::guest(crc, traps, COOP_TWO_PLAYERS)
                };
                cfg.input_delay = delay;
                NetConfig::Rollback(cfg)
            }
            Mode::Lockstep => {
                let mut cfg = if host {
                    SessionConfig::host(crc, traps, COOP_TWO_PLAYERS)
                } else {
                    SessionConfig::guest(crc, traps, COOP_TWO_PLAYERS)
                };
                cfg.delay = delay;
                NetConfig::Lockstep(cfg)
            }
        })
    }

    pub fn new(cfg: NetConfig, rom: Vec<u8>, transport: T, room: &str) -> Result<Self, String> {
        let prediction = match &cfg {
            NetConfig::Rollback(c) => c.max_prediction,
            NetConfig::Lockstep(_) => 0,
        };
        let engine = match cfg {
            NetConfig::Rollback(cfg) => Engine::Rollback {
                ring: StateRing::new(usize::from(cfg.max_prediction) + 2),
                session: RollbackSession::new(cfg, transport).map_err(|e| e.to_string())?,
                skip: 0,
                interrupted: false,
            },
            NetConfig::Lockstep(cfg) => Engine::Lockstep {
                session: Session::new(cfg).map_err(|e| e.to_string())?,
                transport,
            },
        };
        Ok(NetPlay {
            engine,
            rom,
            room: room.into(),
            started: false,
            last_error: None,
            audio_rate: None,
            observe: None,
            record: None,
            prediction,
            t0_ms: 0,
            last_ms: 0,
        })
    }

    pub fn mode(&self) -> Mode {
        match self.engine {
            Engine::Rollback { .. } => Mode::Rollback,
            Engine::Lockstep { .. } => Mode::Lockstep,
        }
    }

    /// The rollback session (None in lockstep).
    pub fn rollback(&self) -> Option<&RollbackSession<T>> {
        match &self.engine {
            Engine::Rollback { session, .. } => Some(session),
            Engine::Lockstep { .. } => None,
        }
    }

    /// The lockstep session (None in rollback).
    pub fn lockstep(&self) -> Option<&Session> {
        match &self.engine {
            Engine::Lockstep { session, .. } => Some(session),
            Engine::Rollback { .. } => None,
        }
    }

    pub fn transport(&self) -> &T {
        match &self.engine {
            Engine::Rollback { session, .. } => session.transport(),
            Engine::Lockstep { transport, .. } => transport,
        }
    }

    /// Next frame this peer will simulate for the first time.
    pub fn frame(&self) -> u32 {
        match &self.engine {
            Engine::Rollback { session, .. } => session.frame(),
            Engine::Lockstep { session, .. } => session.frame(),
        }
    }

    /// Frames that can no longer change (rollback: final on both peers;
    /// lockstep: every stepped frame).
    pub fn settled_frames(&self) -> u32 {
        match &self.engine {
            Engine::Rollback { session, .. } => session.final_frame(),
            Engine::Lockstep { session, .. } => session.frame(),
        }
    }

    /// Desync checks that matched so far.
    pub fn checks_ok(&self) -> u32 {
        match &self.engine {
            Engine::Rollback { session, .. } => session.stats().checksums_ok,
            Engine::Lockstep { session, .. } => session.stats().hashes_ok,
        }
    }

    /// The session ended (quit, peer left, mismatch, desync, ...).
    pub fn closed(&self) -> bool {
        match &self.engine {
            Engine::Rollback { session, .. } => session.state() == RollbackState::Closed,
            Engine::Lockstep { session, .. } => {
                matches!(session.state(), SessionState::Closed | SessionState::Desynced)
            }
        }
    }

    /// The session drives the game (started and not closed). While false the
    /// frontend plays locally; nothing played then reaches the session.
    pub fn live(&self) -> bool {
        self.started && !self.closed()
    }

    pub fn is_host(&self) -> bool {
        let role = match &self.engine {
            Engine::Rollback { session, .. } => session.role(),
            Engine::Lockstep { session, .. } => session.role(),
        };
        role == Role::Host
    }

    /// Pump the network only (receive, timers, send), e.g. to flush a Bye.
    pub fn pump(&mut self, now_ms: u64) {
        match &mut self.engine {
            Engine::Rollback { session, .. } => session.poll(now_ms),
            Engine::Lockstep { session, transport } => session.update(transport, now_ms),
        }
    }

    /// One frontend tick. Returns frames simulated (rollback replays
    /// included); present once afterwards when non-zero. Before the session
    /// is live it only pumps the network (and rebuilds `*game` when the
    /// session starts).
    ///
    /// Rollback: add this peer's pad, advance, run the requests on `game`
    /// (replayed frames' audio dropped). Lockstep: latch the pad, step the
    /// next frame if both pads are known, else return 0 (the game waits).
    pub fn tick(&mut self, game: &mut Game, pad: u16, now_ms: u64) -> u32 {
        let (f0, was_live) = (self.frame(), self.live());
        let skipping = matches!(self.engine, Engine::Rollback { skip, .. } if skip > 0);
        self.last_ms = now_ms;
        let n = match self.engine {
            Engine::Rollback { .. } => self.tick_rollback(game, pad, now_ms),
            Engine::Lockstep { .. } => self.tick_lockstep(game, pad, now_ms),
        };
        self.log_tick(now_ms, f0, was_live && !skipping);
        n
    }

    /// Recorder bookkeeping after a tick: compared checksums, settled pads,
    /// time marks and stalls (a live, non-skipping tick with no new frame).
    fn log_tick(&mut self, now_ms: u64, f0: u32, could_step: bool) {
        // Checksums are taken before a frame in rollback, after it in lockstep.
        let (checks, after) = match &mut self.engine {
            Engine::Rollback { session, .. } => (session.take_checks(), 0),
            Engine::Lockstep { session, .. } => (session.take_checks(), 1),
        };
        let (frame, settled, live) = (self.frame(), self.settled_frames(), self.live());
        let Some(rec) = self.record.as_mut() else { return };
        let ms = now_ms.saturating_sub(self.t0_ms);
        for (f, local, remote) in checks {
            rec.event(&Event::Check { ms, frames: f + after, local, remote });
        }
        rec.settle(settled);
        rec.tick(ms, frame, frame > f0, could_step && live && frame == f0);
    }

    fn log(&mut self, e: Event) {
        if let Some(r) = self.record.as_mut() {
            r.event(&e);
        }
    }

    /// Log the session start (header) and remember the time origin.
    fn log_start(&mut self, game: &Game, now_ms: u64, delay: u8, interval: u16) {
        self.t0_ms = now_ms;
        let h = Header {
            rom_crc: game.rom_info().crc32,
            trapset: trapset_id(game),
            mode: self.mode(),
            host: self.is_host(),
            delay,
            prediction: self.prediction,
            interval,
            timing: game.timing(),
            x3: game.zero_set().is_some(),
            zero: game.p1_zero(),
            load: None,
        };
        if let Some(r) = self.record.as_mut() {
            r.start(&h);
        }
    }

    fn log_closed(&mut self, reason: String) {
        let ms = self.last_ms.saturating_sub(self.t0_ms);
        if let Some(r) = self.record.as_mut() {
            r.event(&Event::Closed { ms, reason });
            r.finish(ms);
        }
    }

    /// [`NetPlay::tick`] for a frontend whose audio device is the clock:
    /// call it while the device queue is below target (a few per wake-up at
    /// most). Returns the frames simulated and exactly one frame of audio:
    /// the new frame's samples, or silence when the tick stepped nothing
    /// (wait recommendation, lockstep stall), so each tick spends one frame
    /// of device time and the queue can neither run away nor starve.
    pub fn tick_audio(&mut self, game: &mut Game, pad: u16, now_ms: u64) -> (u32, Vec<(f32, f32)>) {
        let n = self.tick(game, pad, now_ms);
        let mut pcm = game.take_audio();
        if pcm.is_empty() {
            let hz = f64::from(self.audio_rate.unwrap_or(48_000));
            pcm = vec![(0.0, 0.0); (hz / game.fps()).round() as usize];
        }
        (n, pcm)
    }

    fn tick_rollback(&mut self, game: &mut Game, pad: u16, now_ms: u64) -> u32 {
        let live = self.live();
        let Engine::Rollback { session, ring, skip, .. } = &mut self.engine else {
            unreachable!()
        };
        let mut n = 0;
        if !live {
            session.poll(now_ms);
        } else if *skip > 0 {
            *skip -= 1;
            session.poll(now_ms);
        } else {
            session.add_local_input(pad);
            let f0 = session.frame();
            let err = match session.advance(now_ms) {
                Ok(reqs) => {
                    if let (Some(RollbackRequest::LoadState { frame }), Some(r)) = (reqs.first(), self.record.as_mut()) {
                        r.event(&Event::Rollback { frame: f0, depth: f0 - frame });
                    }
                    apply_rollback(session, ring, game, &reqs, &mut n, &mut self.observe, &mut self.record).err()
                }
                Err(RollbackError::Closed(_)) => None,
                Err(e) => Some(e.to_string()),
            };
            if let Some(e) = err {
                self.last_error = Some(e);
                session.close();
            }
        }
        self.rollback_events(game, now_ms);
        n
    }

    fn rollback_events(&mut self, game: &mut Game, now_ms: u64) {
        let Engine::Rollback { session, .. } = &mut self.engine else {
            return;
        };
        let (events, frame) = (session.events(), session.frame());
        let ms = now_ms.saturating_sub(self.t0_ms);
        for ev in events {
            let Engine::Rollback { session, skip, interrupted, .. } = &mut self.engine else {
                return;
            };
            match ev {
                RollbackEvent::Running { coop_flags, input_delay, check_interval, .. } => {
                    match start_game(&self.rom, coop_flags, self.audio_rate, game) {
                        Ok(()) => {
                            self.started = true;
                            self.log_start(game, now_ms, input_delay, check_interval);
                        }
                        Err(e) => {
                            self.last_error = Some(e);
                            session.close();
                        }
                    }
                }
                RollbackEvent::WaitRecommendation { skip_frames } => {
                    *skip = u32::from(skip_frames);
                    self.log(Event::Wait { ms, frame, skip: skip_frames });
                }
                RollbackEvent::NetworkInterrupted { .. } => {
                    *interrupted = true;
                    self.log(Event::Interrupted { ms, frame });
                }
                RollbackEvent::NetworkResumed => {
                    *interrupted = false;
                    self.log(Event::Resumed { ms, frame });
                }
                RollbackEvent::DesyncDetected { frame, local, remote } => {
                    self.last_error = Some(format!("desync at frame {frame}"));
                    self.log(Event::Desync { ms, frames: frame, local, remote });
                }
                RollbackEvent::Disconnected { reason } => {
                    self.last_error.get_or_insert_with(|| reason.to_string());
                    self.log_closed(reason.to_string());
                }
                RollbackEvent::Synchronizing { .. } => {}
            }
        }
    }

    fn tick_lockstep(&mut self, game: &mut Game, pad: u16, now_ms: u64) -> u32 {
        let live = self.live();
        let Engine::Lockstep { session, transport } = &mut self.engine else {
            unreachable!()
        };
        session.update(transport, now_ms);
        let mut n = 0;
        if live {
            session.latch_local(pad);
            if let Some((frame, pad1, pad2)) = session.poll() {
                game.step(Pads { p1: pad1, p2: pad2 });
                n = 1;
                if let Some(r) = self.record.as_mut() {
                    r.simulated(frame, pad1, pad2);
                }
                if let Some(f) = self.observe.as_mut() {
                    f(game, Simulated { frame, pad1, pad2, replay: false });
                }
                if session.needs_hash(frame) {
                    session.report_hash(frame, net_checksum(game));
                }
            }
            // Send the freshly latched pad now rather than next tick.
            session.update(transport, now_ms);
        }
        self.lockstep_events(game, now_ms);
        n
    }

    fn lockstep_events(&mut self, game: &mut Game, now_ms: u64) {
        let Engine::Lockstep { session, .. } = &mut self.engine else {
            return;
        };
        let ms = now_ms.saturating_sub(self.t0_ms);
        for ev in session.take_events() {
            let Engine::Lockstep { session, .. } = &mut self.engine else {
                return;
            };
            match ev {
                SessionEvent::Started { coop_flags, delay, hash_interval, .. } => {
                    match start_game(&self.rom, coop_flags, self.audio_rate, game) {
                        Ok(()) => {
                            self.started = true;
                            self.log_start(game, now_ms, delay, hash_interval);
                        }
                        Err(e) => {
                            self.last_error = Some(e);
                            session.close();
                        }
                    }
                }
                SessionEvent::Desync { frame, local, remote } => {
                    self.last_error = Some(format!("desync at frame {frame}"));
                    self.log(Event::Desync { ms, frames: frame + 1, local, remote });
                }
                SessionEvent::Closed(reason) => {
                    self.last_error.get_or_insert_with(|| reason.to_string());
                    self.log_closed(reason.to_string());
                }
                SessionEvent::PeerHello { .. }
                | SessionEvent::Stalled { .. }
                | SessionEvent::Resumed { .. } => {}
            }
        }
    }

    /// One-line status: mode and role, then connect stage, running stats
    /// ("waiting for peer" while a lockstep frame is held up), or why it closed.
    pub fn status(&self) -> String {
        let who = if self.is_host() { "host" } else { "guest" };
        let closed = || {
            let reason = match &self.engine {
                Engine::Rollback { session, .. } => session.close_reason(),
                Engine::Lockstep { session, .. } => session.close_reason(),
            };
            let why = self
                .last_error
                .clone()
                .or_else(|| reason.map(CloseReason::to_string))
                .unwrap_or_default();
            format!("closed ({why})")
        };
        let rtt = |ms: Option<u32>| ms.map_or("-".into(), |ms| format!("{ms}ms"));
        let body = match &self.engine {
            Engine::Rollback { session: s, interrupted, .. } => match s.state() {
                RollbackState::Idle => stage_label(s.transport().stage(), &self.room),
                RollbackState::Synchronizing => stage_label(ConnectStage::Connected, &self.room),
                RollbackState::Running => {
                    let st = s.stats();
                    format!(
                        "f={} d={} rtt={} rollbacks={} pred={}/{}{}",
                        st.frame,
                        s.input_delay(),
                        rtt(st.rtt_ms),
                        st.rollbacks,
                        st.prediction_depth,
                        st.max_prediction_depth,
                        if *interrupted { " INTERRUPTED" } else { "" }
                    )
                }
                RollbackState::Closed => closed(),
            },
            Engine::Lockstep { session: s, transport } => match s.state() {
                SessionState::Idle => stage_label(transport.stage(), &self.room),
                SessionState::Handshake => stage_label(ConnectStage::Connected, &self.room),
                SessionState::Running | SessionState::Stalled => {
                    let st = s.stats();
                    let wait = match s.state() {
                        SessionState::Stalled => {
                            format!(" waiting for peer ({:.1}s)", st.stall_ms as f64 / 1000.0)
                        }
                        _ => String::new(),
                    };
                    format!("f={} d={} rtt={}{wait}", st.frame, s.delay(), rtt(st.rtt_ms))
                }
                SessionState::Desynced | SessionState::Closed => closed(),
            },
        };
        format!("{} {who}: {body}", self.mode().name())
    }

    /// Announce the quit (a `Bye`); keep calling [`NetPlay::pump`] (or
    /// `tick`) ~200 ms after so it leaves the machine.
    pub fn close(&mut self) {
        match &mut self.engine {
            Engine::Rollback { session, .. } => session.close(),
            Engine::Lockstep { session, .. } => session.close(),
        }
        self.log_closed(CloseReason::LocalQuit.to_string());
    }
}

/// Rebuild `*game` for a starting session: power-on with the session's co-op
/// flags. No SRAM in MMX, so the host sends no WRAM and power-on is shared.
fn start_game(
    rom: &[u8],
    coop_flags: u32,
    audio_rate: Option<u32>,
    game: &mut Game,
) -> Result<(), String> {
    let mut g = session_game(rom, coop_flags, game.timing()).map_err(|e| e.to_string())?;
    if let Some(hz) = audio_rate {
        g.set_audio_rate(hz);
    }
    // Static art, part of the handshake id (see `NetPlay::config`).
    g.zero_set = game.zero_set.clone();
    g.set_p1_zero(game.p1_zero())?;
    *game = g;
    Ok(())
}

fn apply_rollback<T: Transport>(
    session: &mut RollbackSession<T>,
    ring: &mut StateRing<Vec<u8>>,
    game: &mut Game,
    reqs: &[RollbackRequest],
    n: &mut u32,
    observe: &mut Option<Observer>,
    record: &mut Option<Recorder>,
) -> Result<(), String> {
    for r in reqs {
        match *r {
            RollbackRequest::SaveState { frame } => {
                if session.needs_checksum(frame) {
                    session.report_checksum(frame, net_checksum(game));
                }
                ring.save(frame, net_save(game));
            }
            RollbackRequest::LoadState { frame } => {
                let s = ring
                    .get(frame)
                    .ok_or_else(|| format!("rollback: no saved state for frame {frame}"))?;
                net_load(game, s)?;
            }
            RollbackRequest::Advance { frame, pad1, pad2, replay, .. } => {
                game.step(Pads { p1: pad1, p2: pad2 });
                *n += 1;
                if let Some(r) = record.as_mut() {
                    r.simulated(frame, pad1, pad2);
                }
                if let Some(f) = observe.as_mut() {
                    f(game, Simulated { frame, pad1, pad2, replay });
                }
                if replay {
                    // Heard once already. The new frame is always the last
                    // request, so its samples stay for the frontend.
                    game.take_audio();
                }
            }
        }
    }
    Ok(())
}

/// A 32 KiB LoROM with a "MEGAMAN X" header whose main loop sums pad 1
/// (`$4218`) into `$0300` and pad 2 (`$421A`) into `$0302` (auto-joypad on),
/// so state depends on both pads. For ROM-free netplay tests.
#[doc(hidden)]
pub fn synthetic_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    #[rustfmt::skip]
    let main: &[u8] = &[
        0x78, 0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xFF, 0x1F, 0x9A, // SEI CLC XCE REP #$30 LDX #$1FFF TXS
        0xE2, 0x20, 0xA9, 0x81, 0x8D, 0x00, 0x42, 0xC2, 0x20, // NMI + auto-joypad on
        0xAD, 0x18, 0x42, 0x6D, 0x00, 0x03, 0x8D, 0x00, 0x03, // $8012: $0300 += $4218
        0xAD, 0x1A, 0x42, 0x6D, 0x02, 0x03, 0x8D, 0x02, 0x03, //        $0302 += $421A
        0xEE, 0x00, 0x01, 0x80, 0xE9, //                         INC $0100 / BRA $8012
    ];
    rom[..main.len()].copy_from_slice(main);
    rom[0x50] = 0x40; // $8050: RTI
    rom[0x7FC0..0x7FD5].copy_from_slice(b"MEGAMAN X            ");
    rom[0x7FD5] = 0x20; // LoROM
    rom[0x7FD7] = 0x05; // 32 KiB
    rom[0x7FD9] = 0x01;
    rom[0x7FDA] = 0x33;
    rom[0x7FEA..0x7FEC].copy_from_slice(&[0x50, 0x80]);
    rom[0x7FFA..0x7FFC].copy_from_slice(&[0x50, 0x80]);
    rom[0x7FFC..0x7FFE].copy_from_slice(&[0x00, 0x80]);
    rom[0x7FDC..0x7FE0].copy_from_slice(&[0xFF, 0xFF, 0x00, 0x00]);
    let sum = rom.iter().fold(0u16, |s, &b| s.wrapping_add(u16::from(b)));
    rom[0x7FDC..0x7FDE].copy_from_slice(&(!sum).to_le_bytes());
    rom[0x7FDE..0x7FE0].copy_from_slice(&sum.to_le_bytes());
    rom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BTN_A, BTN_B};
    use mmx_net::{apply_requests, loopback_pair, LoopbackConfig, SyncTestSession};

    /// X, L, R and Select (dropped by the old one-byte wire pad) reach the
    /// other peer intact, in both directions.
    #[test]
    fn full_snes_pad_crosses_the_wire() {
        use crate::{BTN_L, BTN_R, BTN_SELECT, BTN_X};
        let rom = synthetic_rom();
        let (link, ta, tb) = loopback_pair(LoopbackConfig::default(), 1);
        let mk = |host: bool, t| {
            let cfg = NetPlay::<mmx_net::LoopbackTransport>::config(&rom, host, Mode::Rollback, 0, None, false, Timing::Auto);
            let Ok(NetConfig::Rollback(cfg)) = cfg else { panic!("{cfg:?}") };
            RollbackSession::new(cfg, t).unwrap()
        };
        let (mut h, mut g) = (mk(true, ta), mk(false, tb));
        let hp = BTN_X | BTN_L | BTN_R | BTN_SELECT;
        let gp = hp | crate::BTN_B | crate::BTN_UP;
        let (mut seen_h, mut seen_g) = (false, false);
        for tick in 0..300u64 {
            link.advance(1);
            for (s, pad, seen) in [(&mut h, hp, &mut seen_h), (&mut g, gp, &mut seen_g)] {
                s.add_local_input(pad);
                let Ok(reqs) = s.advance(tick * 16) else { continue };
                for r in reqs {
                    if let RollbackRequest::Advance { pad1, pad2, confirmed: true, .. } = r {
                        assert_eq!((pad1, pad2), (hp, gp));
                        *seen = true;
                    }
                }
            }
        }
        assert!(seen_h && seen_g, "no confirmed frame with both pads");
    }

    #[test]
    fn synthetic_rom_reads_the_pads() {
        let mut a = session_game(&synthetic_rom(), 0, Timing::Auto).unwrap();
        let mut b = session_game(&synthetic_rom(), 0, Timing::Auto).unwrap();
        for _ in 0..3 {
            a.step(Pads::default());
            b.step(Pads { p1: BTN_B, p2: BTN_A });
        }
        assert_ne!(net_checksum(&a), net_checksum(&b));
    }

    #[test]
    fn trapset_id_includes_x3() {
        let mut g = session_game(&synthetic_rom(), COOP_TWO_PLAYERS, Timing::Auto).unwrap();
        let plain = trapset_id(&g);
        g.set_zero_set(Some(ZeroSet::default()));
        assert_ne!(trapset_id(&g), plain);
    }

    #[test]
    fn net_state_carries_coop() {
        let mut g = session_game(&synthetic_rom(), COOP_TWO_PLAYERS, Timing::Auto).unwrap();
        g.step(Pads::default());
        g.coop.p2[5] = 0x42;
        g.coop.p2_hidden = false;
        let s = net_save(&g);
        let sum = net_checksum(&g);
        g.coop = coop::CoopState::default();
        g.step(Pads { p1: BTN_B, p2: 0 });
        net_load(&mut g, &s).unwrap();
        assert_eq!(g.coop.p2[5], 0x42);
        assert!(!g.coop.p2_hidden && g.coop.enabled);
        assert_eq!(net_checksum(&g), sum);
        assert_eq!(g.frame_count(), 1);
    }

    #[test]
    fn sync_test_finds_no_nondeterminism() {
        let mut g = session_game(&synthetic_rom(), COOP_TWO_PLAYERS, Timing::Auto).unwrap();
        let mut s = SyncTestSession::new(4);
        let mut ring = StateRing::new(6);
        for f in 0..60u32 {
            let reqs = s.advance((f / 7) as u16, (f / 5) as u16 ^ 0x5A5A);
            apply_requests(&mut g, &mut ring, &reqs, &mut s).unwrap();
        }
        assert_eq!(s.mismatches(), 0);
    }

    /// One simulated frame as a player would see it.
    #[derive(Clone, Copy, PartialEq)]
    struct Snap {
        pads: (u16, u16),
        /// All of WRAM, the whole co-op state, the picture, OAM and P2's tile page.
        sum: u64,
        p2: [u8; crate::ram::X_BLOCK_LEN],
        cam: (u16, u16),
    }

    impl Snap {
        fn new(g: &Game, pad1: u16, pad2: u16) -> Snap {
            let px: Vec<u8> = g.frame().0.iter().flat_map(|p| p.to_le_bytes()).collect();
            let (page, slots) = g.p2_tiles();
            let page: Vec<u8> = page.iter().flat_map(|w| w.to_le_bytes()).collect();
            let coop = bincode::encode_to_vec(&g.coop, bincode::config::standard()).unwrap();
            let w = g.wram();
            let word = |a: usize| u16::from_le_bytes([w[a], w[a + 1]]);
            Snap {
                pads: (pad1, pad2),
                sum: hash_state(&[w, &coop, &px, &page, &slots.to_le_bytes(), &g.oam()]),
                p2: g.coop.p2,
                cam: (word(0x1E4D), word(0x1E50)),
            }
        }

        /// P2 on screen (world position minus camera).
        fn screen(&self) -> (i32, i32) {
            let w = |o: usize| i32::from(u16::from_le_bytes([self.p2[o], self.p2[o + 1]]));
            (w(5) - i32::from(self.cam.0), w(8) - i32::from(self.cam.1))
        }
    }

    /// Real ROM, in stage with P2 anchored (hw-spiky: highway-start, P1 walks
    /// right 250 frames, then idles 60), two peers over a lossy loopback with
    /// scripted pads (both walk / jump / shoot). Every settled frame on both
    /// peers must equal a plain single-`Game` run with the same pads, down to
    /// the picture: rollback is invisible once a frame settles. Also prints how
    /// far the first (predicted) showing of P2 was from where it settled.
    /// Skipped without `MMX_ROM` (and `MMX_X3_ROM` for `x3`).
    fn rollback_invisible_to_p2(x3: bool, delay: u8, loss_per_mille: u16, frames: usize) {
        use crate::{BTN_LEFT, BTN_RIGHT, BTN_Y};
        let Some(rom) = std::env::var_os("MMX_ROM").and_then(|p| std::fs::read(p).ok()) else {
            return eprintln!("MMX_ROM not set; skipping");
        };
        let x3rom = match x3 {
            true => match std::env::var_os("MMX_X3_ROM").and_then(|p| std::fs::read(p).ok()) {
                Some(r) => Some(r),
                None => return eprintln!("MMX_X3_ROM not set; skipping"),
            },
            false => None,
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/highway-start.state");
        let Ok(state) = std::fs::read(st) else { return eprintln!("{st} missing; skipping") };
        let fresh = || {
            let mut g = session_game(&rom, COOP_TWO_PLAYERS, Timing::Auto).unwrap();
            if let Some(r) = &x3rom {
                g.set_x3_rom(r).unwrap();
            }
            g
        };
        let mut base = fresh();
        base.load_state(&state).unwrap();
        for f in 0..310 {
            base.step(Pads { p1: if f < 250 { BTN_RIGHT } else { 0 }, p2: 0 });
        }
        assert!(!base.coop.p2_hidden, "P2 not anchored");
        let s0 = net_save(&base);

        let p1 = |f: u32| match f % 120 {
            0..=59 => BTN_RIGHT,
            60..=79 => BTN_RIGHT | BTN_B,
            80..=99 => 0,
            _ => BTN_Y * u16::from(f % 4 < 2),
        };
        let p2 = |f: u32| match (f / 7).wrapping_mul(2_654_435_761) >> 29 {
            0 | 1 => BTN_RIGHT,
            2 => BTN_RIGHT | BTN_B,
            3 => BTN_LEFT,
            4 => BTN_LEFT | BTN_B,
            5 => BTN_Y,
            6 => BTN_B,
            _ => 0,
        };
        let link_cfg = LoopbackConfig { latency_ticks: 3, jitter_ticks: 2, loss_per_mille, connect_after_ticks: 2 };
        let (link, ta, tb) = loopback_pair(link_cfg, 11);
        let (crc, traps) = (base.rom_info().crc32, trapset_id(&base));
        let mk = |host: bool, t| {
            let mut cfg = if host {
                RollbackConfig::host(crc, traps, COOP_TWO_PLAYERS)
            } else {
                RollbackConfig::guest(crc, traps, COOP_TWO_PLAYERS)
            };
            cfg.input_delay = delay;
            cfg.check_interval = 10;
            let ring = StateRing::new(usize::from(cfg.max_prediction) + 2);
            let mut g = fresh();
            net_load(&mut g, &s0).unwrap();
            // (session, game, ring, first showing per frame, last simulation per frame)
            (RollbackSession::new(cfg, t).unwrap(), g, ring, vec![None::<Snap>; frames + 16], vec![None::<Snap>; frames + 16])
        };
        let mut peers = [mk(true, ta), mk(false, tb)];
        let mut tick = 0u64;
        while peers.iter().any(|(s, ..)| (s.final_frame() as usize) < frames) {
            assert!(tick < 20_000, "stuck");
            link.advance(1);
            for (i, (s, g, ring, shown, settled)) in peers.iter_mut().enumerate() {
                let f = s.frame();
                s.add_local_input(if i == 0 { p1(f) } else { p2(f) });
                let reqs = s.advance(tick * 16).unwrap_or_else(|e| panic!("frame {f}: {e:?}"));
                for r in reqs {
                    apply_requests(g, ring, &[r], s).unwrap();
                    if let RollbackRequest::Advance { frame, pad1, pad2, replay, .. } = r {
                        let (f, snap) = (frame as usize, Some(Snap::new(g, pad1, pad2)));
                        if f < settled.len() {
                            settled[f] = snap;
                            if !replay {
                                shown[f] = snap;
                            }
                        }
                    }
                }
                for ev in s.events() {
                    assert!(!matches!(ev, RollbackEvent::DesyncDetected { .. }), "{ev:?}");
                }
            }
            tick += 1;
        }
        for (who, (s, _, _, shown, settled)) in ["host", "guest"].iter().zip(&peers) {
            let st = s.stats();
            assert!(st.rollbacks > 0, "no rollback exercised");
            let off: Vec<(i32, i32)> = (0..frames)
                .map(|f| {
                    let (a, b) = (shown[f].unwrap().screen(), settled[f].unwrap().screen());
                    ((a.0 - b.0).abs(), (a.1 - b.1).abs())
                })
                .collect();
            eprintln!(
                "x3={x3} d={delay} loss={loss_per_mille} {who}: rollbacks {} max depth {}, P2 first shown \
                 >4 px from where it settled on {}/{frames} frames, max off {:?}",
                st.rollbacks,
                st.max_rollback_depth,
                off.iter().filter(|d| d.0 > 4 || d.1 > 4).count(),
                off.iter().fold((0, 0), |m, d| (m.0.max(d.0), m.1.max(d.1))),
            );
        }
        let mut g = fresh();
        net_load(&mut g, &s0).unwrap();
        for f in 0..frames {
            let (pad1, pad2) = peers[0].4[f].unwrap().pads;
            g.step(Pads { p1: pad1, p2: pad2 });
            let want = Snap::new(&g, pad1, pad2);
            for (who, (.., settled)) in ["host", "guest"].iter().zip(&peers) {
                let got = settled[f].unwrap();
                if got != want {
                    let diff: Vec<String> = (0..want.p2.len())
                        .filter(|&k| got.p2[k] != want.p2[k])
                        .map(|k| format!("+{k:02X}:{:02X}(want {:02X})", got.p2[k], want.p2[k]))
                        .collect();
                    panic!(
                        "x3={x3} d={delay}: {who} differs from the plain run at frame {f}: pads {:04X?} vs \
                         ({pad1:04X}, {pad2:04X}), sum {:016X} vs {:016X}, P2 block diff [{}]",
                        got.pads, got.sum, want.sum, diff.join(" ")
                    );
                }
            }
        }
    }

    #[test]
    fn rollback_invisible_to_p2_rom() {
        rollback_invisible_to_p2(false, 0, 300, 600);
        rollback_invisible_to_p2(false, 2, 150, 600);
    }

    #[test]
    fn rollback_invisible_to_p2_x3() {
        rollback_invisible_to_p2(true, 0, 300, 600);
        rollback_invisible_to_p2(true, 2, 150, 600);
    }

    /// Two `Game`s on the synthetic ROM through `NetPlay` over a lossy,
    /// jittery loopback link with no input delay: stays in sync for 300
    /// settled frames and actually rolls back.
    #[test]
    fn rollback_pair_over_lossy_loopback_stays_in_sync() {
        let rom = synthetic_rom();
        let link_cfg = LoopbackConfig {
            latency_ticks: 3,
            jitter_ticks: 2,
            loss_per_mille: 150,
            connect_after_ticks: 2,
        };
        let (link, ta, tb) = loopback_pair(link_cfg, 7);
        let mk = |host: bool, t| {
            let mut cfg =
                NetPlay::<mmx_net::LoopbackTransport>::config(&rom, host, Mode::Rollback, 0, None, false, Timing::Auto).unwrap();
            cfg.set_check_interval(10);
            NetPlay::new(cfg, rom.clone(), t, "test").unwrap()
        };
        let mut peers = [
            (mk(true, ta), session_game(&rom, 0, Timing::Auto).unwrap(), 0x1111u32),
            (mk(false, tb), session_game(&rom, 0, Timing::Auto).unwrap(), 0x7777u32),
        ];
        let mut tick = 0u64;
        while peers.iter().any(|(p, ..)| p.settled_frames() < 300) {
            assert!(tick < 5000, "stuck: {} / {}", peers[0].0.status(), peers[1].0.status());
            link.advance(1);
            for (net, game, seed) in peers.iter_mut() {
                let f = net.frame();
                let pad = ((f / 9).wrapping_mul(2_654_435_761).wrapping_add(*seed) >> 11) as u16 & 0xFFF0;
                net.tick(game, pad, tick * 16);
                assert!(net.last_error.is_none(), "{}", net.status());
            }
            tick += 1;
        }
        for (net, ..) in &peers {
            let st = net.rollback().unwrap().stats();
            assert!(st.checksums_ok >= 25, "{}: {st:?}", net.status());
            assert!(st.rollbacks > 0, "no rollback exercised: {st:?}");
        }
    }

    /// Lockstep twin of the test above: two `Game`s on the synthetic ROM
    /// through `NetPlay` in `Mode::Lockstep` (delay 3) over the same lossy,
    /// jittery loopback. 300 frames, hashes match, the link really dropped
    /// packets, and both games equal a plain run fed the scripted full-width
    /// pads shifted by the delay (so every pad arrived intact, in order).
    #[test]
    fn lockstep_pair_over_lossy_loopback_stays_in_sync() {
        const FRAMES: u32 = 300;
        const DELAY: u8 = 3;
        let rom = synthetic_rom();
        let link_cfg = LoopbackConfig {
            latency_ticks: 3,
            jitter_ticks: 2,
            loss_per_mille: 150,
            connect_after_ticks: 2,
        };
        let (link, ta, tb) = loopback_pair(link_cfg, 7);
        let mk = |host: bool, t| {
            let mut cfg =
                NetPlay::<mmx_net::LoopbackTransport>::config(&rom, host, Mode::Lockstep, DELAY, None, false, Timing::Auto).unwrap();
            cfg.set_check_interval(10);
            NetPlay::new(cfg, rom.clone(), t, "test").unwrap()
        };
        // The pad latched while frame `f` is next belongs to frame `f + DELAY`.
        let script = |seed: u32, f: u32| ((f / 9).wrapping_mul(2_654_435_761).wrapping_add(seed) >> 11) as u16 & 0xFFF0;
        let mut peers = [
            (mk(true, ta), session_game(&rom, 0, Timing::Auto).unwrap(), 0x1111u32),
            (mk(false, tb), session_game(&rom, 0, Timing::Auto).unwrap(), 0x7777u32),
        ];
        let mut tick = 0u64;
        let mut waited = 0u32;
        while peers.iter().any(|(p, ..)| p.frame() < FRAMES) {
            assert!(tick < 5000, "stuck: {} / {}", peers[0].0.status(), peers[1].0.status());
            link.advance(1);
            for (net, game, seed) in peers.iter_mut() {
                if net.frame() >= FRAMES {
                    net.pump(tick * 16); // done: keep acking / resending for the peer
                    continue;
                }
                let live = net.live();
                let n = net.tick(game, script(*seed, net.frame()), tick * 16);
                assert!(n <= 1);
                waited += u32::from(live && n == 0);
                assert!(net.last_error.is_none(), "{}", net.status());
            }
            tick += 1;
        }
        assert!(link.dropped() > 0, "the link dropped nothing");
        let mut want = session_game(&rom, COOP_TWO_PLAYERS, Timing::Auto).unwrap();
        for f in 0..FRAMES {
            let pad = |seed| if f < u32::from(DELAY) { 0 } else { script(seed, f - u32::from(DELAY)) };
            want.step(Pads { p1: pad(0x1111), p2: pad(0x7777) });
        }
        for (net, game, _) in &peers {
            assert_eq!(net.frame(), FRAMES);
            assert!(net.checks_ok() >= 25, "{}", net.status());
            assert_eq!(net.lockstep().unwrap().delay(), DELAY);
            assert_eq!(net_checksum(game), net_checksum(&want), "{}", net.status());
            assert_eq!(game.wram()[0x300..0x304], want.wram()[0x300..0x304]);
        }
        eprintln!("lockstep: {tick} ticks, {waited} ticks waiting, {} packets dropped", link.dropped());
    }

    /// A rollback host and a lockstep guest refuse each other (their Hellos
    /// carry different protocol versions): both close, neither starts.
    #[test]
    fn mode_mismatch_refuses_to_start() {
        let rom = synthetic_rom();
        let (link, ta, tb) = loopback_pair(LoopbackConfig::default(), 3);
        let cfg = |host, mode| NetPlay::<mmx_net::LoopbackTransport>::config(&rom, host, mode, 2, None, false, Timing::Auto).unwrap();
        let mut h = NetPlay::new(cfg(true, Mode::Rollback), rom.clone(), ta, "t").unwrap();
        let mut g = NetPlay::new(cfg(false, Mode::Lockstep), rom.clone(), tb, "t").unwrap();
        let (mut gh, mut gg) = (session_game(&rom, 0, Timing::Auto).unwrap(), session_game(&rom, 0, Timing::Auto).unwrap());
        for tick in 0..200u64 {
            link.advance(1);
            h.tick(&mut gh, 0, tick * 16);
            g.tick(&mut gg, 0, tick * 16);
        }
        assert!(h.closed() && g.closed(), "{} / {}", h.status(), g.status());
        assert!(!h.started && !g.started);
        let both = format!("{} / {}", h.status(), g.status());
        assert!(both.contains("NetMode"), "{both}");
    }

    /// The frontend pacing rule (native app.rs, web app.js) against a fake
    /// audio device per peer, 1 ms of virtual time per loopback tick, a
    /// 300 ms RTT link with jitter and loss, 20-frame prediction window: top
    /// the device queue up to 50 ms with `tick_audio`, at most 4 per 2 ms
    /// wake-up. The queue never reaches the 200 ms cap (nothing dropped, so
    /// nothing plays fast), the device hears its own rate, and the game runs
    /// at `Game::fps` against the device clock.
    fn audio_paced_pair(mode: Mode) {
        const SECS: u64 = 10;
        let rom = synthetic_rom();
        let link_cfg = LoopbackConfig {
            latency_ticks: 150,
            jitter_ticks: 20,
            loss_per_mille: 20,
            connect_after_ticks: 50,
        };
        let (link, ta, tb) = loopback_pair(link_cfg, 11);
        let mk = |host: bool, t, hz| {
            let mut cfg = NetPlay::<mmx_net::LoopbackTransport>::config(
                &rom,
                host,
                mode,
                mode.default_delay(),
                None,
                false,
                Timing::Auto,
            )
            .unwrap();
            if let NetConfig::Rollback(c) = &mut cfg {
                c.max_prediction = 20;
            }
            let mut p = NetPlay::new(cfg, rom.clone(), t, "test").unwrap();
            p.audio_rate = Some(hz);
            p
        };
        struct Dev {
            hz: u64,
            queue: u64,
            max: u64,
            heard: u64,
            starved: u64,
        }
        let mut peers = [
            (
                mk(true, ta, 44_100),
                Dev {
                    hz: 44_100,
                    queue: 0,
                    max: 0,
                    heard: 0,
                    starved: 0,
                },
            ),
            (
                mk(false, tb, 48_000),
                Dev {
                    hz: 48_000,
                    queue: 0,
                    max: 0,
                    heard: 0,
                    starved: 0,
                },
            ),
        ];
        let mut games = [
            session_game(&rom, 0, Timing::Auto).unwrap(),
            session_game(&rom, 0, Timing::Auto).unwrap(),
        ];
        let mut live_at = [None; 2];
        for ms in 0..(SECS + 2) * 1000 {
            link.advance(1);
            for (i, ((net, dev), game)) in peers.iter_mut().zip(games.iter_mut()).enumerate() {
                // the device drains hz/1000 per ms once the session plays
                if live_at[i].is_some() {
                    let want = dev.hz * (ms + 1) / 1000 - dev.hz * ms / 1000;
                    let got = want.min(dev.queue);
                    dev.queue -= got;
                    dev.heard += got;
                    dev.starved += want - got;
                }
                if ms % 2 != 0 {
                    continue;
                }
                if !net.live() {
                    net.tick(game, 0, ms);
                    if net.live() {
                        live_at[i] = Some((ms, net.frame()));
                    }
                    continue;
                }
                let target = dev.hz / 20;
                let mut k = 0;
                while k < 4 && dev.queue < target {
                    let pad = ((net.frame() / 9).wrapping_mul(2_654_435_761) >> 11) as u16 & 0xFFF0;
                    let (_, pcm) = net.tick_audio(game, pad, ms);
                    dev.queue += pcm.len() as u64;
                    dev.max = dev.max.max(dev.queue);
                    k += 1;
                }
                assert!(net.last_error.is_none(), "{}", net.status());
            }
        }
        for (((net, dev), game), live) in peers.iter().zip(&games).zip(live_at) {
            let (t0, f0) = live.expect("session never started");
            let secs = ((SECS + 2) * 1000 - t0) as f64 / 1000.0;
            let fps = f64::from(net.frame() - f0) / secs;
            eprintln!(
                "{:?} {} Hz: queue max {} ms, heard {:.1} s of {secs:.1} s, starved {} ms, {fps:.2} fps (game {:.2}), {}",
                mode,
                dev.hz,
                dev.max * 1000 / dev.hz,
                dev.heard as f64 / dev.hz as f64,
                dev.starved * 1000 / dev.hz,
                game.fps(),
                net.status()
            );
            assert!(
                dev.max < dev.hz / 5,
                "queue hit the 200 ms cap: {}",
                dev.max
            );
            assert!(
                dev.max <= dev.hz / 20 + dev.hz / 50,
                "queue ran past target + a frame: {}",
                dev.max
            );
            // every tick spends one frame of device time; skips are silence,
            // so rollback frames lag the device by a few percent at most
            // (lockstep at 300 ms RTT stalls most ticks: silence, not speed-up)
            assert!(
                mode == Mode::Lockstep || (fps / game.fps() - 1.0).abs() < 0.05,
                "{fps} fps vs {}",
                game.fps()
            );
        }
    }

    #[test]
    fn rollback_audio_paced_over_300ms_rtt() {
        audio_paced_pair(Mode::Rollback);
    }

    #[test]
    fn lockstep_audio_paced_over_300ms_rtt() {
        audio_paced_pair(Mode::Lockstep);
    }

    /// Real ROM: one frame renders `rate / Game::fps` samples, so a pacer at
    /// `Game::fps` feeds the device exactly its rate. roms/mega-man-x.smc is
    /// the PAL release (region 0x02, 50.007 fps: 960 samples/frame at 48 kHz);
    /// the old hard-coded 60.0988 Hz online timer fed 57.7 kHz into a 48 kHz
    /// device, the 200 ms ring overflowed and dropped: fast music.
    #[test]
    fn rom_audio_per_frame_matches_fps() {
        let Some(path) = std::env::var_os("MMX_ROM").filter(|p| !p.is_empty()) else {
            return eprintln!("MMX_ROM not set; skipping");
        };
        let rom = std::fs::read(path).unwrap();
        for hz in [44_100u32, 48_000] {
            let mut g = session_game(&rom, 0, Timing::Auto).unwrap();
            g.set_audio_rate(hz);
            for _ in 0..60 {
                g.step(Pads::default());
            }
            g.take_audio();
            let mut n = 0;
            for _ in 0..300 {
                g.step(Pads::default());
                n += g.take_audio().len();
            }
            let per_sec = n as f64 / 300.0 * g.fps();
            eprintln!(
                "{hz} Hz: {:.1} samples/frame at {:.3} fps = {per_sec:.0} Hz",
                n as f64 / 300.0,
                g.fps()
            );
            assert!(
                (per_sec / f64::from(hz) - 1.0).abs() < 0.01,
                "{per_sec} vs {hz}"
            );
        }
    }
}
