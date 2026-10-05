# Timing oracle: Mesen 2 against our core, and the 100% TAS (2026-10-05)

The TASVideos 100% movie 3197M (`hetfield90v3-megamanx-100p.bk2`, BizHawk
1.11.6, bsnes v0.87 "Compatibility" core, 119,424 frames) desynced on our
core in Chill Penguin's stage (~14,700, X knocked into a pit), the older
2771M at Chill Penguin (17,160). Both are recorded on the exact USA v1.0
dump (`Mega Man X (U) (V1.0) [!].smc`, SHA-1 `449a0063…7fb1`). This is the
hunt for the timing differences behind it.

## The oracle

`tools/oracle-mesen/` (README there): Mesen 2's C++ core built without .NET
into `$MESEN/build/oracle` (`make -C tools/oracle-mesen`), linked statically
into a 250-line harness that plays the bk2 input log from power-on through an
`IInputProvider` and writes one line per frame in the same format as
`cargo xtask movie --trace PATH`: frame, FNV-1a 64 of WRAM, `$0B9B` (main
loop counter: it stops on a lag frame), `$0B9E` (NMI counter), RNG, X's x/y,
the pad word the game read (`$A7`), `$D1-$D4`, level, lives, HP and `$02FD`
(the low byte of the PC the NMI pushed: which instruction of the idle loop
`$80:80A1 LDA $0B9D / BEQ` it interrupted, a per-frame timing fingerprint).
Both sides also write per-instruction traces (`--cpu-trace A-B PATH`: PC,
scanline, H in master clocks) and SPC700 traces, which is what found the
causes below. Mesen's trace logs after the opcode fetch (subtract 6 on FastROM,
8 on SlowROM); ours before it.

Two of Mesen's own inaccuracies are fixed in patched copies of two Mesen
files at build time (the clone is untouched):

- `SnesDmaController::RunDma` counts a channel's bytes in a `uint8_t` for the
  end-of-DMA clock alignment: MMX's 544-byte OAM DMA every frame ends 2-4
  master clocks off.
- `--bsnes-apu 1`: the SPC/CPU clock ratio of bsnes (below); Mesen's setting
  is an integer sample rate and cannot express it.

Frame alignment: Mesen polls input at the start of vblank; poll `p` ends
video frame `p` and sets the pad for frame `p+1`. Start at movie line 359 is
read at frame 360 on both, and the Intro (8,759 frames of RNG-driven play)
replays identically on both, so the alignment is right.

## Does the movie sync on Mesen?

No, not with Mesen's defaults either. Stage entries / clears per run (frame:
event), 3197M:

| run | Intro | Penguin | Kuwanger | Chameleon | Eagle |
|---|---|---|---|---|---|
| Mesen, SPC 32040 Hz (UI default) | clear 8759 | never cleared (deaths) | - | - | - |
| Mesen, SPC 32000 Hz | clear 8759 | clear 17844 | stage select picks Penguin again | - | - |
| Mesen, bsnes SPC ratio, DMA fix | clear 8759 | clear 17844 | in 18793, clear 25854 | in 26910, clear 35282 | in 36229, deaths from 41887 |
| ours before | clear 8648 | X dies ~14700 | - | - | - |
| **ours now** | clear 8759 | clear 17844 | in 18793, clear 25854 | in 26910, X dies 35231 | - |

(The one death in Kuwanger's stage, 21259, is the movie's: it happens on
every run that gets there, and both movies take it.) 2771M: ours now clears
Intro, Penguin, Kuwanger and Chameleon (35517) and then picks Spark Mandrill
instead of Storm Eagle at the stage select (36436); Mesen (bsnes ratio)
picks Storm Eagle after Kuwanger instead of Chameleon (27898). So the
oracle is not bsnes either; it was used for what it shows directly: where in
an instruction stream our CPU timing differs from a cycle-accurate emulator.

## What was different, in order of first divergence

Found by lockstep instruction traces from power-on (ours vs Mesen),
fixing each difference and re-running.

