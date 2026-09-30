# Co-op QA: the Highway stage, game over, password

Scripted runs over the real ROM (USA v1.0), 2026-09-30. Everything here is
reproducible with

```
export MMX_ROM=roms/mega-man-x.smc
scripts/coop-qa.sh states   # replays the route, writes states/*.state (~1 min)
scripts/coop-qa.sh check    # solo / --coop P2 idle / --coop P2 mirror at every milestone
```

`check` runs each milestone three times from its state: solo, `--coop` with P2
idle, `--coop` with P2 on the same pad script as P1 (`--p2-hold`/`--p2tap`),
diffs P1's dump line (mode bytes, pos, action, HP, lives, camera, pad) and
prints P2's line. PNGs land in `target/qa/<milestone>-{solo,idle,mir}/`.
Frame numbers restart at 1 after each `--load`. The route taps Y every 12
frames; from `hw-a` on it uses the new `dump --refill` cheat (X's HP is put
back to max every frame; pits still kill) so a scripted X survives.

xtask additions (all on `dump` and the shared input script): `--to F` ends
`--hold`/`--p2-hold`; `--p1tap HEX@F:N` holds for N frames (default 4);
`--wram` writes `frame_N.wram` next to each PNG; `--refill`; `--untrap HEX,..`
to A/B single co-op hooks; the dump line also prints `$D4`, camera Y and
`$1F24`.

## Milestones

Mode = `$D1/$D2/$D3/$D4`. World X of the milestone start in brackets.

| # | state (script name) | frames | mode bytes seen | PNG | P2 | P1 parity idle / mirror |
|---|---|---|---|---|---|---|
| M0 walk | `highway-start` (X 128) | 1-600 | `02/04/04/00` | `m0-walk-*/frame_000500.png` | idle: stays at its anchor and is dragged by the camera clamp, drawn half off the left edge (sprite anchor is the centre). mirror: 110 px ahead of P1 (takes no contact damage), falls in the first pit ~f500 and never dies (**B1**) | yes / yes |
| M1 pit death | `highway-start` + jump @555 | 590-1200 | 4 -> `02/04/06/00` (f600, ~200 frames) -> `02/04/00/00` (f800, lives 2->1) -> `00/01` reload -> `02/01` READY (f900) -> `04/00` beam-in, act `$22` (f1080) | `m1-pitdeath-*/frame_001100.png` | hidden from f600 through the whole reload; re-anchored at f1100 once P1 landed (act `$02`, vy 0) | yes / yes |
| M2 pause | `highway-start`, Start @100 and @200 | 99-300 | `$D3` stays 4; `$1F24=1` from f104 to ~f250; camera reads (0,0) while the menu is up | `m2-pause-idle/frame_000160.png` | not drawn in the weapon menu (display hook emits 0 slots), frozen, back after the fade | yes / yes |
| M3 pit 2 | `hw-a` (X 1596) | 1-800 | `02/04/04/00` | `m3-pit2-*/frame_000640.png` | idle and mirror both sink to Y 895 in the pit P1 wall-kicks out of (**B1**) | **no** (lag frame, **B4**) / no |
| M4 Bee Blader 1 | `hw-bee` (X 2720, camera locked at 2564) | 1-1000 | `02/04/04/00`, no special phase for the mid-boss | `m4-bee1-*/frame_000786.png` | idle: visible, falls with the collapsing road to Y 591 and lands (good). mirror: its shots stop P1's damage, the boss never dies (**B3**) | **no** (f782, **B4**) / no (**B3**) |
| M5 collapse + climb | `hw-c` (X 2642, camera Y 479) | 1-760 | `02/04/04/00`, camera Y 479 -> 256 | `m5-climb-*/frame_000600.png` | camera scrolls up with P1's wall-climb, P2 stays at Y 575 off screen (0 OAM slots) forever (**B2**) | yes / yes |
| M6 Bee Blader 2 | `hw-bee2` (X 3452, camera locked at 3204) | 1-1200 | `02/04/04/00` | `m6-bee2-*/frame_000500.png` | anchored at the clamp edge (x 3444), visible; mirror again keeps the boss alive (**B3**) | yes / no (**B3**) |
| M7 stage end | `hw-end` (X 7672, camera max 7424) | 1-600 | `02/04/04/00` | `m7-end-*/frame_000300.png` | anchored and clamped to 7664, visible; road attackers pass through it | yes / no (shots) |
| M8 game over | `gameover` (3 pit deaths, f3300 from start) | 1-450, Start @20 | `02/0A/00/00` (password screen) -> `02/02/16/00` briefly -> `02/04/00/01` -> `04/02/01` -> `04/04/00` | `m8-gameover-*/frame_000350.png` | hidden throughout, re-anchored on the first grounded frame after the beam-in | yes / yes |

Not reached: Vile's ride-armor scene, stage clear (`$D3` 8/`$A`/`$C`), Zero's
cutscene and stage select. At the end of the road (X 7672, the camera's right
limit 7424) road attackers (enemy ids `$11`/`$12`) keep coming from a spawner
(`$1A`/`$1B`) off screen right and Vile never dropped in 3000 frames of
shooting, standing still, or standing mid-screen. The trigger is unknown
(possibly `--refill` interferes, possibly the car wave must be cleared).
`states/r3-*.state` are 120-frame steps through the last third of the stage.

