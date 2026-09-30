# mmx-native (binary `mmxrs`)

Run: `cargo run -p mmx-native --release -- --rom mmx.sfc [--x3-rom mmx3.sfc [--zero]] [--scale 1-8] [--fullscreen] [--coop-local]`
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
F7 load state, 1-9 pick slot, F11 fullscreen, P or F6 pause, `.` single-step
while paused, Esc quit. States live in `$HOME/.mmxrs/slotN.state`.
Title bar: fps, `[coop]` or `[NET ...]`, `[SLOT n]`, `[PAUSED]`, `[FF]`.

Gamepads (gilrs, standard mapping): the pad that last pressed a button is P1;
with `--coop-local` a different pad that presses becomes P2.

Headless (no window, no audio device):

```
mmxrs --headless --frames 600 [--rom mmx.sfc] [--dump-at 300=a.png,600=b.png]
      [--hold 0x1000] [--p2-hold MASK] [--coop-local]
```

Holds the pad masks (SNES auto-read layout, `0x8000` B ... `0x0010` R; hex or
decimal) every frame, writes RGB PNGs of `Game::frame()` after the listed
frames, and prints `frames N wram_sha256 <hex>`. `--movie F.bk2` is parsed but
rejected until `mmx-verify`'s bk2 parser exists (TODO).
