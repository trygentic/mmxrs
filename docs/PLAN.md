# mmxrs plan: Mega Man X (SNES) reconstruction in Rust, with co-op and Zero

Written 2026-09-30 from the z2rs commit history, the jgenesis source and the
MarkAlarm MMX1 disassembly. This is the contract the implementation crates
build against. Keep it short; details live next to the code.

## What z2rs did, in order (the playbook we copy)

1. **M0 bootstrap**: workspace, ROM never committed (`Z2_ROM` env, hash gate),
   RAM map as typed constants, routine ledger (`ports.toml`), movie parsers.
2. **M1/M2 oracle + hybrid execution**: a reference emulator (tetanes) as the
   oracle; the game runs inside an embedded CPU interpreter over a shared RAM
   mirror; every hand-ported routine is a *trap* on its entry address that
   runs Rust instead of the ROM bytes and emulates the return. Playable at 0%
   ported, and at every percentage. Lockstep verify diffs RAM / OAM / palette /
   framebuffer against the oracle every frame and reports the first divergence
   plus the last trap that fired.
3. **M3-M6 porting** bank by bank, register-exact, pinned by lockstep replays
   of TAS movies. A/B every trap (trapped vs original bytes).
4. **M7 frontends**: native (winit/pixels/cpal/gilrs), web (wasm-bindgen +
   AudioWorklet), later Android. Headless mode with `--frames`, `--dump-at`.
5. **Post-parity extras** (the ones we want from day one here): widescreen,
   **two-player co-op by time-slicing the ROM's own player code over a shadow
   copy of the player's RAM block** (`crates/z2-core/src/coop.rs`), rollback
   netplay over WebRTC (`z2-net`, game-agnostic, zero deps; matchbox
   transport behind a feature), save states, HD packs.

## Decisions

| topic | decision |
|---|---|
| emulator | **fork jgenesis** (Rust, GPL-3): `snes-core`, `snes-coprocessors`, `wdc65816-emu`, `spc700-emu`, `arm7tdmi-emu`, `dsp`, `jgenesis-common`, `jgenesis-proc-macros`, `snes-config` copied into `vendor/jgenesis/`. Pure logic, no SDL/wgpu, builds for wasm32. Every local change is marked `// mmxrs patch`. |
| licence | GPL-3.0 for the whole repo (forced by jgenesis). Nothing copied from snesrecomp / MegaManXSNESRecomp (PolyForm NC); behaviour reference only. |
| oracle | the **same core with traps disabled**. The DUT is the same core with traps on. Mesen2 is the external second opinion for divergence hunting (not embedded). |
| trap hook | **no CPU-crate patch.** `SnesEmulator::tick` is one bus cycle; between ticks `Game` checks `!is_mid_instruction()` and looks up `PBR:PC` in the trap table. A `fired` flag, cleared when the CPU goes mid-instruction, makes a trap fire once per boundary visit (DMA / refresh ticks leave PC at the same boundary). Return is emulated by popping the stack (RTS: 2 bytes, RTL: 3) and rewriting registers via `set_cpu_registers`. |
| co-op | z2rs model: P2 is a shadow copy of X's object block `$7E:0BA8..$0C38` (`$90` bytes) plus X's remapped-input bytes `$0BDE..$0BE3`. Around each P2 pass the block is swapped in, the ROM's own player update runs on pad 2, the block is swapped out. P2 work costs zero emulated cycles (`run_cpu_isolated`: CPU ticks without advancing PPU/APU/master clock). |
| Zero | v1: P2 uses X's sprites with **Zero's palette** (a spare CGRAM sprite palette). Real Zero action sprites are a later content task (X1 has none; X2/X3 rips or HD-pack art). |
| ROM version | disassembly is **USA v1.0** (`$FFDB = 0`); mstan's recomp targets v1.1. Gate on the internal header (title `MEGAMAN X`, USA, `$FFD6 = 0` no coprocessor) and record the version byte; addresses in `ram.rs` are v1.0, v1.1 gets a warning until checked. The 512-byte copier header is stripped when present. No ROM exists on the dev Mac as of 2026-09-30, so every ROM test is gated on `MMX_ROM` and skips without it. |
| toolchain | Rust 1.96 (installed). jgenesis HEAD needs `f64::algebraic_*`; the fork replaces the two calls in `fir_resampler.rs` with plain `+`/`*`. |
| WASM | `mmx-core` / `mmx-verify` / `mmx-net` are std-only, no fs, no threads; ROM bytes are passed in. `mmx-web` is a `cdylib` (wasm-bindgen, canvas, AudioWorklet), modelled on `crates/z2-web`. `getrandom` needs the `wasm_js` feature on wasm32 (rand is pulled by jgenesis-common). |
| netplay | copy `z2-net` -> `crates/mmx-net` verbatim (MIT, zero deps, `RollbackGame` trait is game-agnostic), plus `vendor/matchbox_socket` (patched 0.14) and `tools/z2-signal` -> `tools/mmx-signal`. Rollback over WebRTC on native and browser, exactly as z2rs. |
| frontends | `mmx-native`: winit 0.30 / pixels 0.15 / cpal 0.15 / gilrs 0.11 like z2-native. Android later (reuse z2rs shell). |