## Bugs

**B1. P2 never dies in a pit.** P1's pit death fires at `y - cam_y = 257`
(Y 513 with the camera at 256) from code P2's slices don't run. P2 keeps
falling to Y 895, lands on something below the level, stays "walking" off
screen and is only hidden when P1 dies.
`cargo run --release -q -p xtask -- dump --load states/highway-start.state --out target/qa/b1 --frames 600 --at 500,560,600 --hold 0100 --p2-hold 0100 --coop`
(P2 at `(950,895)` by f600). Fix: treat `coop_stages::p2_lost` as a death.

**B2. P2 is left behind by vertical scrolling.** No vertical clamp or
re-anchor: when P1 climbs out of the Bee Blader crash the camera goes from Y
479 to 256 and P2 stays at Y 575, off screen with 0 OAM slots, for good.
`scripts/coop-qa.sh check`, milestone m5-climb (P2 stays at `(2648,575)`,
`count 0`, while the camera ends at Y 256).
Fix: `p2_lost` -> re-anchor next to P1 when `p2_anchor_ok`.

**B3. P2's shots block damage to the mid-boss.** Bee Blader 2 (enemy id
`$22`, HP at `+$27` = 29): P1 shooting alone takes it to 7 by f600; P2
shooting alone takes it to 28 (one hit) and then nothing; both shooting and
it never dies (2400 frames), while P1 alone kills it in ~800. So P2's shots
leave the enemy in a state that also stops P1's shots (hit/invulnerability
bookkeeping keyed on something P2 shares with P1, or the shared shot slots).
Repro (ten Y taps each; read the boss's HP at `$0E68+$27` in the `.wram`):
`dump --load states/hw-bee2.state --out target/qa/b3 --frames 200 --at 200 --refill --coop --p1tap 4000@10:2,4000@22:2,4000@34:2,4000@46:2,4000@58:2,4000@70:2,4000@82:2,4000@94:2,4000@106:2,4000@118:2 --wram` -> HP 24;
the same with `--p2tap` instead of `--p1tap` -> HP 28.

**B4. Co-op changes lag frames, so P1 drifts even with P2 idle.** In heavy
scenes the solo run lags a frame the co-op run doesn't (`$0B9B` is one ahead
in co-op at M3 f270, M4 f782), P1's position then differs by a few pixels.
Cause, confirmed with `--untrap 80D5FA`: the P2 display slice advances the
OAM cursor `$E4`, so the ROM's own "hide the unused slots" loop after lists
3-5 runs 8-10 fewer iterations and the frame finishes earlier. Parity comes
back with the display hook off. Fix idea: emit P2 after the ROM's hide loop
(into slots it just hid), or save/restore `$E4` and use the top slots.
Repro: `dump --load states/hw-a.state --out target/qa/b4 --frames 300 --at 270,300 --hold 0100 --refill`
gives P1 X 1918/1960; add `--coop` -> 1919/1962; add `--coop --untrap 80D5FA` -> 1918/1960 again.

**B5 (minor). Idle P2 drawn half off the left edge.** The clamp window
`[cam, cam+240]` is on the sprite anchor (centre), so a P2 pushed by the
camera shows half a sprite. `[cam+8, cam+248]` would keep it whole. Also the
clamp runs one frame behind the camera (P2 169 vs camera 171 at M0 f120).

Known v1 limits seen again, not re-reported: no contact damage on P2 (M0,
M7), P2 shots change the fight (every mirror NO above except B3's size).

## What `p2_should_exist` / `p2_anchor_ok` encode

`crates/mmx-core/src/coop_stages.rs`:

- `p2_should_exist`: `$D2 == 4` && `$D3 == 4` && `$1F24 == 0` && X action
  `!= $0C` && X HP `& $7F != 0`.
- `p2_anchor_ok`: `p2_should_exist` && X action in {`$00` stand, `$02`
  ready/landed, `$04` walk} && X vy == 0 && P1's Y inside the camera window.
- `p2_lost(p2_y, cam_y)`: P2 at or below `cam_y + 257` (pit) or 64+ px above
  the window (left behind). For B1/B2.

X actions seen: `$00` stand/push, `$02` ready, `$04` walk, `$06` jump, `$08`
fall, `$0A` land, `$0C` death, `$0E` hurt, `$10` wall kick, `$12` wall slide,
`$18` (once, falling beside a wall), `$22` beam-in.

## Password screen and game over

- Title: `--tap 200,400,600,800` reaches the menu at ~f950 (`states/title.state`
  is written by `dump --tap 200,400,600,800 --save 950=...`); Down @960 +
  Start @980 opens the password screen (~f1100).
- Entry digits: `$7E:1E60..$1E6B`, one byte per digit, value - 1 (0 = "1",
  7 = "8"), row-major. B on a digit steps it down (1 -> 8). Cursor: column
  `$1E4C`, row `$1E4F` (3 = the END row); Up from the top row wraps to END.
- Generated password (game-over screen, `$D2=$0A`): `$7E:FFCB..$FFD6`, same
  encoding. Losing all lives in Highway gave **3433 3173 6121**; Start there
  continues Highway (not stage select, since the stage is not cleared).
- Not done: a password past the intro stage (needs the stage cleared, or the
  decoder at the END handler). Stage select, the Chill Penguin run and `$1F7A`
  for other stages are still open.
