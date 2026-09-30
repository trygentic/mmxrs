# Co-op: player 2 as Zero (scaffold, v1)

Code: `crates/mmx-core/src/coop.rs`. It follows the z2rs model. Everything
under "Verified" was run on the real ROM (USA v1.0); the rest is marked.

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
| `$84:9B03` | entry of the enemy-vs-X contact routine; every enemy's AI `JSL`s it with `D` = the enemy (154 call sites) | contact pass: the same routine again, isolated, with P2's block swapped in |

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

Per-player state swapped around every P2 slice besides the block
(`P2_SWAPPED`): landing scratch `$1F31..$1F37`, `$1FA1..$1FB0`, and X's three
companion objects `$0C38..$0C98` (see "Armour parts" below).

## Enemy contact (P2 takes damage)

Found by `cargo xtask trace --find` on the frame P1's HP dropped in a solo
run (`--load highway-start.state --hold 0100`, frame 287, HP 16 -> 14):
`$84:9B03` ran (called from the Spiky's AI at `$82:9C41`), then `$84:9B25`
and `$84:9D07`. The routine, with `D` = the enemy:

1. bails if the enemy has no HP (`$27`) or collision (`$0E`), or `$1F0C` is set;
2. if X is hurt/invincible (`$0C32` or `$0C30`, in the block) it tests the
   ride armour `$0E18` instead;
3. `LDX #$0BA8; JSL $84:9C0E` (box test, scratch in direct page `$00..$11`);
4. hit: `JMP $84:9D07` (cartridge has an extra `LDA $700505; INC A` there,
   so bank-`$84` PCs after it are listing + 5): SRAM probe `$1F9F`, then
   damage `$26` minus armour, action `$0E` (or `$0C` + HP 0 when it runs
   out), HP bit 7, i-frames `$0C32`, `$0BD8`, knockback side `$0C11`.
   Every write is inside the block. Returns A=1 (hit) / 0.

P2's pass is an `Observe` trap on the entry: if P2 is up and not hurt, swap
P2's block in, `call_isolated($84:9B03)` with the enemy's registers, swap out,
and put direct page `$00..$1F` back (P1's pass may bail before rewriting it).
A re-entry guard (`in_contact`) stops the trap firing inside its own call.
The enemy's AI only sees P1's result (A), so an enemy that dies on contact
dies only when it touches P1. Skipping P2 while it is hurt also skips the
ride-armour branch, which is shared.

Not covered: damage that does not go through `$84:9B03`: enemy shots and
hazards that call `$84:9F2A` / `$84:9F25` with `$0BCE` set (12 call sites,
e.g. `$81:B091`, `$82:A611`); these still hit only P1.

## P2 death

- **HP 0** (contact, or anything that runs `$81:96CB`): action `$0C`, HP
  `$80`. `p2_is_down` sees it at the next slice; P2's update never runs the
  death action, so the stage-wide death flow (`$D3=6`, lives) can't start.
  P2 hides for `P2_DOWN_FRAMES` (120) and re-anchors next to P1 at max HP.
- **Pits** are not decided by the player code: the camera code (`$80:E14A`,
  listing PC, run on P1 only) kills X when `Y - $20 >= camera Y + $E0` (by
  `$84:9F2A` with `$0BCE=$7F`). P2 never ran that, and fell to the floor
  below the screen (Y 895) and stood there. `p2_physics` now applies the same
  rule (`fell_off`) to P2 with the live camera and marks P2 down (action
  `$0C`, HP `$80`); no ROM death code runs for P2.
- **Spikes** (tile `$3F`) go through `$81:96CB` inside P2's update slice, so
  they are the HP-0 case. unverified: Highway has no spikes.
- The WRAM diff (`MMX_COOP_DIFF=1`, below) on the pit and HP-0 death frames
  shows nothing written outside the block but direct-page scratch and the
  stack area below `S`.

## Armour parts `$0C38..$0C98`

These three `$20` slots are not dash dust or charge sparks: they are X's
armour-part overlays, spawned by `$81:9D88` (listing `$81:9D7E`) from the
armour bits `$1F99`, updated by the loop `$80:D292` (listing `$80:D2D3`, `JSR`
from `$80:D26C`) and drawn by `$80:D640` (listing `$80:D681`, `JSR` from
`$80:D55A`, just before the player emitter). With no armour they are empty,
and P2's slices on Highway never wrote them. Dash dust, charge sparks and
wall-slide dust are allocated from the shared effect pool `$1928`, so P2's
get fresh slots and don't clobber P1's.

Option (a) (a few lines): the three slots are swapped with the block
(`P2_SWAPPED`), `$80:D292` runs in P2's physics slice after `$81:9D67`, and
`$80:D640` runs in P2's display slice before `$80:D671`. P2 gets its own
armour parts, positioned on P2.

## Sound

P2's actions enqueue on the shared ring `$0B72` (16 x {cmd, param}, write
index `$0BA3`, read index `$0BA4`, both `AND #$1E`). The NMI sends one entry
per frame when the APU acked (`$80:8822`); enqueue (`$80:888F`, `$80:88D4`)
never checks for full, so a 17th pending entry laps the reader and the
backlog is lost. After every P2 slice, if more than 8 entries are pending
(`sound_pending`), `$0BA3` is rewound to its value before the slice: P2's
new sounds are dropped, P1's are kept. Both players are heard otherwise;
sounds panned by `$80:88A2` pan relative to whoever is swapped in.

## Debug switches (native only; read from the environment)

- `MMX_COOP_DIFF=1`: every P2 slice prints the WRAM bytes it left changed
  (after the P1-owned restore): `coop diff frame N $7E0BA3:0E->10 ...`.
- `MMX_COOP_P2_HP=N`: P2 spawns with N HP (to reach HP 0 quickly).

## v1: what P2 can and can't do

P2 can:
- Run, jump, dash, wall-kick and shoot, using the ROM's own code.
- Switch weapons. The energy pool is shared.
- Take enemy contact damage (knockback, i-frames, death at HP 0).
- Die in pits; respawn next to P1 after 120 frames at max HP, no life lost.

P2 can't:
- Take damage from enemy shots or hazards that go through `$84:9F2A`.
- Move the camera, trigger freezes, doors or boss rooms, or pause.
- Get its own HUD bar.

Other v1 limits:
- Enemy AI tracks P1.
- P2's sprites use X's graphics with palette 7. The palette is a placeholder:
  nobody has checked what it holds per stage, and Zero's colours are not
  written.
- **Player tiles are shared.** P2's update rewrites X's VRAM upload queue
  (`$0503..$050F`, seen with `MMX_COOP_DIFF`) after P1's, so both players
  show P2's animation frame's tiles through their own OAM layouts (P1
  standing is drawn with P2's running tiles). Needs a second VRAM tile area
  for P2 (part of the real-Zero-sprites work).
- P2 charging rewrites X's sprite palette in the CGRAM buffer
  (`$0422..$043F`) and `$A1`, so P1 flashes when P2 charges.
- Sound requests are shared (capped, see "Sound").
- P2 advances the shared RNG.
- P2 writes to the SRAM probes `$70:0800` and `$70:0804`. The anti-piracy
  counter is at `+$83` inside the block.

## v2 plan

- **Shots / hazards.** Route `$84:9F2A` hits to the player they touched.
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
- [x] `$A7` holds the raw `$4218/$4219` word in the `Pads` layout. On P2
  (dump `in` = `$0BDE`,`$0BDF`): B -> `$0BDF=$80` (jump), Y -> `$0BDF=$40`,
  A -> `$0BDE=$80`. No dash on Highway: X has no leg parts yet.
- [x] `$D2==4 && $D3==4` is gameplay on the intro stage; the "ready" phase is
  `$D3==2`.
- [x] P2 stands, walks, jumps and lands exactly like P1; P2 is clamped to the
  camera window and follows P1's scrolling; P1's HP and position stay identical
  to a solo run.
- [x] Palette 7 shows P2 in a distinct (red/orange) palette on the intro stage.

Scripted runs (from `/Users/ritz/dev/mmxrs-private/states/highway-start.state`,
X grounded at the start of Highway; frames restart at 1 after `--load`).
Helper states, made once and gitignored:

```
S=states/highway-start.state   # absolute path in the main checkout
cargo run --release -q -p xtask -- dump --load $S --frames 250 --out /tmp/d --at 250 --hold 0100 --save 250=states/hw-spiky.state
cargo run --release -q -p xtask -- dump --load $S --frames 545 --out /tmp/d --at 545 --hold 0100 --save 545=states/hw-pit.state
```

- [x] **P2 contact damage.** P1 stands, P2 walks into the first Spiky:

  ```
  cargo run --release -q -p xtask -- dump --load states/hw-spiky.state --frames 60 --out /tmp/d --at 32,33,40,60 --p2-hold 0100 --coop
  frame 32  X=(488,367) act 00 hp 16 / P2 pos=(548,367) act 04 vy 0 hp 16
  frame 33  X=(488,367) act 00 hp 16 / P2 pos=(550,367) act 0E vy 0 hp 14
  frame 40  X=(488,367) act 00 hp 16 / P2 pos=(547,360) act 0E vy 128 hp 14
  frame 60  X=(488,367) act 00 hp 16 / P2 pos=(536,367) act 0E vy -64 hp 14
  ```

  P2 loses 2 HP, takes the hurt action `$0E` and is knocked back and up
  (same vy sequence 512, 384, ... as P1's solo hit); P1 stays at 16. The
  Spiky survives and hits P1 later (frame 80, P1 hp 14): no double damage,
  no double death.
- [x] **P1 parity with co-op on.** `dump --load $S --frames 480 --at
  200,286,287,288,290,300,350,400,440,445,450,460,480 --hold 0100` with and
  without `--coop`: P1's lines are identical (hit at frame 287 16 -> 14, at
  frame ~440 14 -> 11), while P2 (left standing, then dragged along by the
  camera clamp) is hit once too (hp 14 by frame 400).
- [x] **HP 0.** `MMX_COOP_P2_HP=2 cargo run --release -q -p xtask -- dump
  --load states/hw-spiky.state --frames 200 --out /tmp/d --at
  32,33,34,150,155 --p2-hold 0100 --coop`: frame 33 P2 `act 0C hp 0`, mode
  stays `02/04/04`, lives 2, P1 hp 16; frame 34 `coop_hidden true`; frame 155
  P2 back at `pos=(489,367)` (P1 X + 16).
- [x] **Pit.** P1 stands at X 790, P2 walks into the gap after the first
  truck (X 824..856, P1 solo dies there at frame 638 with `$D3=6`):

  ```
  cargo run --release -q -p xtask -- dump --load states/hw-pit.state --frames 300 --out /tmp/d --at 80,90,200,220,300 --p2-hold 0100 --coop
  frame 80   mode 02/04/04 X=(790,367) hp 11 lives 2 / P2 pos=(856,505) act 12 hp 16
  frame 90   mode 02/04/04 X=(790,367) hp 11 lives 2 / P2 pos=(856,513) act 0C hp 0
  frame 200  mode 02/04/04 X=(790,367) hp 11 lives 2 / P2 (down)
  frame 220  mode 02/04/04 X=(790,367) hp 11 lives 2 / P2 pos=(824,390) act 08 hp 16
  ```

  P2 dies at Y 513, the same Y as P1 solo; `$D3` stays 4, lives stay 2.
- [x] **Armour parts.** Throwaway test (not committed): Highway state,
  `$1F99=$0F`, `call_isolated_ret($81:9D88)` to spawn P1's parts, P2 walks:
  P1's `$0C38..$0C98` identical with and without co-op over 90 frames; P2's
  copies follow P2's X (144 -> 212); P2's OAM count 8 -> 18. The part
  graphics are not in VRAM on Highway, so nothing was checked by eye.
- [x] Sound: P2's jumps/shots enqueue (`$0B80`, `$0BA3` 0E -> 10); cap
  covered by `coop_sound_pending`. unverified: never saw the ring near full.
- [x] Parity after all of the above: `cargo xtask verify --frames 2400
  --coop-off --tap 200,400,600,800,1000 --hold 0100` prints `OK 2400 frames
  identical (4 traps registered on DUT)`.

Still open:

- [ ] Enemy shots / hazards through `$84:9F2A` hit only P1.
- [ ] Enemies that die on contact die only on P1.
- [ ] Shared player tiles (`$0500` upload queue) and charge palette (`$0422..`).
- [ ] Spikes not seen (no spikes on Highway).
- [ ] Restoring `$1E48..$1E89` on every scroll type (only horizontal seen).
- [ ] Other stages, bosses, the weapon menu and the stage-select flow.
