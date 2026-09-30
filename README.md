# mmxrs

Reconstruction of Mega Man X (SNES, USA) in Rust, with two-player co-op
(Zero as player 2) and rollback netplay over WebRTC. Native desktop and
browser (WASM). Modelled on [z2rs](../z2rs-private): the original ROM runs
inside a forked [jgenesis](https://github.com/jsgroth/jgenesis) SNES core and
hand-ported routines are installed as traps on their entry addresses, so the
game is playable at every percentage ported and every routine is diffed in
lockstep against the untrapped core.

Bring your own ROM: nothing from the cartridge is in this repository. See
[LEGAL.md](LEGAL.md). Licence: GPL-3.0 (inherited from jgenesis).

## Layout

| path | what |
|---|---|
| `docs/PLAN.md` | the plan and the `mmx-core` contract; read first |
| `vendor/jgenesis/` | forked SNES core crates, every change marked `mmxrs patch` |
| `crates/mmx-core` | `Game`: emulator wrapper, trap table, RAM map, co-op |
| `crates/mmx-verify` | lockstep oracle-vs-DUT compare, `.bk2` movies |
| `crates/mmx-net` | rollback / lockstep netplay protocol (from z2rs), WebRTC via matchbox |
| `crates/mmx-native` | desktop app + headless runner |
| `crates/mmx-web` | wasm cdylib + static site |
| `tools/xtask` | `cargo xtask verify|dump|ledger|rominfo` |
| `tools/mmx-signal` | signalling server for netplay |
| `ports.toml`, `ram-map.toml` | routine and RAM ledgers |

## Quickstart

```sh
cargo build --workspace                 # no ROM needed
cargo test --workspace                  # ROM-gated tests skip without MMX_ROM
export MMX_ROM=/path/to/Mega\ Man\ X\ \(USA\).sfc
cargo xtask rominfo                     # title, version byte, sha256
cargo xtask verify --frames 600         # oracle vs DUT, zero input
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --coop-local
```

Per-crate docs: `docs/traps.md`, `docs/verify.md`, `docs/coop.md`,
`docs/netplay.md`, `docs/native.md`, `docs/web.md`.
