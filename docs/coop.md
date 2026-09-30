# Co-op: player 2 as Zero (scaffold, v1)

Code: `crates/mmx-core/src/coop.rs`. It follows the z2rs model. **No ROM run has
tested any of this yet.** Addresses come from reading the v1.0 disassembly.

## Model

X's object block `$7E:0BA8..$0C38` (`$90` bytes) is the only per-player state.
X's remapped input `$0BDE..$0BE3` sits inside it. P2 is `CoopState::p2`, a
shadow copy of that block. For each P2 pass, the shadow is swapped into place,
the ROM's own code runs on it, and it is swapped back out. Everything else is
shared by both players: camera, lives `$1F80`, weapon energy, sub-tanks, RNG,
sound queue and enemies.

P2's ROM work goes through `Game::call_isolated`, which ticks only the CPU.
The PPU, APU and master clock don't advance, so P2 adds no emulated cycles and
can't make the main loop lag.

Co-op is off by default. When `enabled == false`, every trap body returns
immediately. A game with the group registered but disabled must therefore be
byte-identical to the oracle.

## Hooks (all `TrapReturn::Observe`, addresses confirmed on the real ROM)

The disassembly's bank-`$80` PCs drift from this cartridge by up to `0x41`
(bank `$81`/`$84` PCs matched), so every hook below was taken from
`cargo xtask trace --calls` on a gameplay frame, not from the listing.

| addr | what | P2 body |
|---|---|---|
| `$80:D1EF` | instruction after the object master's `JSL $81:812E` (P1's action/input phase has just returned) | update slice: ROM input remap `$80:E518` on pad 2, then `$81:812E` |
| `$80:D241` | instruction after `JSL $81:9D67` (P1's terrain/physics phase, run after the enemy loops) | physics slice: `$81:9D67`, clamp into the camera window, death check |
| `$80:D5FA` | OAM lists 3-5, right after the player emitter `$80:D671` inside the OAM build `$80:D544` | display slice: `$80:D671` on P2's block through the OAM cursor `$E4`, then the Zero palette |
| `$84:9D07` | enemy-contact damage on X | stub: counts hits (unverified address) |

Per-frame order in the object master `$80:D1C2`: player phase 1 (`$81:812E`)
-> item/shot/enemy loops -> player phase 2 (`$81:9D67`) -> camera `$80:DDB0`.
Both player phases must run for P2: with only the first, P2 kept a stale
ground flag (`$0BD3` bit 2, cleared by phase 2) and "landed" at the apex of
every jump.

Two more things the real ROM taught:

- **Anchor when grounded.** A block copied while P1 is still in the
  stage-entry drop inherits an action that never lands; P2 fell through the
  floor. `p2_update` waits until P1's vertical speed is zero and its Y has not
  changed since the previous frame.
- **A GP-DMA started inside an isolated call must complete before the next
  instruction.** `run_cpu_isolated` now drains the DMA unit after every CPU
  tick (vendored patch).

## v1: what P2 can and can't do

P2 can:
- Run, jump, dash, wall-kick and shoot, using the ROM's own code.
- Switch weapons. The energy pool is shared.

P2 can't:
- Take contact damage. Enemies hit only P1, so P2 dies only from what the
  player code does to itself: spikes and pits.
- Move the camera, trigger freezes, doors or boss rooms, or pause.
- Get its own HUD bar.

Other v1 limits:
- Enemy AI tracks P1.
- P2's sprites use X's graphics with palette 7. The palette is a placeholder:
  nobody has checked what it holds per stage, and Zero's colours are not
  written.
- The player update writes the effect objects `$0C38/$0C58/$0C78` (outside the
  block), so P2's charge or dash effects can clobber P1's.
- Sound requests are shared.
- P2 advances the shared RNG.
- P2 writes to the SRAM probes `$70:0800` and `$70:0804`. The anti-piracy
  counter is at `+$83` inside the block.

## v2 plan

- **Contact.** At `$84:9B03`, run the hit test `$84:9C0E` again with P2's block
  swapped in. Route the damage (`$84:9D29..`, which writes absolute `$0BAA`,
  `$0BCF` and `$0C11`) to whichever player was touched.
- **Separate HP bar.** Run `$80:D7E4` again with P2's HP at another OAM
  position. Add a revive (touch the downed P2) in place of the timer.
- **Real Zero sprites.** Extract X2/X3 sprites or use HD-pack art as a new
  sprite set (object `+16`) and frame table (`+17`) in `$8D:8000`. Write
  Zero's red into a free CGRAM sprite palette through the `$0300` buffer.

## Verified on the real ROM (2026-09-30, USA v1.0)

- [x] Parity: group registered, `enabled=false`, 2400 frames with P1 walking
  are byte-identical to the untrapped oracle (`cargo xtask verify --coop-off`).
- [x] `call_isolated_ret` runs `$80:D671` (JSR/RTS) and `$80:E518` (JSR/RTS);
  `call_isolated` runs `$81:812E` and `$81:9D67` (JSL/RTL).
- [x] `D=0`, `DB=$8D`, `P=$22` at the display hook.
- [x] `$A7` holds the raw `$4218/$4219` word in the `Pads` layout (P1 B ->
  `$0BDE=$80`, A -> `$0BDF=$80`, Y -> `$0BDE=$40`).
- [x] `$D2==4 && $D3==4` is gameplay on the intro stage; the "ready" phase is
  `$D3==2`.
- [x] P2 stands, walks, jumps and lands exactly like P1; P2 is clamped to the
  camera window and follows P1's scrolling; P1's HP and position stay identical
  to a solo run.
- [x] Palette 7 shows P2 in a distinct (red/orange) palette on the intro stage.

Still open:

- [ ] Contact damage on P2 (`$84:9D07` is unverified; use the trace).
- [ ] Death detection (action `$0C` / HP 0) for pits and spikes.
- [ ] Restoring `$1E48..$1E89` on every scroll type (only horizontal seen).
- [ ] `$0C38/$0C58/$0C78` effect objects: P2's dash/charge effects clobber P1's.
- [ ] Sound requests from P2 go to the shared queue (not masked yet).
- [ ] Other stages, bosses, the weapon menu and the stage-select flow.
