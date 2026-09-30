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
| `$80:D241` | instruction after `JSL $81:9D67` (P1's terrain/physics phase, run after the enemy and shot loops) | physics slice: shot bookkeeping (B3), `$81:9D67`, `$80:D292`, then `coop::follow` (pit death / left behind, see "Following the camera") |
| `$80:D545` | second instruction of the OAM build `$80:D544` (`PHB`; `$D544` is Zero's palette trap) | hide last frame's P2 slots (Y = `$E0`) |
| `$80:9A86` | instruction after `JSR $80:D544` at `$80:9A83` in the per-frame stage code: the whole OAM build (and its hide loop) is done, the camera is final | display slice: `$80:D640` + `$80:D671` on P2's block into the slots past the game's, Zero palette, direct page put back; with the X3 set, the Rust emitter `zero::emit_x3` instead (docs/zero.md "Drawing P2 as X3 Zero") |
| `$80:D3B8` / `$80:D3C1` | shot loop, before / after each `$1228` slot (`D` = the slot) | P2's shots (B3): P2's block swapped in around their update |
| `$80:D1FF` | instruction after the object master's `JSR $80:D3B2` (the shot loop; the enemy loops follow) | saber (X3 set only): the arc's shot into a free `$1228` slot, taken out at `$80:D241` (docs/zero.md "The saber") |
| `$80:9A94` | `LDA $1F30`, where both paths after `$9A86` meet (every gameplay frame, after the display slice) | P2's HP bar and down marker (`coop_hud.rs`, "P2 HP bar and revive") |
| `$84:9B03` | entry of the enemy-vs-X contact routine; every enemy's AI `JSL`s it with `D` = the enemy (154 call sites) | contact pass: the same routine again, isolated, with P2's block swapped in |

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
high-table bytes set to `$AA`), with X = 232 (216 when the boss bar is up)
and palette 3 (`zero_palette`). `p2_oam_base/count` grow to cover them, so
`oam_hide` clears them at the next build like P2's sprites. P2's display
slice leaves `HUD_SLOTS` slots free below 128 (`room`); with P2 hidden and
the game at 128 slots the bar takes the game's last 8. The bar is rebuilt
every frame from P2's block, so there is no cache to carry. ponytail: P2's
bar reuses X's "X" icon, recoloured.

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
slot 8 of the HUD block: the bar's icon (tile `$86`, palette 3) centred on
the spot, blinking every 8 frames of the timer, only while the spot is on
screen. All of it (`CoopState::revive`, `down_frames`) is in `CoopState`,
so netplay rollback carries it. ponytail: P2 is hidden while down; the X3
set's lying frame (135) is not used.

Verified (dump lines shortened; `P2 down N spot S touch T` is a new dump line):

```
MMX_COOP_P2_HP=6 dump --load states/hw-spiky.state --frames 200 --at 30,34,60,100 --coop --p2-hold 0100
frame 30  X hp 16 / P2 hp 6    frame 34  X hp 16 / P2 act 0E hp 4
```

`frame_000030.png` / `frame_000060.png`: P2's bar (right edge, red) drops
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
- **Pits** are not decided by the player code: the camera code (`$80:E14A`,
  listing PC, run on P1 only) kills X when `Y - $20 >= camera Y + $E0` (by
  `$84:9F2A` with `$0BCE=$7F`). P2 never ran that, and fell to the floor
  below the screen (Y 895) and stood there (B1).
- **Left behind vertically** (B2): nothing drags P2 along a vertical scroll;
  after the Bee Blader 1 climb P2 stayed at Y 575 with the camera at 256.
- `coop_stages::p2_lost(y, cam_y)` is the ROM's pit rule above; with P2
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
  its own HP bar on the right edge (see "P2 HP bar and revive").

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
