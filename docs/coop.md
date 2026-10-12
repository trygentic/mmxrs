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

Addresses are the PAL v1.0 dump's. Every one is in `hooks.rs` with a byte
signature and is looked up at load (`Game::hook`), so other dumps (USA v1.1:
bank `$80` `+$20`/`+$2B`) find them too; see `docs/rom-variants.md`.

| addr | what | P2 body |
|---|---|---|
| `$80:D1EF` | instruction after the object master's `JSL $81:812E` (P1's action/input phase has just returned) | update slice: P2's previous position `+$22/+$24` (what the master's entry does for X, see "Walls"), ROM input remap `$80:E518` on pad 2, then `$81:812E` |
| `$80:D241` | instruction after `JSL $81:9D67` (P1's terrain/physics phase, run after the enemy and shot loops) | physics slice: shot bookkeeping (B3), `$81:9D67`, `$80:D292`, then `coop::follow` (pit death / left behind, see "Following the camera"; the level's side walls, see "Walls") |
| `$80:D545` | second instruction of the OAM build `$80:D544` (`PHB`; `$D544` is Zero's palette trap) | hide last frame's P2 slots (Y = `$E0`) |
| `$80:9A86` | instruction after `JSR $80:D544` at `$80:9A83` in the per-frame stage code: the whole OAM build (and its hide loop) is done, the camera is final | display slice: `$80:D640` + `$80:D671` on P2's block into the slots past the game's, Zero palette, direct page put back; with the X3 set, the Rust emitter `zero::emit_x3` instead (docs/zero.md "Drawing P2 as X3 Zero") |
| `$80:D3B8` / `$80:D3C1` | shot loop, before / after each `$1228` slot (`D` = the slot) | P2's shots (B3): P2's block swapped in around their update |
| `$80:D1FF` | instruction after the object master's `JSR $80:D3B2` (the shot loop; the enemy loops follow) | saber (X3 set only): the arc's shot into a free `$1228` slot, taken out at `$80:D241` (docs/zero.md "The saber") |
| `$80:9A94` | `LDA $1F30`, where both paths after `$9A86` meet (every gameplay frame, after the display slice) | P2's HP bar and down marker (`coop_hud.rs`, "P2 HP bar and revive") |
| `$84:9B03` | entry of the enemy-vs-X contact routine; every enemy's AI `JSL`s it with `D` = the enemy (154 call sites) | contact pass: the same routine again, isolated, with P2's block swapped in (not during a cutscene, see "Cutscenes") |
| `$82:E9ED` | the collapsing road chunk's "X stands on me" test (`JSR` from its update) | collapse pass: the same test with P2's block swapped in; P2 starts the collapse (see "Platforms and items") |

Per-frame order in the object master `$80:D1C2`: player phase 1 (`$81:812E`)
-> item/shot/enemy loops -> player phase 2 (`$81:9D67`) -> camera `$80:DDB0`.
The per-frame stage code then runs `JSR $80:D544` (OAM build) at `$80:9A83`.

When P2 exists (`coop_stages`): `p2_update` hides P2 (re-anchor later) when
`!p2_should_exist` (not `$D2=4/$D3=4`, P1 dead or HP 0); on the pause flag
`$1F24` it only skips the slice, so P2 comes back where it was. A hidden P2
anchors on P1's spot only when `p2_anchor_ok` (P1 standing/ready/walking,
vy 0, on screen).

## Following the camera

Playtest: "P2 is clipping through the ground constantly after I change
elevation / the screen scrolls". Cause: the display slice clamped P2's X
into `[cam+8, cam+248]` with no collision check, so a scrolling camera
dragged P2 into walls and floors. Repro (old code):

```
dump --load states/hw-e.state --frames 200 --at 30-200/2 --hold 0100 --p2-hold 0200 --refill --coop
f44  X=(3512,383) cam 3384 / P2 (3416,383) act 08   walks off the ledge behind P1
f64  X=(3541,383) cam 3413 / P2 (3421,431) act 08   clamp pushes it right into the ledge's side
f66  ..                    / P2 (3424,431) act 04   "lands" inside the road structure
f76  ..                    / P2 (3447,415) act 04   ejected up 16 px, still inside
f174..182                  / P2 3591 -> 3719, 32 px a frame (ejection), then pops up to 399, 383
```

(the left of 3416 is the pit P1 jumps at f~190; `target/qa/follow/before-f80.png`: Zero
drawn inside the green road block). Now P2 is never moved by the camera.
`coop::follow`, after P2's physics:

- airborne and below the ROM's pit line (`p2_lost`, B1): down (the only
  path into the down/revive state besides HP 0);
- a screen or more outside the window (`p2_far`: dx outside `[-256, 512)`
  or dy outside `[-224, 448)`), or outside the window (`p2_on_screen`) for
  more than `P2_AWAY_FRAMES` (90) frames in a row: `p2_warp`. At the next
  `p2_anchor_ok` frame the update slice re-anchors P2 on P1's spot (block and
  scratch copied from P1's, as a respawn) with its HP kept, no down state.

`P2_X_OFFSET` is 0 now: P1's own spot is the only place known to be clear of
terrain (16 px right put P2 inside a wall P1 stood against). Off screen P2
keeps running its own physics: idle P2 left 800+ px behind on `hw-a`,
`highway-start`, `hw-e`, `r3-10` stands at its Y the whole time (collision
is not camera-streamed); what moves it there is the ROM (the collapsing
road in `hw-f`/`r3-0`, road attackers). Same repro with the new rule: P2
falls into that pit (`(3365,516)` act `$0C` at f72, like P1 at f222) and
nothing moves it sideways (`target/qa/follow/after-f66.png`); P2 idle on
`highway-start`: warped to P1's spot at f~185 (X 393) and f~365 (X 591).
The ROM's own edge rules for P1 are untouched.

## Cutscenes: P2's pad is locked

Playtest: "Zero (P2) shouldn't be able to move during boss opening
cutscenes": in the Vile scene at the end of the Highway P2 kept running
and shooting while X was held.

**How the ROM takes X's control.** Not by masking the pad: the input remap
`$80:E518` runs every gameplay frame (skipped only while `$1F7F` is set,
the attract demo), and X's `$0BDE` follows the pad all through the Vile
scene (dump `xin`). It is X's *action*: objects force one through a family
of setters in bank `$84` (`$84:9F85..$84:A07B`, `STA $0BAA`, most also
`STA $0C16`), and those actions ignore the input:

| action | setter | callers | what |
|---|---|---|---|
| `$1E` | `$84:9FE6` (released by `$84:9FFE`: action 0) | 34: boss intros in banks `$83`/`$87`/`$88` (e.g. `$83:B161`, the boss init that then waits for the health bar), doors, ... | "hold still": handler `$81:8B4D` lets an airborne X fall (`$82:8174`), lands it, stands |
| `$2E` / `$30` / `$32` | `$84:A04D` / `$A061` / `$A070` | Vile (`$83:CFF3`, `$83:D4FE`, `$88:A58A`) | grabbed, held up, knocked down; X stays in them through the NPC Zero's arrival and the text |
| `$34`..`$40`, `$46` | `$84:A008..$A041`, `$9FF2` | bosses in banks `$87`/`$88` | boss-specific holds |
| `$1A` | X itself, when `$1F23` is set | | stage exit (teleport out) |
| `$18` / `$1C` | `$84:9F85` / `$9FC9` | pickups (`$81:E0xx`, `$81:E7xx`) | P1's own heal / tank fill: not a cutscene, P2 plays on |

