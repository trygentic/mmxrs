# Web frontend (`crates/mmx-web`)

A wasm-bindgen `Emu` over `mmx_core::Game` plus a static page in
`crates/mmx-web/site/` (plain ES modules, no bundler). The ROM is read
locally from the file picker or a drop; nothing is uploaded. A second picker,
"Mega Man X3 ROM (optional, enables Zero's sprites and saber)", calls
`Emu::set_x3_rom(bytes)` (kept for later X ROM loads): P2 is drawn as X3
Zero and P2's X (key T) swings the saber, see docs/zero.md "Playing as Zero".
Load it before Host/Join; both peers need it. The checkbox "Play as Zero
(X3 ROM)" (`Emu::set_p1_zero`) makes player 1 Zero too (key V swings the
saber); it unchecks itself while no X3 ROM is loaded. Online both peers must
tick it (it is part of the handshake id).

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
empty = default STUN), mode (rollback / lockstep, both players the same),
input delay (auto = 2 for rollback, 3 for lockstep; rollback 0-3, lockstep
0-8), *Host* (player 1) / *Join* (player 2) / *Leave*.
`?signal=&room=&ice=&mode=&delay=` prefill it (`?mode=lockstep&delay=3` for a
same-region playtest). Each peer plays with the P1 keys; F5/F7/Tab are off
during a session; the full SNES pad travels.
Details: `docs/netplay.md` "Frontend wiring".

## Widescreen

The *Screen* select (4:3, 16:10, 16:9, 21:9) or `?widescreen=16:9` in the
URL: `Emu::set_widescreen(name)`, display only, so it can change during an
online session and differ between the peers. The canvas takes the frame's
size and aspect (square pixels, 3x, capped by the window). docs/widescreen.md.

The *spawn enemies at the wide edges* checkbox (or `?widegame=1`):
`Emu::set_wide_gameplay(on)`, docs/widescreen.md "Wide gameplay". It changes
the game: set it before connecting (it is fixed while a session exists) and
both peers need the same, or the handshake refuses. Ticked mid-stage, the
margins' columns are spawned at once.

## Keys (`KeyboardEvent.code`, table in `src/lib.rs`)

| | D-pad | B | Y | A | X | L | R | Start | Select |
|---|---|---|---|---|---|---|---|---|---|
| P1 | arrows | Z | X | C | V | Q | E | Enter | Shift |
| P2 (co-op) | WASD | G | F | H | T | R | Y | P | O |

P1 L/R are Q/E. F5 saves and F7 loads one
in-memory slot (gone on reload). Hold Tab to fast-forward (audio muted).
F2 (or `?fps=1`) shows fps and % of full speed over the picture. Until the
browser lets audio start (first click or key press), a badge over the
picture says "Click or press a key to enable sound".

## Timing and audio

The worklet (`site/worklet.js`) holds a stereo ring; the page steps frames
until ~50 ms is queued (max 4 per rAF), and the worklet drops the oldest audio
past 90 ms. Online, `net_tick_audio` is paced the same way (one frame of audio
per tick, silence when the session held the frame back). Without audio (context
suspended until a key press) it falls back to a wall-clock accumulator at
`emu.fps()` (60.0988 NTSC, 50.007 PAL).

## Automation: `window.mmx`

`mmx.pause(true)` stops the loop; `mmx.step(n, p1, p2)` runs n frames and
returns the frame count; `mmx.save()` / `mmx.load(bytes?)`; `mmx.frame()` gives
`{width, height, rgba}`; `mmx.wram()`, `mmx.info()`, `mmx.emu` (the raw `Emu`),
`mmx.loadRomBytes(u8array)`. Pad bits follow `mmx_core::Pads`.
