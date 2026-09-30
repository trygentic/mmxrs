# Web frontend (`crates/mmx-web`)

A wasm-bindgen `Emu` over `mmx_core::Game` plus a static page in
`crates/mmx-web/site/` (plain ES modules, no bundler). The ROM is read
locally from the file picker or a drop; nothing is uploaded.

## Build and serve

    scripts/web-build.sh      # wasm-pack --target web -> site/pkg/ (or cargo + wasm-bindgen CLI)
    scripts/web-serve.sh      # python3 -m http.server 8000 from site/
    open http://localhost:8000/          # ?coop=1 starts with local co-op on

Checks: `cargo test -p mmx-web`, `cargo check -p mmx-web --target wasm32-unknown-unknown`.

## Keys (`KeyboardEvent.code`, table in `src/lib.rs`)

| | D-pad | B | Y | A | X | L | R | Start | Select |
|---|---|---|---|---|---|---|---|---|---|
| P1 | arrows | Z | X | C | V | A or B | S or N | Enter | Shift |
| P2 (co-op) | WASD | G | F | H | T | R | Y | P | O |

In co-op A/S belong to P2, so P1 uses B/N for L/R. F5 saves and F7 loads one
in-memory slot (gone on reload). Hold Tab to fast-forward (audio muted).

## Timing and audio

The worklet (`site/worklet.js`) holds a stereo ring; the page steps frames
until ~50 ms is queued (max 4 per rAF), and the worklet drops the oldest audio
past 90 ms. Without audio (context suspended until a key press) it falls back
to a 60.0988 Hz wall-clock accumulator.

## Automation: `window.mmx`

`mmx.pause(true)` stops the loop; `mmx.step(n, p1, p2)` runs n frames and
returns the frame count; `mmx.save()` / `mmx.load(bytes?)`; `mmx.frame()` gives
`{width, height, rgba}`; `mmx.wram()`, `mmx.info()`, `mmx.emu` (the raw `Emu`),
`mmx.loadRomBytes(u8array)`. Pad bits follow `mmx_core::Pads`.
