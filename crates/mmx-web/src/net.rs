//! Online co-op in the browser (feature `netplay`): the shared
//! `mmx_core::netgame::NetPlay` driver (rollback or lockstep) over `MatchboxTransport` (its message
//! loop runs through `spawn_local`). The page calls `net_tick_audio` while
//! the worklet queue is below target (`net_tick` at `fps()` without audio)
//! and presents when it returns > 0.

use mmx_core::netgame::{Mode, NetPlay};
use mmx_net::MatchboxTransport;
use wasm_bindgen::prelude::*;

use crate::Emu;

#[wasm_bindgen]
impl Emu {
    /// Host (`host = true`, player 1) or join room `room` on signal server
    /// `signal` (`ws://host:3536`). `mode`: `rollback` or `lockstep` (both
    /// players must pick the same). `delay`: input delay in frames, rollback
    /// 0-3, lockstep 0-8; negative = the mode's default (2 / 3). `ice`: the
    /// shared ICE text form (`none`, `stun:..`, `turn:.. username=U
    /// credential=C`), empty = default. Replaces any previous session. Never
    /// blocks.
    pub fn net_connect(
        &mut self,
        signal: &str,
        room: &str,
        host: bool,
        mode: &str,
        delay: i32,
        ice: &str,
    ) -> Result<(), JsValue> {
        let err = |e: String| JsValue::from_str(&e);
        let mode = Mode::parse(mode).ok_or_else(|| err(format!("mode {mode:?}: rollback or lockstep")))?;
        let delay = match u8::try_from(delay) {
            Ok(d) => d,
            Err(_) if delay < 0 => mode.default_delay(),
            Err(_) => return Err(err(format!("input delay {delay} out of range"))),
        };
        let cfg = NetPlay::<MatchboxTransport>::config(&self.rom, host, mode, delay, self.game.zero_set(), self.game.p1_zero()).map_err(err)?;
        let url = mmx_net::room_url(signal, room).map_err(|e| err(e.to_string()))?;
        let ice = match ice.trim() {
            "" => None,
            s => Some(mmx_net::IceConfig::parse(s).map_err(|e| err(format!("ICE: {e}")))?),
        };
        let mut play = NetPlay::new(cfg, self.rom.clone(), MatchboxTransport::connect(&url, ice), room)
            .map_err(err)?;
        play.audio_rate = self.audio_rate;
        self.net = Some(play);
        Ok(())
    }

    /// One tick with this peer's pad at `now_ms` (`performance.now()`).
    /// Before the session is live (connecting, or closed) it only pumps the
    /// network. Returns frames simulated (present when > 0).
    pub fn net_tick(&mut self, pad: u16, now_ms: f64) -> u32 {
        let Some(net) = self.net.as_mut() else { return 0 };
        let n = net.tick(&mut self.game, pad, now_ms as u64);
        if n > 0 {
            self.refresh_frame();
        }
        n
    }

    /// [`Emu::net_tick`] paced by the audio clock: always leaves exactly one
    /// frame of audio for `audio_take` (silence when nothing was stepped).
    pub fn net_tick_audio(&mut self, pad: u16, now_ms: f64) -> u32 {
        let Some(net) = self.net.as_mut() else {
            return 0;
        };
        let (n, pcm) = net.tick_audio(&mut self.game, pad, now_ms as u64);
        self.pcm.extend(pcm);
        if n > 0 {
            self.refresh_frame();
        }
        n
    }

    /// The session drives the game: stop local stepping.
    pub fn net_live(&self) -> bool {
        self.net.as_ref().is_some_and(|n| n.live())
    }

    /// A session exists (connecting, live or closed-but-not-left).
    pub fn net_active(&self) -> bool {
        self.net.is_some()
    }

    /// Status line: connect stage, stats, or why it closed.
    pub fn net_status(&self) -> String {
        self.net.as_ref().map_or_else(|| "idle".into(), |n| n.status())
    }

    /// Say goodbye; keep calling `net_tick` for ~200 ms, then `net_drop`.
    pub fn net_leave(&mut self) {
        if let Some(n) = self.net.as_mut() {
            n.close();
        }
    }

    pub fn net_drop(&mut self) {
        self.net = None;
    }

    pub fn net_default_ice() -> String {
        mmx_net::DEFAULT_ICE_SPEC.into()
    }
}
