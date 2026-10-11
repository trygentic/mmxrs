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
make zero       # play as Zero with the X1 ROM's sprites
```

Manual commands:

```sh
cargo build --workspace                 # no ROM needed
cargo test --workspace                  # ROM-gated tests skip without MMX_ROM
export MMX_ROM=/path/to/Mega\ Man\ X\ \(USA\).sfc
cargo xtask rominfo                     # title, version byte, sha256
cargo xtask verify --frames 600         # oracle vs DUT, zero input
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --coop-local
# X1 Zero by default in local co-op and player 1:
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --coop-local
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --zero
# Optional X3 Zero art and saber (requires your Mega Man X3 (USA) dump):
cargo run --release -p mmx-native -- --rom "$MMX_ROM" --coop-local --x3-rom "/path/to/Mega Man X3 (USA).sfc"
# online co-op: one side --coop-host ROOM, the other --coop-join ROOM, same --mode on both
# (rollback default; on a good link --mode lockstep --delay 3 plays smoothest, see docs/netplay.md)
```

Player 1: arrows, Z jump, X buster, C dash, V saber with the optional X3 art.
Player 2: WASD, G jump, F buster, H dash, T saber with the optional X3 art.
X1 Zero uses his original standing and dash poses; the X3 ROM adds its full
animation set and saber.

Prefer a window to flags? The launcher picks the ROMs, options and online
settings, remembers them, and starts the game (docs/launcher.md):
`cargo build --release -p mmx-native -p mmx-launcher && ./target/release/mmxrs-launcher`.

The launcher now offers 1–4 local controller slots, a character selector per
slot, optional multiply lighting, and optional X/Zero charge glow and sparks.
Each local player has independent movement and a sprite page and palette.
Boss choices draw their sprites from the selected Mega Man X ROM. Sigma and
Vile use ROM poses while moving; other bosses use their idle pose for actions
without a mapped animation. Some stage-scale bosses have compact or partial
player-sized renditions (notably Wolf Sigma and Rangda Bangda).

Per-crate docs: `docs/launcher.md`, `docs/traps.md`, `docs/verify.md`, `docs/coop.md`,
`docs/netplay.md`, `docs/native.md`, `docs/web.md`.