`$1F23` (stage clear, `INC` by `$84:AA28` when a boss dies) makes X's
dispatcher (`$81:8258`) run the victory code whatever the action, then `$D3
= 8`. The player code's own `$0C16` check (`$81:9831`) is also why X's
charge stops in these actions.

**P2.** `coop_stages::p1_scripted` = `$1F23 != 0` or X's action in `$1A`,
`$1E`, `$2E..$40`, `$46` (pure WRAM, same on both netplay peers, nothing
new in `CoopState`). While it holds, `p2_update` does not run the remap:
P2's held/previous/pressed input (`+$36..+$3B`) is 0, so its own actions
resolve as with the pad released (walk stops, a jump falls and lands, then
it stands where it is, facing as it was). A charge held into the scene is
cancelled the ROM's way, not fired by the release: around the locked update
P2's `+$7C` (`$0C24`, "on the ride armour", read only by the charge code
`$81:97DB` besides the ride armour itself) is set, so `$81:97C8` clears
`+$58/+$5A/+$5B` and queues the charge-sound stop `$17`. P2's contact pass
is skipped too (Vile walks over P2 during the capture; nothing may hurt or
move it). P2's shots already in flight finish their flight. The lock takes
a frame: Vile sets X's action from the enemy loop, after P2's update.

Forced movement (a boss door walking X in): P2 is locked and stays where
it is; if that leaves it outside the window, "Following the camera" warps
it to P1's spot at the next `p2_anchor_ok` frame, i.e. once the ROM gives
X back (action 0 is "stand"). unverified: no boss-door state; `$1E` is set
by 34 callers, the door among them assumed, not traced.

**The Vile scene** (PAL `states/highway-vile.state`: the fight already
running; the drop-in before it was not reached, see coop-qa.md). P1 holds
Right, taps B from f1000; P2 holds Right, taps Y every 16 frames and B
every 50 (`--x3`, `vb` = before, `va` = after):

```
dump --load highway-vile.state --x3 X3 --coop --frames 2600 --at 1-2600/10 --hold 0100 \
  --p1tap 8000@1000-3000/40 --p2-hold 0100 --p2tap 4000@1-2600/16:6,8000@5-2600/50:10
f1-930     fight: X has control, so does P2 (walks, jumps, shoots; Vile's HP $74 -> $67 from P2's shots)
f920-932   P2 jumping (act 08), lands at f932, walks on (act 04)
f936       Vile grabs X (act 2E); f938 P2 act 00, input 0000, standing at (7562,431)
f1201      X held up (act 30); P2 still there
f1921      Vile's HP $67 -> $57: the NPC Zero's shot, not P2's
f1951      the NPC Zero (enemy id $33) has run in and stands next to P2: two Zeros side by side
f2101-2501 Vile leaves, text box; X act 32; P2 still standing, input 0000
(then $D3 = 8, X act 1A teleports out: P2 is hidden by p2_should_exist, as before)
```

Before (same run with `--p2-hold 0200`): P2 walked and shot through the
capture and the text (input `0042`, Left + Y, at f1201, f1601, f2401).
Charge test (`--p2tap 4000@850:600`, no `--x3`, holding Y into the capture
at f1149): with only the pad zeroed, the release fired a charged shot from
P2 at f1150; with `+$7C` none, and the sound ring gets the `$17` stop. PNGs:
`target/qa/coop-lock/vile-before.png` (f911, 1001, 1201, 1951, 2101,
2501), `vile-after.png` (f921, 961, 1201, 1951, 2101, 2501). Test
`coop_p2_locked_in_cutscene` (PAL `highway-vile`: P2 charging into the
capture; then 400 frames of input 0, same position/action/HP, no shot).
Two Zeros: P2 is drawn as usual while the ROM's Zero is on screen;
standing still next to it looks intended enough.

## Walls

Playtest: "Zero jumps up and down under the terrain after any wall
contact". Repro (Rev 1; both walk right and jump the first gap at f568):

```
dump --load states/usa11-highway-start.state --x3 X3 --frames 700 --coop --hold 0100 --p2-hold 0100 \
  --p1tap 8000@568:14 --p2tap 8000@568:14 --at 580-700/2
          before                                   after
f590 X (824,359) a08 | P2 (825,362) a08    P2 (824,362) a08
f592 X (824,364) a12 | P2 (828,351) a08    P2 (824,364) a12 vy 0     wall slide, like X
f602 X (824,368) a12 | P2 (871,360) a08    P2 (824,370) a12
f606 X (824,376) a12 | P2 (888,351) a00    P2 (824,378) a12
f620 X (824,404) a12 | P2 (888,353) a08    P2 (824,406) a12
f646 X (856,474) a08 | P2 (888,356) a08    P2 (856,479) a12          next wall; both die in the pit at f672
```

