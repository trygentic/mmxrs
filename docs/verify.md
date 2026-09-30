# Verification

## Oracle

The oracle is the **same core with traps disabled** (`game.traps.set_enabled(false)`):
pure jgenesis emulation of the ROM bytes. The DUT is the same core with the
trap table on, so every ported routine runs as Rust. Same ROM, same pads,
same emulator: any difference is a porting bug, never emulator noise.

## Compare order

After every frame `Lockstep::run` (crate `mmx-verify`) diffs, stopping at the
first difference:

1. WRAM `$7E:0000-$1FFF` (direct page, stack, scheduler, X and object blocks)
2. rest of WRAM (`$7E:2000-$7F:FFFF`)
3. OAM (544 bytes), 4. CGRAM (256 words)
5. framebuffer (skipped with `--no-frame`)

RAM goes first because a wrong byte shows up there frames before it reaches
the screen. `CompareOpts::skip_ranges` masks known-noisy WRAM bytes (none
yet). The report is one greppable line:

```
DIVERGENCE frame=812 region=WRAM addr=$7E:0BAD expected=$34 actual=$35 last_trap=player_update $81:812E @frame 812
```

`frame` is the 0-based input frame whose step diverged.

## Running

```
export MMX_ROM=~/roms/mmx.sfc              # your own dump; never committed
cargo xtask rominfo                        # title / version / sha256 / crc32
cargo xtask verify --frames 3600           # zero input
cargo xtask verify --movie run.bk2         # BizHawk; .lsmv (lsnes) also works
cargo xtask dump --frames 600 --out shots/ --at 60,300,600
cargo xtask ledger                         # ports.toml progress
```

Exit codes: 0 clean, 1 divergence, 2 setup error (e.g. `MMX_ROM` unset).

## When it diverges

- **`last_trap` names a routine**: that port is the prime suspect. A/B it:
  `game.traps.set_untrapped(addr, true)` and rerun. If the divergence goes
  away, diff the Rust body against the disassembly for the named address
  (registers and flags at return count too: the caller may branch on them).
- **No trap, or the trap is innocent**: the bug fired earlier and only just
  became visible. Bisect by untrapping groups of traps.
- **Second opinion**: open the ROM in Mesen2, play the same movie to the
  reported frame, and use the trace logger / memory breakpoint on the
  diverging address to see which instruction writes it. Mesen2 is external,
  not embedded; it settles "is our core or our port wrong" questions.

## Input scripts and the co-op parity check

`verify`, `dump` and `trace` share one input script: `--tap F,..` (Start on
pad 1 for 4 frames at F), `--hold HEX` / `--p2-hold HEX` (OR'd every frame,
from `--from F` on), `--p1tap HEX@F` / `--p2tap HEX@F`. Reaching the intro
stage without a movie: `--tap 200,400,600,800,1000`; gameplay starts around
frame 1250 and P1 is grounded by 1400.

`verify --coop-off` registers the co-op trap group on the DUT with the flag
off; the run must stay identical to the oracle (the parity rule).
