//! Netplay glue shared by `mmx-native` and `mmx-web`: [`Game`] as an
//! [`mmx_net::RollbackGame`] and [`NetPlay`], the
//! per-tick rollback driver both frontends call (z2rs's `RollbackLink` +
//! `rollback_tick`, shrunk). See `docs/netplay.md` "Frontend wiring".
//!
//! Save states for rollback are [`Game::save_state`] plus the co-op state
//! ([`coop::CoopState`], P2's shadow block), which `save_state` leaves out:
//! without it a rollback would rewind P1 and the world but not P2.

use mmx_net::{
    hash_state, CloseReason, ConnectStage, Role, RollbackConfig, RollbackError, RollbackEvent,
    RollbackGame, RollbackRequest, RollbackSession, RollbackState, StateRing, Transport,
    COOP_TWO_PLAYERS,
};

use crate::{coop, Game, Pads, RomError};

/// Bump when anything that changes simulation between two builds changes
/// outside the trap addresses (mixed into [`trapset_id`]).
const NET_REVISION: u64 = 1;

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

/// Handshake trap-set id: FNV over the sorted trap addresses and
/// [`NET_REVISION`]. Peers with different trap sets refuse to start.
pub fn trapset_id(game: &Game) -> u64 {
    let mut addrs: Vec<u32> = game.traps.addrs().collect();
    addrs.sort_unstable();
    let bytes: Vec<u8> = addrs.iter().flat_map(|a| a.to_le_bytes()).collect();
    hash_state(&[&NET_REVISION.to_le_bytes(), &bytes])
}

