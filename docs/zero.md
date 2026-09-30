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

**History.** coop v1 gave P2 a copy of X's block with the same `+$18`, so both
players streamed into tiles `$00-$1F` and whoever changed frame last owned them
(`p2_shared_tiles_garbled_x4.png`). v2 set P2's `+$18 = $40` at anchor, which
drew both correctly on the Highway (`p2_own_tilebase_40_x4.png`) but overwrote
enemy tiles elsewhere (see the table below). Route A' (next section) replaced it.

**VRAM is not free per stage.** Tiles that were all zero in every sample (`.`),
per 16-tile row:

| stage | free rows |
|---|---|
| Highway (walking part) | `$040-$07F` (64 tiles), half of `$020`/`$030` |
| Highway, Vile fight / Zero scene | none in `$040-$07F` (Vile, then Zero, load there) |
| Octopus | none: `$020-$1FF` all used |

So a fixed window works on the Highway's walking part only.

## Route A' (implemented): P2's tiles in a shadow page

Code: `crates/mmx-core/src/p2tiles.rs`, PPU patch in
`vendor/jgenesis/snes-core/src/ppu.rs` (`P2Tiles`) and `ppu/sprites.rs`.

- **PPU.** `Ppu::p2_tiles: P2TilesSlot(P2Tiles)`. `P2Tiles` holds `page`
  (`[u16; 768]`, 48 4bpp tiles laid out as three name-table rows of 16; X's
  window is the first two rows, X3 Zero frames use all three; a 16x16
  sprite's `n, n+1, n+16, n+17` stay inside)
  and `slots: u128` (bit n = OAM slot n), plus `next_page`/`next_slots` and a
  `pending` flag. The host writes the `next_*` copies. They go live on the next
  OAMDATA write, which is the game's NMI OAM upload, so page and mask switch in
  the same vblank as the OAM they describe (no mid-frame tearing, and a lag
  frame keeps the old pair). In `fetch_sprite_tiles` a sprite whose slot bit
  is set reads `page[((tile & 0x3F) % 48) * 16..]` instead of VRAM (a
  fourth row, tiles `$30-$3F`, wraps to row 0; no frame uses it; X's window
  must be 64-tile aligned). That is one shift
  and test per sprite, and with `slots == 0` the renderer is unchanged (the
  `verify --coop-off` parity run stays identical).
- **Save state.** The page is inside the PPU, so `to_save_state` clones it.
  Its bincode encoding inside `SnesEmulator` is empty (`P2TilesSlot`), and
  `Game::save_state` appends the marker `P2T5` and the `P2Tiles` after
  `(emu, frame_count, fired_at)`. States written before the patch (no tail)
  or with the old 512-word page (tail without the marker) load with an empty
  page; `P2T2` states (one 768-word page) load it as page 0, `P2T3` states
  (two pages) as pages 0 and 1, `P2T4` states (three) as pages 0-2.
- **Four pages.** `page` holds four 768-word pages back to back; a slot
  also set in `page1: u128` reads the second, one set in `page2` the third,
  one set in both the fourth (`Game::add_p2_tile_slots(mask, page)`, page =
  the two bits). Page 0 is P2's, page 1 a Zero P1's ("Playing as Zero"),
  page 2 the saber slash arcs (P2's in columns 0-7, P1's in 8-15; "The
  saber"), page 3 the HUD's "Z" icon (below). Netplay rollback states wrap
  `Game::save_state`, so they carry the pages.
- **Why a fourth page.** Over all frames, X3 Zero's body uses all 48 cells
  of pages 0/1, and the slash arc (repacked, 4 16x16 + 6 8x8 in its
  biggest frames 3 and 9) all of columns 0-7 in rows 0-1 plus 0-5 of row 2;
  the only free cells, row 2 columns 6-7 / 14-15, cannot hold a 16x16 (its
  lower half, row 3, wraps to row 0).
- **Filling the page.** P2 keeps X's `+$18` (anchor copies X's block and only
  sets `+$17 |= $80` to force a reload), so the emitter writes X's tile numbers
  for P2. Two `Observe` traps on the streamer: at `$84:8FCA`, if the call runs
  inside `call_isolated` (`Game::isolated_depth > 0`) with `D = $0BA8`, i.e.
  P2's update, record `$A3` and `$1F25`. At its common exit `$84:9083`
  (`PLB; PLP; RTL`, reached from both the queued and the empty path) decode
  the entries P2 appended (`p2tiles::decode_entry`), copy each one's ROM
  source into the page at `dest - (OBSEL base + +$18 * 16)` words
  (`page_offset`), and put `$A3` and `$1F25` back. P2's tiles never enter the
  DMA queue: no VRAM writes and no vblank DMA time.
- **Mask.** The OAM build hook `$80:D544` clears the mask each frame. The co-op
  display slice ends with `p2tiles::mark_p2_slots`: every P2 slot whose tile is
  one of the 32 window tiles gets its bit (armour-part slots with other tiles
  keep reading VRAM).
- **Checked on the real ROM.** Highway start (P2 walks, P1 stands): pixel-identical
  to the v2 `+$18 = $40` build at frames 30-120. Vile fight: P2 next to Vile,
  both intact. Launch Octopus, both players running right for 2400 frames: the
  v2 build draws P2's head over the sea enemies' heads (frames 250-2300 differ),
  and A' draws both correctly.
- **Limits.** The page assumes X's window is 32-tile aligned (`+$18 = 0` on
  every stage seen). The armour parts (`$0C38`) are not streamed through the
  page. If they stream, P1's and P2's share VRAM.

**Zero art hook.** `p2tiles::fill_page(game, word_offset, bytes)` is the only
writer of the page. A Zero art pack replaces the ROM copy in
`stream_exit`: for P2's current frame (`+$17`, `+$31` in P2's block) write
Zero's 4bpp tiles for the same slots, or write whole Zero frames with Zero's own
layouts (then the emitter's tile numbers must index the Zero tiles, which is
fine as long as they stay within 32 tiles in the two-row layout). Nothing else
changes: same page, same mask, same save state.

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

**Recommendation: A'** (now implemented, see above). It fixes today's garbled-pose bug on every stage and
works at native resolution everywhere (native, web, netplay). The Zero art then
drops in as tiles and layouts. B can come later on top for HD art.
Done: PPU page and queue interception. Remaining: Zero importer (`xtask`: PNG
sheet -> 4bpp tiles + layouts + per-frame tile lists), 1 day. Art: 85 layouts,
or about 40 poses with shared bodies. That is days for someone repainting X3
Zero frames into X1's poses, and far more from scratch. Until then P2 shows X's
tiles in Zero's colours on every stage.

**Where the user can get Zero art (do not commit it).** Rip Zero from your own
Mega Man X3 cartridge, where he is playable with walk, jump, dash, wall and
buster frames in the same engine family and 16-colour sprites. X2 has him in
cutscenes only. The route is a tile/sprite viewer such as Mesen2's. Community
sprite-sheet sites have X2/X3 Zero sheets as reference. The art is Capcom's, so
like the ROM it stays out of the repository (LEGAL.md). Only art you painted
yourself may be shared.

## X1 Zero data

Code: `crates/mmx-core/src/zero_data.rs` (`extract_x1(rom) -> ZeroSet`,
`zero_frame_for`). Sheet: `cargo xtask zerosheet --out DIR` (one PNG per frame
plus `sheet.png`). The art is Capcom's: render it locally, never commit it.