Cause: the terrain phase `$84:91BE` (from `$81:9D67`) picks which side
to probe from the move since last frame: `$84:92AC` does `+$05 - +$22`
(zero: no wall probe; sign: probe right or left edge, `$84:9315` ->
`$84:90B3` tile fetch -> `JSR ($935D,x)` by tile class -> `$84:9308`
push-out), and `$84:91ED` compares `+$08` with `+$24` (down: floor probe,
up: ceiling probe). `+$22/+$24` (`$0BCA/$0BCC`) are last frame's X/Y, and
the player code never writes them: the object master's entry does
(`$80:D1ED` Rev 1, `$80:D1C2` PAL: `LDX $1F19; BNE; LDA $0BAD; STA $0BCA;
LDA $0BB0; STA $0BCC`), before its `JSL $81:812E`. Found with `trace --pcs`
on the solo frame 590 (the only stores to `$0BCA`/`+$22`) after
`cooptrace --frame 591 --phase 2` showed P2 taking the `SBC $0007,y`
(left-edge) branch at `$84:92C9` while moving right. P2's copy was frozen
at its anchor spot, so P2 probed the wrong side: past walls, and on the
raised road (Y 351, above the anchor's 367) it probed the ceiling instead
of the floor, fell, got pushed back up, forever. The tile fetch itself is
level-absolute (`$84:916A`: screen map `$7E:E800` by X/Y high bytes ->
`$7E:2000` -> `[$0B92]`), nothing camera-relative. Fix: `p2_update` does
the same two copies on P2's block (unless `$1F19`) before the update.

The level's left/right edges are not terrain: the camera's follow mode
(`$80:DDDB`, `D` = `$1E48`, mode `+$01` = 2) ends at `$80:E0DE`, which
clamps the camera to its limits `+$0E`/`+$10` and then X into
`[left + 8, right + 248]` (P1 only). `coop::follow` applies the same
clamp to P2 in mode 2 (`coop_stages::p2_level_clamp`). Before, P2 holding
Left from highway-start walked to X -1 (65535) at f91 and fell to Y 431
off the level; now it stops at X 8 at f91 like X solo.

Pit wall from below (`--p1tap 0100@1:540 --p2-hold 0100`, P2 walks off
the gap's edge): before P2 went through the far wall (826,390 at f570,
844,448 at f582, dead at 862,517); now it wall-slides at X 824 from Y 390
to 410 (the wall's bottom) and falls on. PNGs (`target/qa11/walls/`):
`strip.png` (top before, bottom after: f592, 596, 602, 606, 620, 640),
`pitwall.png` (f567..585), `leftwall-f121.png`. Parity verify OK on both
dumps, QA idle "yes" on every milestone (PAL and Rev 1).

## Display slice and lag frames (B4)

The OAM build `$80:D544` ends with a hide loop: Y = `$E0` on slots `$E4`
(this frame's count) up to `$E3` (last frame's), then `$E3 = $E4`, `$E4..$EB
= 0`. The old display hook sat inside the build (`$80:D5FA`) and advanced
`$E4`, so that loop ran fewer iterations and a heavy frame finished before
the NMI where solo lagged: P1 drifted a pixel with P2 idle.

Now P2 is emitted at `$80:9A86`, after the build:

```
cargo run --release -q -p xtask -- trace --load states/hw-a.state --frames 270 --hold 0100 --calls
  $80D544 x1     first from $809A83
```

The build's disassembly (cartridge bytes): `$D563 SEP #$30`, flush of the
partial high-table byte, `$D57B LDA $E3; SEC; SBC $E4` .. `$D594 LDA #$E0;
STA $0701,X` loop, `$D5A1 LDA $E4; STA $E3; STZ $E4 $E6 $E8 $EA $EB; PLB;
PLP; RTS`. A full PC trace of frame 270 (throwaway `TRACE_PCS` hack in
`xtask trace`) shows no instruction with operand `$E3`/`$E4`/`$E5`/`$E6`
outside `$80:D544..$D7xx` (only branch offsets / `LDA $0BE3` / `JSR $81E3`
false positives); after `$9A86` the frame runs `$9A86..$9ACE` and the
scheduler's idle loop. Paused frames reach the build by `JMP $D544` at
`$80:9AD2` and never return to `$9A86`, hence the separate hide hook at
`$D545`.

The slice: P2 goes into slots from `$E3` rounded up to 4 (P2 owns whole
high-table bytes; the padding gets Y = `$E0`); the partial last byte is
flushed like the build's tail, keeping any game bits above P2's. The
emitter's overflow exit (`$80:D739`, after slot 127's byte: `PLX; PLX; JMP
$D5A1`) would unwind through `call_isolated`'s sentinel, so P2's sprite
count is read from its frame layout first (`$8D:8000[+$16*3]` -> `[+$17*3]`
-> first byte) and when the game left too few slots P2 takes the game's last
(lowest priority) ones. M3 (hw-a f240..300) uses 100-128 slots; `$E3 = 128`
when the build itself ran out. Registers forced to M=1, X=0, D=0, DB=`$8D`
(at `$9A86`: `D=0000 DB=86 P=32`); all of direct page `$00..$FF` is put back
after, so `$E3..$EB` and the scratch are exactly as the game left them.
ponytail: P2 now sorts behind every game sprite. The emit runs isolated
(zero cycles), so its own CPU cost can't matter, and doesn't: see the parity
runs below.
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

## P2 HP bar and revive

Code: `crates/mmx-core/src/coop_hud.rs`. Playtest: "Zero seemed to have
unlimited HP": nothing showed P2's HP and it came back after 2 s at full HP.

**How X's bar is drawn.** Sprites, not BG/HDMA: seven 16x16 OAM entries in
slots 0..6 (`$0700..$071B`). `$80:D7A3` (listing `$80:D7E4`) jumps on
`$1F11`: state 0 writes X = 8, Y = `$E0`, attr `$34` into all seven once
(slot 0 = the icon, Y `$50`, tile `$86`) and high-table bytes `$0900..` =
`$AA` (large); state 2 = redraw `$80:D801`: only when `$0BCF` bit 7 is set,
clamp HP to `$1F9A`, then for slots 1.. from Y `$40` upwards in 16 px steps
tile `$80` (8 HP full), a partial piece shifted by `2 * rest` px, tile `$82`
(8 HP empty) up to max HP, tile `$84` (cap); clear bit 7. X is never
rewritten, so the bar's position is the init constant, not a table. The
weapon bar (`$80:D8CA`, `$1F12`) is slots 7..15 at X 24 (attr `$36`), the
boss bar (`$80:D9E3`, `$1F0E` = boss) slots 16..23 at X 232; the OAM build's
cursor starts at `$E4 = $10` (`$18` with a boss) so it never touches them.

**P2's bar.** `HOOK_HUD` (`$80:9A94`): when `$1F11 = 2` and P2 may exist,
save direct page and slots 0..6, swap a copy of P2's block in with
`$0BCF = hp | $80` (hp 0 while P2 is down or hidden), `call_isolated_ret(
$80:D801)` with M=1, X=0, D=0, DB=`$80`, take the seven entries it wrote,
put slots 0..6, the block and direct page back. The entries go into 8 slots
(bar + down marker, `HUD_SLOTS`) right after P2's sprites (4-aligned, whole
high-table bytes set to `$AA`), with X = 8, under X's bar. The pieces keep
X's attr (`$34`, the HUD palette 2), so they are X's colours; only the icon
gets P2's palette ("Palettes" below). The bar sits the vertical mirror of the redraw's output, `y' = $B8 - y`
(`BAR_MIRROR`), pieces vflipped (hidden `$E0` entries stay hidden). So P2's
icon sits at Y `$68`, 8 px under X's (`$50`), and its pieces grow down from
it: 16 HP spans Y 104..168, 32 HP (X's cap at Y 0) ends at 200. The weapon
bar is the next column (X 24, Y 8..96 with a weapon selected: `--poke
7E0BDB=02@1-30,7E1F12=00@1`), the boss bar the right edge, so neither is
in the way. `p2_oam_base/count` grow to cover them, so
`oam_hide` clears them at the next build like P2's sprites. P2's display
slice leaves `HUD_SLOTS` slots free below 128 (`room`); with P2 hidden and
the game at 128 slots the bar takes the game's last 8. The bar is rebuilt
every frame from P2's block, so there is no cache to carry. The icon: X's
"X" recoloured to palette 3; with X3 loaded, a Z glyph drawn in code (X3 keeps X's emblem for Zero; see `coop_hud::Z_GLYPH`) emblem from shadow
page 3 (also for a Zero P1's own icon; docs/zero.md "The HUD icon"),
in P2's palette (red/gold on white; without X3 the gold is X's light grey).
Before: the pieces were forced to palette 3 too, i.e. X1's bar tiles in
Zero's colours (green frame and greenish-brown segments; `zero.md` noted
the green outline).

Checked (4x crops of the left edge, frame 30):

```
[MMX_COOP_P2_HP=8] dump --load states/hw-spiky.state --frames 30 --at 30 --coop [--x3 X3] [--zero]
dump --load states/hw-bee2.state --x3 X3 --frames 60 --at 60 --coop --poke 7E1F0E=68@1-60,7E1F0F=0E@1-60
```

hw-spiky full / half HP, with and without X3: X's bar Y 32..96, P2's
104..168, half HP fills the upper half; with X3 P2's icon is the Z;
`--zero` (co-op and solo) X's own icon is the Z, its bar unchanged. hw-bee2
has no boss bar of its own (a mini-boss, `$1F0E = 0`); pointing `$1F0E` at
the Bee Blader brings it up at X 232 (slots 16..22), clear of P2's bar.

**Down and revive.** `go_down` (HP 0, pit, left behind) hides P2 for
`P2_DOWN_FRAMES` (600 = 10 s) and records the fall spot (`Revive::spot`,
P2's position) if it was on screen; a pit or a vertical "lost" is below or
above the window, so `spot = None` and only the timeout applies. While down
(`coop_hud::still_down`, from `p2_update`): the timer counts down; P1 within
24 px (X) of the spot, with the spot on screen, counts `Revive::touch`
(reset when P1 steps away); 60 frames in a row end the wait. Either way P2
re-anchors next to P1 as before (`p2_anchor_ok`) with half of max HP
(`respawn_hp`, `Revive::half_hp`). Leaving gameplay (P1 dies, stage
change) clears `half_hp`: P2 comes back at full HP there. The marker is
slot 8 of the HUD block: the bar's icon (tile `$86`, P2's palette) centred on
the spot, blinking every 8 frames of the timer, only while the spot is on
screen. All of it (`CoopState::revive`, `down_frames`) is in `CoopState`,
so netplay rollback carries it. ponytail: P2 is hidden while down; the X3
set's lying frame (135) is not used.

Verified (dump lines shortened; `P2 down N spot S touch T` is a new dump line):

```
MMX_COOP_P2_HP=6 dump --load states/hw-spiky.state --frames 200 --at 30,34,60,100 --coop --p2-hold 0100
frame 30  X hp 16 / P2 hp 6    frame 34  X hp 16 / P2 act 0E hp 4
```

`frame_000030.png` / `frame_000060.png`: P2's bar (under X's, red) drops
from 6 to 4 HP; X's bar stays full; P1's line identical to a solo run.

