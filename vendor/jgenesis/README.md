# vendored jgenesis (fork)

Origin: https://github.com/jsgroth/jgenesis, commit `cc10b2bd` (workspace
version 0.14.1). Licence: GPL-3.0 (see the repo root `LICENSE`); the whole of
mmxrs is GPL-3.0 because of this code.

Crates copied verbatim from the upstream tree:

| here | upstream |
|---|---|
| `snes-core` | `backend/snes-core` |
| `snes-coprocessors` | `backend/snes-coprocessors` |
| `wdc65816-emu` | `cpu/wdc65816-emu` |
| `spc700-emu` | `cpu/spc700-emu` |
| `arm7tdmi-emu` | `cpu/arm7tdmi-emu` |
| `dsp` | `common/dsp` |
| `jgenesis-common` | `common/jgenesis-common` |
| `jgenesis-proc-macros` | `common/jgenesis-proc-macros` |
| `snes-config` | `config/snes-config` |

These are path dependencies of `crates/mmx-core`, not workspace members
(the root `Cargo.toml` excludes `vendor/`).

## Patches

Every source change carries a `// mmxrs patch` comment.

1. **Manifests** (all nine `Cargo.toml`): `{ workspace = true }` dependencies
   rewritten to the explicit versions from upstream `[workspace.dependencies]`
   or to `path = "../<crate>"`; `version = "0.14.1"` inlined; the
   `[lints] workspace = true` tables removed (no jgenesis workspace here).
   `edition = "2024"` kept.
2. `jgenesis-common/src/audio/fir_resampler.rs`: `f64::algebraic_add` /
   `algebraic_mul` (unstable on Rust 1.96) replaced with `+` / `*`.
3. `snes-core/src/memory.rs`: WRAM is zero-filled at power-on instead of
   `rand::random` (deterministic boot for lockstep and netplay); added
   `Memory::main_ram` / `main_ram_mut`.
4. `snes-core/src/ppu.rs`: added `Ppu::oam_bytes` (512 B low table as
   little-endian words, then the 32 B high table) and `Ppu::cgram`.
5. `snes-core/src/api.rs`: added to `SnesEmulator`: `cpu_registers`,
   `set_cpu_registers`, `at_instruction_boundary`, `next_tick_fetches_opcode`
   (ticks a clone of the CPU against a side-effect-free probe bus, the same
   trick upstream's `bus::next_cpu_cycle_mclk` uses, to tell an opcode fetch
   from an NMI/IRQ entry or WAI), `wram`, `wram_mut`,
   `oam_bytes`, `cgram`, and `run_cpu_isolated(until_pbr_pc, max_cycles)`
   which ticks only the main CPU (plus the CPU-clocked multiply/divide unit
   and deferred writes) against the normal bus; PPU, APU, DMA, coprocessors
   and `total_master_cycles` do not advance.

No change to the CPU crates: routine traps are dispatched by `mmx-core`
between `SnesEmulator::tick` calls (see `docs/traps.md`).
