//! Netplay session log: what a live session really did, small enough to
//! mail. [`Recorder`] is filled by [`crate::netgame::NetPlay`] (set
//! `NetPlay::record`), [`parse`] reads it back for `cargo xtask netreplay`.
//! Format: docs/netplay.md "Session logs".
//!
//! Text, one record per line. Header `key value` lines up to `---`, then:
//!
//! ```text
//! p N P1 P2              next N confirmed frames used pads P1, P2 (hex)
//! t MS F                 wall-clock mark: at MS the next frame was F (every second)
//! c MS N LOCAL REMOTE    checksum of the state after N frames, this peer / the peer (hex)
//! r F D                  rollback of D frames while F was next
//! s MS F DUR             stall from MS while F was next, DUR ms (no new frame)
//! w MS F SKIP            wait recommendation: skip SKIP ticks
//! i MS F / j MS F        network interrupted / resumed
//! d MS N LOCAL REMOTE    desync detected on the state after N frames
//! x MS REASON            session closed
//! ```

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::netgame::Mode;
use crate::Timing;

pub const MAGIC: &str = "mmxnetlog 1";

/// Everything needed to rebuild the session game (`netgame::session_game`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    pub rom_crc: u32,
    pub trapset: u64,
    pub mode: Mode,
    pub host: bool,
    /// Effective (host's) input delay.
    pub delay: u8,
    /// Rollback prediction window (0 in lockstep).
    pub prediction: u8,
    /// Effective checksum interval in frames.
    pub interval: u16,
    pub timing: Timing,
    /// X3 Zero set loaded (needs the X3 ROM to replay).
    pub x3: bool,
    /// P1 plays as Zero.
    pub zero: bool,
    /// Wide gameplay (`Game::set_wide_gameplay`).
    pub wide: bool,
    /// Save state loaded into the session game at frame 0 (netsim only).
    pub load: Option<String>,
}

impl Header {
    pub fn role(&self) -> &'static str {
        if self.host { "host" } else { "guest" }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Time { ms: u64, frame: u32 },
    /// Checksums of the state after `frames` frames (this peer, the peer).
    Check { ms: u64, frames: u32, local: u64, remote: u64 },
    Rollback { frame: u32, depth: u32 },
    Stall { ms: u64, frame: u32, dur_ms: u64 },
    Wait { ms: u64, frame: u32, skip: u8 },
    Interrupted { ms: u64, frame: u32 },
    Resumed { ms: u64, frame: u32 },
    Desync { ms: u64, frames: u32, local: u64, remote: u64 },
    Closed { ms: u64, reason: String },
}

/// A parsed log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Log {
    pub header: Header,
    /// Confirmed `(pad1, pad2)` per frame from 0.
    pub pads: Vec<(u16, u16)>,
    pub events: Vec<Event>,
}

/// Builds a log while a session runs. Nothing is written before
/// [`Recorder::start`] (the session's `Running`).
#[derive(Debug, Default)]
pub struct Recorder {
    /// Save state the driver loads at frame 0 (netsim), for the header.
    pub load: Option<String>,
    started: bool,
    finished: bool,
    out: String,
    /// Current pad run `(count, pad1, pad2)`.
    run: (u32, u16, u16),
    /// Pads of simulated but not yet settled frames (last simulation wins).
    pending: BTreeMap<u32, (u16, u16)>,
    next_pad: u32,
    stall: Option<(u64, u32)>,
    next_mark_ms: u64,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn start(&mut self, h: &Header) {
        let b = u8::from;
        let _ = write!(
            self.out,
            "{MAGIC}\nrom_crc {:08x}\ntrapset {:016x}\nmode {}\nrole {}\ndelay {}\nprediction {}\ninterval {}\ntiming {}\nx3 {}\nzero {}\n",
            h.rom_crc, h.trapset, h.mode.name(), h.role(), h.delay, h.prediction, h.interval, h.timing.name(), b(h.x3), b(h.zero)
        );
        if h.wide {
            self.out.push_str("wide 1\n");
        }
        if let Some(l) = h.load.as_ref().or(self.load.as_ref()) {
            let _ = writeln!(self.out, "load {l}");
        }
        self.out.push_str("---\n");
        self.started = true;
    }

    /// A frame was simulated with these pads (again, after a rollback).
    pub fn simulated(&mut self, frame: u32, pad1: u16, pad2: u16) {
        if self.started && !self.finished && frame >= self.next_pad {
            self.pending.insert(frame, (pad1, pad2));
        }
    }

