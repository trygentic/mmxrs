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

Re-run after the B1-B5 fixes (same day). `check` now gives P1
parity idle / mirror: M0 yes/yes, M1 yes/yes, M2 yes/yes, M3 **no**/no,
M4 yes/no, M5 yes/yes, M6 yes/**yes**, M7 yes/no, M8 yes/yes. M3 idle is no
longer B4 (see B4b); the mirror "no"s are P2's shots and contacts changing
the fight. The table below is J's original run; the P2 column changes are:
M0 mirror P2 dies in the first pit (f560) and respawns; M3 P2 no longer
sinks to Y 895; M5 P2 is taken out at f~570 and re-anchored at f760; M6
mirror kills the boss.

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

**B1. P2 never dies in a pit.** FIXED (`fell_off` on main, now
`coop_stages::p2_lost` in the physics slice; P2 down at Y 517 by f590,
respawn after 120 frames; docs/coop.md "QA fixes"). P1's pit death fires at `y - cam_y = 257`
(Y 513 with the camera at 256) from code P2's slices don't run. P2 keeps
falling to Y 895, lands on something below the level, stays "walking" off
screen and is only hidden when P1 dies.
`cargo run --release -q -p xtask -- dump --load states/highway-start.state --out target/qa/b1 --frames 600 --at 500,560,600 --hold 0100 --p2-hold 0100 --coop`
(P2 at `(950,895)` by f600). Fix: treat `coop_stages::p2_lost` as a death.

**B2. P2 is left behind by vertical scrolling.** FIXED (`p2_lost`: P2 down
at f~570 when the camera passes Y 319, re-anchored at f760 next to P1). No vertical clamp or
re-anchor: when P1 climbs out of the Bee Blader crash the camera goes from Y
479 to 256 and P2 stays at Y 575, off screen with 0 OAM slots, for good.
`scripts/coop-qa.sh check`, milestone m5-climb (P2 stays at `(2648,575)`,
`count 0`, while the camera ends at Y 256).
Fix: `p2_lost` -> re-anchor next to P1 when `p2_anchor_ok`.

**B3. P2's shots block damage to the mid-boss.** FIXED. Cause: X's live
shot count `$0BDD` (block `+$35`) is incremented by the fire routine through
direct page (P2's copy) but decremented by the shot's despawn with an
absolute `DEC $0BDD` from the shot loop (P1's copy). P2 hit the cap, P1's
count wrapped to `$FF`. Coop now tracks P2's shot slots (`$1228`) and gives
each despawn back (docs/coop.md "Shots"). Repro below now: P1 taps -> 24,
P2 taps -> 24; 1500 frames of shooting kill the boss at f~700 with P1, P2
or both. Bee Blader 2 (enemy id
`$22`, HP at `+$27` = 29): P1 shooting alone takes it to 7 by f600; P2
shooting alone takes it to 28 (one hit) and then nothing; both shooting and
it never dies (2400 frames), while P1 alone kills it in ~800. So P2's shots
leave the enemy in a state that also stops P1's shots (hit/invulnerability
bookkeeping keyed on something P2 shares with P1, or the shared shot slots).
Repro (ten Y taps each; read the boss's HP at `$0E68+$27` in the `.wram`):
`dump --load states/hw-bee2.state --out target/qa/b3 --frames 200 --at 200 --refill --coop --p1tap 4000@10:2,4000@22:2,4000@34:2,4000@46:2,4000@58:2,4000@70:2,4000@82:2,4000@94:2,4000@106:2,4000@118:2 --wram` -> HP 24;
the same with `--p2tap` instead of `--p1tap` -> HP 28.

**B4. Co-op changes lag frames, so P1 drifts even with P2 idle.** FIXED:
the display slice moved to `$80:9A86` (after `JSR $80:D544`), P2 goes into
the slots after the game's and `$E3`/`$E4` are left as the game had them;
P2's old slots are hidden at `$80:D545`. Repro below: 1918/1960 with
`--coop` too. In heavy
scenes the solo run lags a frame the co-op run doesn't (`$0B9B` is one ahead
in co-op at M3 f270, M4 f782), P1's position then differs by a few pixels.
Cause, confirmed with `--untrap 80D5FA`: the P2 display slice advances the
OAM cursor `$E4`, so the ROM's own "hide the unused slots" loop after lists
3-5 runs 8-10 fewer iterations and the frame finishes earlier. Parity comes
back with the display hook off. Fix idea: emit P2 after the ROM's hide loop
(into slots it just hid), or save/restore `$E4` and use the top slots.
Repro: `dump --load states/hw-a.state --out target/qa/b4 --frames 300 --at 270,300 --hold 0100 --refill`
gives P1 X 1918/1960; add `--coop` -> 1919/1962; add `--coop --untrap 80D5FA` -> 1918/1960 again.

**B4b (open). P2's tile DMA shifts lag frames.** M3 idle after the B4
fix: P2 is hit at f272 (the clamp drags it into the road attackers), its
new poses queue tiles in the `$0500` VRAM queue, the NMI DMA gets longer and
P1 lags a frame earlier (4-5 px behind solo from f275). Dropping P2's queue
entries restores parity (throwaway check). Needs the shadow tile page
(docs/zero.md A').

**B5 (minor). Idle P2 drawn half off the left edge.** FIXED: clamp
`[cam+8, cam+248]`, applied in the display slice against the camera the
ROM scrolled to this frame (M0 f120: camera 171, P2 179). The clamp window
`[cam, cam+240]` is on the sprite anchor (centre), so a P2 pushed by the
camera shows half a sprite. `[cam+8, cam+248]` would keep it whole. Also the
clamp runs one frame behind the camera (P2 169 vs camera 171 at M0 f120).

Known v1 limits seen again, not re-reported: no contact damage on P2 (M0,
M7), P2 shots change the fight (every mirror NO above except B3's size).

## What `p2_should_exist` / `p2_anchor_ok` encode

`crates/mmx-core/src/coop_stages.rs`, wired into `coop.rs` (update slice:
hide on `!p2_should_exist`, except pause only freezes; anchor on
`p2_anchor_ok`. Physics slice: `p2_lost` -> P2 down):

- `p2_should_exist`: `$D2 == 4` && `$D3 == 4` && `$1F24 == 0` && X action
  `!= $0C` && X HP `& $7F != 0`.
- `p2_anchor_ok`: `p2_should_exist` && X action in {`$00` stand, `$02`
  ready/landed, `$04` walk} && X vy == 0 && P1's Y inside the camera window.
- `p2_lost(p2_y, cam_y)`: the ROM's pit rule for X (`$80:E14A`: `Y - $20 >=
  cam_y + $E0`, signed; P1 is seen dying at `y - cam_y = 257` because X
  falls several px a frame) or 64+ px above the window. For B1/B2.

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

## Results on the merged tree (2026-09-30)

`scripts/coop-qa.sh check` from the regenerated milestone states:

| milestone | idle | mirror |
|---|---|---|
| m0-walk | yes | yes |
| m1-pitdeath | yes | yes |
| m2-pause | yes | yes |
| m3-pit2 | yes (B4b closed by the shadow tile page) | no: P2's shots/contacts change the fight |
| m4-bee1 | yes | no: same |
| m5-climb | yes | yes |
| m6-bee2 | yes | yes |
| m7-end | yes | no: same |
| m8-gameover | yes | yes |

Every idle run keeps P1 byte-identical to solo; every mirror "no" is a gameplay
difference P2 causes on purpose, not a leak.
