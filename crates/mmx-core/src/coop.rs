//! Two-player co-op: player 2 as a shadow copy of X's object block, driven by
//! the ROM's own player code on pad 2 (z2rs model).
//! See `docs/PLAN.md` "Co-op hooks".

/// Off by default; never registered by any verification path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoopState {
    pub enabled: bool,
}
