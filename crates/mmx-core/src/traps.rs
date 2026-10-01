//! Routine trap table: Rust bodies installed on 24-bit routine entry
//! addresses. Mirrors z2rs `traps.rs`, keyed by `PBR:PC` (no bank aliasing
//! on LoROM: every routine has one 24-bit address).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::Game;

/// A ported routine. Receives the whole game (WRAM, registers, PPU state).
/// Must leave the registers/flags the original `RTS`/`RTL` would leave; the
/// dispatcher emulates the return itself.
pub type TrapFn = fn(&mut Game);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapReturn {
    /// Entered by `JSR`: pop 2 bytes, `PC = word + 1`.
    Rts,
    /// Entered by `JSL`: pop 3 bytes, `PC = word + 1`, `PBR = byte`.
    Rtl,
    /// Run the body, then let the original bytes run as well (hooks that
    /// observe or add work, e.g. the co-op slices).
    Observe,
}

#[derive(Debug, Clone, Copy)]
pub struct TrapInfo {
    pub name: &'static str,
    pub ret: TrapReturn,
    pub body: TrapFn,
}

#[derive(Default)]
pub struct TrapTable {
    traps: HashMap<u32, TrapInfo>,
    untrapped: HashSet<u32>,
    enabled: bool,
}

impl TrapTable {
    pub fn new() -> Self {
        Self { traps: HashMap::new(), untrapped: HashSet::new(), enabled: true }
    }
    pub fn register(&mut self, addr: u32, name: &'static str, ret: TrapReturn, body: TrapFn) {
        self.traps.insert(addr & 0xFF_FFFF, TrapInfo { name, ret, body });
    }
    pub fn unregister(&mut self, addr: u32) {
        self.traps.remove(&(addr & 0xFF_FFFF));
    }
    /// Global kill switch: `false` makes this an oracle (pure emulation).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// A/B: run the original bytes for one address without unregistering.
    pub fn set_untrapped(&mut self, addr: u32, untrapped: bool) {
        if untrapped {
            self.untrapped.insert(addr);
        } else {
            self.untrapped.remove(&addr);
        }
    }
    pub fn is_trapped(&self, addr: u32) -> bool {
        self.enabled && self.traps.contains_key(&addr) && !self.untrapped.contains(&addr)
    }
    pub fn get(&self, addr: u32) -> Option<TrapInfo> {
        if self.is_trapped(addr) { self.traps.get(&addr).copied() } else { None }
    }
    pub fn len(&self) -> usize {
        self.traps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.traps.is_empty()
    }
    pub fn addrs(&self) -> impl Iterator<Item = u32> + '_ {
        self.traps.keys().copied()
    }
}

pub const TRAP_LOG_CAP: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapRecord {
    pub addr: u32,
    pub name: &'static str,
    pub frame: u64,
}

/// Ring of recently fired traps; `last()` names the suspect for a lockstep
/// divergence.
#[derive(Default)]
pub struct TrapLog {
    ring: VecDeque<TrapRecord>,
}

impl TrapLog {
    pub fn push(&mut self, r: TrapRecord) {
        if self.ring.len() == TRAP_LOG_CAP {
            self.ring.pop_front();
        }
        self.ring.push_back(r);
    }
    pub fn last(&self) -> Option<&TrapRecord> {
        self.ring.back()
    }
    pub fn iter(&self) -> impl Iterator<Item = &TrapRecord> {
        self.ring.iter()
    }
    pub fn clear(&mut self) {
        self.ring.clear();
    }
}
