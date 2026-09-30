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
  (`[u16; 512]`, 32 4bpp tiles laid out as two name-table rows of 16, the same
  shape as X's window, so a 16x16 sprite's `n, n+1, n+16, n+17` stay inside)
  and `slots: u128` (bit n = OAM slot n), plus `next_page`/`next_slots` and a
  `pending` flag. The host writes the `next_*` copies. They go live on the next
  OAMDATA write, which is the game's NMI OAM upload, so page and mask switch in
  the same vblank as the OAM they describe (no mid-frame tearing, and a lag
  frame keeps the old pair). In `fetch_sprite_tiles` a sprite whose slot bit
  is set reads `page[(tile & 0x1F) * 16..]` instead of VRAM. That is one shift
  and test per sprite, and with `slots == 0` the renderer is unchanged (the
  `verify --coop-off` parity run stays identical).
- **Save state.** The page is inside the PPU, so `to_save_state` clones it.
  Its bincode encoding inside `SnesEmulator` is empty (`P2TilesSlot`), and
  `Game::save_state` appends the `P2Tiles` after `(emu, frame_count,
  fired_at)`. States written before the patch (the `states/*.state` files) have
  no tail and load with an empty page. Netplay rollback states wrap
  `Game::save_state`, so they carry the page.
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
Octopus: stand 0-2 (blink), stand-shoot 49-50, walk 13-22 (shooting 3-12),
jump 32-34 (shooting 23-25), fall 35-36 (26-27), land 37-38, hurt 51-53,
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