1. **CPU start (frame 1).** Mesen runs PPU/APU/timers 186 master clocks
   before the CPU's first cycle after power-on/reset ("delay the CPU takes to
   get out of reset"); ours started the CPU at H=0. Whole-frame timing was
   shifted by 186 clocks from power-on. Fix: `SnesEmulator::run_startup_delay`.
2. **NMI edge (frame 2+).** The 65816 polls interrupts at the start of an
   instruction's last cycle. Ours raised the NMI at H=0 of line 225, so an
   instruction whose last cycle started at H=0..5 took the NMI one
   instruction early; Mesen (S-CPU schematics) has it seen from H=6. With the
   idle loop `LDA $0B9D` (26 clocks) / `BEQ` (18) that changes which
   instruction is interrupted on about a third of all frames, and so the
   phase of the next frame's work. Fix: `NMI_MCLK = 6` (memory.rs).
3. **DMA end alignment (frame 2).** After a DMA the CPU waits to a whole CPU
   cycle since the DMA began; ours counted the DRAM refreshes (40 clocks)
   inside the DMA into that length, bsnes and Mesen count DMA clocks only.
   Fix: `DmaUnit::exclude_refresh`.
4. **H/V IRQ edge (frame 2, line 96: MMX's HUD IRQ).** Ours: visible to the
   CPU at H=10+4·HTIME (H) / H=10 (V). bsnes v0.87 (`hcounter(10)` plus the
   hold cycle) and Mesen (counter ticks at hclock 2 mod 4, 2-tick delay) both
   give 18+4·HTIME / 14. The IRQ was taken up to an instruction early every
   frame. Fix: `IRQ_H_OFFSET_MCLKS = 18`, `IRQ_V_OFFSET_MCLKS = 14` (ppu.rs).
   **Needed for the movie**: without it 3197M dies at 17045 in Penguin.
5. **APU port reads.** The CPU samples the bus 4 clocks before the end of its
   cycle (bsnes `add_clocks(speed - 4)`, Mesen the same); ours read
   `$2140-$2143` at the start of the cycle, before the APU had run the
   cycle's first 2 clocks. Fix: the bus ticks the APU to the read point
   first (`Bus::apu_sync`).
6. **APU clock ratio (the big one).** The game uploads and handshakes with
   the SPC700 through polling loops (`$80:8736 CMP $2140 / BNE`): how long a
   load takes, and so on which frame the game resumes and lags, depends on
   the SPC's speed relative to the CPU. Ours ran 24,606,720 Hz (32040·768) per
   21,477,270 Hz; bsnes v0.87, the movies' emulator, runs 24,607,104 Hz
   (`smp.ntsc_frequency = 24607104; //32040.5 * 768`) per 21,477,272 Hz.
   1.6e-5 relative: a few thousand SPC cycles over the first few minutes.
   Mesen's 32000 Hz / 32040 Hz settings bracket it and fail in opposite ways.
   Fix: the APU ticks at bsnes's ratio (apu.rs, `BSNES_*`). Real APU
   resonators are 24.576 MHz nominal and measure ~24.607 MHz; this is the
   typical measured value bsnes adopted. **Needed for the movie**: with
   everything else and the old ratio, 3197M dies at 17045.

Plus, for movie playback only: `Game::set_allow_opposing_directions(true)`
in `xtask movie`. 3197M presses Left+Right at frame 14258; jgenesis filtered
opposing directions (a d-pad rocker cannot press both, a keyboard can), the
console itself passes whatever the pad's switches say. Off by default for
play.

With 1-5 our CPU trace matches Mesen's instruction for instruction (PC, line,
H) from power-on through frame 12 and frames 150-240, except where Mesen's
SPC700 is ~1.5 SPC cycles behind ours (Mesen spends 2 SPC cycles reading the
reset vector at power-on and catches the SPC up to `target - 1`): from frame
160 a handshake poll sees the SPC's answer one loop iteration later on Mesen.
That is Mesen modelling, not something to copy blindly; bsnes starts the SMP
at `$FFC0` at clock 0 like ours.

