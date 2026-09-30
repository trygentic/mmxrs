# Netplay (two-player co-op)

mmxrs runs two-player co-op with a deterministic **rollback** session
([Rollback mode](#rollback-mode)), the default on the desktop app and in the
browser, or **input-delay lockstep**, which both frontends keep selectable
(`--mode lockstep`, the web page's mode menu; see
[Frontend wiring](#frontend-wiring) for which to pick).
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

Every packet is `[WIRE_VERSION=2, tag, payload]`, little-endian, exact length
(trailing bytes are rejected); a peer with another version is refused at the
handshake. Pads are `u16` in the `mmx_core::Pads` layout (all twelve buttons).
Largest packet: 8207 bytes (Start with WRAM; MMX always sends 0 WRAM bytes, so
in practice it is a full Input batch, 75 bytes). The WRAM field is a NES
leftover kept only because it threads through the configs and events.

| tag  | message | channel    | contents |
|------|---------|------------|----------|
| 0x01 | Hello   | reliable   | proto, role, rom_crc32, trapset_id, coop_flags, delay, hash_interval |
| 0x02 | Start   | reliable   | host to guest: frame 0, effective delay / coop_flags / hash_interval, host WRAM (0 or 8192 bytes) |
| 0x10 | Input   | unreliable (+reliable fallback) | ack, first frame, 0..32 pads (u16 LE each) |
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
- The remote player would visibly "snap" after a correction: their X jumps
  to where the real input put him. Repeat-last prediction is right while a pad
  is held; it is wrong mainly at presses and releases, and Mega Man X is fast,
  so expect more corrections than a slow game would see. [Rollback
  smoothing](#rollback-smoothing) glides the correction in over a few frames
  instead (display only).
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

### Rollback smoothing

A correction moves the remote player (P2 on the host, P1 on the guest) from
where it was drawn to where the re-simulation puts it, up to ~30 px at 300 ms
RTT, in one frame. `NetPlay` hides that jump with a display offset
(`netgame::Smoothing`), per presented frame, right before the new frame's
step:

- **correction**: the remote player's screen position on the frame on screen
  (as it was presented) minus the same frame after this tick's re-simulation;
  added to the offset. Taking the frame already on screen, not the new one,
  keeps the corrected velocity: the drawn player moves as the re-simulation
  says, only from where it was.
- **decay**: the offset shrinks toward 0 by `min(5, 2 + |offset| / 3)` px a
  frame (`SMOOTH_STEP_MAX` = 5: a 30 px correction is drawn off for 7 more
  frames: 25, 20, 15, 10, 5, 1, 0). On an axis where the player itself moves
  the decay's way at `v` px/frame, by at most `max(5 - 2v, 2)`
  (`SMOOTH_STEP_MIN`): that motion may itself be a misprediction, and the two
  add up on screen.
- **teleports**: a correction (or offset) over 96 px (`SMOOTH_TELEPORT`: a
  warp, a respawn) is drawn at once, and the offset is dropped while the
  player is not drawn (P2 down or hidden).
- Only the remote player; the local player is never touched. Lockstep never
  smooths (every frame shown is final). `NetPlay::smoothing = false` (netsim
  `--no-smoothing`) turns it off.

**Display only.** The offset lives in `NetPlay`, never in `CoopState`, WRAM,
OAM or any save state, so checksums, the rollback states and the
plain-run invariant are untouched. It is applied at render time by the
vendored PPU (`Ppu::set_sprite_offset(slots, dx, dy)`, `// mmxrs patch`):
the masked OAM slots' X/Y are shifted when sprites are scanned and fetched
(a slot pushed off screen is skipped rather than wrapped); the field encodes
as nothing, so a loaded state renders unshifted until it is set again.
`Game::set_display_offset(player, dx, dy)` resolves the mask from the last
OAM build (display.rs): P1's slots are `$E4` at `$80:D55A` (`JSR $80:D640`,
armour parts) up to `$E4` at `$80:D560` (after `JSR $80:D671`, the player;
a Zero P1 goes through the same cursor), two Observe traps confirmed with
`trace --calls`; P2's are the range its display slice emitted (HUD bar
excluded); hidden slots (Y = `$E0`) are left out. A step renders the OAM
built by the previous step (`display_offset_moves_one_player` checks it), so
`NetPlay` sets the offset after the last replay step, right before the new
frame's step: replayed frames render unshifted (they are never presented),
the presented one with the offset. When the session ends the offset is
cleared (`NetPlay::tick` while not live, `Emu::net_drop` on the web).

Measured with `xtask netsim` (see [Testing the netcode](#testing-the-netcode)):
dozens of snaps over 6 px become none, at the price of about twice the mean
drawn-vs-settled error (the player catches up over a few frames instead of
jumping).

### Driving a rollback session

```rust
let mut s = RollbackSession::new(RollbackConfig::host(rom_crc32, trapset_id, flags), transport)?;
let mut ring = StateRing::new(usize::from(cfg.max_prediction) + 2);
let mut skip = 0;

// every frontend tick (one per frame of audio the device plays, see tick_audio)
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

## Picking a mode and a delay for the distance

| link | mode | flags | why |
|---|---|---|---|
| same region (round trip under ~80 ms) | lockstep | `--mode lockstep --delay 3` | no snapping; the 50 ms delay hides the round trip |
| across continents (round trip 250-350 ms, 15-20 frames) | rollback | `--mode rollback --delay 3 --prediction 20` | lockstep's max delay (8 frames) cannot cover it: at 300 ms RTT it runs 43 of 50 fps at delay 8 and a third of full speed at delay 3 (see [Testing the netcode](#testing-the-netcode)); rollback keeps running, the remote player snaps after corrections |

`--prediction` is rollback's window (frames it will run ahead of confirmed
remote input; default 8, max 30). It is local policy, not negotiated, so each
side may set its own. A window smaller than the round trip makes the session
stall until inputs arrive, which looks like a low frame rate.

## Testing the netcode

Two tools reproduce and explain what players see on a bad link:
`cargo xtask netsim` runs a whole session offline under chosen network
conditions, and session logs (`mmxrs --net-record`, the web page's *Log*
button) capture what really happened online, for `cargo xtask netreplay`.

### `xtask netsim`: a session over a simulated link

Two real-ROM session games, each driven by `NetPlay` exactly like a frontend
(same `tick_audio` pacing: a fake audio device per peer, 48 kHz host / 44.1 kHz
guest, topped up to 50 ms at most 4 ticks per 2 ms wake-up, 200 ms cap), over
`loopback_pair` with 1 ms of virtual time per loopback tick: a minute of play
at 300 ms RTT costs about a minute of CPU in release and no waiting.

```sh
export MMX_ROM=roms/mega-man-x.smc
cargo build --release -p xtask
# intercontinental: 300 ms round trip, 40 ms jitter, 3 % loss;
# P1 (host) walks right and jumps every 40 frames, P2 (guest) walks right/left
target/release/xtask netsim --load states/hw-spiky.state \
  --rtt 300 --jitter 40 --loss 3 --mode rollback --delay 3 --prediction 20 --frames 3600 \
  --hold 0100 --p1tap 8000@1-3600/40:12 --p2tap 0100@1-3600/120:60,0200@61-3600/120:60 \
  --report rb.json --record rb
```

Flags: `--mode rollback|lockstep`, `--delay N` (default: the mode's),
`--prediction N` (rollback window, default 8), `--rtt MS` (the link adds
RTT/2 each way), `--jitter MS` (extra 0..=MS per packet, reorders),
`--loss PCT` (unreliable packets), `--frames N` (settled frames to run),
`--seed N` (link RNG), `--load STATE` (loaded into both session games right
after `Running`, before frame 0), `--x3 PATH` / `$MMX_X3_ROM`, `--zero`,
`--timing ntsc|pal` / `$MMX_TIMING`, `--no-smoothing` (rollback smoothing off
in both peers; the simulation is the same either way). The pads come from the usual input
script (`--hold`, `--tap`, `--p1tap` for pad 1 = the host; `--p2-hold`,
`--p2tap` for pad 2 = the guest; frame numbers count from the session
start, 1-based, and name the tick where that frame is next, so the pad lands
`delay` frames later). `--report F.json` writes the numbers below as JSON,
`--record PREFIX` writes both peers' session logs (`PREFIX-host.log`,
`PREFIX-guest.log`). Exit status 1 on a desync, a session that closed, or any
settled frame that differs from the plain run.

Result of the command above (PAL ROM, 50.007 fps):

```
netsim: rollback delay 3 prediction 20, RTT 300 ms, jitter 40 ms, loss 3%, 3600 frames, 72.8 s virtual, 298 packets dropped, smoothing on
desync: no
host : 3605 frames, 3600 settled, 49.67 fps of 50.01 (0.2% ticks stalled), 61 checks ok
       rollbacks 60 (max depth 15, 348 frames replayed), prediction window full 0 times, 2 wait recommendations
       remote player first drawn vs settled: max 21 px, mean 0.35 px, 3.0% of frames > 4 px
       remote player drawn: 0 snaps > 6 px (jerk max 5 px), 5 teleports, vs settled max 20 px mean 0.98 px, 0 clipping, left play 2x
       remote player raw  : 23 snaps > 6 px (jerk max 20 px), 5 teleports, vs settled max 20 px mean 0.50 px, 0 clipping, left play 2x
       audio: queue max 70 ms, dropped 0 ms, starved 2 ms
guest: 3605 frames, 3600 settled, 49.77 fps of 50.01 (0.0% ticks stalled), 61 checks ok
       rollbacks 176 (max depth 8, 1131 frames replayed), prediction window full 0 times, 0 wait recommendations
       remote player first drawn vs settled: max 31 px, mean 2.38 px, 17.8% of frames > 4 px
       remote player drawn: 0 snaps > 6 px (jerk max 6 px), 4 teleports, vs settled max 27 px mean 3.79 px, 0 clipping, left play 0x
       remote player raw  : 136 snaps > 6 px (jerk max 27 px), 4 teleports, vs settled max 27 px mean 1.69 px, 0 clipping, left play 0x
       audio: queue max 70 ms, dropped 0 ms, starved 2 ms
plain single-Game run: identical for all 3600 settled frames on both peers
recommendation: at 300 ms RTT (+40 ms jitter) lockstep needs delay >= 11 (max 8): use rollback with --prediction >= 16
```

With `--no-smoothing` everything is the same except that the `drawn` lines
equal the `raw` ones (23 and 136 snaps). Before and after rollback smoothing,
on this baseline and on a ledge-and-pit scenario on `hw-e` (P1 runs right
jumping every 40 frames and falls into the pit twice, dying; P2 walks left
off the ledge into the pit, is down, respawns at P1, then follows jumping;
2400 frames, same link):

```sh
target/release/xtask netsim --load states/hw-e.state --frames 2400 \
  --rtt 300 --jitter 40 --loss 3 --mode rollback --delay 3 --prediction 20 \
  --hold 0100 --p1tap 8000@1-2400/40:12 \
  --p2tap 0200@1:200,0100@800-2400/120:60,8000@800-2400/60:10
```

| scenario, peer (remote player) | snaps > 6 px off / on | jerk max off / on | drawn vs settled mean off / on | max | clipping | teleports |
|---|---|---|---|---|---|---|
| hw-spiky, host (P2) | 23 / **0** | 20 / 5 px | 0.50 / 0.98 px | 20 px | 0 | 5 |
| hw-spiky, guest (P1) | 136 / **0** | 27 / 6 px | 1.69 / 3.79 px | 27 px | 0 | 4 |
| hw-e, host (P2) | 8 / **0** | 23 / 6 px | 0.93 / 1.93 px | 23 px | 0 | 0 (left play 3x: pit, down) |
| hw-e, guest (P1) | 70 / **0** | 27 / 6 px | 1.18 / 2.61 px | 27 px | 0 | 8 (2 pit deaths and respawns) |

No desync and identical to the plain run in all of them.

The same link and script in lockstep at its largest delay
(`--mode lockstep --delay 8`, no `--prediction`):

```
netsim: lockstep delay 8 prediction 0, RTT 300 ms, jitter 40 ms, loss 3%, 3600 frames, 84.3 s virtual, 556 packets dropped
desync: no
host : 3601 frames, 3601 settled, 42.83 fps of 50.01 (14.0% ticks stalled), 60 checks ok
       remote player first drawn vs settled: max 0 px, mean 0.00 px, 0.0% of frames > 4 px
       audio: queue max 70 ms, dropped 0 ms, starved 2 ms
guest: 3600 frames, 3600 settled, 42.90 fps of 50.01 (13.9% ticks stalled), 60 checks ok
       remote player first drawn vs settled: max 0 px, mean 0.00 px, 0.0% of frames > 4 px
       audio: queue max 70 ms, dropped 0 ms, starved 2 ms
plain single-Game run: identical for all 3600 settled frames on both peers
recommendation: this run stalled 14% of ticks; at 300 ms RTT (+40 ms jitter) lockstep needs delay >= 11 (max 8): use rollback with --prediction >= 17
```

So across the world lockstep is smooth (nothing snaps) but runs 43 of 50
fps even at delay 8, and at the default delay 3 about a third of full speed
(the synthetic-ROM test measures 20 of 60 fps at 300 ms RTT). Rollback keeps
full speed; the guest's first prediction of P1 (who jumps) is up to 31 px
from where it settles on 18 % of frames, the host's of P2 (who only walks)
much less, and rollback smoothing turns the corrections into glides instead
of snaps.

What each line means:

- **desync**: a peer's checksum check failed (`desync at frame N`, the frame
  whose state was compared), or the session closed for another reason.
- **frames / settled**: frames simulated for the first time / frames that
  can no longer change (rollback: final on both peers; lockstep: all).
- **fps of F (% ticks stalled)**: new frames per second of virtual time
  since the session started, against the ROM's rate. A stalled tick produced
  no new frame (lockstep waiting for the remote pad, rollback's prediction
  window full, or a wait recommendation being honoured) and plays a frame of
  silence, so the stall percentage is the slowdown players feel.
- **checks ok**: checksum comparisons that matched.
- **rollbacks (max depth, frames replayed)**: corrections of a wrong
  prediction and how many frames they re-simulated. **prediction window
  full** counts ticks where rollback refused to run further ahead of the
  confirmed remote input (`RollbackStats::stalled_advances`; raise
  `--prediction`). **wait recommendations**: times this peer was told to skip
  ticks because it ran ahead of the other.
- **remote player first drawn vs settled**: for every frame, where the other
  player (P2 on the host, P1 on the guest) was on screen the first time the
  frame was drawn, against where it ended up once the frame settled, as
  max(|dx|, |dy|) in pixels. That is the bouncing / clipping players report.
  Always 0 in lockstep.
- **remote player drawn / raw**: what the eye sees, per presented frame. A
  presented frame shows the previous frame's OAM (display.rs), so frame
  `f`'s picture is the remote player's screen position just before the step
  of `f + 1` (after that tick's rollback, if any), plus the smoothing offset
  (`drawn`) or without it (`raw`, what the picture was before smoothing),
  against the settled frame `f`. **snaps**: presented frames whose
  correction jerk, `|drawn delta - settled delta|` between consecutive
  presented frames (max of |dx|, |dy|), is over 6 px; **jerk max** is the
  largest. **teleports**: frame pairs where the settled or the raw position
  jumped over 96 px (a warp, a death and respawn), left out of the jerk: drawn
  at once on purpose. **vs settled**: drawn minus settled position, max and
  mean. **clipping**: frames where the settled player stands on a floor
  (`$0BD3|$0BD4` bit 2) and is drawn more than 2 px below it. **left play**:
  times the settled remote player went out of play (P2 down or hidden).
- **audio**: the fake device's deepest queue, audio dropped at the 200 ms
  cap (dropping is what sped the music up before `tick_audio`), and audio the
  device found missing.
- **plain single-Game run**: every settled frame on both peers (WRAM, P2's
  block and OAM, hashed per frame) equals one `Game` fed the host's settled
  pads, and both peers settled the same pads: the invariant
  `rollback_invisible_to_p2_*` checks, for any link.
- **recommendation**: lockstep runs at most `2 * delay` frames per round
  trip (each peer waits for the other's pad, which was sent `delay` frames
  ahead; measured: delay 3 at 300 ms RTT gives 20 fps), so full speed needs a
  delay of the one-way trip plus jitter in frames, plus one. Rollback's
  window should cover a round trip plus jitter minus the input delay (the
  peers drift apart until wait recommendations pull them back). Prefixed
  with what went wrong in this run, if anything.

The JSON (`--report`) holds the same numbers: `desync`, `desync_frame`,
`closed`, `plain_frames`, `plain_mismatch_frame`, `ok`, `recommendation`
and per peer `fps`, `game_fps`, `stall_pct`, `checks_ok`, `rollbacks`,
`max_rollback_depth`, `replayed_frames`, `prediction_full`,
`wait_recommendations`, `remote_err_max_px`, `remote_err_mean_px`,
`remote_err_over4_pct`, `drawn` and `raw` (each `frames`, `snaps`,
`jerk_max_px`, `teleports`, `err_max_px`, `err_mean_px`, `clipping`,
`left_play`), `audio_queue_max_ms`, `audio_dropped_ms`, `audio_starved_ms`;
top level `smoothing`.

### Session logs: recording a real session

```sh
mmxrs --coop-host myroom --mode rollback --delay 3 --prediction 20 --net-record host.log
mmxrs --coop-join myroom --mode rollback --delay 3 --prediction 20 --net-record guest.log
```

In the browser every session records; the *Log* button next to *Leave*
downloads the current (or last) session's log (`Emu.net_record()`). The log
starts at `Running` (both peers at power-on, frame 0); mmxrs appends to the
file every second, so a crash loses at most a second. Size: a few hundred KB
per hour (the 72 s netsim run above wrote 7.5 KB per peer).

Format (`mmx_core::netlog`): text, one record per line. A header of
`key value` lines ending with `---`:

```
mmxnetlog 1
rom_crc 41965122            ROM CRC32
trapset da97cf921293f9da    handshake trap-set id (build + X3/Zero flags)
mode rollback               rollback | lockstep
role guest                  host (P1) | guest (P2)
delay 3                     effective (host's) input delay
prediction 20               rollback window (0 in lockstep)
interval 60                 checksum interval in frames
timing auto                 auto | ntsc | pal
x3 0                        X3 Zero set loaded
zero 0                      P1 plays as Zero
load states/hw-spiky.state  (netsim only) state loaded at frame 0
---
```

then the body. `MS` is wall-clock milliseconds since `Running`, `F` the next
frame to simulate at that moment, `N` a frame count: the state after `N`
frames (rollback checks the state before frame `N`, lockstep after frame
`N - 1`; the log normalises both). Pads and checksums are hex.

| line | meaning |
|---|---|
| `p N P1 P2` | the next `N` confirmed frames used pads `P1`, `P2` (run-length; frames are implicit, counting from 0) |
| `t MS F` | time mark, once a second |
| `c MS N LOCAL REMOTE` | checksum of the state after `N` frames here, and as reported by the peer (equal unless desynced) |
| `r F D` | rollback of `D` frames while `F` was next |
| `s MS F DUR` | stall: from `MS`, `DUR` ms without a new frame while `F` was next (wait recommendations not counted) |
| `w MS F SKIP` | wait recommendation: skip `SKIP` ticks |
| `i MS F` / `j MS F` | network interrupted (peer silent) / resumed |
| `d MS N LOCAL REMOTE` | desync detected on the state after `N` frames |
| `x MS REASON` | session closed (`local quit`, `peer quit`, `desync`, ...) |

### `xtask netreplay`: what really happened

```sh
cargo xtask netreplay host.log                  # one peer's log
cargo xtask netreplay host.log guest.log        # both: also compared with each other
      [--load STATE] [--x3 PATH] [--rom PATH]   # --load only for netsim logs made with --load
```

It rebuilds the session game the way the peers did
(`netgame::session_game`, co-op on, the X3 set and `--zero` per the header;
it refuses when the ROM CRC or the trap-set id differ, i.e. another ROM,
build or flags), steps the confirmed pad track and compares the checksum
after every logged interval with the log's local and remote values. For the
netsim run above:

```
rollback host: delay 3, prediction 20, check every 60 frames, timing auto, x3 0, zero 0; 3600 confirmed frames over 1:12
stalls: none
rollbacks by depth: 5-8: 58, 9-16: 2
60 checksum matches, 2 wait recommendations, 0 network interruptions
replay of 3600 frames matches all 60 checksums in the log
...
input tracks identical for 3600 frames (3600 / 3600 recorded)
checksums agree at all 60 common intervals
```

On a desync it prints `host and guest disagreed at frame N (state after N
frames); the replay matches host` (or `guest`, or `neither`): the peer the
replay matches simulated correctly, so the other one diverged at or before
frame N (since the previous matching check). `both peers agreed at frame N
... but the replay has X` means the replay is not the session's game (ROM,
build or flags differ). With two logs it also checks that the input tracks
are identical (they must be; a difference is a netcode bug) and compares the
two local checksums per interval. The summary turns "my cofounder saw X at
minute 3" into frames: time marks map minutes to frames, stalls and
rollbacks show where the link hurt. Exit status 1 when a checksum or the
input tracks differ.

### Tests

- `cargo test -p xtask net`: `netsim_synthetic_reports` (both modes on the
  synthetic ROM at 300 ms RTT: no desync, plain run identical, report and
  JSON fields present, rollback rolls back, lockstep stalls),
  `netsim_detects_forced_desync` (the guest's WRAM is corrupted from frame
  150: both modes report `desync: YES` within a few check intervals),
  `netreplay_on_netsim_logs` (netsim `--record` logs replay cleanly and agree
  with each other; a tampered checksum is pinned to its frame and to the peer
  that was right), `netsim_motion_metrics_synthetic` (the drawn/raw fields).
  With `MMX_ROM` (debug build: a few minutes each): `smoothing_removes_snaps_at_300ms`
  (the baseline above: no desync, plain run identical, at least 12 raw snaps
  per peer, 0 drawn, no clipping, drawn mean error at most 2.5x the raw one)
  and `smoothing_hw_e_pit_and_respawn` (the hw-e run above: the same checks,
  and P2 left play at least once).
- `cargo test -p mmx-core smoothing_math` (the offset's decay, clamp,
  teleport and hidden rules, no ROM) and `display` (`display_slots_mask`;
  with `MMX_ROM`, `display_offset_moves_one_player`: a step shows the OAM the
  previous step built, P1's and P2's slot masks are found and disjoint, an
  offset moves only pixels near that player and changes no state).
- `cargo test -p mmx-core netlog`: `log_round_trips` (header, run-length
  pads settled out of order, every event kind), `stalls_and_time_marks`.
- `cargo test -p mmx-native netplay`: the localhost matchbox pairs record
  both peers with `--net-record`; the logs must hold the same input track and
  matching checksums.

## Frontend wiring

Both frontends run either mode through one driver,
`mmx_core::netgame::NetPlay` (`crates/mmx-core/src/netgame.rs`), picked by
`netgame::Mode`:

| | `Mode::Rollback` (default) | `Mode::Lockstep` |
|---|---|---|
| session | `RollbackSession` | `Session` (input-delay lockstep) |
| input delay | 0-3 frames, default 2 | 0-8 frames, default 3 (50 ms) |
| late remote pad | predicted (last pad repeated); corrected by re-simulating | the game waits; status shows `waiting for peer` |
| what you see | never waits; a wrong prediction is corrected up to ~30 px, which [rollback smoothing](#rollback-smoothing) glides in over a few frames (0 snaps over 6 px at 300 ms RTT in `xtask netsim`, was 23 / 136) | smooth: every frame shown is final |
| pick it for | lossy or long-distance links (RTT well above 50 ms) | same-region links (RTT under ~2x the delay) |

**Recommended for playtests on a good link: `--mode lockstep --delay 3`** on
both sides (web: mode *lockstep*, delay *3* or *auto*, or `?mode=lockstep`).
With lockstep a frame steps only once both pads are known, so on a link whose
round trip stays under the delay (3 frames = 50 ms) nothing ever waits or
snaps; a lost packet costs a short stall instead of a visible correction.
Switch to rollback when the link is lossy or far away and those stalls get
frequent.

The mode is **not negotiated**: both players must pick the same one. The two
sessions announce different protocol versions in their Hello (`PROTO_VERSION`
vs `ROLLBACK_PROTO_VERSION`), so a mismatch refuses to start and the status
reads `closed (mismatch: NetMode)` on the side that noticed (the other side
may show `mismatch: Proto`). Within a mode the host's delay wins. No protocol
change was needed for this.

- `NetPlay::config(rom, host, mode, delay, x3, p1_zero)` builds the Hello: ROM CRC32
  (`rom_info().crc32`), trap-set id (FNV over the sorted trap addresses of a
  session game, co-op traps included, plus a revision constant and a byte
  "X3 Zero set loaded" and a byte "P1 plays as Zero" (`--zero`): peers where only one loaded the X3 ROM refuse to
  start; the session game takes the set from the frontend's game), co-op flags
  `COOP_TWO_PLAYERS`, input delay. No WRAM is sent (MMX has no SRAM; both
  peers start from power-on).
- `tick_audio(game, pad, now_ms)`: what the frontends call, while their audio
  queue is below ~50 ms: `tick` plus exactly one frame of audio (the new
  frame's, or silence when nothing stepped), so the device clock paces the
  session at the ROM's own rate (`Game::fps`; the PAL ROM runs 50.007).
  Without audio they call `tick` from a `Game::fps` timer.
- `tick(game, pad, now_ms)` once per frontend tick: pumps the network;
  once live, **lockstep** latches the local pad and steps the next frame if
  both pads are known (one frame per tick, 0 while waiting; hashes every 60
  frames), **rollback** adds the local pad, advances, and runs the requests. Rollback save states
  are `Game::save_state`, which carries the co-op state (`CoopState`, the
  `COOP` tail, [savestates.md](savestates.md)) so a rollback rewinds P2 too. Checksums are FNV-1a over
  WRAM `$7E:0000..$2000` and P2's shadow block. Replayed frames' audio is
  dropped. On `Running` / `Started` it replaces the game with a power-on game with the
  co-op traps registered and `coop.enabled` set from the flags.
- `status()` is the one-line status both frontends show, prefixed with the
  mode (`rollback host: ...`, `lockstep guest: ...`).
- `pump(now_ms)` pumps the network only (flushing a goodbye), `settled_frames()`,
  `checks_ok()`, `closed()`, `transport()` work in both modes; `rollback()` /
  `lockstep()` expose the underlying session.
- **Wire pad**: the full SNES `u16` pad travels as-is (X, L, R and Select
  included).

Host = player 1, guest = player 2; each peer plays with its own P1 keys / first
gamepad. Pause, fast-forward, frame-step and save states are disabled while a
session exists. The game keeps running locally while connecting.

### Desktop (`mmxrs`)

```sh
cargo run --release -p mmx-signal                     # once, anywhere reachable
mmxrs --rom mmx.sfc --coop-host myroom                 # player 1
mmxrs --rom mmx.sfc --coop-join myroom                 # player 2
      [--signal ws://HOST:3536] [--ice none|'stun:.. turn:.. username=U credential=C']
      [--mode rollback|lockstep] [--delay N] [--prediction N] [--net-record LOG]

# playtest on a good link (both sides):
mmxrs --rom mmx.sfc --coop-host myroom --mode lockstep --delay 3
mmxrs --rom mmx.sfc --coop-join myroom --mode lockstep --delay 3
```

`--signal` defaults to `ws://127.0.0.1:3536`, `--ice` to the Google STUN pair,
`--mode` to rollback, `--delay` to the mode's default (rollback 2, range 0-3;
lockstep 3, range 0-8; host's value wins). The window title shows
`[NET rollback host: 2/4 waiting for the other player in myroom]`, then
`[NET rollback host: f=.. d=.. rtt=..ms rollbacks=.. pred=cur/max]`
(` INTERRUPTED` when the peer goes quiet) or
`[NET lockstep host: f=.. d=.. rtt=..ms]` (` waiting for peer (1.2s)` while
stalled), or `closed (reason)`; each connect
stage is printed to stderr once. `Esc` / closing the window sends a goodbye
and waits 200 ms. The transport is cargo feature `netplay` (default on; it
pulls webrtc-rs + ring, which need a C compiler); `--no-default-features`
builds without it and `--coop-host/--coop-join` then fail with a message.

### Browser (`mmx-web`, feature `netplay`)

Build the bundle with `--features netplay` (e.g. `wasm-pack build
crates/mmx-web --target web --release --out-dir site/pkg -- --features netplay`).
The panel above the screen has the signal URL, room, ICE text, mode
(rollback / lockstep), delay (*auto* = the mode's default) and
*Host* / *Join* / *Leave*; the status line under it shows the same text as the
desktop title. `?signal=`, `?room=`, `?ice=`, `?mode=`, `?delay=` prefill it. The `Emu`
API: `net_connect(signal, room, host, mode, delay, ice)` (`mode` is
`"rollback"` or `"lockstep"`, `delay < 0` = default), `net_tick(pad, now_ms)`
(frames simulated; present when > 0), `net_live()`, `net_active()`,
`net_status()`, `net_leave()` then `net_drop()` ~200 ms later,
`Emu.net_default_ice()`, `net_record()` (the session log, see
[Session logs](#session-logs-recording-a-real-session); the *Log* button
downloads it). A bundle without the feature has none of these and
the panel says so.

### Tests

- `cargo test -p mmx-core netgame`: `SyncTestSession` over the synthetic ROM
  (a 32 KiB LoROM whose loop sums both pads into WRAM,
  `netgame::synthetic_rom`), and two `Game`s through `NetPlay` over a lossy
  (15 %), jittery loopback link with delay 0 for 300 settled frames: no
  desync, >= 25 checksum matches, real rollbacks
  (`rollback_pair_over_lossy_loopback_stays_in_sync`). The lockstep twin
  (`lockstep_pair_over_lossy_loopback_stays_in_sync`, delay 3, same link)
  steps 300 frames, >= 25 hash matches, and both games must equal a plain run
  fed the scripted full-width pads shifted by the delay.
  `mode_mismatch_refuses_to_start`: a rollback host and a lockstep guest both
  close without starting (`NetMode`).
- `cargo test -p mmx-native netplay`: in-process mmx-signal, two
  `NetPlay<MatchboxTransport>` over 127.0.0.1 (ICE `none`), 300
  settled frames on the synthetic ROM, then a quit the guest must see, once
  per mode (rollback at delay 0, lockstep at delay 3). With
  `MMX_ROM` set the same runs on the real ROM with co-op on (120 frames in
  debug, 600 in release), both modes.

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
