//! Per-phase wall time of [`crate::Game::step`] (docs/widescreen.md
//! "Performance"). Off until the host installs a clock ([`set_clock`]; the
//! core has none, wasm): then every trap body is timed under its trap name,
//! exclusive of nested traps, plus a few named passes. `xtask bench` with
//! `MMX_PROFILE=1` prints the table.

use std::cell::RefCell;

pub use snes_core::prof::set_clock;
use snes_core::prof::now;

#[derive(Default)]
struct Prof {
    /// Open scopes: (name, start, time spent in nested scopes).
    stack: Vec<(&'static str, u64, u64)>,
    /// Exclusive nanoseconds per name since the last [`take`].
    totals: Vec<(&'static str, u64)>,
}

thread_local! {
    static P: RefCell<Prof> = RefCell::default();
}

/// Times its lifetime under `name` (no-op without a clock).
pub(crate) struct Scope(bool);

pub(crate) fn scope(name: &'static str) -> Scope {
    let Some(t) = now() else { return Scope(false) };
    P.with_borrow_mut(|p| p.stack.push((name, t, 0)));
    Scope(true)
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        let t = now().unwrap_or(0);
        P.with_borrow_mut(|p| {
            let Some((name, start, child)) = p.stack.pop() else { return };
            let all = t.saturating_sub(start);
            if let Some(parent) = p.stack.last_mut() {
                parent.2 += all;
            }
            add(&mut p.totals, name, all.saturating_sub(child));
        });
    }
}

fn add(totals: &mut Vec<(&'static str, u64)>, name: &'static str, ns: u64) {
    match totals.iter_mut().find(|(n, _)| *n == name) {
        Some(e) => e.1 += ns,
        None => totals.push((name, ns)),
    }
}

/// Exclusive ns per phase since the last call. [`EMULATOR`] is the step's
/// own time (CPU/PPU/APU ticks, trap dispatch) minus `ppu margins`.
pub fn take() -> Vec<(&'static str, u64)> {
    let wide = snes_core::prof::take_wide_ns();
    P.with_borrow_mut(|p| {
        let mut v = std::mem::take(&mut p.totals);
        if wide > 0 {
            if let Some(e) = v.iter_mut().find(|(n, _)| *n == EMULATOR) {
                e.1 = e.1.saturating_sub(wide);
            }
            add(&mut v, "ppu margins", wide);
        }
        v
    })
}

pub const EMULATOR: &str = "emulator";
