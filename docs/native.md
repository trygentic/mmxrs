# mmx-native (binary `mmxrs`)

Run: `cargo run -p mmx-native --release -- --rom mmx.sfc [--scale 1-8] [--fullscreen] [--coop-local]`
(`--rom` falls back to `$MMX_ROM`; default scale 3). Pacing follows the audio
device (~50 ms buffered); without an output device a 60.0988 Hz timer drives it.

| action | P1 keys | P2 keys (`--coop-local` only) | pad |
|---|---|---|---|
| d-pad | arrows | W A S D | d-pad / left stick |
| B jump / Y shoot / A dash / X | Z / X / C / V | G / F / H / T | South / West / East / North |
| L / R | Q / E | R / Y | LB/LT / RB/RT |
| Start / Select | Enter / Shift | P / O | Start / Select |

P1 L/R are Q/E; pause is F6. The P1, P2 and hotkey tables never share a key.

Hotkeys: Tab hold fast-forward (8 frames per present, muted), F5 save state,
F7 load state, 1-9 pick slot, F11 fullscreen, P or F6 pause, `.` single-step
while paused, Esc quit. States live in `$HOME/.mmxrs/slotN.state`.
Title bar: fps, `[coop]`, `[SLOT n]`, `[PAUSED]`, `[FF]`.

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
