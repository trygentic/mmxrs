# mmx-native (binary `mmxrs`)

Run: `cargo run -p mmx-native --release -- --rom mmx.sfc [--x3-rom mmx3.sfc [--zero]] [--scale 1-8] [--fullscreen] [--coop-local] [--pads] [--widescreen off|16:10|16:9|21:9|N] [--wide-gameplay] [--overclock off|1.5|2]` (docs/widescreen.md;
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
F7 load state, 1-9 pick slot, F11 fullscreen, F10 next widescreen preset (also online: display only), P or F6 pause, `.` single-step
while paused, Esc quit. States live in `$HOME/.mmxrs/slotN.state` (format and
bug reports: [savestates.md](savestates.md)).
Title bar: fps, `[coop]` or `[NET ...]`, `[SLOT n]`, `[PAUSED]`, `[FF]`.

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
frames, and prints `frames N wram_sha256 <hex>`. `--movie F.bk2` is parsed but
rejected until `mmx-verify`'s bk2 parser exists (TODO).

## Timing (PAL dumps)

The emulator follows the cartridge header's region byte: a European dump runs
at 50.007 fps like a PAL console, with slower music. `--timing ntsc` forces
60.0988 fps (the code is the same as USA v1.0); `--timing pal` forces 50. The
choice is part of the netplay handshake, so both players must use the same.
`cargo xtask rominfo` prints the region.
