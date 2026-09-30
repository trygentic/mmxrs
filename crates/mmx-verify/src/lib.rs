//! mmx-verify: lockstep a trapped `Game` (DUT) against an untrapped one
//! (oracle) and report the first divergence; plus BizHawk/lsnes movie
//! parsers. std-only, no fs: callers pass bytes in. See `docs/verify.md`.

mod lockstep;
#[cfg(feature = "movies")]
pub mod movie_bk2;

pub use lockstep::{hash_state, CompareOpts, Divergence, Lockstep, Port, Region};
