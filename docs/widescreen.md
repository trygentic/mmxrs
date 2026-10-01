# Widescreen

The SNES draws a 256x224 picture. Widescreen keeps that picture exactly as
it is and draws extra scenery left and right of it, so the game fills a
16:10, 16:9 or 21:9 window. Same idea as z2rs (`z2rs-private/docs/widescreen.md`),
different mechanism: the NES port composes a second buffer from a render
record; here the vendored PPU itself renders each scanline wider, and
mmx-core feeds it the stage's real tiles for the columns VRAM does not hold.

It is display only: no WRAM, VRAM, OAM or CGRAM write, nothing in a save
state, no effect on the netplay checksum, and it may be on for one netplay
peer only. With it off the PPU takes exactly the old path (`xtask verify`
parity run unchanged).

## Presets

Margins are whole 8-px tiles. Height stays 224 (239 in overscan).

| preset | tiles per side | margin | output | aspect |
|---|---|---|---|---|
| `off` | 0 | 0 | 256x224 | 1.14 (square pixels) |
| `16:10` | 7 | 56 | 368x224 | 1.64 (16:10 is 358 px) |
| `16:9` | 9 | 72 | 400x224 | 1.79 (16:9 is 398 px) |
| `21:9` | 17 | 136 | 528x224 | 2.36 (21:9 is 523 px) |
| `N` | N (0-24) | 8N | 256+16N | |

`mmx_core::Widescreen` (`parse`, `name`, `next`, `margin_px`, `width`),
`Game::set_widescreen` / `set_wide_margin`. `Game::frame()` reports the wide
size from the next frame on. Pixels are square, as in the frontends.

## How MMX draws a stage (measured)

`cargo xtask bginfo --load S --frames N [--lines]` prints the PPU's BG setup,
the camera, the decoder-vs-VRAM agreement and (with `--lines`) frame N's
per-line registers, which is how everything below was measured
(highway-start, highway-mid, octopus-start, and walks from them).

- **Mode 1** in the stages measured (Highway, Launch Octopus), 8x8 tiles,
  BG3 high priority bit set. OBJ base `$6000` words, 8x8/16x16 sprites.
- **BG1 = terrain**, tilemap `$5000` (words), **64x32** (512x256 px), tiles
  at `$1000`. Scroll = the camera exactly: `BG1HOFS = $1E4D`,
  `BG1VOFS = $1E50` (highway-mid: 1078/256).
- **BG2 = backdrop** (city / sea), tilemap `$5800`, 64x32, tiles `$1000`.
  Scroll = the BG2 camera (object `$1E88`: X `$1E8D`, Y `$1E90`); on the
  Highway it moves at half speed (539 for 1078).
- **BG3** tilemap `$0800`, 32x64, 2bpp; off on the Highway, the water
  surface on Launch Octopus (main screen only, colour math add/half over the
  sub screen BG1+BG2+OBJ: translucent water). Not from the layout; VRAM is
  used as is (its 32 columns wrap, which suits a repeating texture).
- **HUD = sprites** (OAM slots 0-4 at X 8), not a BG. It stays where the
  game puts it: at the original picture's left edge (as in z2rs, see HUD).
- **HDMA**: none on the Highway (all 224 lines identical). Launch Octopus
  waves BG2's vertical scroll on lines 170-224 (591..594). Applies in the
  margins too, since the margins are drawn from each line's own registers.
- **Windows**: none enabled in either stage. **Colour math**: Highway
  subtracts fixed colour 0 (a no-op) on BG1/BG3/backdrop; Octopus as above.

### The stage layout (decoded, verified against VRAM)

The terrain tile fetch `$84:916A` (docs/coop.md "Walls") reads the same
tables; for drawing:

```
screen  = $7E:E800[(y >> 8) * 32 + (x >> 8)]          BG1 (32x32 screens of 256 px)
          $7E:EC00[...]                                  BG2
map     = u16 at $7E:2000 + screen * $200              BG1 (16x16 maps of 16 px per screen)
          u16 at $7E:A600 + screen * $200              BG2 (= screen + $43 of the same table)
              + ((y >> 4) & 15) * 32 + ((x >> 4) & 15) * 2
entry   = u16 at [$0B95] + map * 8 + sub * 2,  sub = ((y >> 3) & 1) * 2 + ((x >> 3) & 1)
```

`[$0B95]` is a long pointer to the stage's map table (`$99:8000` on the
Highway): four BG tilemap entries per 16x16 map, TL TR BL BR, in VRAM
format. (`[$0B92]`, `$95:E1E3` on the Highway, is the per-map collision byte
the terrain code uses.) `mmx_core::widescreen::layout_entry` decodes it.

Verification: the decoder reproduces VRAM for every on-screen cell on
highway-start, highway-mid and octopus-start, BG1 and BG2 (`agreement` =
100%; the ROM test `decoder_matches_vram_on_screen`).

### What VRAM holds beyond the picture

BG1 and BG2 tilemaps are 64 columns, a ring the game streams one column at
a time as the camera moves. Measured by comparing the decoder with VRAM per
column (camera-relative px):

| state | valid VRAM columns |
|---|---|
| highway-mid (came from the left) | -244 .. +268 |
| highway-mid, 120 frames walking right | -244 .. +268 |
| highway-mid, 120 frames walking left | -104 .. +412 |
| highway-start (camera 0) | .. +280 |

So the game writes a column only ~12-16 px before it scrolls in: the ring
always covers 512 px, but which side holds the extra 256 depends on where
the camera came from. A margin is only reliably valid for ~16 px; the rest
is stale (last area's columns). Rows: 32 rows = 256 px for 224 visible, same
story vertically. Hence the margin tiles come from the decoder, not VRAM.

## PPU patch (`vendor/jgenesis/snes-core/src/ppu/wide.rs`, `// mmxrs patch`)

- `Ppu::set_wide_margin(m)` (0 = off, max 192). After each line render
  (`render_current_line` -> `render_wide_margins`) the margins of that line
  are drawn into a separate `(256 + 2m) x 239` buffer from the line's
  registers: mode, scroll, TM/TS, window enables, colour math, fixed colour,
  brightness. `compose_wide` copies the normal frame into its centre at frame
  end and `SnesEmulator::tick` hands that to the renderer instead of the
  256-wide frame. The 256-px path is untouched.
- BG pixels at margin x: `x = sx + HOFS`, `y = line + VOFS`; the tilemap
  entry comes from the host's **margin map** when it has one for that cell
  (`set_margin_tiles(bg, cells)`: 128x128 cells over scroll space mod 1024,
  `0` = none, `0x1_0000 | entry`), else from VRAM.
- Sprites: every OAM entry whose pixels land in a margin, evaluated like the
  main path (previous line, display offset and P2 shadow pages honoured), no
  32/34 limits, lower slot wins. X is 9-bit: >= 256 is negative as on
  hardware (the left margin); an X in `256..256+m` is also drawn at X (the
  right margin).
- Windows and the colour window take the edge pixel's value (x 0 for the
  left margin, 255 for the right).
- Not widened: mode 7 (margins show OBJ/backdrop only), mosaic, offset-per-tile
  (plain scroll), hi-res and interlaced frames (output at their normal size).
- `Wide` encodes as nothing, clones as empty (save states, rollback copies
  skip it) and `load_state` keeps the live one: a frontend setting.
- Research hooks: `set_line_record` / `line_regs` (per-line registers),
  `bg_map_entry`, `ppu_registers_debug`.