```
MMX_COOP_P2_HP=2 dump --load states/hw-spiky.state --frames 320 --coop --p2tap 0100@1:34 \
  --p1tap 4000@40:2,4000@55:2,4000@70:2,4000@85:2,4000@100:2,4000@115:2,0100@150:30
frame 33  X=(488,367) / P2 act 0C hp 0 hidden; P2 down 600 spot Some((550, 367)) touch 0
frame 180 X=(528,367) / P2 down 453 spot Some((550, 367)) touch 3
frame 230 X=(528,367) / P2 down 403 spot Some((550, 367)) touch 53
frame 239 X=(528,367) / P2 pos=(544,367) act 00 hp 8
```

Touch revive: P1 shoots the Spiky, walks to X 528 (22 px from the spot),
60 frames later P2 is back at 8 of 16. PNGs: f33 empty bar, f200 marker at
the spot, f240 P2 next to P1 with a half bar.

```
dump --load states/hw-pit.state --frames 720 --coop --p2tap 0100@1:100
frame 89  X=(790,367) / P2 pos=(856,513) act 0C hp 0; P2 down 594 spot None touch 0
frame 689 X=(790,367) / P2 pos=(806,367) act 00 hp 8
```

Timeout: the pit spot is below the screen, no marker; P2 back after 600
frames at half HP. Heavy frames (M3, `hw-a`, 300 frames, `--wram`): the bar
is in the rendered PNG on every frame; the 7 WRAM snapshots without it were
taken mid-build on lag frames (last trap `coop_p2_oam_hide`), where the
previous frame's OAM is what is on screen.

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
  P2 hides for `P2_DOWN_FRAMES` (was 120, now 600 or a revive, see "P2 HP
  bar and revive") and re-anchors next to P1 (now at half HP).
- **Pits** are not decided by the player code: the camera code (Rev 1
  `$80:E11E`, run on P1 only) first clamps a camera that exceeds its bottom
  limit (`$1E5C`), then kills X when `Y - $20 >= bottom + $E0` (by
  `$84:9F2A` with `$0BCE=$7F`). P2 never ran that, and fell to the floor
  below the screen (Y 895) and stood there (B1).
- **Left behind vertically** (B2): nothing drags P2 along a vertical scroll;
  after the Bee Blader 1 climb P2 stayed at Y 575 with the camera at 256.
- `coop_stages::p2_lost(y, camera_bottom)` uses that native bottom limit,
  rather than the rendered camera Y, which can lag a legitimate descent
  after a road collapse. With P2
  airborne it marks P2 down (action `$0C`, HP `$80`): hidden for
  `P2_DOWN_FRAMES`, then re-anchored next to P1, no life lost; no ROM death
  code runs for P2. Left behind (B2, or above the window) is not a death any
  more: see "Following the camera".
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

## Effects `$1928`

31 slots of `$20` (`+0` active, `+$0A` type): dash dust, sparks, the hurt
flash. Updated by the loop `$80:D31A` (PAL; `$80:D345` Rev 1), `JSR` from
the object master, `D` = the slot: `LDA #$80; TRB $0E; LDA $0A; ASL; TAX;
JSR (tbl,X)` per active slot (hook `effect_slot`, the `LDA #$80`). An
update queues the effect on a draw list; the OAM build draws it with the
sprite emitter (listing `$80:D67C`, `X` = the object).

Every slot a P2 slice makes active is P2's (`CoopState::p2_fx`, bookkept in
`with_p2_swapped`; dropped when the slot is free). For those,
`coop_objects::effect_slot` does the `TRB`, runs the table entry isolated
with P2's block swapped in and skips to the loop step (`+11`), and
`effect_oam` runs the emitter isolated and returns from its `JSR` (not
drawn if it might pass OAM slot 127: the emitter's overflow exit would
unwind through the sentinel). The hurt flash (`$0B`) reads its owner's
position every frame: before this it sat on P1, and its real-time update
and draw made P1 lag a frame early on NTSC (docs/rom-variants.md).
ponytail: a P2 effect freed by someone else and reused by P1 before P2's
next slice would stay P2's for a frame.

## Shots (B3)

X's shots live in `$1228` (8 slots of `$40`, `+0` = active; `ram::ITEMS_BASE`).
The fire routine (`$81:93A8` / `$81:943D`) refuses when `$0BDD` (X's live
shot count, block `+$35`) `>=` the cap `+$67`, and does `INC $35` (direct
page: P2's copy in P2's slice). The shot's despawn does `DEC $0BDD`
(absolute; `$81:A2C0`, `$81:A3FD`, `$81:A4BA`, `$81:A541`) from the shot
loop, where P1's block is live. So each P2 shot took one from P1's count:
P2 hit the cap after a couple of shots (Bee Blader 2: one hit, then P2's
Y presses spawned nothing, `MMX_COOP_DIFF` shows no `$1228` write) and P1's
count wrapped to `$FF` (`>=` the cap, so P1 couldn't fire either; "both
shooting, never dies"). The boss itself has no player-keyed state.

The same shot loop also placed P2's shots: a shot's first update
(`$81:A54C`) takes its position from `$0BAD/$0BB0` (+ the muzzle offset
`$86:BE39[+3C]`) and its facing from `$0BB9`, and a charging shot
re-anchors to them every frame (`$81:A56E`). With P1's block live, P2's
shots came out of X and flew X's way (hw-spiky, P2 at 447 facing left: the
new slot at 504 facing right, X at 488).

