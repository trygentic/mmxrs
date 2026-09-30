# Zero (player 2): palette and sprites

Code: `crates/mmx-core/src/zero.rs`. Measured on the USA v1.0 cartridge,
2026-09-30. Save states used (git-ignored, in `states/`):
`highway-start`, `highway-mid` (+900 frames), `highway-vile` (Vile fight
starts), `octopus-start` (Launch Octopus, reached through the Highway ending).

## Sprite palettes (CGRAM 128..255)

Sprite palettes are loaded by `$82:8000` (`X` = slot, `Y` = 2 * palette id, table
`$86:8133`, groups of `[count, ptr16, dest]` with the colours in bank `$85`).
The stage init loads id 0 (X, `$85:B700`) into slot 1, id `$12` (HUD) into slot 2
and id `$20` (`$85:B860`) into slot 3. Slots 4-7 are reloaded by enemies.

Highway start (identical after 400 frames of walking, except slot 5):

| slot | colours (BGR555) | user |
|---|---|---|
| 0 | `4215 7FFF 7FFE .. 7FF1` (white ramp) | flashes, 6-80 frames per run |
| 1 | `35AF 739C 239D 091E 7B6F 7A8A 5963 7E00 7900 40C4 42DF 2597 1110 739C 4210 0C63` | **X** (block `+$11 = $62`) |
| 2 | `3DF2 7FFF 3B9F 1EDE 325E 1DDF 087F 5294 7F45 61E5 5903 031F 051A 6318 4631 0C63` | HUD, always |
| 3 | `35D0 7FFF 5BED 3780 3720 2EA0 7FD8 7FA6 7B1F 7F20 38F9 6E9F 555F 7E80 7D80 14A5` | **no sprite seen** |
| 4 | `35D0 4106 69C5 7F00 ..` | enemies |
| 5 | `4A0D 46BF 19BD ..` / `4A6F 1DC7 ..` | enemies (reloaded) |
| 6 | `41AC 4AFF 221D ..` | enemies; Zero's palette in the Vile scene |
| 7 | `4A0D 0084 199F 1084 6B9D 4E74 35CE 04FE 04D5 08CD 4145 7A8D 5963 22FF 117B 0C63` (`$85:BCE0`) | Road Attacker cars |

Frames with a non-P2 visible sprite per palette (`[0..7]`):

| run | frames | usage |
|---|---|---|
| Highway, walk + jump | 3000 | `[10, 2527, 2812, 0, 227, 1927, 399, 135]` |
| Highway, walk + jump + shoot | 2400 | `[80, 2132, 2400, 0, 952, 1261, 198, 67]` |
| Highway, Vile fight + Zero scene + stage select | 5000 | `[78, 3902, 4164, 0, 2271, 603, 1638, 101]` |
| Octopus, walk + jump + shoot | 3000 | `[66, 2443, 2846, 0, 265, 0, 0, 1574]` |

Palette 7 is not free: the Highway cars use it, and on the Octopus stage it is
the busiest enemy palette. With `coop` forcing 7, P2 wears the car colours, and
writing Zero into 7 recolours the cars. **Palette 3 had no sprite in any run.**
No `LDA #imm / STA $11` in the listing sets palette 3 either. What uses id `$20`
is not known; pickups are the likely user, and they would turn Zero-coloured
while P2 is on screen. The title screen was not checked.

## Zero's palette

Zero is on screen in the Highway ending. He is drawn with sprite palette 6
(tiles `$042-$059`):

`3988 0128 7ACD 5963 7FBD 5294 3DEF 055E 04D5 08CD 42DF 2597 1110 271E 1258 14A7`

That is `$85:C0C0`, palette id `$7B`. It has X's layout (skin in 10-12, black in
15). Its red ramp is 7-9, white/greys 4-6 and gold (hair) 13-14.

`zero::palette_for` recolours X's palette:

| X slot | X colour | P2 |
|---|---|---|
| 7-9 | blue `7E00 7900 40C4` (helmet, arms, boots) | Zero red `055E 04D5 08CD` |
| 4-6 | light cyan `7B6F 7A8A 5963` (chest, thighs) | Zero white/greys `7FBD 5294 3DEF` |
| rest | visor, gem, skin, whites, black | kept |

The brief asked for "light blue -> red, cyan -> gold". I tried it, and on screen
it gave an orange X. The real Zero is red with white and grey chest and joints,
and X's tiles have no hair to carry the gold. Compare
`p2_zero_frame40_x5.png` with `real_zero_vile_scene_x5.png` in the scratchpad.

**Upload.** The NMI (`$80:8207` in the listing) DMAs all 512 bytes of `$0300`
into CGRAM when `$A1 != 0`, then clears `$A1`. There is no dirty range, only the
whole buffer. The game's own loaders do `INC $A1` / `STY $A1`.
`zero::write_palette` runs as an `Observe` trap on the OAM build `$80:D544`. It
rewrites P2's slot in `$0300` and sets `$A1` only when the slot differs. That
also repairs the slot after the game reloads palettes. It checked out on the real
ROM: CGRAM matched `$0300` afterwards, and the cars kept palette 7. Fades read
the buffer, so P2 fades with the screen while it is hidden. While P2 is visible,
the trap puts its full colours back.

## X's sprites in VRAM

- OBSEL: sprite tiles at VRAM word `$6000`, gap 0 (tiles `$000-$1FF`,
  `$6000-$7FFF`).
