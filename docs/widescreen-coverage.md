# Widescreen coverage: a 100% TAS and per-stage traversals

Question: play a public 100% tool-assisted movie in single player with
widescreen on; does anything break? Done 2026-10-05 on `472e410` + this
change. Settings for every run: single player (no `--coop`, no `--zero`),
wide gameplay OFF (`movie` refuses `--wide-gameplay`), CPU overclock OFF
(xtask's default; printed by `movie` as `overclock Off`).

PNG paths below are under `target/movie/` (local, not committed); the
commands at the end regenerate them.

## The movies (TASVideos)

Downloaded (input files only, parsed as data) into `movies/` (gitignored).

| publication | author | file | size (file / zip) | recorded on | ROM | frames | rerecords |
|---|---|---|---|---|---|---|---|
| [3197M](https://tasvideos.org/3197M), current 100%, 33:07.13 | Hetfield90 | `hetfield90v3-megamanx-100p.bk2` | 67,419 / 58,170 B | BizHawk 1.11.6, bsnes core, "Compatibility" profile | USA v1.0 `Megaman X (U) (V1.0) [!]`, SHA1 `449A0063…7FB1` | 119,424 | 36,236 |
| [2771M](https://tasvideos.org/2771M), obsolete 100%, 33:48.86 | Hetfield90 | `hetfield90-megamanx-100p.bk2` | 29,641 / 22,380 B | BizHawk 1.9.1, bsnes core | USA v1.0, same SHA1 | 121,932 | 53,768 |
| [1261M](https://tasvideos.org/1261M), obsolete 100%, 33:58.87 | dezbeast | `dezbeast-megamanx.smv` | 244,970 / 7,819 B | Snes9x 1.43+ v15.1 | USA v1.1 `Megaman X (U) (V1.1) [!]`, CRC32 `ded53c64` (= our Rev 1 dump: SHA1 `c65216760ba9…2097`) | 122,333 | 126,805 |

All three: one controller, start from power-on, no resets. Route (4595S
submission notes): Intro, Chill Penguin, Boomer Kuwanger, Sting Chameleon,
Storm Eagle, Flame Mammoth, Penguin revisit, Spark Mandrill, Armored
Armadillo, Launch Octopus, Sigma 1-4; no passwords.

There is no USA v1.0 dump here, so the bk2 movies run on Rev 1 (and, as a
check, on the PAL v1.0 dump forced to NTSC timing: identical result).

## Sync (widescreen off)

**Update 2026-10-05 (docs/oracle.md):** on the exact USA v1.0 dump and after
the timing fixes found with the Mesen 2 oracle (bsnes's APU clock ratio, the
H/V IRQ and NMI edges, DMA end alignment, APU port read point, CPU start
delay; plus Left+Right passed through for movies), 3197M now syncs through
the Intro, Chill Penguin (boss included), Boomer Kuwanger and into Sting
Chameleon's stage to ~35,231 (X dies there); 2771M clears Intro, Penguin,
Kuwanger and Chameleon and desyncs at the following stage select (36,436).
The table below is the state before that work.

| movie | how far it stays in sync | first sign of desync |
|---|---|---|
| 3197M | Power-on, title, the whole Intro highway incl. the Vile fight (stage clear `$D3=08` at frame 8648), Zero's scene, the stage select (cursor to Penguin at 9027), Penguin's intro, and Chill Penguin's stage up to the Leg Parts (dashing from 13800) and the Ride Armor (in at 14375, out with the movie's own Up+B at 14428) | ~14700: X jumps off the cliff edge at x 5208, is hit by a bird and falls into the pit (dies 14894-15085). Later the movie's inputs keep X stuck in the Ride Armor for the rest of the run |
| 2771M | Intro and the whole of Chill Penguin's stage to the boss (boss bar 15937) | Chill Penguin kills X at 17160 (the TAS kills him); never leaves the stage |
| 1261M (smv) | Not even the Intro: X dies at ~1800 | Snes9x 1.43's timing |

Frame alignment is right: both bk2 movies press Start first at movie frame
359 and the Intro (thousands of frames of RNG-driven enemies and the Vile
fight) replays exactly, so one movie line = one `Game::step` with lag frames
included, no offset. Tried, none of which moved the desync: power-on WRAM
filled with `$55` like bsnes v0.87 (`--wram-fill 55`); the v1.0 code (PAL
dump at NTSC); single inserted or dropped frames (`--movie-offset ±1@F`) at
26 points between 9700 and 14500 (lag-frame mismatch hypothesis: none made X
survive). The RNG (`$0BA6`) only advances in game logic, not in the idle
loop, so it is not a timing sink. Most likely cause: a small CPU/DMA/APU
timing difference between our core and bsnes v0.87 that shifts a lag frame
or an enemy's behaviour in Penguin's stage. Not pursued further.

So the 100% run covers, faithfully, the Intro stage and Chill Penguin's
stage up to its second half (and, with 2771M, to the boss fight). Everything
after that is the fallback below, which is not the 100% run.

## The widescreen runs on the movies

`--twin`: a widescreen-off game is stepped beside the wide one with the same
inputs; WRAM, OAM, CGRAM and VRAM are compared every frame, and the wide
frame's centre (columns 32..224 of the 256, the HUD moves out) against the
unwidened frame. `MMX_MARGIN_CHECK` residency check on. Picture checks every
30 frames in stage.

| movie, preset | frames | panics | state diffs | centre diffs | margin stale/slot/page/palette (late) | picture flags flat L/R, seam L/R, tilecol | verdict |
|---|---|---|---|---|---|---|---|
| 3197M 16:9, Intro | 508-8945 | 0 | 0 | 0 | 0/0/0/0 (366) | 2/2, 4/7, 5 | clean |
| 3197M 16:9, Penguin | 9603-16000 | 0 | 0 | 0 | 0/0/0/0 (117) | 0/3, 9/3, 14 | clean (cosmetic: outside-the-level backdrop) |
| 3197M 21:9, Intro | 508-8945 | 0 | 0 | 0 | 0/0/0/0 (380) | 8/2, 4/7, 14 | clean |
| 3197M 21:9, Penguin | 9603-16000 | 0 | 0 | 0 | 0/0/0/0 (117) | 6/3, 9/3, 30 | clean (same) |
| 2771M 16:9, Intro + Penguin to boss | 1-18000 | 0 | 0 | 0 | 0/0/0/0 (230) | 4/14, 12/60, 13 | clean (boss room: right margin is sky beyond the room wall) |
| 2771M 21:9, same | 1-18000 | 0 | 0 | 0 | 0/0/0/0 (244) | 16/16, 12/60, 34 | cosmetic (boss name repeated, below) |

Picture flags looked at (`target/movie/v3-16x9/`, `v3-21x9/`, `v2-16x9/`):
every flag inspected is a real edge or the area outside the level: the
stage's first column at the Intro's and Penguin's start (`f000630_find_*`,
`f009720_find_*`: left margin flat backdrop, camera at x 0), Penguin's
vertical shaft (`f011550_find_flat-right.png`: sky beyond the room), the
boss room (`v2-16x9/f015937_boss-bar-on.png`, `f016800.png`: sky right of
the room wall; X's charged shot drawn over it; boss bar at the wide right
edge), girders and lamp posts (`f002280_*`, `f002370_*`). Seen and fine:
Vile's fight and ship (`f006600.png`), stage clear and Zero's scene
(`f008648_*`, `f008820_*`), Penguin's Ride Armor section, the pause menu.

Cosmetic, not in the stages (BG maps of menus wrap; the margins show their
other half or the backdrop):

- Boss intro at 21:9 only: the boss's name is drawn again in both margins
  ("PENGUIN" left, "CHILL P" right), `v3-21x9/f009545_mode02021600-L08.png`.
  16:9 is clean (`v3-16x9/f009545_*`).
- Stage select: fragments of the frame's border panels in both margins,
  `v3-21x9/f009027_mode02000104-L08.png`, `v3-16x9/f009027_*`.
- Password screen after a stage and the pause/weapon menu: flat green
  margins beside a strip of the menu's machinery,
  `v3-16x9/f008910_find_flat-left.png`, `v2-16x9/f017970_find_flat-left.png`.

## Fallback: all stages by scripted traversal (not the 100% run)

From the movie's own stage select after the Intro (`het-9000` state, 6
lives), the cursor is put on each stage (column `$1E4C`/`$1E6A`, row
`$1E4F`/`$1E6C`, stage `$1E65`; found by diffing WRAM around d-pad presses:
Octopus (1,0), Penguin (2,0), Armadillo (0,1), Mammoth (3,1), Eagle (0,2),
Kuwanger (3,2), Chameleon (2,3), Mandrill (1,3)) and confirmed with Y, so
each stage loads as the game loads it. Sigma 1-4 are not on the select
screen: Octopus is confirmed and `LEVEL_ID` (`$1F7A`) forced to `$09..$0C`
until the stage loads, so their graphics and state come from that path, not
from the real Sigma unlock. Then 7200 frames (about 6450 in stage): hold
right, B 20 of every 45 frames, Y 3 of every 40, `--refill`, lives poked to
9. This is a traversal script: it gets stuck at walls, ladders and vertical
shafts, and reaches no boss.

| stage | gameplay frames | deaths | end (X, camera) | state / centre diffs | margin stale/slot/page/palette | picture flags 16:9 / 21:9 (flat L/R, seam L/R, tilecol) | verdict |
|---|---|---|---|---|---|---|---|
| Launch Octopus `$01` | 745-7200 | 2 | dies at x 2296 | 0 / 0 (34 frames before the fix below) | 0/0/0/0 | 0/0,0/0,0 / 0/0,0/0,4 | broken before the fix (simulation changed); clean after. Known cloud-layer end in the left margin at the start (`fb-16x9/L01/f004800.png`) |
| Sting Chameleon `$02` | 846-7200 | 1 | x 2162 | 0 / 0 | 0/0/0/0 (late 7) | 0/0,0/0,0 / 13/0,0/0,1 | clean (flat-left: left of the stage start, `fb-21x9/L02/f000930_*`) |
| Armored Armadillo `$03` | 764-7200 | 3 | x 533 | 0 / 0 | 0/2361/0/0 | 0/0,29/1,10 / 25/0,29/1,13 | clean on screen; the `slot` findings are Batton Bones (id `$2D`, `+$18`=`$40`, set `$4B` slot `$36`) drawn correctly in the margins (`fb-16x9/L03/f001500.png`, `f003300.png`): a false positive of the check |
| Flame Mammoth `$04` | 723-7200 | 0 | x 2168 | 0 / 0 | 0/305/0/0 | 0/0,1/2,13 / 0/0,1/2,16 | clean on screen; `slot` on id `$2A` (set `$45`, table slot `$00`): enemies look right (`mam/f001520.png`, `f001560.png`, `f002500.png`) |
| Storm Eagle `$05` | 737-7200 | 8 | dies near x 517 | 0 / 0 | 0/0/0/0 | 0/0,52/8,19 / 52/0,52/8,35 | cosmetic: early in the stage (camera x ~124) the right margin below the clouds is flat sky blue with a hard edge at the window boundary, the distant mountains and the runway stop there while the girders continue (`fb-21x9/L05/f001200.png`, `fb-16x9/L05/f001200.png`); further on the margins fill (`fb-21x9/L05/f002100.png`) |
| Spark Mandrill `$06` | 709-7200 | 0 | x 440 | 0 / 0 | 0/0/0/0 | 0/0,7/0,1 / 7/0,7/0,2 | clean (`fb-21x9/L06/f003000.png`) |
| Boomer Kuwanger `$07` | 738-7200 | 0 | climbing at y ~5900 | 0 / 0 | 0/0/0/0 | 0/213,0/0,0 / 213/213,0/0,0 | cosmetic/unverified: the tower shaft has black margins on both sides the whole time (`fb-16x9/L07/f003000.png`); probably empty layout outside the shaft, not checked against the map |
| Chill Penguin `$08` | 735-7200 | 0 | x 2776 | 0 / 0 | 0/0/0/357 (21:9: 535) | 0/129,6/130,2 / 7/129,6/130,4 | clean on screen; `palette` findings all on one pickup-table object (`$1468`, id `$0B`) at frames 1700-1742 next to a destroyed enemy's debris; the margin pieces look like the centre ones (`pen-pal/f001700.png`..`f001741.png`); the flat/seam flags are the vertical shaft (as in the movie run) |
| Sigma 1 `$09` | 141-7200 | 3 | x 1429 | 0 / 0 | 0/0/0/0 | 0/0,57/0,1 / 50/0,57/0,4 | clean (left of the stage start; `fb-16x9/L09/f002610_find_tilecol.png`) |
| Sigma 2 `$0A` | 141-7200 | 8 | x 539 | 0 / 0 | 0/0/0/0 | 0/0,70/2,0 / 68/0,70/2,1 | clean (`fb-16x9/L0A/f001320_*`) |
| Sigma 3 `$0B` | 123-7200 | 0 | stuck at x 152 | 0 / 0 | 0/0/0/0 | 0/32,234/234,0 / 234/32,234/234,0 | cosmetic/unverified: black margins both sides in the shaft (`fb-16x9/L0B/f000210_*`, `f003000.png`), as Kuwanger |
| Sigma 4 `$0C` | 124-7200 | 0 | stuck at x 139 | 0 / 0 | 0/0/0/0 | 0/0,0/234,0 / 234/0,0/234,0 | as Sigma 3 (`fb-16x9/L0C/f000210_*`) |

No panic in any run (movies or fallback, both presets).

## Broken, fixed: widescreen changed the simulation in Launch Octopus

With 16:9 on, Launch Octopus's WRAM differed from the unwidened game on 34
of ~6450 frames: dead stack bytes `$012B-$0130` and `$02FD` (A1 vs A4: the
NMI interrupted the main loop at a different instruction). It healed the
next frame each time, but it is inside the netplay checksum range
(`$0000-$1FFF`) and it means the wide game's CPU timing differed, so a
borderline frame could have lagged differently. Cause: the margin sprite
pass runs the ROM's sprite emitter isolated (`run_cpu_isolated`), which
ticks the DMA unit; a call sitting on an HDMA start/reload point ran that
HDMA inside the call, and `CpuSnapshot` did not hold the DMA unit, so the
real HDMA (and its CPU stall) then never happened at its time. Octopus is
the stage with HDMA (water). Fix: `CpuSnapshot` holds the DMA unit too
(`vendor/jgenesis/snes-core/src/api.rs`). Test:
`margin_pass_keeps_hdma_timing` (PAL `octopus-start`, 900 frames, every
WRAM byte every frame; failed at frame 91 before the fix). After it, every
run in this document has 0 state and 0 centre differences.

Not checked: co-op's own isolated calls (P2's update) also tick the DMA
unit; they change the game for both peers equally (not display-only), but
an HDMA run inside one would happen early. Not investigated here.

