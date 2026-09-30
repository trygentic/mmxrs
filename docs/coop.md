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

## Hooks (all `TrapReturn::Observe`)

| addr | what | P2 body |
|---|---|---|
| `$80:D450` | shots loop, `JSR` from `$80:D24A` in the object master `$80:D203`, after `JSL $81:812E` at `$80:D22C` | update slice |
| `$80:D63B` | OAM lists 3-5, `JSR` from `$80:D5A1`, right after the player emitter `$80:D6B2` | display slice |
| `$84:9D07` | enemy-contact damage on X (a `JMP` target in `$84:9B03`, after the hit test `$84:9C0E` with `X=$0BA8`) | stub: counts hits |

**Update slice.** Runs only when `$D2==4 && $D3==4`. Otherwise P2 is hidden and
gets re-anchored when it next appears (a copy of P1's block, +16 px in X, HP =
max HP `$1F9A`). The slice:

1. Swaps the block in.
2. Writes `$0BDE/$0BE0/$0BE2` from pad 2 (`$AD`), using a Rust port of
   `$80:E559`.
3. Runs `$81:812E` isolated.
4. Clamps X to `[cam, cam+240]`.
5. Swaps the block out.
6. Restores the P1-owned shared bytes: camera requests `$1E48..$1E89` and the
   freeze flags `$1F13..$1F19`.

P2 is down when its action is `$0C` or HP&$7F is 0. It is then hidden for 120
frames and respawns next to P1 at no life cost. The death handler never runs
on P2, because it drives the stage-wide death flow.

**Display slice.** The emitter `$80:D6BD` places sprites through the game's own
OAM cursor at direct page `$E4` (next slot; `$E5/$E6` hold the high-table
state). The slice records `$E4`, swaps P2 in, runs `$80:D6B2` isolated, swaps
back, and forces the Zero palette on slots `[old $E4, new $E4)`. P2's sprites
therefore land in the next free slots and can't collide with the game's
entries. When the table is full, the lower-priority lists 3-5 lose slots
first. The slice is skipped if fewer than 32 slots remain, because the
emitter's overflow exit `$80:D77B` pops two frames.

**Input.** `$80:E559` packs the pad as `B Y A X L R Sel St`. Each of the six
config masks at `$7E:FFC0` fires when all of its bits are held. The results go
to `$0BDE` bits 7/5/4 (cfg 2/3/4) and `$0BDF` bits 7/6/4 (cfg 1/0/5) plus the
d-pad nibble. Pressed = `!prev & held`. Pause (`$80:9E7E`) tests `$0BE3 & $10`
(cfg 5, the menu action). P2 has Start, Select and cfg 5 masked out, so it
can't pause.

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

## Verify once `MMX_ROM` exists

- [ ] Lockstep: with the group registered and `enabled=false`, output is
      byte-identical to the oracle over a movie.
- [ ] `call_isolated` handles `$80:D6B2` (JSR/RTS) as well as `$81:812E`
      (JSL/RTL). Both restore A/X/Y/P/D/DB.
- [ ] `D=0` and `DB=$8D` at `$80:D63B`. `DB=$86` and `M=0,X=1` at `$80:D450`.
- [ ] `$A7/$A8` and `$AD/$AE` are the raw `$4218/$4219` bytes (the `Pads`
      layout).
- [ ] The default button table `$86:EE20` maps cfg 0..5 to shot, jump, dash,
      L, R and menu. The test `CFG` in `coop.rs` is a guess.
- [ ] `$D2==4 && $D3==4` is exactly "player controllable". Check boss intros,
      the teleport-in and Chill Penguin's ice.
- [ ] Restoring `$1E48..$1E89` keeps the camera on P1 and breaks no scroll
      locks. Restoring `$1F13..$1F19` is safe.
- [ ] P2's X uses a sprite anchor. Tune the clamp window after seeing it on
      screen.
- [ ] X's maximum sprite count is at most 32 (the overflow guard).
- [ ] Palette 7 is free in every stage, or pick another.
- [ ] Death detection (action `$0C` / HP 0) catches pits and spikes before any
      global write.
- [ ] `$0C38/$0C58/$0C78` clobbering: decide whether to shadow them too.
- [ ] P1-only side effects in `$81:812E` from the static walk: `$1F0B-$1F0F`,
      `$1F23`, `$1F25`, `$1F2C`, `$1F31-$1F3B`, `$1F81`, `$1F85,X`, `$1F99`,
      `$1F9D/$1F9E`, `$1FA1-$1FB0`, `$7F:829B/839B`, `$7E:F000+`. Check which
      of these P2 must not write.