- **X is streamed, not static.** The player update ends with `JSL $84:8FCA`
  (`$81:8185`). When block `+$17` bit 7 is set (the frame changed), that routine
  queues the frame's tiles into the DMA queue `$0500` (8-byte entries:
  `VMAIN, dest word, size, src24`, index `$A3`, budget counter `$1F25`). The
  lists come from `+$31` (`$85:A597` for X, one per frame). The source bank base
  comes from `$7F:8000[+$10]`, and the graphics are uncompressed 4bpp in banks
  `$AD`/`$AE`. The VRAM destination is `+$18 * 16` words, and the emitter adds
  the same `+$18` to every tile number (`$80:D737`).
- One X frame is at most 2 x 512 B (tiles `$00-$1F`, VRAM `$6000-$61FF`). Seen
  queued: `$6000 <- $AD:C1A0`, `$6100 <- $AD:C3A0`.
- X's set (`+$16 = 0`) has **85 frame layouts** (`$8D:8000` -> `$8D:8210..$830F`,
  3 bytes each). Most frames use 5-13 hardware sprites, and frames 65-66 use
  21 and 27. X runs
  36 action states (`+$02`, jump table `$81:82A1`, `$00..$46`). Named from
  play: `00` stand, `06` walk, `08` air, `0A` land, `0C` death, `0E` hurt, `10`
  wall slide, `22`/`44` teleport-in, `1A` stage exit, `2E`/`30`/`32` Vile
  capture. Each has shooting variants.

**Current bug (coop v1).** P2's block is a copy of X's with `+$18 = 0`, so P2
streams into X's window. Whichever player changed frame last owns tiles
`$00-$1F`, and when their poses differ the other player is drawn with the wrong
pose (`p2_shared_tiles_garbled_x4.png`). Setting P2's `+$18 = $40` and `+$17 |=
$80` at anchor time makes the ROM stream P2 into `$6400`, and both players then
draw correctly (`p2_own_tilebase_40_x4.png`, Highway).

**VRAM is not free per stage.** Tiles that were all zero in every sample (`.`),
per 16-tile row:

| stage | free rows |
|---|---|
| Highway (walking part) | `$040-$07F` (64 tiles), half of `$020`/`$030` |
| Highway, Vile fight / Zero scene | none in `$040-$07F` (Vile, then Zero, load there) |
| Octopus | none: `$020-$1FF` all used |

So a fixed window works on the Highway's walking part only.

## Real Zero sprites: routes

Zero has no action set in X1. His cutscene set covers standing, talking and
shooting only. Every X state needs Zero art: 85 layouts, or about 40 poses once
the shooting variants reuse a body. Stand, walk (10+), jump rise, apex and fall,
land, dash, wall slide and kick, hurt, death, teleport in and out, and the
shooting overlays of each (Zero uses a buster in X1 and X2, so the poses match X's).

**A. Stream a second tile set for P2 (ROM path).** P2 gets its own `+$18`
window, its own frame-DMA lists (`+$31`) and layouts (a new set in a `$8D:8000`
table, or a Rust emitter), with Zero's tiles appended to the ROM image
(LoROM banks `$B0+` are free, and the loader controls the image). Per-frame cost
is the same as X: at most 1 KiB of DMA when P2's animation frame changes, about
8k master cycles of vblank, and it shares X's `$1F25` budget. The weak point is
the window: from the table above no stage-wide free 1 KiB exists, so it needs a
per-stage window table, or a reservation in the enemy graphics allocator
(`$7F:8000` slots, not yet reversed).

**A'. Same, with a shadow tile page in the PPU (vendored patch).** An `mmxrs
patch` in `snes-core`: a 1 KiB `p2_page` plus the OAM slot range P2 owns. The
sprite fetch (`ppu/sprites.rs` ~393) reads tiles for those slots from the page.
After P2's update, Rust removes P2's entries from the `$0500` queue and copies
them into the page itself (or copies Zero's tiles from a Rust-side sheet). There
is no VRAM contention on any stage and no emulated cycles. The page goes in the
save state, so rollback netplay keeps working. Palette as now.

**B. HD-pack replacement at the frontend** (the z2rs `hd-packs.md` model). The
frontend composites Zero art over P2's OAM slots, keyed by (set, frame, flip)
from P2's block. That gives any resolution and no VRAM, but it needs both
frontends (native and web) to composite and `mmx-core` to export P2's slots and
frame per frame. Sprite priority against BG has to be re-derived, and any pose
without art shows X's (currently garbled) tiles.

**Recommendation: A'.** It fixes today's garbled-pose bug on every stage and
works at native resolution everywhere (native, web, netplay). The Zero art then
drops in as tiles and layouts. B can come later on top for HD art.
Estimate: PPU page and queue interception, 1 day. Zero importer (`xtask`: PNG
sheet -> 4bpp tiles + layouts + per-frame tile lists), 1 day. Art: 85 layouts,
or about 40 poses with shared bodies. That is days for someone repainting X3
Zero frames into X1's poses, and far more from scratch. Short term, with no art,
set P2 `+$18 = $40` on the Highway (a one-line coop change).

**Where the user can get Zero art (do not commit it).** Rip Zero from your own
Mega Man X3 cartridge, where he is playable with walk, jump, dash, wall and
buster frames in the same engine family and 16-colour sprites. X2 has him in
cutscenes only. The route is a tile/sprite viewer such as Mesen2's. Community
sprite-sheet sites have X2/X3 Zero sheets as reference. The art is Capcom's, so
like the ROM it stays out of the repository (LEGAL.md). Only art you painted
yourself may be shared.