## Workspace

```
Cargo.toml                 workspace; vendor/ excluded like z2rs
vendor/jgenesis/*          forked crates (GPL-3), each change marked `mmxrs patch`
vendor/matchbox_socket     z2rs's patched copy (MIT/Apache)
crates/mmx-core            Game (SnesEmulator wrapper), traps, ram map, coop, savestate
crates/mmx-verify          Lockstep, Divergence, .bk2 movie parser
crates/mmx-net             z2-net copy
crates/mmx-native          desktop app + headless
crates/mmx-web             wasm cdylib + site/
tools/xtask                verify / ledger / dump CLI
tools/mmx-signal           signalling server
docs/                      this plan, ram-map.md, coop.md, netplay.md
ports.toml, ram-map.toml   ledgers (hand-maintained until an extractor exists)
```

## `mmx-core` public contract (the other crates code against this)

```rust
pub struct Pads { pub p1: u16, pub p2: u16 }      // SNES auto-read layout: bit15 B,14 Y,13 Select,12 Start,11 Up,10 Down,9 Left,8 Right,7 A,6 X,5 L,4 R
pub struct Game { .. }
impl Game {
    pub fn from_rom(rom: Vec<u8>) -> Result<Game, RomError>;   // strips copier header, checks internal header
    pub fn rom_info(&self) -> &RomInfo;                         // title, version byte, sha256, crc32
    pub fn step(&mut self, pads: Pads);                         // one video frame, traps fire
    pub fn frame(&self) -> (&[u32], usize, usize);              // 0x00RRGGBB, width, height
    pub fn take_audio(&mut self) -> &[(f32, f32)];              // stereo samples since last take, 48 kHz default
    pub fn wram(&self) -> &[u8; 0x20000]; pub fn wram_mut(&mut self) -> &mut [u8; 0x20000];
    pub fn oam(&self) -> [u8; 544]; pub fn cgram(&self) -> [u16; 256];
    pub fn regs(&self) -> Registers; pub fn set_regs(&mut self, r: Registers);
    pub fn frame_count(&self) -> u64;
    pub fn save_state(&self) -> Vec<u8>; pub fn load_state(&mut self, s: &[u8]) -> Result<(), String>;
    pub traps: TrapTable; pub traplog: TrapLog; pub coop: CoopState;
}
pub type TrapFn = fn(&mut Game);
pub enum TrapReturn { Rts, Rtl, Observe }   // Observe: run the body, then let the ROM bytes run too
impl TrapTable { pub fn register(&mut self, addr: u32, name: &'static str, ret: TrapReturn, f: TrapFn); pub fn set_enabled(&mut self, on: bool); pub fn set_untrapped(&mut self, addr: u32, on: bool); pub fn is_trapped(&self, addr: u32) -> bool; }
pub mod ram;   // v1.0 addresses as `pub const X_HP: u32 = 0x7E0BCF;` etc.
```

`Game::step` never touches the filesystem, never sleeps, never uses the wall
clock. `mmx-core` must build for `wasm32-unknown-unknown`.

## RAM map (v1.0, bank $7E; from the disassembly analysis, confidence high unless marked)

