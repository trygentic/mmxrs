# Netplay (two-player co-op)

mmxrs runs two-player co-op with a deterministic **rollback** session
([Rollback mode](#rollback-mode)), the default on the desktop app and in the
browser, or **input-delay lockstep**, which both frontends keep selectable.
Both peers run the same game from power-on,
feed it the same `(pad1, pad2)` for each frame, and compare state hashes
regularly. No late join, no spectators. Netplay is opt-in and never touches the verification path
(`xtask verify`, headless, movies).

Crates:

- `crates/mmx-net`: the protocol (`Session`, `RollbackSession`, wire format, `Transport` trait,
  deterministic `LoopbackTransport`). No dependencies by default; builds for
  host and `wasm32-unknown-unknown`. Feature `matchbox` adds `MatchboxTransport`
  (WebRTC via [matchbox](https://github.com/johanhelsing/matchbox) 0.14, with a
  small vendored patch, see [Why matchbox is vendored](#why-matchbox-is-vendored)).
  `mmx_net::IceConfig` parses the ICE server setting both frontends share.
- `tools/mmx-signal`: the signalling server (matchbox_signaling, FullMesh, two
  peers per room).

## Protocol

Every packet is `[WIRE_VERSION=1, tag, payload]`, little-endian, exact length
(trailing bytes are rejected). Largest packet: 8207 bytes (Start with WRAM).

| tag  | message | channel    | contents |
|------|---------|------------|----------|
| 0x01 | Hello   | reliable   | proto, role, rom_crc32, trapset_id, coop_flags, delay, hash_interval |
| 0x02 | Start   | reliable   | host to guest: frame 0, effective delay / coop_flags / hash_interval, host WRAM (0 or 8192 bytes) |
| 0x10 | Input   | unreliable (+reliable fallback) | ack, first frame, 0..32 pads |
| 0x20 | Hash    | reliable   | frame, FNV-1a 64 |
| 0x30/0x31 | Ping/Pong | unreliable | token (RTT) |
| 0x32 | Status  | unreliable | rollback only: frame, lead (time sync) |
| 0x40 | Bye     | reliable   | reason |

Handshake: when the link comes up both peers send Hello. Each side checks the
protocol version, ROM CRC32, trap-set id and roles (one host, one guest). Co-op
flags are host-authoritative and must be **non-zero** and supported by the guest
(the guest's Hello lists what it supports), so the guest always gets gameplay.
The host then sends Start (delay, flags, hash interval and its WRAM save), and
both sides emit `SessionEvent::Started`. Frontends rebuild a fresh game there,
apply the flags and copy the WRAM.

Lockstep rules:

- Host = player 1 (`pad1`), guest = player 2 (`pad2`), on both machines.
- Input delay `D` (0..=8, default 2, host's value wins). The pad latched while
  frame `f` is next belongs to frame `f + D`. Frames `0..D` use pad 0.
- Frame `f` is steppable only when both pads are known. `Session::poll` returns
  frames in order, exactly once.
- Each update sends every local frame the peer has not acknowledged (oldest
  first, up to 32) on the unreliable channel. If acknowledgements stall for
  200 ms, or a stall starts, a copy also goes over the reliable channel (at most
  every 200 ms). A two-way packet blackout therefore recovers on its own.
- Waiting 250 ms for a remote pad emits `Stalled`; the next stepped frame emits
  `Resumed`. Waiting 60 s (configurable) closes with `Timeout`. A handshake
  that does not finish within 30 s also times out. Waiting in the room for a
  peer never times out.
- Every 60 frames each peer hashes caller-chosen state (`hash_state(&[ram,
  wram, oam])`). Different hashes end the session as `Desync` on both sides.

Time only enters as the caller's `now_ms`. It drives timers and RTT, never
inputs. There are no floats, no wall clock and no hash-map iteration in the
protocol.

## Rollback mode

`RollbackSession` is a second, selectable netplay mode in `mmx-net`. Lockstep
waits for both pads before it steps a frame. Rollback applies the local pad at
once, guesses the remote pad, and corrects the guess when the real pad arrives.
Both sessions use the same transports, wire envelope and handshake. A rollback
peer's Hello carries protocol `0x0101`, so a lockstep peer and a rollback peer
refuse each other with `Mismatch(NetMode)` and never start.

mmxrs does not use [ggrs](https://github.com/gschup/ggrs). matchbox 0.14's
`ggrs` feature implements the socket trait of ggrs **0.11**, while the current
release is 0.13 (the traits don't unify, so the build fails). ggrs also reads
`Instant::now()` and `rand::random` inside its protocol and uses `HashMap` for
per-frame state. That rules out deterministic loopback tests driven by a caller
clock, and it would add bincode, serde, rand, parking_lot and the unmaintained
`instant` crate to a crate that has no dependencies today. Rollback here is
std-only code (about 1,300 lines with docs) built on the lockstep session's input log,
batching and handshake.

### How it works

- Host = player 1, guest = player 2. Frames count from 0 after the handshake
  (same Hello/Start as lockstep, with host-authoritative delay, co-op flags,
  checksum interval and WRAM).
- **Input delay** `D` (0..=3, default **2**, host's value wins). The pad added
  while frame `f` is next applies to frame `f + D`. With `D = 0` it applies to
  the frame simulated on the same tick.
- **Prediction.** A remote pad not yet received is assumed to equal the last
  remote pad received in order. The session remembers every prediction it
  used.
- **Rollback.** When a real remote pad differs from the prediction used for
  frame `r`, the next `advance` returns `LoadState { r }`, then
  `SaveState`/`Advance { replay: true }` pairs for `r..now`, then the normal
  `SaveState { now }` + `Advance { now }`. The caller owns the saved states
  (`StateRing` with `max_prediction + 2` slots is enough), so `mmx-net` never
  depends on `mmx-core`.
- **Prediction window** (default 8 frames, 0..=30). The session never runs more
  than this many frames past the last confirmed remote pad. When the window is
  full, `advance` returns no new `Advance` (a stall) instead of predicting
  further. With a window of 0 it behaves like lockstep.
- **Desync detection on confirmed frames only.** A state at frame `f` is final
  once every remote pad before `f` is known and any rollback that covered it
  has been handed to the caller. Only then, and every `check_interval` frames
  (default 60), does the session send the checksum the caller reported for
  that save (`report_checksum`, where the latest save wins). A mismatch emits
  `DesyncDetected` and closes both peers with `Desync`.
- **Time sync.** Every 100 ms each peer sends `Status { frame, lead }`. The
  receiver estimates the remote frame as `frame + RTT/2` (RTT smoothed), and
  its own lead as its frame minus that estimate. The two peers' leads are
  averaged over 16 samples. If half the difference is 2 frames or more, the
  side that is ahead gets `WaitRecommendation { skip_frames }` (at most 8, at
  most once per 60 frames), and it should skip that many ticks.
- **Disconnects.** Silence for 500 ms emits `NetworkInterrupted`, and the next
  packet emits `NetworkResumed`. Silence past `disconnect_timeout_ms` (60 s by
  default) closes with `Timeout`. Peer left, quit, room full, transport errors
  and handshake mismatches close with the same `CloseReason`s as lockstep.
- **Sync test.** `SyncTestSession::new(d)` rolls back `d` frames on every frame
  and compares each re-saved state's checksum with the first one. A
  non-deterministic save/load or step shows up as `DesyncDetected` locally,
  with no peer involved.

Time only enters through `now_ms`, and all ordered state uses `BTreeMap` or
`VecDeque`.

### What it hides, and its limits

- Lockstep with delay `D` feels `D` frames (`D x 16.7 ms`) late for both
  players on every input, and it stalls whenever a pad is late. With rollback
  at delay `D` and window `W`, the local input lands after exactly `D` frames.
  Remote pads up to about `(W + D) x 16.7 ms` of one-way latency plus jitter
  are absorbed by prediction, which is about 165 ms at the defaults. Beyond
  that the session stalls like lockstep.
- A misprediction costs a re-simulation of up to `W` frames in one tick, so
  the emulator must step `W + 1` frames within a 16 ms frame budget at worst.
  Raise `D` on slow machines: each frame of delay removes one frame of
  rollback depth.
- The remote player can visibly "snap" after a correction: their X jumps to
  where the real input put him. Repeat-last prediction is right while a pad is
  held; it is wrong mainly at presses and releases, and Mega Man X is fast, so
  expect more corrections than a slow game would see.
- **Audio.** Replayed frames must not be heard again: drop the samples of
  `Advance { replay: true }` frames. Sound a frame produced from a wrong
  prediction was already played and cannot be taken back, so a mispredicted
  jump or buster sound may play and then not "happen". Presenting audio with
  `D >= 1` keeps this rare.
- **Video.** Render only after all requests of a tick ran. Replayed frames are
  never presented.
- Save states must capture everything that affects later frames (CPU, PPU,
  APU, mapper, RAM, co-op state). Run `SyncTestSession` against the real game
  before enabling rollback in a frontend.
- Pausing, fast-forward, save states and movies stay disabled during a
  session, exactly as with lockstep.

### Driving a rollback session

```rust
let mut s = RollbackSession::new(RollbackConfig::host(rom_crc32, trapset_id, flags), transport)?;
let mut ring = StateRing::new(usize::from(cfg.max_prediction) + 2);
let mut skip = 0;

// every frontend tick (native: winit loop at 60 Hz; web: requestAnimationFrame)
if skip > 0 {
    skip -= 1;
    s.poll(now_ms);                        // network only
} else {
    s.add_local_input(local_pad);          // ignored while stalled
    match s.advance(now_ms) {
        Ok(requests) => for r in requests {
            match r {
                SaveState { frame } => {
                    if s.needs_checksum(frame) { s.report_checksum(frame, game.state_hash()); }
                    ring.save(frame, game.save_state());
                }
                LoadState { frame } => game.load_state(ring.get(frame).expect("held")),
                Advance { pad1, pad2, replay, .. } => {
                    game.set_audio_muted(replay);   // drop samples of replayed frames
                    game.step2(pad1, pad2);
                }
            }
        },
        Err(RollbackError::Closed(reason)) => { /* show reason, leave netplay */ }
        Err(e) => { /* MissingLocalInput: a frontend bug */ }
    }
    present(game.framebuffer());           // once per tick, after all requests
}
for ev in s.events() {
    match ev {
        Running { coop_flags, wram, .. } => { /* rebuild game from power-on, apply flags + WRAM */ }
        WaitRecommendation { skip_frames } => skip = skip_frames,
        NetworkInterrupted { .. } | NetworkResumed => { /* status line */ }
        DesyncDetected { .. } | Disconnected { .. } => { /* show reason */ }
        Synchronizing { .. } => {}
    }
}
```

`mmx_net::apply_requests(game, ring, requests, session)` runs the same loop for
any `RollbackGame` and is what the tests use. `stats()` reports RTT,
prediction depth, rollbacks and replayed frames, stalls, leads and checksum
matches.

## Driving a session

```rust
// once
let url = mmx_net::room_url("ws://127.0.0.1:3536", "myroom")?;       // ws://127.0.0.1:3536/mmx-myroom
let mut t = mmx_net::MatchboxTransport::connect(&url, ice);          // never blocks
let mut s = mmx_net::Session::new(SessionConfig::host(rom_crc32, trapset_id, flags))?;

// every frontend tick
s.update(&mut t, now_ms);
for ev in s.take_events() { /* Started -> rebuild game; Stalled -> mute; Closed(r) -> show r */ }
for _ in 0..frames_due {
    s.latch_local(local_pad);
    let Some((frame, pad1, pad2)) = s.poll() else { break };
    game.step2(pad1, pad2);
    if s.needs_hash(frame) { s.report_hash(frame, mmx_net::hash_state(&[ram, wram, oam])); }
}
s.update(&mut t, now_ms); // flush the freshly latched input

// quitting
s.close(); s.update(&mut t, now_ms); // then keep calling update for ~200 ms before dropping
```

Native frontends call this from the winit loop; `MatchboxTransport` runs its
message loop on its own thread. On the web, call it from the
`requestAnimationFrame` tick; the loop runs through `spawn_local`.

## Frontend wiring

TODO: desktop (`mmx-native`) and browser (`mmx-web`) netplay
wiring, their flags/panel, the rollback tick with `StateRing` over the real
game, status lines, and the browser e2e harness. z2rs's `docs/netplay.md`
describes its versions of these (`--coop-host`, `--coop-join`, `--net-mode`,
`--net-delay`, `--ice`, `make netplay-e2e*`, `make netplay-play`) and is the
model to follow.

## Connecting: stages, ICE servers and timeouts

Connecting goes through four stages. Both frontends show them: the web status
line as `1/4 connecting to the signal server… 0.3 s`, the desktop window title
as `[NET host: 2/4 waiting for the other player in myroom]` (each stage is also
printed to stderr once). `Transport::stage()` reports them
(`mmx_net::ConnectStage`).

| stage | what is happening | ends when | deadline |
|---|---|---|---|
| 1/4 connecting to the signal server | websocket to mmx-signal, joining the room | the server admits us | 10 s (`SIGNALLING_TIMEOUT_MS`); a refused port fails after ~3 s |
| 2/4 waiting for the other player | alone in the room | the other peer joins | none: wait as long as you like, *Leave* / `Esc` cancels |
| 3/4 establishing the peer link | WebRTC offer/answer, ICE checks, data channels | both data channels are open | 15 s (`LINK_TIMEOUT_MS`) |
| 4/4 connected, starting the session | Hello / Start handshake | `Started` | 30 s (`DEFAULT_HANDSHAKE_TIMEOUT_MS`) |

A missed deadline ends the attempt with a reason that says what to check, for
example *the other player is in the room but the peer link did not open within
15 s; on one machine or LAN set ICE servers to 'none' and check the firewall,
across the internet make sure UDP is not blocked or add a TURN server*. On the
web the panel then returns to idle, so *Host* / *Join* can be pressed again.

**The game keeps running while connecting**, on both frontends: until the
session starts, the page and the desktop app step the local game as usual.
Nothing played there reaches the session, because `Started` restarts both
peers from power-on. *Leave* (web) or `Esc` (desktop) cancels cleanly at any
stage and frees the place in the room at once.

Two browser windows on one machine connect in about 1-2 s with the default
setting; with ICE servers set to `none` it is faster still.

### ICE servers

The same text form works in the web *ICE* field, `--ice` and `netplay.ice_url`:

```text
stun:stun.l.google.com:19302 stun:stun1.l.google.com:19302      (default)
none                                                             (no ICE servers)
stun:stun.example.org:3478 turn:turn.example.org:3478 username=alice credential=secret
```

URLs are separated by spaces or commas. `username=` / `credential=` apply to
the `turn:` URLs; on the desktop, `netplay.ice_username` /
`netplay.ice_credential` in the config fill them in too, so the secret does not
have to be on the command line. Credentials are never logged.

- **Same machine or same LAN**: `none` is enough and connects fastest. The
  peers reach each other through their local (host) candidates; no traffic
  leaves the network.
- **Internet, typical home routers**: the default STUN servers. STUN only tells
  each peer its public address; the game traffic still goes directly between
  the two peers.
- **Internet, strict NATs**: when either side is behind a symmetric NAT or
  carrier-grade NAT (common on mobile networks), or a firewall that blocks
  outbound UDP, a direct path cannot be found and you need a **TURN** server,
  which relays the traffic. Symptom: stage 3/4 runs into its 15 s deadline even
  though both players are in the room. No public TURN server is bundled;
  [coturn](https://github.com/coturn/coturn) is the usual self-hosted choice.
  Both peers should list the same TURN server.

### Troubleshooting

| what you see | likely cause | what to do |
|---|---|---|
| stuck at 1/4, then *could not connect to the signal server* | mmx-signal not running, wrong host/port, firewall | start `cargo run --release -p mmx-signal`; check the URL is `ws://HOST:3536`; open the port |
| *this page is https, so the signal URL must be wss://* | mixed content | put mmx-signal behind a TLS proxy and use `wss://` |
| stays at 2/4 | the other player is not in the **same room on the same server** | compare the signal URL and room name on both sides |
| 3/4 times out on one machine or LAN | local firewall blocking UDP, or a VPN interface | try ICE `none`; allow the browser / native app through the firewall |
| 3/4 times out across the internet | symmetric NAT / CGNAT / UDP blocked | add a TURN server on both sides |
| *room full* (desktop) / closes at once (web) | two players already in that room | pick another room name |
| *mismatch: Rom* or *TrapSet* | different ROM dump or different mmxrs build | use the same ROM and the same version on both sides |
| *peer paused* during play | the other browser tab is in the background | bring it to the front; the session survives 60 s |

### Why matchbox is vendored

`vendor/matchbox_socket` is matchbox_socket 0.14.0 with three small patches,
each marked `mmxrs patch (from z2rs)`, applied through `[patch.crates-io]` in the root
`Cargo.toml`:

1. **Browser: bounded ICE gathering.** Upstream's browser transport sends its
   offer (and answer) only after ICE gathering has *completed*, and a browser
   completes gathering only once every configured STUN/TURN server has answered
   or timed out. This was the cause of "online co-op in the browser never
   connects and the game freezes": in a reproduction with two headless Chromium
   windows on one machine and the default Google STUN pair, the host created
   its offer 0.85 s after the click and had its host and server-reflexive
   candidates 60 ms later, but `stun.l.google.com` never answered, so
   `iceGatheringState` stayed `gathering` until the binding request timed out
   (`icecandidateerror 701`) at **40.7 s**. Only then was the offer sent; the
   answer came back 80 ms later and both data channels opened 100 ms after
   that. Meanwhile both pages reported `idle`, and the page stopped stepping
   the game as soon as the session existed. The patch waits at most **1 s**,
   sends the description with the candidates gathered so far, and trickles any
   later candidates through the signalling server (buffered until the
   existing trickle handler takes over, so none is lost). Native (webrtc-rs)
   already trickled and is unchanged.
2. **Browser + native: an empty ICE URL list means no ICE servers** (upstream
   always creates one server entry, which cannot express "none").
3. **The default signaller builder is exported**, so mmx-net can wrap it to
   observe signalling events for the connect stages above.

The alternatives were weaker: only changing the default ICE servers still
leaves any unresponsive STUN or TURN server able to stall a connection for
tens of seconds, and a JavaScript shim around `RTCPeerConnection` would patch
the same behaviour less visibly. Drop the vendored copy once upstream matchbox
bounds its gathering wait (or trickles in the browser) and exposes its default
signaller.

## Running the signalling server

```sh
cargo run --release -p mmx-signal -- --bind 0.0.0.0:3536
```

- Room URL: `ws://HOST:3536/mmx-<room>`, where the room name is 1-32 characters
  of `A-Z a-z 0-9 _ -`. The URL path is the room. A room admits two peers; a
  third connection gets HTTP 409, which native clients report as
  `LinkState::RoomFull`. Browsers hide the status code, so a refused browser
  peer sees a generic `Closed`.
- Logs go to stderr, one line per admit, reject, connect and disconnect.
  `--verbose` adds HTTP tracing.
- **No TLS.** Browsers on an https page require `wss://`, so put mmx-signal
  behind a TLS-terminating reverse proxy (caddy, nginx). Proxies that close idle
  websockets are fine: matchbox sends a keep-alive every 10 s. A page served
  over https that is pointed at a plain `ws://` URL fails as *mixed content*:
  the browser blocks the socket before mmxrs sees it, so the status line just
  says the session closed and the console carries the real reason. Serving the
  page over plain http (e.g. on localhost) works with `ws://`.
- The server is unauthenticated. Anyone who knows a room name can take the free
  slot, so use non-guessable room names.

## NAT, TURN and privacy

- Connectivity uses Google STUN by default (`mmx_net::DEFAULT_ICE_URLS`). Most
  home NATs work. Symmetric NAT and CGNAT pairs need a TURN server; see
  [ICE servers](#ice-servers). No public TURN server is bundled.
- WebRTC reveals each peer's public IP address to the other peer and to the
  STUN/TURN server. With ICE servers set to `none`, only local addresses are
  exchanged (browsers additionally hide them behind mDNS names).
- The browser client waits at most 1 s for ICE gathering before sending its
  offer or answer, then trickles the rest (see
  [Why matchbox is vendored](#why-matchbox-is-vendored)).

## Building

- Protocol only: `cargo test -p mmx-net`.
- Transport: `cargo build -p mmx-net --features matchbox`. On native this pulls
  webrtc-rs and `ring`, which needs a C compiler. The wasm32 build
  (`--target wasm32-unknown-unknown`) needs no C toolchain.
- `cargo test -p mmx-net --features matchbox` also runs a localhost end-to-end
  test: it starts mmx-signal in-process, connects two peers for 120 frames, and
  checks that a third peer is refused. It also checks the connect stages arrive
  in order with ICE servers set to `none` (and connect in under 5 s), and that
  an unreachable signal server ends with a reason naming mmx-signal.
- `cargo run --release -p mmx-net --features matchbox --example net_peer -- --room R --trapset HEX`
  runs a ROM-free native host for connection tests.
