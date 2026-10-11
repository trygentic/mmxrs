# mmx-native (binary `mmxrs`)

Run: `cargo run -p mmx-native --release -- --rom mmx.sfc [--x3-rom mmx3.sfc [--zero]] [--scale 1-8] [--fullscreen] [--coop-local] [--pads] [--widescreen off|16:10|16:9|21:9|auto|N] [--wide-gameplay] [--overclock off|1.5|2]` (docs/widescreen.md;
`--wide-gameplay` makes enemies spawn, act and leave at the wide edges whatever the preset: it
changes the game, so online both players need it or the handshake refuses; `--overclock` runs
the SNES CPU 1.5x/2x as fast so busy scenes stop slowing down, **default 2x**, `--overclock off` restores the
original slowdown; it changes the game too, online both
players need the same; docs/widescreen.md "Slowdown and overclock")
(`--rom` falls back to `$MMX_ROM`, `--x3-rom` to `$MMX_X3_ROM`; default scale 3).
`--x3-rom` takes your Mega Man X3 (USA) image: P2 is drawn as X3 Zero and P2's
X swings the saber (docs/zero.md "Playing as Zero"); online, both peers need it.
`--zero` makes player 1 Zero as well (V swings the saber); it needs the X3 ROM
(without one mmxrs exits with an error) and, online, both peers must pass it. Pacing follows the audio
device (~50 ms buffered), online too (one session tick per frame of audio;
a tick that steps nothing queues a frame of silence); without an output device
a timer at the ROM's rate drives it (`Game::fps`: 60.0988 NTSC, 50.007 PAL;
`roms/mega-man-x.smc` is the PAL release). `MMX_AUDIO_STATS=1` prints samples/s,
ring depth and drops once a second.

Local multiplayer accepts `--players 1..4`; the first two slots use the
keyboard layouts below, and each further slot needs its own gamepad. A
controller joins the first unclaimed slot when it presses a button. Use
`--p1-character ID` through `--p4-character ID` to select characters, or use
the launcher dropdowns. Boss visuals are extracted from the X1 ROM at
startup; if extraction fails, the app reports the character and error.

VS mode starts with `--vs --players 2..4 --bone-sheets DIR` (or the launcher's
**VS arena match** option). Each player moves an independent cursor through
the full 19-character roster and locks a fighter with an attack button or
Start; Select unlocks it. Player 1 then chooses an arena from the Highway and
eight Maverick stage openings. Fights use the ROM's movement and attacks with
player projectile damage, a round winner, and cumulative scores. Press Start
after a round to fight again. `--vs-stage ID` presets the arena cursor; IDs
are `highway`, `octopus`, `chameleon`, `armadillo`, `mammoth`, `eagle`,
`mandrill`, `kuwanger`, and `penguin`. VS is local only; players 3 and 4 need
gamepads. The animation sheets include Bass and Proto Man, who have no X1 ROM
sprites. Vile also needs the on-foot PNG described below.
Zero uses X1 ROM art by default; `--x3-rom` selects X3 art. Local players 3 and 4 have independent control state,
sprite pages and palettes. Sigma and Vile
cycle ROM poses while moving; other bosses use their idle pose for unmapped
actions. Wolf Sigma's large face is reduced to a player-sized sprite, and
Rangda Bangda uses its ROM sprite component. `--dynamic-lighting` adds a
per-pixel multiply light pass; `--charge-effects` adds X/Zero charge glow and particles. Both are
display-only options. `--bone-sheets DIR` loads optional 60 FPS Bone Animation
Editor `bae-sheet` JSON and PNG pages from `DIR/<character-id>/` for the
selected characters. Missing action clips use the sheet's idle clip, and a
missing sheet for a ROM character keeps its ROM art. Bass and Proto Man use
user-supplied sheets and require them in the pack. Four-player HP bars appear
vertically on the left and right screen edges. Online co-op currently keeps
player 2 as Zero.
Vile uses the private on-foot crop at
`roms/derived/bone_sheets/user_sources/vile_idle.png` even without
`--bone-sheets`; `--vile-sprite PNG` overrides that path. A selected Vile
cannot start if the crop is missing, so the ROM's incorrect large boss sprite
is never shown as a substitute.

For a repeatable recording, pass `--demo-bots --p1-character x
--p2-character zero --p3-character sigma --p4-character vile`. Demo mode
enables four local players, taps through the title into Highway, then drives
their movement, jumps, and shots. `--dynamic-lighting --charge-effects` can
be added to show both display effects. The same bot inputs work with
`--headless --frames N` for capture checks.
For uninterrupted showcase recording, demo mode restores a living player's
health when it falls to 5 or less. Downed players use the usual co-op respawn.
`--showcase-characters --bone-sheets DIR` also enables the four-player demo
and cycles the complete character roster for recording.

| action | P1 keys | P2 keys (`--coop-local` only) | pad |
|---|---|---|---|
| d-pad | arrows | W A S D | d-pad / left stick |
| B jump / Y shoot / A dash / X | Z / X / C / V | G / F / H / T | South / West / East / North |
| L / R | Q / E | R / Y | LB/LT / RB/RT |
| Start / Select | Enter / Shift | P / O | Start / Select |