    /// Frames `< below` are final: move their pads to the log.
    pub fn settle(&mut self, below: u32) {
        while self.next_pad < below {
            let Some(p) = self.pending.remove(&self.next_pad) else { break };
            self.pad(p.0, p.1);
            self.next_pad += 1;
        }
    }

    fn pad(&mut self, p1: u16, p2: u16) {
        let (n, a, b) = self.run;
        if n > 0 && (a, b) == (p1, p2) {
            self.run.0 += 1;
            return;
        }
        self.flush_run();
        self.run = (1, p1, p2);
    }

    fn flush_run(&mut self) {
        let (n, a, b) = self.run;
        if n > 0 {
            let _ = writeln!(self.out, "p {n} {a:04x} {b:04x}");
        }
        self.run.0 = 0;
    }

    /// Once per frontend tick: `frame` is the next frame to simulate,
    /// `progressed` whether this tick simulated a new one, `waiting` a stall
    /// that is not a wait recommendation. Writes time marks and stalls.
    pub fn tick(&mut self, ms: u64, frame: u32, progressed: bool, stalled: bool) {
        if !self.started || self.finished {
            return;
        }
        if ms >= self.next_mark_ms {
            let _ = writeln!(self.out, "t {ms} {frame}");
            self.next_mark_ms = ms - ms % 1000 + 1000;
        }
        match (self.stall, progressed) {
            (None, false) if stalled => self.stall = Some((ms, frame)),
            (Some((t, f)), true) => {
                let _ = writeln!(self.out, "s {t} {f} {}", ms - t);
                self.stall = None;
            }
            _ => {}
        }
    }

    pub fn event(&mut self, e: &Event) {
        if !self.started || self.finished {
            return;
        }
        let _ = match e {
            Event::Time { ms, frame } => writeln!(self.out, "t {ms} {frame}"),
            Event::Check { ms, frames, local, remote } => writeln!(self.out, "c {ms} {frames} {local:016x} {remote:016x}"),
            Event::Rollback { frame, depth } => writeln!(self.out, "r {frame} {depth}"),
            Event::Stall { ms, frame, dur_ms } => writeln!(self.out, "s {ms} {frame} {dur_ms}"),
            Event::Wait { ms, frame, skip } => writeln!(self.out, "w {ms} {frame} {skip}"),
            Event::Interrupted { ms, frame } => writeln!(self.out, "i {ms} {frame}"),
            Event::Resumed { ms, frame } => writeln!(self.out, "j {ms} {frame}"),
            Event::Desync { ms, frames, local, remote } => writeln!(self.out, "d {ms} {frames} {local:016x} {remote:016x}"),
            Event::Closed { ms, reason } => writeln!(self.out, "x {ms} {reason}"),
        };
    }

    /// Take what was written since the last drain (native: append to the
    /// file). The current pad run stays until it ends or [`Recorder::finish`].
    pub fn drain(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    /// End of session: write the open pad run (and stall).
    pub fn finish(&mut self, ms: u64) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some((t, f)) = self.stall.take() {
            let _ = writeln!(self.out, "s {t} {f} {}", ms.saturating_sub(t));
        }
        self.flush_run();
    }

    /// The whole log so far (never drained), open run included (web download).
    pub fn snapshot(&self) -> String {
        let mut s = self.out.clone();
        let (n, a, b) = self.run;
        if n > 0 {
            let _ = writeln!(s, "p {n} {a:04x} {b:04x}");
        }
        s
    }
}

