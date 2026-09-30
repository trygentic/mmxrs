# Routine traps

A trap is a Rust body registered on a 24-bit routine entry (`PBR:PC`). When
the CPU is about to execute the opcode there, the body runs instead of (or,
for `Observe`, before) the ROM bytes. Code: `crates/mmx-core/src/game.rs`
(`dispatch_traps`), table in `traps.rs`.

## The boundary hook

`SnesEmulator::tick` advances one CPU bus cycle (or a DMA / DRAM-refresh
slice). `Game::step` calls `dispatch_traps` before every tick:

1. CPU mid-instruction: clear `fired_at`, return.
2. At a boundary: if `PBR:PC == fired_at`, this boundary visit was already
   handled (DMA and refresh ticks leave the CPU parked at the same boundary
   for many ticks), return. Untrapped addresses are recorded in `fired_at`
   too, so the table is looked up once per instruction.
3. Trapped address: ask `next_tick_fetches_opcode()`. If the next tick would
   start an NMI/IRQ (or the CPU sits in WAI), do nothing; the handler runs,
   its `RTI` comes back here and the trap fires then. Without this an
   `Observe` trap fired twice when the NMI landed on its entry, and an
   `Rts` trap ran its body before an NMI the ROM would have taken first.
4. Fire: log to `traplog` (`addr`, `name`, `frame_count`), run the body,
   then emulate the return and loop (the return address may be trapped too).

`step` runs past the rendered frame to the next instruction boundary, so a
step always ends between instructions: `set_regs`, `call_isolated` and save
states are safe between steps. `fired_at` is part of the save state.

## Returns

- `Rts`: pop 2 bytes from the stack, `PC = word + 1`.
- `Rtl`: pop 3 bytes, `PC = word + 1`, `PBR = third byte`.
- `Observe`: registers untouched; the ROM routine then runs normally.

The stack is read from WRAM (bank 0 `$0000-$1FFF` mirrors `$7E:0000`);
emulation mode wraps `S` within page 1. The body must leave A/X/Y/P/D/DBR
as the original routine would; the dispatcher only rewrites S, PC, PBR.

## call_isolated

`call_isolated_ret(addr, Rts|Rtl)` pushes a `JSR`- or `JSL`-shaped return to
`bank(addr):$FFFF` (the sentinel: high byte of the IRQ vector, never code),
sets `PBR:PC = addr` with the caller's other registers, and ticks only the
CPU (`SnesEmulator::run_cpu_isolated`, one tick at a time) until the
sentinel is reached; then every register is restored. PPU, APU, DMA and the
master clock do not move, so the call costs zero emulated time. Traps fire
inside with the same dispatch. Re-entrancy: a trap body may call
`call_isolated`; the outer `fired_at` is saved and restored around it.
Caveats: no new NMI/IRQ edge can arrive (the callee must not wait for one);
a routine that never returns panics after 2,000,000 CPU cycles; a pending
DMA is not modelled at the trap boundary (it runs after the body).

## Why no CPU-crate patch

`wdc65816-emu` is untouched. Everything the hook needs is outside the CPU:
`is_mid_instruction()` and `registers()/set_registers()` are already public,
and "will the next tick fetch an opcode" is answered in `snes-core` by
ticking a *clone* of the CPU against a side-effect-free probe bus, the same
trick upstream uses for `next_cpu_cycle_mclk`. It costs one clone per hit on
a trapped address, not per instruction. Keeping the CPU pristine keeps the
oracle (traps disabled) and the DUT on exactly the same interpreter.

## Isolated calls and DMA

`run_cpu_isolated` (vendored patch) drains the general-purpose DMA unit after
every CPU tick, so a routine that starts a DMA (`$420B`) sees the transfer
complete before its next instruction, as on hardware. The PPU, APU and master
clock still do not advance.

## Debugging traps

- `cargo xtask trace --frames F --calls`: every call target of frame F with
  its first caller. This is how the real hook addresses were found; the
  disassembly's bank-`$80` PCs drift from the cartridge.
- `trace --pcs FILE` writes every PC of the frame (feed a disassembler).
- `Game::isolated_trace = Some(vec![])` records every PC fetched inside
  `call_isolated*`; `cargo xtask cooptrace --frame F` diffs P1's and P2's
  player update and prints each branch that went a different way (`--phase 2`:
  the terrain phase `$81:9D67`; `--dump PREFIX`: both PC lists).
- `cargo xtask lagdiff --frames N [script] [--refill] [--dump A-B]`: solo
  and co-op (P2 idle) in lockstep; the first real-time opcode fetch where
  P1's PC or master clock differs (isolated calls are not in the trace).
  With P2 drawn, the traces differ from frame 2 (NMI uploads), so `--dump`
  both traces and compare phase times (how the Rev 1 lag frame was found).