P1 L/R are Q/E; pause is F6. The P1, P2 and hotkey tables never share a key.

Online co-op (rollback or lockstep over WebRTC, see `docs/netplay.md` "Frontend wiring"):

```
mmxrs --rom mmx.sfc (--coop-host ROOM | --coop-join ROOM)
      [--signal ws://HOST:3536] [--ice none|'stun:.. turn:.. username=U credential=C']
      [--mode rollback|lockstep] [--delay N]
```

Both players pass the same `--mode` (default rollback; a mismatch refuses to
start). `--delay`: rollback 0-3 (default 2), lockstep 0-8 (default 3). On a
same-region link `--mode lockstep --delay 3` plays smoothest: the remote player
never snaps, at the cost of short stalls when a packet is lost.

Host is player 1, guest player 2; both play with the P1 keys / first pad.
Needs `mmx-signal` running (`cargo run --release -p mmx-signal`). The title
shows the mode and connect stage, then `f= d= rtt= rollbacks= pred=` (rollback)
or `f= d= rtt=` plus `waiting for peer` while stalled (lockstep), or why it closed.
Pause, fast-forward, frame-step and save states are off during a session; Esc
sends a goodbye. The full SNES pad travels. Cargo feature `netplay` (default) carries the
WebRTC transport; `--no-default-features` builds without it.

Hotkeys: Tab hold fast-forward (8 frames per present, muted), F5 save state,
F7 load state, 1-9 pick slot, F11 fullscreen, F10 next widescreen setting (off, 16:10, 16:9, 21:9, auto; also online: display only), P or F6 pause, `.` single-step
while paused, Esc quit. States live in `$HOME/.mmxrs/slotN.state` (format and
bug reports: [savestates.md](savestates.md)).
Title bar: fps, `[coop]` or `[NET ...]`, `[SLOT n]`, `[PAUSED]`, `[FF]`.

## Window and scaling

The picture is scaled to the largest size that fits the window at the
frame's aspect, fractional scales included, centred with black bars, on
resize, maximize and fullscreen alike (`src/video.rs`, drawn through
`pixels`' `render_with` with our own pass instead of its integer-only
scaling renderer). Fractional nearest-neighbour shimmers (columns 7 or 8 px
wide at 7.14x), so the shader is "sharp bilinear": the same as a
nearest-neighbour pre-scale by `ceil(scale)` then a linear resize to the fit
size, in one pass; texels stay flat with a blended seam under one screen
pixel wide. At an exact integer scale it is plain nearest-neighbour.

`--widescreen auto` picks the margin from the window's aspect on every resize
and stretches away the last few percent so the window is filled edge to edge
(docs/widescreen.md "Presets"). An explicit preset keeps its aspect and is
letterboxed. F10 to a preset reshapes a plain window to the preset's aspect
(not a maximized or fullscreen one).

`MMX_VIDEO_DEBUG=1` logs each layout to stderr:

```
video: start: window 3440x1440 (scale factor 1), widescreen auto margin 17 tiles, frame 528x224, dest 3440x1440 at (0,0), scale 6.515x6.429 (fills window)
video: frame now 528x224, dest 3440x1440 at (0,0)
```

## Gamepads

The pad that last pressed a button is P1; with `--coop-local` a different pad
that presses becomes P2. Each connect/disconnect is logged to stderr with the
pad's name and source.

Two sources:

- **gilrs** (all desktops), standard mapping.
- **Apple GameController** (macOS only, `src/gc_pad.rs`, ported from z2rs).
  gilrs's IOKit backend misses some pads (a Switch Pro Controller over
  Bluetooth) or lists them but reads no input (Apple-driven Xbox pads);
  GameController sees Switch Pro, DualShock 4, DualSense, Xbox and MFi pads.
  Controllers are polled every frame (the extended-gamepad profile: face
  buttons, shoulders, triggers, menu/options, d-pad, left stick with a 0.5
  dead zone). Dedupe rule: while GameController lists at least one pad, gilrs
  input is ignored (it would be the same pads, often seen twice); gilrs only
  drives pads when GameController has none.

The layout is positional, the same on every pad: bottom face button = SNES B
(jump), left = Y (buster), right = A (dash), top = X. GameController's
`buttonA` is the bottom button on every pad, so Nintendo's printed labels come
out swapped against Xbox's (Switch B is jump, Switch A is dash), which is what
a SNES player expects since the Switch layout is the SNES one.

| SNES | Xbox | PlayStation | Switch Pro | gilrs | GameController |
|---|---|---|---|---|---|
| B (jump) | A | Cross | B | South | buttonA |
| Y (buster) | X | Square | Y | West | buttonX |
| A (dash) | B | Circle | A | East | buttonB |
| X | Y | Triangle | X | North | buttonY |
| L | LB / LT | L1 / L2 | L / ZL | LeftTrigger / LeftTrigger2 | leftShoulder / leftTrigger |
| R | RB / RT | R1 / R2 | R / ZR | RightTrigger / RightTrigger2 | rightShoulder / rightTrigger |
| Start | Menu | Options | + | Start | buttonMenu |
| Select | View | Create / Share | - | Select | buttonOptions |
| d-pad | d-pad / left stick | same | same | DPad* / LeftStick | dpad / leftThumbstick |