/// Parse a log. Unknown header keys and body tags are skipped (newer logs).
pub fn parse(text: &str) -> Result<Log, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim() == MAGIC => {}
        _ => return Err(format!("not a netplay log (first line must be {MAGIC:?})")),
    }
    let mut log = Log::default();
    let h = &mut log.header;
    for (i, l) in lines.by_ref() {
        let l = l.trim();
        if l == "---" {
            break;
        }
        let (k, v) = l.split_once(' ').unwrap_or((l, ""));
        let bad = || format!("line {}: bad {k} {v:?}", i + 1);
        let hex32 = |v: &str| u32::from_str_radix(v, 16).map_err(|_| bad());
        match k {
            "rom_crc" => h.rom_crc = hex32(v)?,
            "trapset" => h.trapset = u64::from_str_radix(v, 16).map_err(|_| bad())?,
            "mode" => h.mode = Mode::parse(v).ok_or_else(bad)?,
            "role" => h.host = v == "host",
            "delay" => h.delay = v.parse().map_err(|_| bad())?,
            "prediction" => h.prediction = v.parse().map_err(|_| bad())?,
            "interval" => h.interval = v.parse().map_err(|_| bad())?,
            "timing" => h.timing = Timing::parse(v).ok_or_else(bad)?,
            "x3" => h.x3 = v == "1",
            "zero" => h.zero = v == "1",
            "wide" => h.wide = v == "1",
            "load" => h.load = Some(v.to_string()),
            _ => {}
        }
    }
    for (i, l) in lines {
        let mut w = l.split_whitespace();
        let Some(tag) = w.next() else { continue };
        let rest: Vec<&str> = w.collect();
        let bad = || format!("line {}: {l:?}", i + 1);
        let n = |k: usize| rest.get(k).and_then(|v| v.parse::<u64>().ok()).ok_or_else(bad);
        let hex = |k: usize| rest.get(k).and_then(|v| u64::from_str_radix(v, 16).ok()).ok_or_else(bad);
        let e = match tag {
            "p" => {
                let (count, a, b) = (n(0)?, hex(1)? as u16, hex(2)? as u16);
                log.pads.extend(std::iter::repeat_n((a, b), count as usize));
                continue;
            }
            "t" => Event::Time { ms: n(0)?, frame: n(1)? as u32 },
            "c" => Event::Check { ms: n(0)?, frames: n(1)? as u32, local: hex(2)?, remote: hex(3)? },
            "r" => Event::Rollback { frame: n(0)? as u32, depth: n(1)? as u32 },
            "s" => Event::Stall { ms: n(0)?, frame: n(1)? as u32, dur_ms: n(2)? },
            "w" => Event::Wait { ms: n(0)?, frame: n(1)? as u32, skip: n(2)? as u8 },
            "i" => Event::Interrupted { ms: n(0)?, frame: n(1)? as u32 },
            "j" => Event::Resumed { ms: n(0)?, frame: n(1)? as u32 },
            "d" => Event::Desync { ms: n(0)?, frames: n(1)? as u32, local: hex(2)?, remote: hex(3)? },
            "x" => Event::Closed { ms: n(0)?, reason: rest[1..].join(" ") },
            _ => continue,
        };
        log.events.push(e);
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_round_trips() {
        let h = Header {
            rom_crc: 0x1234_abcd,
            trapset: 0xfeed_beef_0000_0001,
            mode: Mode::Rollback,
            host: false,
            delay: 3,
            prediction: 20,
            interval: 60,
            timing: Timing::Ntsc,
            x3: true,
            zero: false,
            wide: true,
            load: Some("states/hw-spiky.state".into()),
        };
        let mut r = Recorder::new();
        r.simulated(0, 9, 9); // before start: ignored
        r.start(&h);
        let pads: Vec<(u16, u16)> = (0..50u16).map(|f| (f / 10, 0x8000 * u16::from(f % 20 < 5))).collect();
        // simulate each frame twice (prediction, then the correction), settle in chunks
        for (f, &(a, b)) in pads.iter().enumerate() {
            r.simulated(f as u32, 0xFFFF, 0xFFFF);
            r.simulated(f as u32, a, b);
            if f % 7 == 6 {
                r.settle(f as u32 - 2);
            }
        }
        let events = vec![
            Event::Check { ms: 1000, frames: 60, local: 1, remote: u64::MAX },
            Event::Rollback { frame: 12, depth: 5 },
            Event::Stall { ms: 5, frame: 3, dur_ms: 40 },
            Event::Wait { ms: 7, frame: 9, skip: 2 },
            Event::Interrupted { ms: 8, frame: 9 },
            Event::Resumed { ms: 9, frame: 9 },
            Event::Desync { ms: 10, frames: 120, local: 5, remote: 6 },
            Event::Closed { ms: 11, reason: "peer left the room".into() },
        ];
        for e in &events {
            r.event(e);
        }
        let mut text = r.drain();
        r.settle(50);
        r.finish(12);
        text += &r.drain();
        let log = parse(&text).unwrap();
        assert_eq!(log.header, h);
        assert_eq!(log.pads, pads);
        assert_eq!(log.events, events);
        assert!(text.len() < 500, "{} bytes:\n{text}", text.len());
    }

    #[test]
    fn stalls_and_time_marks() {
        let mut r = Recorder::new();
        r.start(&Header::default());
        r.tick(0, 0, true, false);
        r.tick(16, 1, false, true);
        r.tick(32, 1, false, true);
        r.tick(1048, 1, true, false);
        let log = parse(&r.snapshot()).unwrap();
        assert_eq!(
            log.events,
            vec![
                Event::Time { ms: 0, frame: 0 },
                Event::Time { ms: 1048, frame: 1 },
                Event::Stall { ms: 16, frame: 1, dur_ms: 1032 },
            ]
        );
        assert!(parse("hello").is_err());
    }
}