Fix: `p2_update` records the `$1228` slots that became active during P2's
slice when P2's `+$35` went up (`CoopState::p2_shots`). Two hooks in the
shot loop (`$80:D3B2`, listing `$80:D3F3`) swap P2's block in around each
P2 slot's update: `$80:D3B8` (`SEP #$30`, `D` = the slot) and `$80:D3C1`
(`REP #$21`, the loop step, reached whether the slot ran or not). So the
shot reads P2's position and facing, and its despawn's `DEC $0BDD` /
`DEC $0C0B` hit P2's copy. The physics hook drops the freed slots from
`p2_shots`. A P2 that respawned (`+$35 = 0`) with shots still out would
wrap: `$FF` is put back to 0. Test `coop_p2_shot_from_p2`; lemon and
charged shots checked (`target/z/shot_strip.png`, highway-start charge:
the half charge sits on Zero, then flies left). unverified: special
weapons; `$84:9DE0` also does `DEC $0C20` (block `+$78`) for weapon `$08`.

**Zero's muzzle.** The muzzle table is cartridge `$86:BDDD` (2 bytes per
entry, `dx` toward the facing, `dy` signed); `+3C` = 2 * (pose base +
`+0F & $0F`), from the fire routines `$81:93B2`/`$81:9435` (bases: stand 0,
jump 1, fall 4, land 7, walk 9, dash `$13`, wall slide `$17`, wall kick
`$1A`). The anchor is `$81:A57B`; the half charge (`+0A = 1`) re-anchors x
only. X's muzzle sat at Zero's fist (stand: X (16,-3), Zero's cannon 27 px
out, 7 up). `zero_data::ZERO_MUZZLE` (measured in X3, docs in the table)
moves a Zero player's shot at `HOOK_SHOT_SLOT_DONE` (the owner's block is
live there) whenever it sits exactly at X's muzzle: the first update and
every re-anchor. P2's slots when the X3 set is loaded, every other slot when
P1 is Zero (`--zero`). No-op otherwise (parity).

## Palettes

The game uses all eight sprite palettes: 0 white flashes and sparkles, 1 X,
2 the HUD (bars, lemons, full-charge sparkles), 3 X's charged shots (id
`$20`, `$85:B860`), 4-7 enemies, reloaded per stage. Co-op used to write
P2's Zero palette over 3 every frame (`ZERO_PALETTE`; it had looked unused
because the survey runs never charged), so every charged shot, X's and
P2's, and the third-level sparkles were drawn in Zero's colours (playtest:
"greenish brown"). That predates the charge work of `a815c90` (palette 3
since `cde36b3`; `60826bc` writes it the same way); `a815c90`'s loader
traps only skip P2's flash loads into slot 1.

Fix: **P2's palette is a ninth sprite palette in the vendored PPU**, not in
CGRAM. `P2Tiles` (the shadow-page state, `ppu.rs`) has `pal: u128` (slots)
and `palette: [u16; 16]` with pending copies, latched with the pages and
the OAM on the game's OAM upload. A sprite whose slot is in `pal` takes its
colours from `palette`; its OAM palette bits (still 3) only decide colour
math (< 4: none, as before). Main screen and widescreen margins
(`MarginSprite::extra_palette`) both do it. With `pal = 0` nothing
changes (`verify --coop-off` identical). mmx-core: `Game::set_p2_palette`
(the colours; `zero::write_palette` from the OAM-build hook: X3 Zero's, or
`palette_for(X's)` without X3, with the charge flash), `add_p2_palette_slots`
(cleared each OAM build in `p2tiles::clear_slots`; set for P2's body,
armour parts and slash arc, P2's HP icon and down marker, P2's margin
sprites, not for P2's sparkles, which keep the ROM's palettes 0/2/3).
Saved in the state (`P2T7`). P2's shots are the game's objects in palette
3, so they now look like X's. A Zero P1 still wears CGRAM slot 1 (X is not
drawn then, and X's lemons are palette 2, shots 3), so two Zeros keep two
independent charge flashes.