Pairing on macOS: System Settings > Bluetooth, put the pad in pairing mode
(Switch Pro: hold the sync button on top; DualShock/DualSense: Share/Create +
PS; Xbox: the pair button) and click Connect. If macOS's System Settings >
Game Controllers lets you remap a pad's buttons, that remap applies to
GameController pads here too.

Diagnostic: `--pads` (or `MMX_PAD_DEBUG=1`) prints connected pads and one line
per pad whenever its SNES bits change, e.g.

```
gamepad connected (gilrs): Nintendo Switch Pro Controller
gamepad connected (GameController): Pro Controller (Switch Pro Controller)
pad P1 Pro Controller (Switch Pro Controller) [GC]: 0x8000 B
```

(`[gilrs, ignored: GC active]` marks a gilrs pad under the dedupe rule.) With
`--pads` GameController also delivers input while another app, such as the
terminal, is frontmost.

## Headless

Headless (no window, no audio device):

```
mmxrs --headless --frames 600 [--rom mmx.sfc] [--dump-at 300=a.png,600=b.png]
      [--hold 0x1000] [--p2-hold MASK] [--coop-local]
```

Holds the pad masks (SNES auto-read layout, `0x8000` B ... `0x0010` R; hex or
decimal) every frame, writes RGB PNGs of `Game::frame()` after the listed
frames, and prints `frames N wram_sha256 <hex>`.

`--movie F.bk2|F.lsmv|F.smv` replays recorded inputs in both desktop and
headless runners. Movies must start from power-on and contain no resets.
Playback passes opposing directions through and defaults to CPU overclock
off. Wide gameplay, netplay, VS and scripted bots cannot be combined with
movies. Match the movie's ROM revision and region for reliable replay.

For experimental four-player replay, add `--movie-coop-delay 8` and select
the characters. X follows P1's original track; the other players copy it
8, 16 and 24 video frames later during gameplay. Menu and pause buttons stay
on P1. Followers release controls outside gameplay and at movie end. This
uses real co-op physics: damage, enemy interactions, spawn positions and
extra CPU load can make a single-player movie diverge.

`--movie-smart` (`--movie-route` is an alias) enables the Highway replay harness. It
precomputes the movie's first-stage solo route, then steers the live players
toward delayed waypoints and retries jumps when they get stuck. Knockback
freezes the affected player's route cursor; after recovery, the harness
reacquires a nearby normal waypoint and resets jump and attack timing. Bots
also aim at live enemies rather than assuming the movie has killed them. It uses
ordinary controller inputs, so collisions and damage remain live. It stops
at the end of the first stage; later stages are not supported by this mode.
The default spacing is 8, 16 and 24 source frames. During movie co-op, Zero
and Sigma replace buster fire with saber swings. Vile fires one maximum-charge X
blast from his shoulder, with at least 120 video frames between shots, with its native animation tinted yellow and its
full-charge collision, speed and damage. Weapon collision uses logical game
pixels even when the sprites are rendered at HD scale.
Save/load hotkeys are disabled while replaying because game states do not
include the playback cursor.

Example with [Hetfield90's tool-assisted input movie](https://tasvideos.org/3197M)
(download the emulator replay from that page into the ignored `movies/` directory):

```sh
cargo run -p mmx-native --release -- \
  --rom "roms/Mega Man X (USA) (Rev 1).sfc" \
  --movie movies/hetfield90v3-megamanx-100p.bk2 --movie-smart --demo-survival \
  --p1-character x --p2-character zero --p3-character sigma --p4-character vile \
  --bone-sheets roms/derived/bone_sheets --widescreen 16:9 \
  --dynamic-lighting --charge-effects
```

The movie was authored on USA v1.0. The local Rev 1 dump can provide the
Highway route but should not be treated as a fully synchronized TAS replay.
Credit the original player and identify footage driven by their inputs as
tool-assisted when sharing it.

The desktop and headless renderers composite health, weapon and boss meters
after dynamic lighting and charge particles. UI remains at full brightness
at both the native and HD render scales.

## Timing (PAL dumps)

The emulator follows the cartridge header's region byte: a European dump runs
at 50.007 fps like a PAL console, with slower music. `--timing ntsc` forces
60.0988 fps (the code is the same as USA v1.0); `--timing pal` forces 50. The
choice is part of the netplay handshake, so both players must use the same.
`cargo xtask rominfo` prints the region.

For assisted footage, `--demo-survival` restores low health on living players
and shortens downed followers' ordinary revive wait to 30 frames. Pit deaths
still use the ROM and co-op respawn logic. Scripted scenes and versus mode
are not changed by the assist. Omit this flag for normal damage and revive timing.
