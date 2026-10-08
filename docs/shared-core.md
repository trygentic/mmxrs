# Shared SNES core

This game now links the shared Trygentic jgenesis crates at immutable revision
`f39748c2e704b1d27b00f5e0aaac1f133fc14c19`. The independent game wrapper, routine ports, settings, multiplayer and
mod APIs remain in this repository. The old vendored emulator tree has been removed.

The shared core source and provenance are in
[trygentic/jgenesis](https://github.com/trygentic/jgenesis/tree/f39748c2e704b1d27b00f5e0aaac1f133fc14c19).
All repositories are currently private; developers and CI need read access to the jgenesis fork.
Cargo fetches through SSH. With an SSH key authorized for that repository, run:

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build
```

Cargo.lock records the exact Git source. Do not update individual core packages to different
revisions: the CPU/frontend/config types must come from the same dependency graph.
Make core fixes in trygentic/jgenesis and test both MMX and LTTP before updating this pin. For local
cross-repository development, temporarily patch **all nine** crates from this Git URL to
their upstream-layout paths in a local jgenesis checkout (common/, cpu/, config/, backend/) with Cargo's `[patch."https://github.com/trygentic/jgenesis.git"]` table.
Keep those machine-specific paths out of committed manifests/lockfiles.

MMX uses TimingProfile::MmxHardware (the default). Its existing snapshots use LegacyMmxSaveState, preserving the original state header and P2/co-op tails.

This migration shares source; Cargo still statically links the core into each runner.
An install-once runtime and SDK work are deferred.

The shared core is GPL-3.0-only. Retain its full license/notices and provide corresponding
core plus runner source when distributing binaries. A private repository URL alone does
not fulfill source delivery to recipients without access. See LEGAL.md in both repositories.
Historical documents mentioning vendor/jgenesis refer to the pre-migration layout.
