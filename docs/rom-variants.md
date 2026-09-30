# ROM variants

Every routine the co-op, Zero and display traps hook or call (27 of them) is
listed in `crates/mmx-core/src/hooks.rs` with its address on our dump and a
16-byte signature (`??` over operands that point into ROM: call/jump targets,
long addresses into banks `$80+`, words `>= $8000`). At load
(`Game::from_rom*`) each signature is checked at that address, then searched
for (same bank first, then the whole ROM); every trap registration and every
`call_isolated*` goes through the result (`Game::hook`). A hook found nowhere
or in more than one place is NOT FOUND, and `coop::register_coop_traps`
refuses co-op (and `--zero`) with the list instead of running without it.
Data tables are not signed: `$8D:8000` (sprite layouts) is identical on both
dumps below; `zero_data::extract_x1` (research only) reads `$86:F69B`, which
moved on v1.1.

| dump | sha256 | hooks | checked |
|---|---|---|---|
| Europe (PAL) v1.0, `$FFD9 = 2` | `1c41bb11c9df5aa8…` | all where traced (the table's own addresses) | everything: tests, verify, `coop-qa.sh`, netplay |
| USA (NTSC) v1.1 (Rev 1), `$FFD9 = 1` | `b8f70a6e7fb93819…` | 16 relocated: bank `$80` `+$20` (`$9A86`, `$9A94`), `+$2B` (from `$D1EF` on); banks `$81/$82/$84` as PAL | parity verify, `zero_p1_palette_stable`, `hooks::resolves_on_usa_v11`, `coop-qa.sh` on its own states |
| USA (NTSC) v1.0 | - | not seen yet (the disassembly's PCs are up to `0x41` off ours in bank `$80`) | - |

`cargo xtask rominfo` prints where each hook landed ("as PAL", "relocated by
N", "NOT FOUND"); the native frontend prints the same on stderr and the web
page on the console when anything moved. With a new dump, please run and
send back:

    MMX_ROM=path/to/dump cargo xtask rominfo
    MMX_ROM=path/to/dump cargo xtask verify --frames 2400 --coop-off --tap 200,400,600,800,1000 --hold 0100
    MMX_ROM=path/to/dump cargo xtask dump --out target/v --frames 1500 --tap 200,400,600,800,1000 --at 1500 --save 1500=states/NEW-highway-start.state
    QA_STATES=states/NEW HIGHWAY_START=states/NEW-highway-start.state QA_OUT=target/qa-NEW MMX_ROM=path/to/dump scripts/coop-qa.sh all

On v1.1 (`states/usa11/`, NTSC) `coop-qa.sh` gives P1 idle parity yes for
M0-M2, M4-M6 and M8, but **no** for M3 (P1 gets one extra lag frame at f269,
both players in knockback; parity comes back with `--untrap 849B03`, the
contact pass) and M7 (camera 3 px off at f600). The same route replayed on
v1.1 at PAL timing (`MMX_TIMING=pal`, `states/usa11pal/`) is idle-clean on
all nine, and the contact routine `$84:9B03..$84:9E00` is byte-identical on
both dumps, so this is P2's presence tipping NTSC's shorter frame budget,
not a misplaced hook. Open.

(QA states are per dump; at PAL timing the boot needs taps up to 1400 and
~1700 frames. A save state from one dump does not load on
another.) Tests: `MMX_ROM` pins the PAL signatures when it is the PAL dump;
`MMX_ROM_V11` runs the v1.1 resolution test.

Netplay: both peers must load the identical dump. The handshake compares the
ROM CRC32 and the trap-set id (which hashes the resolved trap addresses), so a
PAL peer and a USA peer refuse to connect rather than desync.
