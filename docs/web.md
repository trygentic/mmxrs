# Web frontend (`crates/mmx-web`)

A wasm-bindgen `Emu` over `mmx_core::Game` plus a static page in
`crates/mmx-web/site/` (plain ES modules, no bundler). The ROM is read
locally from the file picker or a drop; nothing is uploaded. A second picker,
"Mega Man X3 ROM (optional, enables Zero's sprites and saber)", calls
`Emu::set_x3_rom(bytes)` (kept for later X ROM loads): P2 is drawn as X3
Zero and P2's X (key T) swings the saber, see docs/zero.md "Playing as Zero".
Load it before Host/Join; both peers need it.

## Build and serve

    scripts/web-build.sh      # wasm-pack --target web -> site/pkg/ (or cargo + wasm-bindgen CLI)
    scripts/web-serve.sh      # python3 -m http.server 8000 from site/
    open http://localhost:8000/          # ?coop=1 starts with local co-op on

Checks: `cargo test -p mmx-web`, `cargo check -p mmx-web --target wasm32-unknown-unknown`.

## Online co-op (feature `netplay`)

    wasm-pack build crates/mmx-web --target web --release --out-dir site/pkg -- --features netplay
    cargo check -p mmx-web --features netplay --target wasm32-unknown-unknown

The panel above the screen: signal URL (default `ws://127.0.0.1:3536`, run
`cargo run --release -p mmx-signal`), room, ICE (`none` on one machine / LAN,
empty = default STUN), input delay 0-3, *Host* (player 1) / *Join* (player 2) /
*Leave*. `?signal=&room=&ice=&delay=` prefill it. Each peer plays with the P1
keys; F5/F7/Tab are off during a session; only B/Y/A/Start/d-pad travel.
Details: `docs/netplay.md` "Frontend wiring".

## Keys (`KeyboardEvent.code`, table in `src/lib.rs`)

| | D-pad | B | Y | A | X | L | R | Start | Select |
|---|---|---|---|---|---|---|---|---|---|
| P1 | arrows | Z | X | C | V | Q | E | Enter | Shift |
| P2 (co-op) | WASD | G | F | H | T | R | Y | P | O |

P1 L/R are Q/E. F5 saves and F7 loads one
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
