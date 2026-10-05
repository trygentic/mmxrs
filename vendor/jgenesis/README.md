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

- `snes-core/src/api.rs` `run_cpu_isolated`: after each CPU tick the DMA unit
  is ticked until idle, so a GP-DMA started by the isolated routine completes
  before its next instruction (2026-09-30).

- `snes-core/src/ppu.rs`, `ppu/sprites.rs`, `api.rs`: player 2's shadow sprite
  tile page for mmx-core (`P2Tiles`: 768-word page (48 tiles, three rows) +
  128-bit OAM slot mask,
  with pending copies that go live on the next OAMDATA write). A sprite whose
  slot bit is set reads its tiles from the page (`(tile & 0x3F) % 48`) instead of
  VRAM. `SnesEmulator::p2_tile_page_mut`, `set_p2_tile_slots`, `p2_tiles`,
  `set_p2_tiles`. Encoded as nothing inside the emulator state
  (`P2TilesSlot`), so old save states still decode; mmx-core appends it to its
  save state (2026-09-30).

- `snes-core/src/ppu/wide.rs` (new), `ppu.rs`, `ppu/sprites.rs`, `api.rs`:
  display-only widescreen margins (`docs/widescreen.md`). `set_wide_margin`
  makes every line render `margin` extra pixels each side into a separate
  buffer (BG from VRAM or the host's margin map `set_margin_tiles`, OAM
  sprites in the margins, the line's own colour math / brightness);
  `SnesEmulator::tick` hands the widened frame to the renderer. `Wide`
  encodes as nothing, clones empty, and survives `load_state`. Also
  `bg_map_entry`, `line_regs` / `set_line_record`, `ppu_registers_debug`
  (research), and `offset_sprite` made `pub(super)` (2026-10-01).

- Timing against hardware / bsnes v0.87 / Mesen 2 (`docs/oracle.md`; found
  by diffing the TASVideos 100% movies frame by frame against Mesen 2;
  2026-10-05). No new state fields, so old save states still load:
  - `snes-core/src/apu.rs`: the APU/CPU clock ratio is bsnes v0.87's,
    24,607,104 APU clocks per 21,477,272 CPU clocks (upstream: 32040 * 768
    per 21,477,270); `audio.rs`: the resampler's input rate is 32040.5 Hz
    to match.
  - `snes-core/src/ppu.rs`: H/V IRQs reach the CPU at H=18+4n (H, HV) and
    H=14 (V only) master clocks, not 10 (`IRQ_H_OFFSET_MCLKS`,
    `IRQ_V_OFFSET_MCLKS`).
  - `snes-core/src/memory.rs`: the VBlank NMI (and the RDNMI flag) rises at
    H=6 of the first VBlank line, not H=0 (`NMI_MCLK`).
  - `snes-core/src/memory/dma.rs`, `api.rs`: a DRAM refresh during DMA does
    not count towards the DMA length used for the end-of-DMA CPU-clock
    alignment (`DmaUnit::exclude_refresh`); `#[must_use]` back on
    `start_dma_if_inactive`.
  - `snes-core/src/bus.rs`, `api.rs`: a CPU read of `$2140-$217F` ticks the
    APU up to 4 master clocks before the end of the CPU cycle first
    (`Bus::apu_sync` / `apu_pre_ticked` / `apu_sample`; the main loop ticks
    the APU the rest of the cycle).
  - `snes-core/src/api.rs`: the CPU starts 186 master clocks after power-on /
    reset (`run_startup_delay`; PPU, APU, timers run meanwhile).
  - Debug: `SnesEmulator::ppu_position`, `SPC_TRACE` / `SPC_CYCLES`
    (thread-local SPC700 instruction trace, off unless enabled).