Ablations (full 3197M and 2771M runs, same outcome = same stage table):
items 1, 3 and 5 do not change either movie's outcome (kept: hardware
behaviour both references agree on, or Mesen's measured one); NMI at H=8
(bsnes's poll granularity) instead of 6 fails at 17045 like the old core;
the old IRQ edge or the old APU ratio fail at 17045.

## Where it stops now

3197M: Intro, Chill Penguin (incl. the boss and the Leg Parts), Boomer
Kuwanger, and Sting Chameleon's stage to frame ~35,231, where X dies
(Mesen with the bsnes ratio clears Chameleon at 35,282 and then desyncs in
Storm Eagle's stage). The first state difference against that Mesen run is a
lag frame on one side only at frame 8985 (the stage select after the Intro,
during a multi-frame decompression at `$80:B2xx`: our CPU reaches the idle
loop just before the NMI, Mesen's just after); the Chameleon desync is a lag
frame on our side only at frame 32069 (`$0B9B` does not advance), after which
X's position runs one frame behind Mesen's. Lockstep traces of frames
32060-32069 show the same instruction stream on both with only the NMI/IRQ
entry phase differing (the `$02FD` fingerprint), i.e. accumulated phase from
earlier, not a new mechanism. The remaining candidates, not resolved:
Mesen's SPC reset/catch-up offset vs bsnes's (which side bsnes is on decides
which emulator is right at each handshake), bsnes's "Compatibility" PPU
profile, and the SPC700 cycle timings themselves (DSP/timer edge cases were
not traced).

2771M: Intro, Penguin, Kuwanger, Chameleon cleared (35,517); the stage select
then picks Spark Mandrill (36,436) instead of Storm Eagle.

## Effects beyond the movie

All of this changes emulated timing for every game session: lag frames move
(the game's slowdown is now closer to hardware), so `NET_REVISION` is 7 and
peers on older builds will not connect. Save states still load (no new
emulator state fields). Co-op QA (`scripts/coop-qa.sh check`): P1 parity
with P2 idle is now yes on every milestone, M3 included (it was the one
"no", B4b: a lag frame); mirror parity M3/M7 no, as before for M7 (P2's shots
and contacts change the fight).

## Commands

```
make -C tools/oracle-mesen -j14 MESEN=/Users/ritz/dev/Mesen2
unzip -p movies/hetfield90v3-megamanx-100p.bk2 'Input Log.txt' > /tmp/v3.txt
$MESEN/build/oracle/mesen-oracle ROM /tmp/v3.txt 119424 mesen.txt --bsnes-apu 1
MMX_ROM=ROM cargo xtask movie --movie movies/hetfield90v3-megamanx-100p.bk2 \
    --trace ours.txt --png-every 0 --pict-every 0
# instruction level, a frame window (both sides)
$MESEN/build/oracle/mesen-oracle ROM /tmp/v3.txt 32070 m.txt --bsnes-apu 1 --cpu-trace 32060-32069 mcpu.txt
MMX_ROM=ROM cargo xtask movie --movie ... --frames 32070 --cpu-trace 32060-32069 ocpu.txt [--spc-trace ospc.txt]
```

## Widescreen over the newly synced part

`cargo xtask movie --movie movies/hetfield90v3-megamanx-100p.bk2 --frames 35300
--wide 16:9 --twin --png-every 0 --out target/movie/v3-16x9-oracle` (v1.0
dump): 0 twin state differences (widescreen does not change the simulation),
0 margin stale/slot/page/palette findings, no panic. New: picture-centre
differences against the unwidened frame in Boomer Kuwanger's stage (708
frames, from 19231, mostly 526 pixels: the tower shaft with an enemy at the
left wall, `f019231_find_centre.png`) and Sting Chameleon's (4209 frames,
`f027473_find_centre.png`). Intro and Penguin stay 0/0 as before. Not
investigated here; candidates are sprites dropped from the centre by the
per-line sprite limits once margin sprites share the line.