## What is not covered

- The 100% run past Sting Chameleon's stage: it desyncs on our core
  (docs/oracle.md).
- Boss fights other than Chill Penguin's (2771M, until X dies), the Penguin
  revisit, the capsules other than the Leg Parts, Sigma stages' real unlock
  path, the ending: the traversal reaches no boss.
- Mode 7, mosaic and offset-per-tile scenes (known: not widened).
- Kuwanger's / Sigma 3-4's black margins against the stage map.

## Commands

Environment: `MMX_ROM` = the Rev 1 dump (`Mega Man X (USA) (Rev 1).sfc`).

```
# sync check (widescreen off), status every 600 frames
cargo xtask movie --movie movies/hetfield90v3-megamanx-100p.bk2 --wide off --log-every 600 --out target/movie/off
# the widescreen runs
cargo xtask movie --movie movies/hetfield90v3-megamanx-100p.bk2 --frames 16000 --wide 16:9 --twin \
    --png-every 300 --save-every 1000 --save-dir target/movie/states/v3-16x9 --out target/movie/v3-16x9
cargo xtask movie --movie movies/hetfield90-megamanx-100p.bk2 --frames 18000 --wide 21:9 --twin --out target/movie/v2-21x9
# stage select state used by the fallback
cargo xtask dump --movie movies/hetfield90v3-megamanx-100p.bk2 --frames 9000 --out target/movie/tmp --at 9000 \
    --save 9000=target/movie/states/het-9000.state
# one stage (here Storm Eagle: column 0, row 2, stage 05) at 21:9
cargo xtask movie --load target/movie/states/het-9000.state --frames 7200 --wide 21:9 --twin --png-every 300 \
    --poke 7E1E4C=0@1-39,7E1E6A=0@1-39,7E1E4F=2@1-39,7E1E6C=2@1-39,7E1E65=05@1-39,7E1F80=09@1-7200 \
    --hold 0100 --from 1000 --p1tap 4000@40,8000@1000-7200/45:20,4000@1000-7200/40:3 --refill --out target/movie/fb-21x9/L05
# a Sigma stage: confirm Octopus, force LEVEL_ID
    --poke 7E1E4C=1@1-39,7E1E6A=1@1-39,7E1E4F=0@1-39,7E1E6C=0@1-39,7E1E65=1@1-39,7E1F7A=09@45-700,7E1F80=09@1-7200
# the Octopus regression (PAL dump, states/octopus-start.state)
MMX_ROM=…/mega-man-x.smc cargo test -p mmx-core --test widescreen margin_pass_keeps_hdma_timing
```

`movie` writes `DIR/segments.md` (the per-segment table above),
`DIR/events.txt` (mode changes, deaths, boss bar, pause, each with a PNG),
`DIR/findings.txt` (twin, centre, margin and picture findings) and PNGs; on
a panic it writes the last state before it and prints a repro command (exit
3). `--twin-keep` lets the two games drift instead of re-aligning after a
difference; `--movie-offset N[@F],..` inserts/drops frames.