- scheduler: 7 task slots `$30-$9F` stride `$10`; current slot `$A0`
- modes: `$D1` (0 new-game init, 2 running), `$D2` (0 intro, 2 stage select, 4 in stage, `$0A` game over), `$D3` in-stage phase (0 load, 2 ready, **4 gameplay**, 6 dying, 8 clear), `$D4`
- frame counters: `$0B9B` main loop, `$0B9C` in-stage, `$0B9E` NMI; NMI-done flag `$0B9D`
- RNG `$0BA6-$0BA7`
- pads: P1 held `$A7`, prev `$A9`, pressed `$AB`; P2 held `$AD`, prev `$AF`, pressed `$B1`; X's remapped input held `$0BDE`, prev `$0BE0`, pressed `$0BE2` (built by `$80:E559`)
- **X object `$0BA8..$0C38`** (player code runs with `D=$0BA8`): `+02 $0BAA` action (`$0C` death, `$0E` hurt), `+05 $0BAD` X pos, `+08 $0BB0` Y pos, `+04/+07` subpixels, `+1A $0BC2` vx, `+1C $0BC4` vy, `+27 $0BCF` HP (bit7 = HUD redraw), `+33 $0BDB` weapon, `+6B $0C13` i-frames, `+11 $0BB9` facing bit 6
- enemies `$0E68-$1227` stride `$40` × 15 (`+00` active, `+0A` id, `+05/+08` pos, `+26` contact damage, `+27` HP); shots `$0C98` `$20`×12; items/enemy shots `$1228` `$40`×8; effects `$1928` `$20`×31
- camera BG1 `$1E4D` X / `$1E50` Y; level `$1F7A`; lives `$1F80`; sub-tanks `$1F83-$1F86`; weapon energy `$1F88+2i`; max HP `$1F9A`; armor `$1F99`; heart tanks `$1F9C`; pause `$1F24`; boss object ptr `$1F0E`
- OAM buffer `$0700` (544 B), CGRAM buffer `$0300`; sound queue ring `$0B72` (16×{cmd,param}), enqueue `$80:888F`
- anti-piracy counter `$1F9F` (SRAM probes at `$70:xxxx`; no SRAM mapped = pass)

## Routine map (v1.0)

reset `$80:8000`; main loop / scheduler `$80:8099`; NMI `$80:8168`; IRQ `$80:84A1`;
in-stage per-frame `$80:9A6A` → input remap `$80:E559` → RNG `$84:9086` →
**object update master `$80:D203`** (player `$81:812E`, shots loop `$80:D450`,
enemies loop `$80:D500`, camera `$80:DDF1`) → spawn by camera `$80:DC4C` →
pause check `$80:9E7E` → **OAM build + HUD `$80:D585`** (health bar `$80:D7E4`,
boss bar `$80:DA24`). Player damage `$84:9D07` (enemy contact), death `$81:96CB`,
hurt `$81:967C`; enemy spawn `$82:8321`; title `$80:91AA`.

## Co-op hooks (mirrors z2rs `coop.rs`)

1. `$80:D450` (shots loop, runs right after the player update) in `Observe`
   mode: swap in P2 block, build P2's remapped input from pad 2 (`$AD`), run
   `$81:812E` isolated, clamp P2 into the camera window, swap out. Then the
   original shots loop runs.
2. OAM build: after the player's OAM entries are emitted, emit them again for
   P2 into spare slots with the Zero palette. Exact hook found from
   `$80:D585` once a ROM is available.
3. Contact damage `$84:9D07`: one pass per player (later).
4. Off by default; with the trap group registered and the flag off the game
   must be byte-identical to the untrapped oracle (lockstep test).

## Milestones for this pass (no ROM on the machine)

- M0 (now): workspace, vendored fork compiles on 1.96 and on wasm32, `Game`
  boots a **synthetic LoROM** in tests (trap fires, RTS/RTL emulated, Observe
  runs both), lockstep harness, `.bk2` parser, xtask verify, native app with
  headless `--frames/--dump-at`, `mmx-net` copy, web cdylib skeleton, co-op
  scaffold with unit tests on the swap logic.
- M1 (needs `MMX_ROM`): boot to title, record SHA-256 / version, first
  lockstep replay, find OAM hook, P2 visible.
- M2: port the bank-`$80` frame driver and player routines register-exact
  behind traps; separate HP bar + revive; real Zero sprites; Android.
