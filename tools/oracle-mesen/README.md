# mesen-oracle

Headless Mesen 2 (C++ core only, no .NET UI) that plays a BizHawk `.bk2`
input log from power-on and writes the same per-frame trace as
`cargo xtask movie --trace`, so the two emulators can be diffed frame by
frame. docs/oracle.md has the method and the findings.

## Build

Needs clang++ and zlib (macOS: both come with the Xcode command line tools).
No SDL, no .NET. The Mesen 2 clone is only read; objects, the static library,
two patched copies of Mesen sources and the binary go to `$(MESEN)/build/oracle`
(Mesen's `.gitignore` covers `build/`).

```
make -C tools/oracle-mesen -j14 MESEN=/Users/ritz/dev/Mesen2
# -> /Users/ritz/dev/Mesen2/build/oracle/mesen-oracle
```

The first build compiles the whole core (~430 files, about a minute on 14
cores). The two patched copies (see the Makefile):

- `SnesDmaController.cpp`: a GP-DMA channel's byte count is kept in a `uint8_t`
  for the end-of-DMA clock alignment, so DMAs over 255 bytes (MMX's 544-byte
  OAM DMA every frame) end 2-4 master clocks off. Fixed in the copy.
- `Spc.cpp`: `--bsnes-apu 1` sets the SPC/CPU clock ratio of bsnes v0.87
  (24,607,104 Hz SPC master per 21,477,272 Hz CPU master); Mesen's own setting
  is an integer sample rate (32000 + N Hz) that cannot express it.

## Run

```
unzip -p movies/hetfield90v3-megamanx-100p.bk2 'Input Log.txt' > /tmp/v3.txt
mesen-oracle ROM /tmp/v3.txt 119424 mesen.txt --bsnes-apu 1
cargo xtask movie --movie movies/hetfield90v3-megamanx-100p.bk2 --trace ours.txt --png-every 0 --pict-every 0
```

Trace line (both tools): `frame wram-fnv64 $0B9B $0B9E rng x y pad $D1-$D4 level
lives hp $02FD`, then `| ...` extras (Mesen: master clock and PC at the end of
the frame; ours: PC, master clock and PPU position). `$02FD` is the low byte of
the PC the NMI pushed (`A1`/`A4`: which instruction of the idle loop it
interrupted), a cheap per-frame timing fingerprint.

Options: `--dump F|A-B/S,..` + `--dump-dir DIR` (WRAM to `mesen-wram-F.bin`;
ours: `--dump-wram`, `OUT/wram-F.bin`), `--cpu-trace A-B PATH` (every CPU
instruction of frames A..B: PC, scanline, H; Mesen logs after the opcode
fetch, so subtract its 6/8 clocks; ours: `--cpu-trace A-B PATH`, logged before
the fetch), `--spc-trace 1` (SPC700 rows in the same file; ours:
`--spc-trace PATH`), `--ram zero|ones|random` (power-on WRAM, default zero
like ours), `--spc-adjust N` (Mesen's 32000+N Hz SPC rate when not
`--bsnes-apu`).

How it drives Mesen: links the core statically, `Emulator::Initialize`,
`LoadRom` (Mesen runs its own emulation thread at unlimited speed), and an
`IInputProvider` registered on `GameLoaded`. Mesen polls the providers once at
power-on and then at the start of every vblank; poll `p` ends video frame `p`
(the harness logs it there) and sets the pad for frame `p+1`, which the
auto-joypad read of the same vblank latches. The process exits from that
callback after the last frame.
