# Shared SNES core

This game now links the shared RecompArch jgenesis crates at immutable revision
`dd7efd1cbd5a246a72a64e6bfed225292741c42a`. The independent game wrapper, routine ports, settings, multiplayer and
mod APIs remain in this repository. The old vendored emulator tree has been removed.

The shared core source and provenance are in
[trygentic/RecompArch](https://github.com/trygentic/RecompArch/tree/dd7efd1cbd5a246a72a64e6bfed225292741c42a/cores/jgenesis).
All repositories are currently private; developers and CI need read access to RecompArch.
Cargo fetches through SSH. With an SSH key authorized for that repository, run:

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build
```

Cargo.lock records the exact Git source. Do not update individual core packages to different
revisions: the CPU/frontend/config types must come from the same dependency graph.
Make core fixes in RecompArch and test both MMX and LTTP before updating this pin. For local
cross-repository development, temporarily patch **all nine** crates from this Git URL to
paths in a local RecompArch checkout with Cargo's `[patch."ssh://git@github.com/trygentic/RecompArch.git"]` table.
Keep those machine-specific paths out of committed manifests/lockfiles.

MMX uses TimingProfile::MmxHardware (the default). Its existing snapshots use LegacyMmxSaveState, preserving the original state header and P2/co-op tails.

This migration shares source; Cargo still statically links the core into each runner. One
installed runtime serving multiple game modules is the next architectural phase, with a
versioned process/module interface. It must preserve these per-game compatibility settings.

The shared core is GPL-3.0-only. Retain its full license/notices and provide corresponding
core plus runner source when distributing binaries. A private repository URL alone does
not fulfill source delivery to recipients without access. See LEGAL.md in both repositories.
Historical documents mentioning vendor/jgenesis refer to the pre-migration layout.