**Where Zero is.** Highway ending, enemy table slot 1 (`$0EA8`), id `+0A = $33`,
`+11 = $2C` (palette 6, priority 2), sprite set `+16 = $53`, graphics slot
`+10 = $51`, DMA lists `+31 = $85:A96E`, `+18 = 0`. His portrait in the text
box is enemy slot 0: id `$3C`, set `$7A`, graphics `$71`, lists `$85:A8E8`,
`+18 = $20`. From power-on to frame 4200 (Dr. Cain's log, X's spec screen,
the warning text) there is no Zero object and no Zero tile in VRAM; the
attract demo after that was not checked.

**Tiles: compressed, decompressed once, then streamed.** At scene load the
graphics loader (`$80:B230`, the listing's `$80:B271`; queued as a task by
`$80:B252` through `$80:813B`, called per id from `$80:B0DA`; those two are
listing PCs) decompresses
graphics id `$98` into `$7F:0000 + $F8` and records the start in
`$7F:8000[id*2]` (Zero: `$7F:2DA0`). Graphics table `$86:F69B`, 5 B per id:
`size16, src16, bank` (the listing labels it `$86:F6F7`; the cartridge operand
is `$F69B`). Zero, id `$51`: `$2100` B (264 tiles) from `$A3:E4F6`. Portrait,
id `$71`: `$520` B (41 tiles) from `$A6:EAB7`.

Format (Rust `decompress`): per 8 output bytes, a control byte and a fill byte;
control bits MSB first, 1 = next literal byte, 0 = the fill byte. The source
skips from `$xx:FFFF` to `$xx+1:8000`. `decode(ROM)` equals the WRAM buffer
for the first 8012 bytes (the tail was overwritten by a later allocation in the
dump) and equals VRAM `$6400-$65FF` for the stand page (test
`zero_data_extract_x1`).

Then `$84:8FCA` streams it like X: `+31` points at a word table in bank `$85`
(offset per frame, relative to the table); each list entry is 5 B `len/16,
src16 (+ $7F:8000[+10*2]), bank, dest_hi` (bit 7 = last), dest word
`(dest_hi & $7F) << 8 + (+18 << 4)`. Zero's pages go to `$6400`/`$6500`, VRAM
tiles `$40-$5F`. A frame with an empty list keeps the previous page.

**Layouts.** `$8D:8000 + set*3` -> frame table (`$8D:95F9` for set `$53`, 35
frames), `+ frame*3` -> layout: count, then 4 B per sprite `x, y, tile, attr`
(`attr` bit 5 = 16x16, 6 = hflip, 7 = vflip, 1-3 palette ORed with `+11`;
priority and name bit come from `+11`). Emitter `$80:D6BD`: tile `+ +18`,
flipped `x' = -x - 8` (or `-16`).

**Palette** `$85:C0C0` (id `$7B`), see above.

**Frames** (index = set `$53` frame; 35-46 are set `$7A` portrait frames):

| # | name | # | name |
|---|---|---|---|
| 0-3 | dash 1-4 (with dust; the dash-in) | 18 | turn (3/4 view) |
| 4 | stand | 19 | fist |
| 5 | stand blink | 20 | arms up |
| 6 | stand breathe | 21 | leap |
| 7 | aim | 22-23 | clinging 1-2 |
| 8-10 | shoot charged 1-3 (shot ball drawn) | 24-27 | shocked 1-4 (sparks) |
| 11 | shoot recoil | 28-31 | lying 1-4 |
| 12-15 | materialize 1-4 (beam in/out) | 32-34 | kneel 1-3 |
| 16 | beam land | 35-38 | portrait facing right, talk 1-4 |
| 17 | beam | 39 | portrait hurt |
| | | 40-43 | portrait facing left, talk 1-4 |
| | | 44-46 | portrait eyes shut 1-3 |