/// The game a session plays: power-on, co-op traps registered, co-op on when
/// the session's flags say so. Both peers build exactly this on `Running`.
pub fn session_game(rom: &[u8], coop_flags: u32) -> Result<Game, RomError> {
    let mut g = Game::from_rom(rom.to_vec())?;
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

/// A rollback session plus everything a frontend tick needs: the state ring,
/// the ROM (to rebuild the game on `Running`), skip/interrupt bookkeeping.
pub struct NetPlay<T: Transport> {
    pub session: RollbackSession<T>,
    ring: StateRing<Vec<u8>>,
    rom: Vec<u8>,
    pub room: String,
    /// `Running` arrived and the game was rebuilt from power-on.
    pub started: bool,
    skip: u32,
    interrupted: bool,
    /// Why the session ended (desync, peer left, ...), for the status line.
    pub last_error: Option<String>,
    /// Applied to the rebuilt game (the frontend's audio device rate).
    pub audio_rate: Option<u32>,
}

impl<T: Transport> NetPlay<T> {
    /// Handshake config for `rom`: ROM CRC32, trap-set id of a session game,
    /// `COOP_TWO_PLAYERS`, rollback input delay `delay`.
    pub fn config(rom: &[u8], host: bool, delay: u8) -> Result<RollbackConfig, String> {
        let g = session_game(rom, COOP_TWO_PLAYERS).map_err(|e| e.to_string())?;
        let (crc, traps) = (g.rom_info().crc32, trapset_id(&g));
        let mut cfg = if host {
            RollbackConfig::host(crc, traps, COOP_TWO_PLAYERS)
        } else {
            RollbackConfig::guest(crc, traps, COOP_TWO_PLAYERS)
        };
        cfg.input_delay = delay;
        Ok(cfg)
    }

    pub fn new(cfg: RollbackConfig, rom: Vec<u8>, transport: T, room: &str) -> Result<Self, String> {
        let ring = StateRing::new(usize::from(cfg.max_prediction) + 2);
        let session = RollbackSession::new(cfg, transport).map_err(|e| e.to_string())?;
        Ok(NetPlay {
            session,
            ring,
            rom,
            room: room.into(),
            started: false,
            skip: 0,
            interrupted: false,
            last_error: None,
            audio_rate: None,
        })
    }

    /// The session drives the game (started and not closed). While false the
    /// frontend plays locally; nothing played then reaches the session.
    pub fn live(&self) -> bool {
        self.started && self.session.state() != RollbackState::Closed
    }

    pub fn is_host(&self) -> bool {
        self.session.role() == Role::Host
    }

    /// One frontend tick: add this peer's pad, advance, run the requests on
    /// `game` (replayed frames' audio dropped), then handle events (`Running`
    /// replaces `*game`). Returns frames simulated, replays included; present
    /// once afterwards when non-zero. Before the session is live it only
    /// pumps the network.
    pub fn tick(&mut self, game: &mut Game, pad: u16, now_ms: u64) -> u32 {
        let mut n = 0;
        if self.skip > 0 && self.live() {
            self.skip -= 1;
            self.session.poll(now_ms);
        } else if !self.live() {
            self.session.poll(now_ms);
        } else {
            self.session.add_local_input(pad);
            match self.session.advance(now_ms) {
                Ok(reqs) => {
                    if let Err(e) = self.apply(game, &reqs, &mut n) {
                        self.last_error = Some(e);
                        self.session.close();
                    }
                }
                Err(RollbackError::Closed(_)) => {}
                Err(e) => {
                    self.last_error = Some(e.to_string());
                    self.session.close();
                }
            }
        }
        self.events(game);
        n
    }

    fn apply(&mut self, game: &mut Game, reqs: &[RollbackRequest], n: &mut u32) -> Result<(), String> {
        for r in reqs {
            match *r {
                RollbackRequest::SaveState { frame } => {
                    if self.session.needs_checksum(frame) {
                        self.session.report_checksum(frame, net_checksum(game));
                    }
                    self.ring.save(frame, net_save(game));
                }
                RollbackRequest::LoadState { frame } => {
                    let s = self
                        .ring
                        .get(frame)
                        .ok_or_else(|| format!("rollback: no saved state for frame {frame}"))?;
                    net_load(game, s)?;
                }
                RollbackRequest::Advance {
                    pad1, pad2, replay, ..
                } => {
                    game.step(Pads { p1: pad1, p2: pad2 });
                    *n += 1;
                    if replay {
                        // Heard once already. The new frame is always the
                        // last request, so its samples stay for the frontend.
                        game.take_audio();
                    }
                }
            }
        }
        Ok(())
    }

    fn events(&mut self, game: &mut Game) {
        for ev in self.session.events() {
            match ev {
                RollbackEvent::Running { coop_flags, .. } => {
                    // No SRAM in MMX: the host sends no WRAM, power-on is shared.
                    match session_game(&self.rom, coop_flags) {
                        Ok(mut g) => {
                            if let Some(hz) = self.audio_rate {
                                g.set_audio_rate(hz);
                            }
                            *game = g;
                            self.started = true;
                        }
                        Err(e) => {
                            self.last_error = Some(e.to_string());
                            self.session.close();
                        }
                    }
                }
                RollbackEvent::WaitRecommendation { skip_frames } => {
                    self.skip = u32::from(skip_frames)
                }
                RollbackEvent::NetworkInterrupted { .. } => self.interrupted = true,
                RollbackEvent::NetworkResumed => self.interrupted = false,
                RollbackEvent::DesyncDetected { frame, .. } => {
                    self.last_error = Some(format!("desync at frame {frame}"))
                }
                RollbackEvent::Disconnected { reason } => {
                    self.last_error.get_or_insert_with(|| reason.to_string());
                }
                RollbackEvent::Synchronizing { .. } => {}
            }
        }
    }

    /// One-line status: connect stage, running stats, or why it closed.
    pub fn status(&self) -> String {
        let who = if self.is_host() { "host" } else { "guest" };
        let s = &self.session;
        let body = match s.state() {
            RollbackState::Idle => stage_label(s.transport().stage(), &self.room),
            RollbackState::Synchronizing => stage_label(ConnectStage::Connected, &self.room),
            RollbackState::Running => {
                let st = s.stats();
                let rtt = st.rtt_ms.map_or("-".into(), |ms| format!("{ms}ms"));
                format!(
                    "f={} d={} rtt={rtt} rollbacks={} pred={}/{}{}",
                    st.frame,
                    s.input_delay(),
                    st.rollbacks,
                    st.prediction_depth,
                    st.max_prediction_depth,
                    if self.interrupted { " INTERRUPTED" } else { "" }
                )
            }
            RollbackState::Closed => {
                let why = self
                    .last_error
                    .clone()
                    .or_else(|| s.close_reason().map(CloseReason::to_string))
                    .unwrap_or_default();
                format!("closed ({why})")
            }
        };
        format!("rollback {who}: {body}")
    }

    /// Announce the quit (a `Bye`); keep calling `session.poll` ~200 ms after.
    pub fn close(&mut self) {
        self.session.close();
    }
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
            let mut cfg = NetPlay::<mmx_net::LoopbackTransport>::config(&rom, host, 0).unwrap();
            cfg.input_delay = 0;
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
        let mut a = session_game(&synthetic_rom(), 0).unwrap();
        let mut b = session_game(&synthetic_rom(), 0).unwrap();
        for _ in 0..3 {
            a.step(Pads::default());
            b.step(Pads { p1: BTN_B, p2: BTN_A });
        }
        assert_ne!(net_checksum(&a), net_checksum(&b));
    }

    #[test]
    fn net_state_carries_coop() {
        let mut g = session_game(&synthetic_rom(), COOP_TWO_PLAYERS).unwrap();
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
        let mut g = session_game(&synthetic_rom(), COOP_TWO_PLAYERS).unwrap();
        let mut s = SyncTestSession::new(4);
        let mut ring = StateRing::new(6);
        for f in 0..60u32 {
            let reqs = s.advance((f / 7) as u16, (f / 5) as u16 ^ 0x5A5A);
            apply_requests(&mut g, &mut ring, &reqs, &mut s).unwrap();
        }
        assert_eq!(s.mismatches(), 0);
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
            let mut cfg = NetPlay::<mmx_net::LoopbackTransport>::config(&rom, host, 0).unwrap();
            cfg.check_interval = 10;
            NetPlay::new(cfg, rom.clone(), t, "test").unwrap()
        };
        let mut peers = [
            (mk(true, ta), session_game(&rom, 0).unwrap(), 0x1111u32),
            (mk(false, tb), session_game(&rom, 0).unwrap(), 0x7777u32),
        ];
        let mut tick = 0u64;
        while peers.iter().any(|(p, ..)| p.session.final_frame() < 300) {
            assert!(tick < 5000, "stuck: {} / {}", peers[0].0.status(), peers[1].0.status());
            link.advance(1);
            for (net, game, seed) in peers.iter_mut() {
                let f = net.session.frame();
                let pad = ((f / 9).wrapping_mul(2_654_435_761).wrapping_add(*seed) >> 11) as u16 & 0xFFF0;
                net.tick(game, pad, tick * 16);
                assert!(net.last_error.is_none(), "{}", net.status());
            }
            tick += 1;
        }
        for (net, ..) in &peers {
            let st = net.session.stats();
            assert!(st.checksums_ok >= 25, "{}: {st:?}", net.status());
            assert!(st.rollbacks > 0, "no rollback exercised: {st:?}");
        }
    }
}
