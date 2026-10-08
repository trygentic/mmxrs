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
| shared Trygentic jgenesis | pinned GPL-3.0-only SNES core, including MMX and LTTP host hooks; see [shared-core development](docs/shared-core.md) |
| `crates/mmx-core` | `Game`: emulator wrapper, trap table, RAM map, co-op |
| `crates/mmx-verify` | lockstep oracle-vs-DUT compare, `.bk2` movies |
| `crates/mmx-net` | rollback / lockstep netplay protocol (from z2rs), WebRTC via matchbox |
| `crates/mmx-native` | desktop app + headless runner |
| `crates/mmx-launcher` | egui window that configures and starts `mmxrs` |
| `crates/mmx-web` | wasm cdylib + static site |
| `tools/xtask` | `cargo xtask verify|dump|ledger|rominfo` |
| `tools/mmx-signal` | signalling server for netplay |
| `ports.toml`, `ram-map.toml` | routine and RAM ledgers |

## Quickstart

```sh
make run        # build and open the launcher
make game       # build and start the game directly (ROM from MMX_ROM or roms/)
make coop       # local two-player
make zero       # play as Zero (needs the X3 dump)
```

Manual commands:

```sh
cargo build --workspace                 # no ROM needed
cargo test --workspace                  # ROM-gated tests skip without MMX_ROM
export MMX_ROM=/path/to/Mega\ Man\ X\ \(USA\).sfc
cargo xtask rominfo                     # title, version byte, sha256
cargo xtask verify --frames 600         # oracle vs DUT, zero input
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --coop-local
# Zero with X3's sprites and the saber (optional Mega Man X3 (USA) dump):
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --coop-local --x3-rom "/path/to/Mega Man X3 (USA).sfc"
# player 1 plays as Zero too (single player or co-op; needs the X3 ROM):
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --x3-rom "/path/to/Mega Man X3 (USA).sfc" --zero
# online co-op: one side --coop-host ROOM, the other --coop-join ROOM, same --mode on both
# (rollback default; on a good link --mode lockstep --delay 3 plays smoothest, see docs/netplay.md)
```

Player 1: arrows, Z jump, X buster, C dash, V saber (with `--zero`). Player 2:
WASD, G jump, F buster, H dash, T saber (X3 ROM loaded). Without an X3 ROM
player 2 is X's sprites in Zero's colours, buster only.

Prefer a window to flags? The launcher picks the ROMs, options and online
settings, remembers them, and starts the game (docs/launcher.md):
`cargo build --release -p mmx-native -p mmx-launcher && ./target/release/mmxrs-launcher`.

Per-crate docs: `docs/launcher.md`, `docs/traps.md`, `docs/verify.md`, `docs/coop.md`,
`docs/netplay.md`, `docs/native.md`, `docs/web.md`.