Highway cutscene sequence (`+17`): dash 0-3, stand 4, aim 7, shoot 8/9/10,
recoil 11, 18, idle 4/5/6 while talking, beam out 13, 16, 17. Frames 19-34 are
the Sigma-stage scene (Zero on Vile's ride armour); they share the set.
Against the fan sheet: no walk cycle (X1 has only the dash), no jump/fall, no
wall poses, no saber; "unused poses" are 18-20, hurt is 24-27, lying/dying
28-31, beam-in 12-17. Buster shots and explosions are separate objects, not
in set `$53`.

**X -> Zero mapping** (`zero_frame_for(action, frame)`), X frames measured on
Octopus: stand 0-2 (blink), stand-shoot 49-50, walk 3-12 (shooting 13-22; corrected, see X3 below),
jump 23-25 (shooting 32-34), fall 25-27 (34-36), land 28-29 (37-38), hurt 51-53,
ready 81.

| X action | Zero frame |
|---|---|
| `$00` stand / `$02` ready | 4, X frame 1/2 -> 5/6, 49 -> 11, 50 -> 7 |
| `$06` jump, `$08` fall | 21 leap |
| `$0A` land | 34 kneel |
| `$0C` death | 28-31 lying |
| `$0E` hurt | 24-27 shocked |
| `$22` beam-in | 17 beam (X's beam-in frame numbers not measured; 12-16 unused) |

Fallback (`None`, draw X's frame in Zero's colours): `$04` walk and
walk-shoot, `$10` wall kick, `$12` wall slide, every other action (dash,
ladders, stage exit `$1A`, Vile capture `$2E-$32`, ...). The dash frames 0-3
are there for a dash action once one is known.

States: `states/zero-arrive.state` (Zero dashing in) and
`states/zero-scene.state` (standing, frame 4 page in VRAM): `xtask dump --load
states/highway-vile.state --frames 3000 --p1tap 8000@1000-3000/40 --save
1950=states/zero-arrive.state,3000=states/zero-scene.state` (B every 40 frames
advances the text). The test reads `MMX_ZERO_STATE` or
`states/zero-scene.state` and skips the VRAM check without it.

## X3 extraction plan

Research only. `roms/` in the main checkout does hold `Mega Man X3 (USA).zip`
and an X2 image; only their headers and hashes were read, plus one peek at the
X3 set table below.

**Gate** (headerless, 2 MiB): title `MEGAMAN X3`, map `$20` (LoROM), type
`$F3` (Cx4), size `$0B`, region `$01`, version 0, checksum `$4055`;
CRC32 `FA0FE671`, SHA-1 `b226f7ec59283b05c1e276e2f433893f45027cac`,
SHA-256 `65b03268afac296330e8ff8d60dd0825879e13ed658b37713c034a3bd074f1d7`.
X2 for reference: `MEGAMAN X2`, type `$F3`, 1.5 MiB, CRC32 `947B0355`.

**What is known.**
- The Cx4 does maths: trig, wireframes, scaling/rotation and sprite
  positioning (command `$00` "sprite functions") [1][2]. Sprite tiles
  are not decompressed by it.
- X3 keeps X1's object engine. Justin3009's Zero Project source addresses
  "sprite assembly" (layouts) in bank `$8F`, pointer tables in `$8D` (Frog
  armour walk at `$8D:96F2`), and per-frame VRAM loads as ROM -> VRAM pairs with
  tile counts, e.g. saber projectile frame 1: ROM `$3AE660` -> `$6580`,
  `$3AE740` -> `$E680` [3]. The table at `$8D:8000` in the X3 ROM is 3-byte
  pointers like X1's (`$0D:8231, $0D:835D, ...`).
- The layout entry order differs: X3 is `attr, x, y, tile` (set 0 frame 0:
  `00 F8 E8 05`, `20 F5 F0 00`), X1 is `x, y, tile, attr`.
- Playable Zero's tiles are streamed per frame from uncompressed ROM like X's
  (the Zero Project data points at raw ROM offsets). Level/enemy graphics use
  X3's LZSS-type compression (Euclid's document [4]); cutscene-only art may
  too.

**Plan for `extract_x3(rom) -> ZeroSet`.**
1. Gate on the hash above.
2. Find Zero's player block the same way as here: a save state with Zero
   playable (X3 lets you call him from a Sub-screen via the Zero capsule once
   per stage), dump WRAM, read `+16/+17/+31/+18` of the player slot.
3. Walk `$8D:8000[set]` for the frame count and each layout (X3 entry order),
   and the `+31` DMA lists for the ROM ranges (5-byte entries, as in X1,
   verified by diffing VRAM after a frame change). Tiles are copied raw.
4. Saber: the swing is part of Zero's own frame set (body + blade sprites);
   the blade projectile / trail is a separate object whose layouts start at
   `$8F:DBA5` [3]. Add both, named from a rendered sheet, then map X actions
   to Zero frames, now with walk, jump, dash, wall and saber frames.
5. If some Zero art (portrait, cutscene) is compressed, port the X3
   decompressor from [4] or trace it the way `$80:B230` was traced here.
6. Test: `decode(ROM) == VRAM` against an X3 state with Zero on screen.

Effort: about 2 days with an X3 ROM on the desk (1 day to find the set and
lists and confirm the entry formats, 1 day for names, mapping and tests),
plus X3 ROM loading in `Game` if states have to come from our emulator (Cx4
is emulated by the vendored core; `Game::from_rom` rejects coprocessors
today, so a separate probe binary or a relaxed loader is needed).

**X2: not supported.** X2 (also Cx4) has Zero in cutscenes and as a boss
(the X-Hunter base fight when his parts are not recovered): buster, dash and
jump frames at best, no saber, no player set, no wall frames. It would give a
few more poses than X1 but needs its own decoder; X3 is the only source of a
full playable Zero.

Sources:
[1] [SnesLab, Capcom Cx4](https://sneslab.net/wiki/Capcom_Cx4),
[2] [superfamicom.org, Capcom Cx4 - Hitachi HG51B169](https://wiki.superfamicom.org/capcom-cx4-hitachi-hg51b169),
[3] [justin3009/MMX3-ZeroProject, MMX3_SpriteSetup.asm](https://github.com/justin3009/MMX3-ZeroProject/blob/master/MMX3_SpriteSetup.asm),
[4] [Euclid, Mega Man X3 Decompression Routine (romhacking.net)](https://www.romhacking.net/documents/205/).

## X3 Zero data

Code: `crates/mmx-core/src/zero_data.rs` (`extract_x3(rom) -> ZeroSet`,
`zero_x3_frame_for`). Sheet: `cargo xtask zerosheet --x3 PATH --out DIR`
(96x88 cells, origin 48,44). Measured 2026-09-30 on Mega Man X3 (USA),
CRC32 `FA0FE671`. The art is Capcom's: render it locally, never commit it.

**Booting X3.** `Game::from_rom_any` skips the title and coprocessor checks
(the vendored core emulates the Cx4 without a firmware file). `xtask dump
--rom-any PATH` drives it; `--poke ADDR=VAL@A[-B]` writes a WRAM byte before
frames A..B. The X1-specific status line `dump` prints is meaningless for X3.

**Calling Zero.** Pause (Start), press R to slide to Zero's monitor, then press
**Start** on that page (the handler at `$80:C9EF` tests `$0A13 & $10`, the Start
bit of the pressed-buttons high byte; Y/B/A/X do nothing). A static monitor
means the swap is refused: `$80:CBB2` refuses when X is in a ride armour
(`$09DA = $2C`), already swapping (`$76`), on level `$1FAE` = 0 (the intro
stage, and also 9, `$0C`, `$0D`), `$1F5D` bit 7, when Zero is already out
(`$0A8E` bit 1 -> X's portrait instead), `$1FE1` bit 0 (Zero used this stage) or
`$1FB2` bit 6 (Zero gone). The states below come from the intro stage with
`$1FAE` poked to 1 during the menu and back to 0 once Zero is in:
`dump --rom-any X3 --load states/x3-intro-play.state --frames 600 --tap 10,150
--poke 7E1FAE=01@1-200,7E1FAE=00@390 --p1tap 0010@80 --save
600=states/x3-zero-stand.state`.

States (git-ignored `states/`): `x3-intro-text` (power-on + 3000 frames, Start
taps 600-1400), `x3-intro-play` (X playable), `x3-zero-stand` (Zero in, frame 0
page in VRAM), `x3-zero-at-wall` (Zero at the wall at x 3545), and one per
action, each saved on the frame named: `-walk`, `-jump`, `-fall`, `-dash`,
`-shoot`, `-charged` (act `$4A`), `-saber` (act `$4C`, frame 88, the arc),
`-wallslide`, `-wallkick`, `-hurt`, `-death`. Scripts: X3 default pad, Y shoot,
B jump, A dash; hold Y ~150 frames for the charge.

**Player block** `$7E:09D8` (X's and Zero's; X1's is `$0BA8`): `+02` action,
`+03` sub-state, `+05` x, `+08` y, `+11` attr (bit 6 = facing right), `+16` set,
`+17` frame, `+18` VRAM slot (0), `+27` HP (`$09FF`), `+31` DMA list table
(bank `$85`), `+6B` invulnerability timer (`$0A43`). `$0A8E` bit 1 = Zero is
the current character.

**Sets.** Zero playing: set `$4A` (117 layouts, table `$0D:99A4`), lists
`+31 = $85:D6A8`. Death switches to set `$4B` (21 layouts, `$0D:9B03`), lists
`$85:DB47`, frame 18. The `$8D:8000` pointers are `$0D:xxxx`, layouts
`$0F:xxxx` (mirrors of `$8D`/`$8F`). Layout: count, then `attr, x, y, tile`
per sprite (attr bit 5 = 16x16, 6 = hflip, 7 = vflip; palette bits are 0, the
palette comes from `+11`). Checked against OAM in the stand state: tile `$0B`
at (-22,16), flipped `x' = -x - 8`, origin (96,167).

**Tiles: raw, streamed per frame, up to 48.** List table: a word offset per
frame (16-bit, wraps inside bank `$85`: set `$4B` frames 18-20 point back into
`$D9xx`), then 6-byte entries `len/16, src24, dest16` (bit 15 of dest = last),
e.g. frame 0: `1A 008031 0060`, `18 A08131 00E1` = 13 tiles `$B1:8000 -> $6000`,
12 tiles `$B1:81A0 -> $6100`. Offset pointing at a lone `00` = keep the page.
Sources are uncompressed 4bpp in banks `$B0`/`$B1` (the `$31`/`$30` bank bytes are
mirrors). Destination `$6000 + 16 * slot`, OBSEL base `$6000`, `+18 = 0`.
decode(ROM) == VRAM `$6000-$62FF` for the stand state (test
`zero_data_extract_x3`; `MMX_X3_ROM`, `MMX_X3_ZERO_STATE` or
`states/x3-zero-stand.state`). **Many frames load a third row** (`$6200`, tiles
`$20-$2F`): dash, charged shot, saber, beam-in; frames 65-66 use 48 tiles, and
layouts reach tile `$2C`. So `ZeroSet` pages are 48 slots for X3 (`page()`
returns 1.5 KiB) against 32 for X1; the shadow page has the third row for it. 1918 distinct tiles over both
sets.

**Palette** `$8C:B580` (in CGRAM as sprite palette 1 while Zero plays):
`2188 1284 728C 5100 639C 5294 318C 009C 0094 008C 2194 429C 2388 031C 0194 1084`.
The charge flash swaps in a purple palette.

**Frames** (set `$4A` 0-116, then set `$4B` as 117-137), from `+02/+17` logs:

| # | name | # | name |
|---|---|---|---|
| 0-2 | stand, blink 1-2 (act `$00`) | 54-59 | dash start, dash 1, 3, end, dash shoot 1, 3 (`$14`, end `$20`) |
| 3-12 | walk 1-10 (`$04`) | 60-61 | beam, beam land (`$22`) |
| 13-22 | walk shoot 1-10 | 62-66 | materialize 1-5 (played 66 -> 62) |
| 23-25 | jump 1-3 (`$06`) | 67-74 | ladder (unverified) |
| 26-27, 101 | fall 1-3 (`$08`), 100 = apex | 75, 76, 80 | blank |
| 28-29 | land 1-2 (`$0A`) | 77-79 | hurt alt (tiles of 51-53, unseen) |
| 30-31 | stand weak (unverified) | 81 | ready (`$02`) |
| 32-38, 102-103 | jump/fall/land shoot | 82-85 | charged shot (`$4A`) |
| 39-41 | wall slide 1-3 (`$12`) | 86-90 | saber 1-5 (`$4C`), 88 = arc |
| 42-43 | wall kick 1-2 (`$10`) | 91-94 | air charged shot (`$4E`) |
| 44-48 | wall slide/kick shoot | 95-98 | air saber (`$50`) |
| 49-50 | stand shoot | 99 | stand alt |
| 51-53 | hurt 1-2, hurt flash (`$0E`) | 104-111 | dash 2, 4-6, dash shoot 2, 4-6 |
| | | 112-116 | alt charged/saber poses (unverified; 113, 115 are fragments) |
| 117-134 | set `$4B` event poses (stand, point, leap, kneel) | 135-137 | death (`$0C`, 135), death 2, pieces (fragment) |

Dash loop: 54, 55, 104, 56, 105, 106, 107, 56...; with the buster held the
shoot twins (58, 108, 59, 109-111) run instead.

**The saber.** In X3 Zero has no plain slash: holding Y charges; release fires
the big green buster shot (act `$4A`, frames 82-85, the projectile is its own
object), and Y again swings the saber (act `$4C` 86-90 on the ground, `$50`
95-98 in the air). The blade is part of the body frames (84, 88, 93, 97: a
blue-white arc). The green wave is a separate object, but it comes with the
*release* (act `$4A`/`$4E`, on the blade flourish 84/93), not with the `$4C`
swing (which fires a blue buster ball, set `$4E`, on frame 89). Extracted as
`ZeroSet.wave`, see "The green wave" below (no longer used). `$8F:DBA5` (the
Zero Project's "Z-Saber projectile") is set `$4F` frame 12: X's flying saber
wave. Set `$4F` frames 0-11 are the green slash arc the port draws on
Zero's swing (`ZeroSet.slash`, "The saber" below).

**X1 -> X3 mapping.** X1's set 0 uses the same frame numbers as Zero's set
`$4A` (measured on X1 from `hw-c` with jumps and shots, and the beam-in from
`gameover`): walk 3-12 (shooting 13-22), jump 23-25 (32-34), fall 25-27
(34-36), land 28-29 (37-38), wall slide 39-41 (44-46), wall kick 42-43 (47-48),
stand shoot 49-50, hurt 51-53, beam-in 60, 61, 66..62. (The X1 section had
walk/jump and their shooting twins swapped; corrected there.)
`zero_x3_frame_for(action, frame)`: death `$0C` -> 135; X frames
0-74 and 81 map to themselves; others fall back to the action's base pose
(stand 0, walk 3, jump 23, fall 26, land 28, hurt 51, wall kick 42, wall slide
41, dash 55, beam 60), else `None`.

**Unfinished.** Ladder/weak/alt frames
are named from the sheet only; X1 frames 75-80 and 82-84 have no measured
meaning, so they fall back.

## Drawing P2 as X3 Zero

Code: `zero::emit_x3` (called from the co-op display slice), `Game::set_zero_set`
/ `Game::set_x3_rom`. With a set loaded the display slice does not run the ROM
emitters (`$80:D640`/`$80:D671`) for P2; the Rust emitter writes P2's OAM
entries into the same slots (from `$E3` rounded up to 4, padding hidden):

- frame `f = zero_x3_frame_for(+02, +17)` (an action X3 has no pose for:
  stand, frame 0); nothing drawn while `+0E = 0` (the i-frame blink).
- origin `(+05 - $1E4D, +08 + (i8)+19 - $1E50 - 8)`: the ROM emitter's
  (`$80:D67C`) plus `X3_Y_OFFSET = -8` (Zero's X3 stand frame has his feet at
  `+24`, X's at `+16`, in both ROMs; P2 runs X's physics).
- per sprite (`zero::oam_entry`, the ROM's `$80:D6EA`/`$D73F`): facing right
  (`+11` bit 6) mirrors `x' = -x - 8` (or `-16`) and toggles hflip; culled when
  `x + 16 >= $10F` or `y + 15 >= $EF` (unsigned); attr = `+11 & $30`
  (priority) | palette 3 | the layout's flips; high table: x bit 8 and size,
  P2 owns whole bytes. Checked against the ROM emitter on X's own layouts in
  both facings (test `zero_emitter_matches_rom`, walk/jump from `highway-start`).
- tiles: `set.page(f)` (48 tiles, 1.5 KiB) into the shadow page when `f`
  changes (`CoopState::zero_frame`, reset by `Game::load_state`); every P2
  slot gets its mask bit. The X1 streamer interception still runs for P2 but
  copies nothing (`p2tiles::stream_exit`).
- palette: `ZeroSet.palette` into sprite palette 3 through `$0300`/`$A1`
  (replaces `palette_for`). X's armour parts are not drawn on Zero.

Checked on the real ROMs (PNGs in `target/z/`; `--x3 PATH` or `MMX_X3_ROM`,
`xtask` = `cargo run --release -q -p xtask --`):

```
xtask dump --load states/highway-start.state --frames 130 --out target/z/walk --at 20,50,58,96,110,120 --p2-hold 0100 --p2tap 8000@90:20 --coop   # walk 8/10/11, jump 23-25, fall 27
xtask dump --load states/highway-start.state --frames 50 --out target/z/dash08 --at 40,44,48 --poke 7E1F99=08@1-50 --p2tap 0080@38:12 --coop   # $1F99 bit 3 = legs: dash 55/56
xtask dump --load states/hw-c.state --frames 500 --out target/z/wall --at 340-440/20 --hold 0100 --p1tap "$T" --p2-hold 0100 --p2tap "$T" --refill --coop   # T = coop-qa's m5 route: wall slide 39/41, kick 42/43
xtask dump --load states/hw-spiky.state --frames 60 --out target/z/hurt --at 32,34,38,44 --p2-hold 0100 --coop   # hurt 51-53
xtask dump --load states/hw-pit.state --frames 100 --out target/z/pit --at 60,75,85 --p2-hold 0100 --coop   # wall slide down the pit, then down
xtask dump --load states/octopus-start.state --frames 300 --out target/z/octo --at 100-300/50 --hold 0100 --p2-hold 0100 --p2tap 8000@140:20 --coop
```

`hw-spiky`/`hw-pit` are made as in docs/coop.md. Death: P2 hides on the
frame it goes down (co-op's rule), so Zero's death frame 135 is never shown.
P2 still sorts behind every game sprite (behind X and enemies). A dump PNG
after step N shows the OAM and page built in step N-1 (the NMI uploads them
at the end of the frame), so the `zf` printed next to it is one frame ahead.

## The saber

Code: `crates/mmx-core/src/saber.rs`, state `CoopState::saber` (rollback
carries it). Only with the X3 set; without it X does nothing (buster-only Zero).

- **Swing.** Pad 2's X (`$0040`, unused by X1's default button config) starts
  a swing when P2 stands, walks, lands or dashes (actions `00 02 04 0A 14`:
  X3 frames 86-90) or jumps/falls (`06 08`: 95-98), 4 frames per pose, the
  arc pose (88 / 97) `ARC_FRAMES` = 10 (X3 holds it 5; playtesters could not
  see the blade at 67 ms, half of it lost to the i-frame blink). X3's own
  `$4C`: 86 1 frame, 87 2, 88 5, 89 1, 90 17; `$50`: 95 2, 96 5, 97 1, 98 13.
  No swing on walls or while hurt; hurt or death cancels it.
- **Mechanism: P2's own update keeps running with a masked pad.** On the
  ground the ROM gets no buttons, so X's code stops Zero (stand); in the air
  only left/right pass, so the jump arc and air control are the ROM's. The
  display shows the saber pose instead of the ROM frame. No X1 action was
  borrowed: `$0A` (land) and hurt without knockback both change physics or
  i-frames, the mask doesn't. Y (buster) works outside swings.

### The green slash arc

The user: "the green slashing arc when Zero slashes enemies with his
saber" (the fan sheet's large green crescent along the blade, about 40 x 40,
hugging Zero's front from head height down).

**X3 draws no green arc on Zero's swing.** From `x3-zero-stand` (hold Y
150 frames, release, Y again at 185; `dump --rom-any X3 ... --at 150-230/1
--wram --vram --p1tap 4000@10:150,4000@185:2`), per frame the shot table
`$10D8` and OAM palettes: the swing (`$4C` 86-90) adds sprites only in
Zero's own palette 1 (7 -> 9 -> 12 -> 14 sprites: the blue-white blade in
the body layouts), and the only object it spawns is set `$4E` (id 3, the
blue/yellow buster ball, OAM palette 3) on frame 89, 34 px in front, flying
at 8 px a frame. A single Y tap or a double tap is the plain buster (id 0,
set `$08`). Strips: `target/z/slash_x3_own_swing_4x.png`,
`slash_x3_own_air_4x.png`.

**The arc is set `$4F`** (X's beam-saber slash, the neighbour of Zero's
sets): table `$0D:9BB4`, layouts `$0F:DA99..`, lists `+31 = $85:E5EE`
(written by the shot init at `$81:8E6x`, which spawns frame 14, X's flying
wave, with box `$86:B85F`), tiles raw 4bpp from `$BA:E000` (`$3AE000`),
streamed to VRAM `$6300-$66xx` (tiles `$30-$6F`: up to four rows). Frames:

| # | what | drawn box (unflipped = facing left, from the origin) |
|---|---|---|
| 0-1 | blade start | x 2..23, y -22..-1 |
| 2 | rising arc (over the head) | x -19..22, y -39..-11 |
| 3 | full crescent, in front, head to waist | x -48..2, y -37..3 |
| 4-5 | tail near the feet | x -32..-16, y -3..9 |
| 6-11 | the same, 5 px further forward, 2 up (used for the air swing) | |
| 12-15 | X's flying wave | |
| 16-19 | sparks | |

Drawn at the wearer's origin, frame 3 lies on Zero's blade in pose 88 and
frame 9 in 97 (no offset needed). Checked in the engine: poking Zero's
block to set `$4F` (`--poke
7E09EE=4F@5,7E0A09=EE@5,7E0A0A=E5@5,7E09EF=83@5`, saved after frame 8 as
`states/x3-zero-slash.state`) streams frame 3 into `$6300-$6662` and draws
it at Zero's place; test `zero_data_extract_x3_slash` compares the
extraction with that VRAM. `extract_x3_slash` repacks each frame into 8
columns x 3 rows (16x16 sprites in rows 0-1, 8x8 in the rest; frame 3 needs
22 tiles) so it fits half of shadow page 2. Sheet: `xtask zerosheet --x3
PATH --slash --out DIR`.

**Palette** `$8C:A460` (`2184 73BC 5394 3B8E 2308 1284 731C 5A3A 728A 3098
7394 730F 4559 7205 7180 18C6`): the arc draws with 1-5 (white, four greens)
and 15. It is the only `$8C` palette with greens in 1-5, and it is X3's
sprite palette 3 while X plays (`x3-intro-play` CGRAM), the effects
palette. X's saber itself was not reached in our states (it needs Zero's
saber handed over), so the pairing is inferred; in Zero's own palette the
arc comes out blue and white. **Remapped onto Zero's palette**, as the wave
was (no sprite palette is free with co-op on: 0 flashes, 1 X or a Zero P1,
2 HUD, 3 P2's Zero, 4-7 enemies): `zero::SLASH_ON_ZERO` 1 -> 4 (`639C`
white), 2-4 -> 12 (`2388` green), 5 -> 1 (`1284`, the same dark green),
15 -> 15. It reads as a green crescent with white bands. No CGRAM writes.

**Timing** (ours; X3 has none for Zero): the arc pose shows frame 2 for 2
frames, then 3 for 8; the pose after it (89 / 98) the tail 4, 5 for 2
frames each; air + 6 (`saber::slash_frame`).

**Drawing.** `zero::slash_entries`: at the wearer's drawn origin (the
body's, `X3_Y_OFFSET` included), mirrored by his facing, on top of the body
(P2: in its slots before the body; P1: at the build cursor before the
body), blinking with the body. Tiles in shadow page 2 (P2 columns 0-7, P1
8-15; `CoopState::wave_frames` tracks the loaded frame, the name is
historical).

### Damage

A shot that only the enemy loop sees. On the arc pose (88/97), after the
shot loop (`$80:D1FF`, the instruction after `JSR $80:D3B2`; trap
`coop_saber_shot`) a shot is written into a free `$1228` slot: `+00 = 1`,
id `+0A = 2`, position `+05/+08` = the crescent's centre (23 px toward the
facing; `+08 + (i8)+19 - 8 - 17`; air 28 and `- 19`), `+11` = the
player's, box `+20 = $86:BB34` (`00 00 18 17`, 48 x 46 px against the
drawn 50 x 40; X1's bank has no closer box), `+30 = 0`. Enemies test it in
their AI with the ROM's enemy-vs-shot routine `$84:9B43` (shot active,
`+30 = 0`, box test `$84:9C0E`), and `$84:9E15` applies the damage: the
enemy's row `$86:EEDB[+28]` at the shot id. Id 2 is X's full charge
(Highway row 3: lemon 1, half charge 2, full charge 4). The physics hook
takes the slot out again (zeroed), after the enemy loop. The shot's update
(`$80:F666[+0A]`) never runs, it is never drawn and never counted, so the
`$0BDD` bookkeeping (docs/coop.md "Shots (B3)") never sees it. One hit per
swing: the damage routine does `INC +30` on the shot, or the enemy sets
`+01 = 6`; `take_back` counts either.

Checked:

- `hw-spiky`, P2 walks right 20 frames, X at 22: the arc's first frame hits
  the Spiky with its centre 55 px in front of Zero (the tip reaches 48),
  2 -> 1, once (test `saber_hits_spiky`; P1's `$0BDD` stays 0).
- `hw-spiky`, P2 swings where it stands, the Spiky 88 px away on the arc's
  first frame: no hit (test `saber_misses_spiky_from_afar`; the old wave
  took it from there).
- `hw-bee2`, P2 jumps every 40 frames and taps X 8 frames into the jump:
  Bee Blader (`$0E68+$27`) 29 -> 25 -> 21 -> 17 -> 13 -> 9 -> 5 -> 1, 4 per
  swing, while it drifts from x 3435 to 3363 (P2 at 3452; its box is large).
- P1 as Zero: test `saber_p1_hits_spiky`; rollback:
  `saber_frames_survive_rollback`.

Strips (4x, `target/z/`): `slash_p1_ground_4x.png`, `slash_p1_air_4x.png`
(`--zero`), `slash_p2_ground_4x.png`, `slash_p2_air_4x.png` (`--coop`, P2
next to X), `slash_x3_own_swing_4x.png`, `slash_x3_own_air_4x.png` (X3's).
`x` = `xtask dump --x3 PATH`:

```
x --load states/highway-start.state --frames 44 --out DIR --at 10-44/1 --p1tap 0040@10:2 --zero
x --load states/highway-start.state --frames 50 --out DIR --at 10-50/1 --p1tap 8000@10:20,0040@16:2 --zero
x --load states/highway-start.state --frames 44 --out DIR --at 10-44/1 --p2tap 0100@1:12,0040@16:2 --coop
x --load states/highway-start.state --frames 50 --out DIR --at 10-50/1 --p2tap 0100@1:12,8000@14:20,0040@20:2 --coop
T=$(seq 10 40 1190 | awk '{printf "8000@%d:20,0040@%d:2,", $1, $1+8}' | sed 's/,$//')
x --load states/hw-bee2.state --frames 1200 --out target/z/bee --at 50-1200/50 --refill --coop --p2tap "$T" --wram
```

### The green wave (not used by the swing)

**Not drawn and not thrown any more**: the user meant the slash arc above,
not a projectile. The extraction (`ZeroSet.wave`, `x3_wave_step`) and its
test stay. ponytail: the wave exists in the ROM (set `$4D`, below) and
could become a charged-saber move later (X3 throws it on the charge
release, act `$4A`/`$4E`).

What follows is the earlier port, kept for the measurements.

**In X3** (`states/x3-zero-stand.state`, hold Y 150 frames, release; states
`x3-zero-wave` = that run's frame 173, wave in flight):

- Object in X3's shot table at `$10D8` (stride `$40`; first free slot), id
  `+0A = 1`, set `+16 = $4D` (table `$0D:9B60`, layouts `$0F:D92E..`),
  lists `+31 = $85:DD84`, `+18 = 8`, `+11 = $66` (palette 3, facing right).
  Spawned on the release's blade pose (84 ground, 93 air), 1 frame after the
  pose appears. Zero's `$4C` swing spawns set `$4E` (a blue buster ball) on
  frame 89 instead; no green there.
- Flight, relative to Zero's origin at the spawn (`zero_data::x3_wave_step`):
  `t0 (+8, -8)` frame 0; `t1 (+29, -5)` frame 0; `t2 (+29, -2)`, `t3 (+29,
  +1)` frame 1; then +6 px a frame, frames 3, 4, 2, 3, ... In the air: `+27`
  and it keeps its height (`-8`); on the ground it drops to `+1`. Gone after
  31 frames (`+02 = 4` at t = 31, about 197 px out: off screen). `+30` stays 0.
- Hit box `+20` (bank `$86`, `x y half-w half-h`, x toward the facing): t0
  `$B7FB` = `00 00 04 04`, t1 `$B807` = `FF 00 10 09`, then `$B80B`/`$B80F`
  = `F1 00 10 09` (32 x 18 px, centred 15 px in front of the object).
- Tiles: streamed per frame like Zero's (6-byte entries; frame 0: 6 tiles
  `$B2:E000 -> $6300`, 6 `$B2:E0C0 -> $6400`, + `+18 * 16` words), raw 4bpp in
  bank `$B2`; layout tiles `$30-$35` / `$40-$45` (OAM `+8`), at most 10 tiles,
  4 sprites, 48 x 16 px. Frames 5-6 exist but were never seen.
- Palette: X3 sprite palette 3, static, `$8C:B5A0` (right after Zero's):
  `4108 739C 738C 7308 7284 7200 5394 338C 2308 2288 439C 039C 029C 731C 621C 1084`.
  The wave draws with 1 (white) and 6-9 (greens) only.

Sheet: `xtask zerosheet --x3 PATH --wave --out DIR`. It is a green blast
with a crescent head and a ringed tail, not the fan sheet's 48 x 32 arcs.
Test `zero_data_extract_x3_wave` (decode == VRAM `$6380`/`$6480` in
`x3-zero-wave`).

**In the port (removed).** `Saber.wave` (`saber.rs`, so rollback carries it): the
first frame of the arc pose (88 / 97) throws it from Zero's drawn origin,
then it follows `x3_wave_step` for `WAVE_FRAMES` = 31 frames, past the end
of the swing; a new arc replaces it. Drawn by `zero::wave_entries` in front
of Zero (P2: before its body in its slots, P1: before its body at the build
cursor), mirrored by the facing at the spawn, tiles in shadow page 2 (P2
columns 0-7, P1 8-15; `CoopState::wave_frames` tracks what is loaded).
Drawn during the i-frame blink too.

**Palette: remapped onto Zero's.** No sprite palette is free with co-op
on: 0 flashes, 1 X (or a Zero P1), 2 the HUD, 3 P2's Zero, 4-7 enemies
(table above). Zero's own palette has white and two greens, so the wave's
five colours map onto it (`zero::WAVE_ON_ZERO`: 1 -> 4 `639C`, 6/7 -> 12
`2388`, 8/9 -> 1 `1284`) and it draws with its wearer's palette (P1: 1, P2:
3). No CGRAM writes.

**Damage (removed).** The wave was its own shot (`saber::wave_shot_bytes`, slot
`Saber::wave_slot`, same insert/take-back as the blade's): id 2, position =
the box centre, box `$86:BDC1` (`00 00 04 04`) on t0 and `$86:D381` (`00 00
10 09`, X3's 32 x 18) after. One hit per swing, blade and wave together:
`take_back` now also counts `+01 != 0` as a hit (the enemy sets `+01 = 6`
and leaves `+30 = 0` on the Spiky; before this the Spiky took 2 hits from
one swing once the wave passed through it). Checked:

- `hw-spiky`, P2 swings where it stands, 88 px from the Spiky: 2 -> 1 at
  frame 16, once (test `saber_wave_hits_spiky_from_afar`).
- `hw-bee2` (the run below): Bee Blader 29 -> 25 -> 21 -> 17 -> 13 -> 9 ->
  5 -> 1 -> 0, 4 per connecting swing, hits from 17 px out to 101 px (P2 at
  x 3452, Bee 3435 -> 3351); the blade alone stopped at 9 when it drifted
  out of reach.

Strips (`target/z1/`, `target/z/`): `z1/wave_strip_3x.png` (P1 ground,
frames 16-42 every 2), `z1/wave_air_strip_3x.png`, `z/wave_strip_3x.png`
(P2), `z/x3_wave_strip_3x.png` (X3's own, frames 164-190); `x` = `xtask
dump --x3 PATH`.

```
x --load states/highway-start.state --frames 44 --out target/z1/wave --at 16-44/1 --p1tap 0040@10:2 --zero
x --load states/highway-start.state --frames 50 --out target/z1/wave_air --at 18-48/1 --p1tap 8000@10:20,0040@16:2 --zero
x --load states/highway-start.state --frames 44 --out target/z/wave --at 16-44/1 --p2tap 0040@10:2 --coop
xtask dump --rom-any X3 --load states/x3-zero-stand.state --frames 260 --out DIR --at 150-260/1 --wram --vram --p1tap 4000@10:150,4000@185:2
```

## Sounds

With the X3 ROM loaded, Zero's saber (P1 `--zero` and P2) plays X3's
whoosh. Nothing else: **X3 has no Zero-specific hurt or death sounds.**

**What X3 plays** (`xtask dump --rom-any X3 ... --apu` prints the DSP's
sounding voices per frame: `SRCN`, `PITCH`, `ENVX`, ADSR/GAIN, volume, KON;
`--mute HEX` silences voices in the output; `--wav PATH` writes the run's
audio at 32 kHz). SPC sample directory `DIR = $39` (`$3900`). Sound effects
are on voice 0 (music takes it back when free: the src `$19` notes on voice
0 are music, not SFX).

- **Saber.** From `x3-zero-stand`: hold Y 150 frames (charge), release
  (act `$4A`, the charged shot: voice 0 src `$0E` 1 frame later), Y again
  after that sound ends (frame 215: act `$4C`, poses 86-90). The swing
  itself keys on nothing; at pose 90 (10 frames in, frame 225, when he
  throws the ball, set `$4E`) voice 0 plays src `$0E` again: that is the
  sound heard on X3 Zero's slash. With the second Y at 185 (inside the
  charged shot's sound) X3 drops it entirely. Src `$0E`: start `$415A`,
  loop `$4190`, 8 BRR blocks (128 samples, the last 32 loop; a short bump
  then noise), ROM `$89:AF25` (file `$4AF25`, the bytes in SPC RAM are the
  ROM's, test `x3_whoosh_matches_spc_ram`). Volume `$1E`/`$1E`, ADSR `$FF
  $E0` (instant attack, full sustain), pitch swept per frame `$02A0` up to
  `$0AA0` (21 kHz sample rate) at frame 6, then down to `$0030` (375 Hz)
  at frame 38, key-off after 42 frames (GAIN `$BF`, ~2 ms fade). The
  charged shot is the same sweep.
- **Hurt / death.** Walking into the intro stage's first enemy from
  `x3-intro-play` (X) and `x3-zero-stand` (Zero), `--hold 0100`: hurt at
  frame 205 and every 64 frames after, death at 860 (X) / 1244 (Zero).
  Both characters get the same voice-0 sequence: hurt = src `$0E` (pitch
  `$01C0` falling, 8 frames) then src `$0C` (`$0340` rising to `$0C40`,
  16 frames), volume `$4E`; death = that, then src `$04` on voices 0-2
  (pitches `$21E0`/`$3FD0`/`$21B0` falling, repeated with falling volume).
  No voice clip, nothing Zero-only. So X1's own hurt/death sounds stay
  (nothing is stripped from X1's queue `$0B72`).

**What ships.** `zero_data::extract_x3_sounds` (from `extract_x3`, so the
ROM's title and CRC are checked) decodes the 8 blocks with a BRR decoder
(`brr_decode`, the vendored DSP's shift/filter/15-bit math; test
`brr_filters`) and renders the whoosh once: the measured pitch table
`X3_WHOOSH_PITCH` (42 NTSC frames, 12-bit fraction as the DSP), linear
interpolation plus a `[1 2 1]/4` smoothing for the DSP's Gaussian, a 64-
sample fade, gain 80/256; integer math, so both netplay peers get the same
`Sample { pcm, rate: 32000 }` (22427 samples, 0.7 s) in `ZeroSet.sounds`.
Against X3's own voice 0 (`--mute FE`, the ball at frame 225) the render
has the same RMS (1983 vs 1987) and its spectral-centroid track correlates
0.97 (ours ~1.3x brighter: the Gaussian is duller than the smoothing).

**Mixer** (`sfx.rs`). `CoopState.sfx: [u16; 2]` = frames into the whoosh
+ 1 for P2 and P1 (0 = silent), so rollback carries it. `saber::whoosh`
(the arc pose's first frame, 8 frames into our swing; X3's comes 10 frames
in) sets it to 1 after `saber::update` (P2's slice, P1's `$80:E518` trap);
a new swing restarts it, as X3's driver re-keys voice 0. At the end of
`Game::step`, `sfx::mix` adds each playing slot's frame of samples (`rate /
fps` of them, so PAL paces it in wall time too) to that frame's output
(linear resample to the device rate, clamped to +-1) and advances it. Replayed
netplay frames mix the same and their audio is dropped as before; a game
without the X3 set never mixes (`sfx` stays 0). Tests: `sfx_mix_paces_and_ends`,
`sfx_whoosh_in_game_output` (hw-spiky, P1 swings at 28: the output's
cross-correlation with the sample peaks at 0.69 exactly where the arc
frame's audio starts; a run with `ZeroSet.sounds = None` differs only from
there, by the whoosh), `saber_frames_survive_rollback` (audio equal too).

Note: a save state carries the APU's output rate, so `set_audio_rate` goes
after `load_state`.

```
x3 = xtask dump --rom-any X3 --apu --wav OUT.wav
x3 --load states/x3-zero-stand.state --frames 320 --out DIR --at 150-320/1 --wram --p1tap 4000@10:150,4000@215:2 [--mute FE]
x3 --load states/x3-intro-play.state --frames 900 --out DIR --at 1-900/1 --wram --hold 0100    # X hurt 205.., death 860
x3 --load states/x3-zero-stand.state --frames 1500 --out DIR --at 1-1500/1 --wram --hold 0100  # Zero hurt 205.., death 1244
xtask zerosheet --x3 X3 --sfx --out target/sfx      # target/sfx/swing.wav
xtask dump --load states/hw-spiky.state --zero --frames 150 --out target/sfx/dump --p1tap 0100@1:26,0040@28:2,0040@70:2,0040@110:2 --wav target/sfx/p1_zero_swings.wav
```

## Playing as Zero

Supply your own Mega Man X3 (USA) image (CRC32 `FA0FE671`, copier header
ok); it is only read, never stored, and like the X1 ROM it stays out of the
repository:

- native: `mmxrs --rom mmx.sfc --x3-rom mmx3.sfc --coop-local` (or
  `MMX_X3_ROM`);
- web: the "Mega Man X3 ROM (optional, enables Zero's sprites and saber)"
  picker (`Emu::set_x3_rom`);
- scripted runs: `xtask dump --x3 PATH` (or `MMX_X3_ROM`).

What changes: P2 is drawn with X3 Zero's frames and palette (walk, jump,
dash, wall, hurt, beam-in, ...) instead of X's tiles in Zero's colours, and
P2's X swings the saber (above). Physics, HP, hit boxes and the buster are
X's. Netplay: both peers must load it; the handshake id includes "X3 loaded",
so a mismatch refuses to start (docs/netplay.md).

### The HUD icon

With X3 loaded, the life bar icon of a Zero (P2's, the down marker, and a
Zero P1's own, OAM slot 0) is X3's own "Z" emblem instead of X's "X"
(`coop_hud.rs`; placement of P2's bar: docs/coop.md "P2 HP bar and
revive"). X3 draws it while Zero plays (`x3-zero-stand`): OAM slot 0, X 8,
Y 80, tile `$86` 16x16, attr `$34` (palette 2). Tiles `$86 $87 $96 $97`
(sprite base `$6000`) are uncompressed in the ROM at `$AC:8D20`,
`$AC:8D40`, `$AC:8DE0`, `$AC:8E00`; the HUD palette (CGRAM 160..175) is at
`$8C:B0E0` (also `$8C:E828`): `3DF2 7FFF 5FBF 1F1F 325E 1D9F 0CBB 5294
7F45 61E5 5903 031F 555F 6318 4631 0C63`. `zero_data::extract_x3_hud_icon`
-> `ZeroSet::hud_icon` (tiles + palette); test `zero_data_x3_hud_icon`
checks both against VRAM and CGRAM.

The emblem draws with 1 (white plate), 13-15 (greys), 11 (yellow body) and
6 (red stroke ends). `coop_hud::HUD_ON_ZERO` remaps it onto Zero's palette
(which P2's palette 3 and a Zero P1's palette 1 hold): body -> red 7, ends
-> gold 13 (all red read as an X at 16 px), plate 4, greys 5, 6, 15. The
four tiles go into shadow page 3 at cells 6, 7, 22, 23, where the PPU's
`(t & $3F) % 48` puts tile `$86`, so the OAM entries keep X's tile number
and only their slots are flagged for page 3 (`add_p2_tile_slots(.., 3)`).
Written every HUD frame (64 words), so loaded states need nothing. P1:
slot 0 gets palette 1 and the flag each frame the bar is shown. Without X3
nothing changes: X's icon, recoloured to palette 3 for P2.

### Player 1 as Zero (`--zero`)

Code: `zero::ZeroP1` (in `CoopState`, so rollback carries it), `Game::set_p1_zero`,
the traps in `zero::register_zero_traps`. Needs the X3 set (`set_p1_zero(true)`
errors without it); registers the co-op trap group, whose P2 hooks stay idle
while co-op is off. Works alone, in local co-op and online (both peers pass
`--zero`; the handshake id has a "P1 is Zero" byte). A game without `--zero`
is unchanged (`verify --coop-off` identical, `coop-qa.sh check` idle parity
yes at every milestone).

- **Drawing.** The OAM build calls the armour-part emitter `$80:D640` and
  the player emitter `$80:D671` by `JSR`. Both get an `Observe` trap that,
  outside `call_isolated` (P2's own calls still run), returns for the ROM
  (`RTS` by hand): `D640` draws nothing (no armour overlays on Zero), `D671`
  runs `zero::emit_p1`, which writes P1's X3 frame the way the ROM emitter
  writes X: entries at the build cursor `$E4`, 2 high bits per slot shifted
  into `$E6` and stored at `$0900 + $E5` every 4 slots (the build's tail
  flushes a partial byte, the hide loop and `$E3` see the slots as the
  game's). So P1 keeps X's place in the sprite order (in front of the
  Highway cars, as X is) and is drawn on paused frames too. Frame from P1's
  live block (`+02`/`+17`, saber pose first), nothing while `+0E = 0` (the
  i-frame blink), origin and flips as for P2 (`X3_Y_OFFSET`), attr = `+11 &
  $3E` (X's priority and palette). Tiles: shadow page 1, refilled when the
  frame changes (`ZeroP1::frame`). Stops short of slot 127 (the ROM's
  overflow exit is left to the lists after it).
- **Palette.** The OAM build hook writes Zero's palette over X's sprite
  palette 1 in the `$0300` mirror every frame (only when it changed), so
  stage loads and weapon swaps are repaired the same frame. Weapon colours
  and X's charge flash do not show on Zero; the charge sparkles and dash
  dust (other objects) do.
- **Saber.** An `Observe` trap on the input remapper `$80:E518` (outside
  `call_isolated`, i.e. P1's call) runs `saber::update` on pad 1 and writes
  the result back to `$A7` (held) and `$AB` (pressed): X starts a swing,
  during one the pad is masked as for P2 (Start/Select pass). Damage: the
  `$80:D1FF` shot and the physics hook's take-back handle both swings (each
  `Saber` has its own slot). The buster stays on Y.

Controls:

| | move | jump | dash | buster | saber |
|---|---|---|---|---|---|
| P1 keyboard | arrows | Z | C | X | V |
| P2 keyboard | WASD | G | H | F | T |
| pad | d-pad | B (South) | A (East) | Y (West) | X (North) |

Checked on the real ROMs (PNG strips in `target/z1/`, `x` = `xtask dump --x3 PATH`):

```
x --load states/highway-start.state --frames 130 --out target/z1/walk --at 10,40,58,96,110,120 --hold 0100 --p1tap 8000@90:20 --zero   # stand, walk, jump, fall; in front of the truck
x --load states/highway-start.state --frames 120 --out target/z1/act --at 44,48,62,66,100,118 --poke 7E1F99=08@1-120 --p1tap 0080@38:12,4000@60:1,4000@70:48 --zero   # dash, buster from the cannon, charge
x --load states/hw-c.state --frames 440 --out target/z1/wall --at 320-440/8 --hold 0100 --p1tap "$T" --refill --zero   # T = coop-qa's m5 route: wall slide 39/40, kick 43
x --load states/hw-spiky.state --frames 50 --out target/z1/saber26 --at 24-50/2 --p1tap 0100@1:26,0040@28:2 --zero   # arc (88) takes the Spiky 2 -> 1 (test saber_p1_hits_spiky)
x --load states/highway-start.state --frames 130 --out target/z1/coop --at 30,60,100,120 --hold 0100 --p2-hold 0100 --p2tap 8000@90:20,0040@112:2 --p1tap 0040@50:2 --coop --zero   # both Zeros, both sabers
```