Tests `coop_x_charged_shot_matches_solo` (both charge levels, with and
without X3: CGRAM palette 3 and the shot's OAM entries equal the solo
run's every frame) and `coop_p2_bar_matches_x` (`tests/coop_palette.rs`).
Checked on Rev 1 `usa11/hw-a` (Y held 45 / 120 frames) and PAL `hw-spiky`
(half HP, down marker), co-op with and without X3, `--zero`, both Zeros.

## Platforms and items (`coop_objects.rs`)

Both bugs had the same cause as enemy contact: the object's own update tests
X's box with `LDX #$0BA8; JSL $84:9C0E` and then writes X's block by
absolute address, so only P1 is ever carried or healed. Found by grepping
the listing for absolute writes to `$0BAD`/`$0BB0`/`$0BD4`/`$0BCF`, then
confirmed with a throwaway write watch (log the PC of every write to
`$0BAD,$0BD4`, D and X) in `trace` and `trace --find`. Bank `$81`/`$82`/`$84`
PCs drift here too: the cartridge addresses below come from byte-pattern
searches of the ROM (listing + `$0A` in `$84:AB..`, + `$10` in `$81:DE..E3..`,
+ 7 in `$82:E3..E7..`).

**Platforms.** Listing `$84:AB77` = cartridge **`$84:AB81`**, the generic
solid-object routine, `JSL`ed by 16 platform objects with `D` = the object.
If the object's `+$2C` ("X stood on me last frame") is set and X isn't
already grounded (`$0BD3|$0BD4` bit 2), X moves by the object's delta
(`+$05 - +$22`, `+$08 - +$24`); then `+$2C = 0`, box test, and X on top ->
`+$2C = 1`, `$0BD4 |= 4` (the object-ground bit the player's phase 1/2 read);
from the side -> `$0BAD` pushed out, `$0BD4 |= 1/2`. Companion `$84:AB56`
(listing `$84:AB4C`): box test only, `$0BD4 |= $80/$40`. On the Highway the
user is the collapsing road chunk (`$1428` table type 9, `$82:E720`, call
sites `$82:E7A5`/`$82:E7AE`; states `r2-2`, `r3-0`, `hw-f`).

**Road attackers.** A road attacker is two enemy slots at the same spot:
id `$11` (update `$81:D551`, the shooter) and id `$12` (update `$81:DCD0`,
listing `$81:DCC0`, the car body). With the driver alive neither carries X
(the old recheck: 26 jump timings on `r3-10`, X never above Y 427, hit on
landing; `$84:AB81`: 0 hits). Kill the driver (`+$01` goes to 4, or 6 when
the car stops) and the car body `JSR`s its own copy of the solid-object
routine: listing `$81:DE0C` = cartridge **`$81:DE1C`** (`RTS`, called from
`$81:DD16` when `+$01 >= 4` and `!= $0A`). Same code as `$84:AB81`: `+$2C`
= "X stood on me last frame", carry by `+$05 - +$22` / `+$08 - +$24` into
`$0BAD`/`$0BB0` unless `$0BD3|$0BD4` bit 2, box test vs `$0BA8`, on top ->
`+$2C = 1`, `$0BD4 |= 4`; side -> push-out. Extras: `+$3A` = last frame's
`+$2C` (copied at entry), and on a fresh landing `+$37 = 2`, the 2 px dip;
the caller also lowers the car by `+$2C` px. `$84:AB81` stays at 0 hits:
that's why the first search found nothing. A parked driverless car (`+$01
= 6`) is solid too.

State `hw-car` (from `r3-10`, `--p1tap 4000@40-400/10:3 --refill`, saved
at f112): driver of car slot 0/1 dead, the car drives left at 1.5 px/f,
X in knockback just above it. P1 alone, no input: X lands on it at f7 and
rides at car `+$05` - 4, Y 418, to f100+ (then the car drives into the pit
at ~f120).

P2 gets it with the same trap body as `$84:AB81` (`coop_objects::HOOK_CAR`,
P2's `+$2C` in `p2_on_obj`, run with `call_isolated_ret(.., Rts)`). `+$3A`/`+$37` are left as
P2's pass wrote them: P1's pass recopies `+$3A` at entry, and the dip on
P2's landing is fine. Check (`dump --load states/hw-car.state --coop
--refill --p1tap 0100@1:60,0200@62:40 --p2tap 8000@70:15,0200@70:33`):
P1 walks off the car's rear and stays on the road (Y 431, car `+$2C` 0);
P2, spawned at f25 on the car, rides at `+$05` - 2, jumps at f70 (holding
Left, air speed = car speed), lands back at f104 (dip: `+$08` 436, 437),
then rides at `+$05` - 1 through f130, down into the pit with the car
(P2 Y 417 -> 439 as `+$08` 435 -> 457). `--untrap 81DE1C`: P2 spawns on
the car and falls through it to the road (Y 420 -> 431 by f31), the bug.
Strip: `target/qa/car-p2/strip.png` (f66, 80, 96, 106, 114, 124). P1 with
P2 on board too: both carried (`--coop`, no input: P1 at `+$05` - 4, P2 at
`+$05` - 2).

Other carriers in the Highway (enemy ids seen in all the Highway states:
`$0F $11 $12 $15 $19 $1A $1B $22 $27 $32 $36`; their update ranges in the
listing grepped for `$0BAD`/`$0BB0`/`$0BD4` writes, `LDX #$0BA8`, calls to
`$84:AB77`): only `$12` (above), `$22` Bee Blader, whose dead state
(`+$01 = $0E`, listing `$82:BA90`) calls the generic `$84:AB77` (so the
falling wreck is solid, already covered by `HOOK_PLATFORM`; unverified in
play), and `$32` Vile, whose `$0BAD`/`$0BB0` writes are the grab (X held,
`$0C26` bit 7), not a carrier. The trucks are level art (not in the enemy
or `$1428` tables).

P2's pass: an `Observe` trap on each entry reruns the routine isolated
with P2's block swapped in (`with_p2_swapped`), the guard `in_contact` set.
The object's `+$2C` is per player: P2's copy is `CoopState::p2_on_obj` (one
bit per object slot `$0E68 + n * $40`, n < 43), put in place only for P2's
pass and P1's put back after, with direct page `$00..$1F`. P2's phase 1 runs
before the object loops and phase 2 after, as P1's, so the ground bit lands
in time.

**Items.** They are objects of the `$1428` table, not `$1228` (that one is
X's shots and enemy shots). Dispatch stubs at cartridge `$80:F325..` (`JSL
entry; RTS` per type): type 1 weapon energy **`$81:DEB7`**, type 2 health
**`$82:E342`**, type 4 1-up **`$81:E33A`**. Each entry is `LDX $01; JSR
(tbl,X); RTL`. Health: the box test hit sets `+$02 = 6`; then HP full ->
the amount goes to the sub-tanks (shared); else freeze (`$1F13..$1F19`,
`$0BB6`, X to action `$18` via `$84:9F85`) and one HP every 4 frames by
`STA $0BCF` (`ORA #$80`, the HUD redraw bit) up to 8 (big) / 2 (small),
then unfreeze (`$84:9FAD` restores the action). Weapon energy: the same
into the energy of the weapon in `$0BDB`. 1-up: `$1F80` + 1 at once.

P2's pass (`Observe` on the three entries, `D` = the pickup):

1. Pickup owned by P2 (`CoopState::p2_items`, same slot bits; cleared when
   the slot's `+$01` is 0, a fresh object): the routine runs isolated with
   P2's block and the ROM's pass is skipped by setting `PC` to the entry's
   `RTL` (`entry + 5`). The fill goes to P2's HP; P2 is the one in action
   `$18`; the world freeze flags are P1_OWNED, so P1 plays on.
2. Else, only when P2 is within 48 px: from one WRAM snapshot, dry-run the
   routine for P1 (as is), then for P2 (swapped). Collected = `+$02 == 6`
   or `$1F80` changed. P1 collecting wins (snapshot back, the ROM's pass
   does it); P2 collecting keeps P2's run, takes ownership and skips the
   ROM's pass; neither: snapshot (and P2's block) back.

So a pickup is taken once, P1 first on a tie. P1's `$0BCF` (and its bit 7)
is never written by P2's passes.

Verified (state: `highway-start`, hold Right, tap Y every 12 frames, saved
at f270 with `--save 270=states/hw-pellet.state`: a small health pellet dropped by the first
Spiky at X 599, Y 379):

```
MMX_COOP_P2_HP=5 dump --load states/hw-pellet.state --frames 160 --at 1-160/8 --coop --p2-hold 0100 --to 60
f33 X=(516,367) act 00 hp 16 | P2 pos=(579,367) act 04 hp 5
f41 X=(516,367) act 00 hp 16 | P2 pos=(590,367) act 18 hp 5     # P2 frozen, filling
f49 X=(516,367) act 00 hp 16 | P2 pos=(590,367) act 04 hp 7
dump ... --hold 0100 --p2-hold 0100 --to 60 --poke 7E0BCF=05@1 --coop   # both walk in, P2 ahead
f38 X=(572,367) act 04 hp 5 | P2 pos=(590,367) act 18 hp 5
f50 X=(590,367) act 04 hp 5 | P2 pos=(594,367) act 04 hp 7     # P1 walks over it: gone
dump ... --hold 0100 --to 60 --poke 7E0BCF=05@1 [--coop]         # P1 alone, P2 idle
f50 X=(590,367) act 18 hp 5   f60 X=(590,367) act 00 hp 7        # same lines solo and --coop
```

Platforms, `r2-2` (P1 and P2 on the chunk; then P1 walks off it first):

```
dump --load states/r2-2.state --frames 80 --at 1-80/6 --coop --refill
f31 X=(3978,351) act 00 | P2 pos=(3994,351) act 00
f49 X=(3978,406) act 00 | P2 pos=(3994,406) act 00   # before: P2 act 08 vy -1472, free fall
f61 X=(3978,477) act 00 | P2 pos=(3994,477) act 00
dump ... --p1tap 0200@1:20 --coop     # P1 stands at 3949 on the road, P2 rides down alone
f49 X=(3949,351) act 00 | P2 pos=(3993,406) act 00
```

P1's lines in the second run are identical to the solo run.

**Collapsing road: the trigger.** Playtest: "Zero can't trigger crumbling
platforms": with X elsewhere, Zero standing on a collapsing section did
nothing. Carrying (above) was P2's, but what starts the collapse is a
separate test in the chunk's own update (`$1428` type 9, cartridge
`$82:E727`, listing `$82:E720`, +7): states 2/6/`$A`/`$E` call it while the
timer `+$2F` is 0, listing `$82:E9E6` = cartridge **`$82:E9ED`** (`RTS`):
`LDX #$0BA8; JSL $84:9C0E` (box test vs X), `$0BD3` bit 2 (X on the
terrain: the road chunk is level tiles too, not `$0BD4`), `$0002 < 0` (X
above it), `|$0BAD - +$05| < 16`; then `+$2F = $1E` (frames to the fall),
`+$1F = $30`, `+$1A/+$1C = 0`, sound `$22`. A direct test of X's block, not
the `+$2C` "stood on me" byte (that one is only the carry; `p2_on_obj`
keeps P2's) and not a stage trigger. Same address and bytes on PAL and
Rev 1 (`hooks.rs` `collapse_trigger`).

Fix: `coop_objects::HOOK_COLLAPSE`, an `Observe` trap on `$82:E9ED`: when
P2 is up and the timer is 0, the test runs isolated with P2's block
swapped in (direct page `$00..$1F` put back). If P2 started the collapse,
the ROM's pass (P1's) is skipped (`RTS`, with the routine's `SEP #$30`):
P1 on it too would only restart the same countdown and queue the crumble
sound twice. Repro (PAL; `hw-collapse` = `r2-1` + 120 frames holding Right,
X at 3912, chunk at 3984):

```
dump --load hw-collapse.state --frames 160 --at 1-160/4 --coop --p2-hold 0100 --to 50 --wram
        before                                       after
f37  P2 (3965,351) act 04 | chunk +2F 00       P2 (3965,351) | +2F 00
f41  P2 (3971,351) act 04 | +2F 00             P2 (3971,351) | +2F 1D +1F 30   P2 within 16 px: starts
f49  P2 (3983,351) act 04 | +2F 00             P2 (3983,351) | +2F 15
f73  P2 (3983,351) act 00 | +2F 00 (forever)   P2 (3983,347) act 0A | chunk falls (+01 = 4)
f101                                           P2 (3983,438) riding it down; f121 down in the pit
```

X stands at 3912 throughout, its lines identical in both. Solo, X walking
right starts the same chunk at X 3969 (timer `$1C` at 3972), the same
threshold as P2 (3971). PNGs: `target/qa/collapse/before.png`,
`after.png` (f41, 61, 81, 101). Test `coop_p2_triggers_collapse` (PAL
`states/r2-1.state`; asserts it does not start with the trap off).

Other stand-to-trigger objects: the listing has 57 `LDX #$0BA8` tests;
the only ones followed by a `$0BD3`/`$0BD4` read are this chunk, the
platform pair `$84:AB77`/`$AB4C` (hooked), `$82:D875` and `$84:9A05`,
`$88:B5B3`, `$88:C33D` (side-touch flags `$0BD4 |= $80/$40`, like `$AB4C`),
and `$87:B4E9` (an object that switches state when X is in its box or
grounded, `$0BD3`). None of the latter are on the Highway; unverified, P2
does not trigger them.

Not covered: other pickups (heart tanks, sub-tanks, armour capsules: their
own routines, e.g. listing `$81:E4AB`, `$81:EB2A`), weapon energy and the
1-up are hooked but were not seen on the Highway; platform code outside
`$84:AB81` and the road attacker's `$81:DE1C` (enemy-specific carriers
such as listing `$82:C599`, `$82:A5FE` the grabber); the ride armour.

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
- Run, jump, dash, wall-kick and shoot (its own shot count), using the ROM's own code.
- Switch weapons. The energy pool is shared.
- Take enemy contact damage (knockback, i-frames, death at HP 0).
- Die in pits, or be taken out when the camera scrolls away vertically;
  be revived by P1 (or come back after 10 s) at half HP, no life lost;
  its own HP bar under X's (see "P2 HP bar and revive").

- Start a collapsing road chunk by standing on it, like X.
- Not move or be hurt while the ROM holds X in a cutscene (see "Cutscenes").

P2 can't:
- Take damage from enemy shots or hazards that go through `$84:9F2A`.
- Move the camera, trigger freezes, doors or boss rooms, or pause.

Other v1 limits:
- Enemy AI tracks P1.
- P2's sprites use X's graphics with palette 7. The palette is a placeholder:
  nobody has checked what it holds per stage, and Zero's colours are not
  written.
- **Player tiles** (was: shared): P2's streamer entries are diverted from the `$0500` queue into the PPU's shadow tile page and P2's OAM slots read from it (p2tiles.rs, docs/zero.md), so P1 and P2 draw their own poses on every stage.

## v2 plan

- **Shots / hazards.** Route `$84:9F2A` hits to the player they touched.
- **Real Zero sprites.** Extract X2/X3 sprites or use HD-pack art as a new
  sprite set (object `+16`) and frame table (`+17`) in `$8D:8000`. Write
  Zero's red into a free CGRAM sprite palette through the `$0300` buffer.

## Verified on the real ROM (2026-09-30, USA v1.0)

- [x] Parity: group registered, `enabled=false`, 2400 frames with P1 walking
  are byte-identical to the untrapped oracle (`cargo xtask verify --coop-off`).
- [x] `call_isolated_ret` runs `$80:D671` (JSR/RTS) and `$80:E518` (JSR/RTS);
  `call_isolated` runs `$81:812E` and `$81:9D67` (JSL/RTL).
- [x] `D=0`, `DB=$8D`, `P=$22` at the old display hook `$80:D5FA`; at `$80:9A86` it is `D=0`, `DB=$86`, `P=$32`, so the slice sets M=1, X=0, `DB=$8D` itself.
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

### QA fixes B1-B5 (2026-09-30)

States from `scripts/coop-qa.sh states` (in the worktree's `states/`), dump
lines shortened to mode, X, action, HP, camera and P2's pos/action/HP/OAM.

- [x] **B4, lag-frame parity.** J's repro, no `--untrap`:

  ```
  cargo run --release -q -p xtask -- dump --load states/hw-a.state --out target/qa/b4 --frames 300 --at 270,300 --hold 0100 --refill [--coop]
  solo    frame 270 X=(1918,351) cam (1790,259)   frame 300 X=(1960,351) cam (1832,256)
  --coop  frame 270 X=(1918,351) cam (1790,259) / P2 pos=(1798,351) act 0E hp 12 oam base 116 count 9
          frame 300 X=(1960,351) cam (1832,256) / P2 pos=(1840,351) act 0E hp 12 oam base 92 count 9
  ```

  (before: 1919/1962 with `--coop`). `scripts/coop-qa.sh check`: P1 parity
  "yes" in the idle column on every milestone but M3, and M3's cause is
  another one (see "Still open": P2's tile DMA). Mirror column: yes on M0,
  M1, M2, M5, M6 (was no, B3), M8; no on M3, M4, M7 (P2's shots and
  contacts change the fight, e.g. M4's Bee Blader dies sooner and the road
  collapses earlier). `cargo xtask verify --frames 2400 --coop-off --tap
  200,400,600,800,1000 --hold 0100`: `OK 2400 frames identical (6 traps
  registered on DUT)`.
- [x] **B1, pit.** J's repro with P1 stopping at the edge (`--p1tap` holds
  Right for 540 frames), P2 walking on:

  ```
  cargo run --release -q -p xtask -- dump --load states/highway-start.state --out target/qa/b1 --frames 800 --at 540,580,590,700,710,800 --p1tap 0100@1:540 --p2-hold 0100 --coop
  frame 540 X=(782,367) act 04 hp 11 cam (654,256) / P2 pos=(800,367) act 04 hp 11
  frame 580 X=(782,367) act 00 hp 11 cam (654,256) / P2 pos=(857,499) act 08 vy -1472 hp 11
  frame 590 X=(782,367) act 00 hp 11 cam (654,256) / P2 (hidden) pos=(862,517) act 0C hp 0
  frame 700 X=(782,367) act 00 hp 11 cam (654,256) / P2 (hidden) pos=(862,517) act 0C hp 0
  frame 710 X=(782,367) act 00 hp 11 cam (654,256) / P2 pos=(804,367) act 04 hp 16
  frame 800 X=(782,367) act 00 hp 11 cam (654,256) / P2 (hidden) pos=(856,512) act 0C hp 0
  ```

  P2 dies in the pit, comes back next to P1 at full HP 120 frames later
  (and, still holding Right, dies again); mode stays `02/04/04`, P1 and lives
  untouched. With J's original `--hold 0100 --p2-hold 0100`, P2 is down
  (Y 517, act `0C`, hidden) at f600 and P1, falling into the same pit, sets
  `$D3=6` by f700 (was: P2 standing at `(950,895)`).
- [x] **B2, vertical scroll.** M5 route (`hw-c`, wall kicks up the Bee
  Blader crash, `--p1tap 8000@20:25,8000@80:25,8000@140:25,8000@200:25,` +
  `8000@F:8` every 16 frames from 330 to 698, `--hold 0100 --refill --coop`):

  ```
  frame 500 X=(2769,466) act 10 cam (2641,366) / P2 pos=(2649,575) act 00 hp 16
  frame 560 X=(2767,420) act 10 cam (2639,324) / P2 pos=(2648,575) act 00 hp 16 oam count 0
  frame 580 X=(2776,418) act 08 cam (2648,299) / P2 (hidden) pos=(2648,575) act 0C hp 0
  frame 700 X=(2773,344) act 08 cam (2645,256) / P2 (hidden)
  frame 760 X=(2861,351) act 04 cam (2733,256) / P2 pos=(2797,351) act 00 hp 16
  ```

  P2 is taken out once the camera is 288 px above it and re-anchors on the
  first grounded P1 frame after the 120-frame wait. M4 (P2 falling with the
  collapsing road to Y 591 while the camera lags) is not a false positive.
- [x] **B5, clamp** (superseded: no clamp since "Following the camera"). `dump --load states/highway-start.state --frames 240
  --at 120,240 --hold 0100 --coop`: f120 camera 171, P2 179; f240 camera
  346, P2 354: P2 = camera + 8, same frame, whole sprite on screen
  (`target/qa/m0-walk-idle/frame_000240.png`).
- [x] **B3, shots.** Bee Blader 2 (`hw-bee2`, enemy slot 0 `$0E68`, HP
  `+$27`), Y tapped every 12 frames from f10 for 1500 frames, HP every 50
  frames (`x` = slot freed):

  ```
  dump --load states/hw-bee2.state --out target/qa/kp2 --frames 1500 --at 50,100,.. --refill --coop --p2tap 4000@10:2,4000@22:2,.. --wram
  P1  28 24 24 24 22 19 18 18 15 11 10 7 3 0 0 x0 ..
  P2  28 24 24 24 22 19 19 19 16 12 11 8 4 0 0 x0 ..
  both 28 24 24 24 22 19 18 18 15 11 10 7 3 0 0 x0 ..
  ```

  (before: P2 alone stuck at 28, both never killed it.) P2 shooting vs
  nobody shooting: P1's block `$0BA8..$0C38` identical at every 50th frame
  to f400. Both together are no faster than P1 alone here (boss i-frames
  after a hit, `+$3E`; unverified). P1's `$0BDD` stays in 0..3.
- [x] Pause (M2): P2 frozen and not drawn in the weapon menu (hide hook at
  `$D545`), `target/qa/m2-pause-idle/frame_000160.png`; back where it was
  after the fade.

Still open:

- [ ] Local players 3 and 4 have independent inputs, shadow player blocks,
  update/physics/contact passes, OAM slots, and save-state data. Their sprites
  still share P2's single PPU shadow tile page and extra palette. When their
  animation poses differ, one player's tile upload can change another's art;
  the visible pose or colours can be wrong. The P2-specific companion-object,
  effect, and HUD paths also need separate ownership for full four-player
  parity. The real-ROM `local_four` test covers separate movement and saved
  positions, not visual correctness.

- [ ] **P2's tile DMA can still shift P1's lag frames** (M3 idle: P2 is hit
  at f272, P1 is 4-5 px behind solo from f275). P2's animation changes
  queue its tiles into the `$0500` VRAM queue (`$0510`/`$0518`, `$A3`,
  `$1F25` in `MMX_COOP_DIFF`); the NMI runs that DMA in real time, so the
  NMI is longer and the main loop lags earlier. Throwaway check: dropping
  P2's queue entries after its update slice (restore `$0500..$057F`, `$A3`,
  `$1F25`) gives M3 idle parity over 800 frames. The fix is the shadow tile
  page (docs/zero.md route A': P2's tiles written outside emulated time),
  the earlier work.
- [ ] Enemy shots / hazards through `$84:9F2A` hit only P1.
- [ ] Enemies that die on contact die only on P1.
- [x] Shared player tiles: fixed by the PPU shadow tile page (docs/zero.md route A'); P2 no longer writes VRAM. Still open: P2 charging rewrites X's sprite palette `$0422..`/`$A1` (P1 flashes).
- [ ] Spikes not seen (no spikes on Highway).
- [ ] Restoring `$1E48..$1E89` on every scroll type (only horizontal seen).
- [ ] Other stages, bosses, the weapon menu and the stage-select flow.
