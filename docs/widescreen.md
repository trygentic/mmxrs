# Widescreen

The SNES draws a 256x224 picture. Widescreen keeps that picture exactly as
it is and draws extra scenery left and right of it, so the game fills a
16:10, 16:9 or 21:9 window. Same idea as z2rs (`z2rs-private/docs/widescreen.md`),
different mechanism: the NES port composes a second buffer from a render
record; here the vendored PPU itself renders each scanline wider, and
mmx-core feeds it the stage's real tiles for the columns VRAM does not hold.

Objects the game culls at the picture's edges are drawn in the margins too
("Objects in the margins").

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
- Margin sprites: `set_margin_sprites(Vec<MarginSprite>)` (signed X, Y,
  tile, OAM attribute byte, size, optional P2 shadow page). Not OAM: a list
  in `Wide`, drawn in the margins only, under every OAM sprite. The host's
  list goes live on the next OAMDATA write (the game's OAM upload), like the
  P2 pages, so it changes together with the OAM it completes.
- Windows and the colour window take the edge pixel's value (x 0 for the
  left margin, 255 for the right).
- Not widened: mode 7 (margins show OBJ/backdrop only), mosaic, offset-per-tile
  (plain scroll), hi-res and interlaced frames (output at their normal size).
- `Wide` encodes as nothing, clones as empty (save states, rollback copies
  skip it) and `load_state` keeps the live one: a frontend setting.
- Research hooks: `set_line_record` / `line_regs` (per-line registers),
  `bg_map_entry`, `ppu_registers_debug`.

## The margin provider (`crates/mmx-core/src/widescreen.rs`)

After every `Game::step` with a margin, `update_margin_tiles` decodes BG1
and BG2 around each layer's camera (`[cam - m - 32, cam + 256 + m + 32)` x
`[camY - 32, camY + 256)`) into the PPU's margin map, keyed by level
position mod 1024. MMX scrolls each BG by exactly its camera, so that key is
the BG's scroll space; one frame of camera lag between the step and the
next frame's NMI is covered by the 32-px apron. Guards:

- only in a stage (`$D2 = 4`, `$D3 != 0`); elsewhere (title, intro, stage
  select, ...) the margins are VRAM as is: those 32/64-column maps wrap,
  which reads well (the title's ribbons simply continue);
- only for a BG whose decoded on-screen cells match VRAM (`agreement`
  >= 90%), so a stage or scene that uses BG1/BG2 some other way falls back
  to VRAM instead of drawing nonsense;
- off the 8192x8192 layout (left of x 0 at a stage's start) BG1 is blank
  and BG2 is VRAM's wrap. The camera never shows those areas, the margins
  do: you see the backdrop past the stage's first wall.

The margins show the layout as data, including areas past the camera's
limits (a boss door's far side, the next section) the game never scrolls
to. BG3 (the Octopus water surface) is VRAM, which repeats.

## HUD

The HUD is OAM (slots 0-4 at X 8, the weapon bar next to it), so it stays
where the game draws it: at the original picture's left edge, 8 px in. Same
choice as z2rs. Moving it to the wide picture's edge would need a
display-only X offset for those slots (`set_sprite_offset` clips at X -64,
so it would need its own path); not done.

## Objects in the margins

Code: `crates/mmx-core/src/margin_sprites.rs`; PNGs below.

### Where the game culls (measured)

- **Per piece, at OAM build time.** The object emitter (listing `$80:D67C`,
  `X` = the object, `JSR` from every draw list) computes each piece's X as
  its layout offset (mirrored by facing) plus `+$05 - $1E4D`, stores it,
  and skips the piece when `x + 16 >= $10F` or `y + 15 >= $EF` (16-bit,
  unsigned: X `-16..=254`, Y `-15..=223` kept). An object straddling the edge
  loses only its outer pieces; the kept ones at X `-16..-1` and `240..254`
  already spill into the margins (the PPU draws them there).
- **Per object, when it queues itself to be drawn.** An object's update
  `JSL`s `$82:80B4` (240 call sites): visible only with `x - cam` in
  `-32..288` and `y - camY` in `-16..240`, then `+$0E = $81` and its address
  goes on draw list `+$12 / 2 + 1` (negative: list 0) of the six at `$0920`
  `$0960` `$09A0` `$09E0` `$0A20` `$0A60` (32 each, counts `$E7..$EC`);
  otherwise `+$0E = 0` and it is not drawn. `$82:808F` (17 sites, the big
  ones) is the same with `-96..352` x `-80..304`. The build draws lists 0-2
  (`$80:D5B4`), the armour parts, the player, lists 3-5 (`$80:D5FA`).
  Effects (`$1928`) clear `+$0E` bit 7 before each update, so bit 7 is
  "queued this frame" for every table.
- **Despawn is not there.** `$82:806E` (119 sites) is the "still in range"
  test, `-64..320` x `-64..288`, carry set = out; enemies delete themselves
  on it. Measured (hw-a walking right): the Sky Claw at X -65 is alive (not
  queued since -32), gone 18 frames later at about -83.
- **Spawning is by the original window.** The layout spawner `$80:DC0B`
  runs when the camera crosses a 32-px column: scrolling right it spawns
  the column at `cam + 256` (rounded down to 32; 288 px tall from
  `camY - 32`), scrolling left the column at `cam`, vertically the rows at
  `camY - 32` / `camY + 240` over 320 px from `cam - 32`. Enemies appear at
  X 253..263 on the Highway walks.

So what the margins can show, at 21:9 (136 px a side):

| case | margin | what is drawn |
|---|---|---|
| enemy leaving on the left (walking, or scrolled out) | left | from -16 out to its despawn near -64 (about 80 px with its width); then it no longer exists |
| enemy entering on the right | right | at most the ~32 px between its spawn and the edge; nothing further out exists yet |
| big objects at the edge (Bee Blader, road attacker cars, the truck-carried claws) | both | their outer pieces (the Bee Blader's rear half at the right edge, cars driving in and out) |
| shots leaving | right/left | until they despawn, a few px past the edge |
| items dropped near the edge | either | while they live (same rules, `$1428` table) |
| P2 left behind (co-op) | either | P2 as X-in-Zero-colours or X3 Zero until it warps back (`P2_AWAY_FRAMES`, 90 frames) |
| effects (dust, sparks, the Sky Claw's cable links) | either | only while the game queues them (off the window some stop moving) |
| objects the spawner has not spawned | - | nothing: they do not exist |

### The margin sprite pass

At the display hook `$80:9A86` (after the OAM build and P2's slice;
`coop::display_slice`), with a margin on:

1. Every live object of the `$0E68..$1928` tables (43 x `$40`: enemies,
   shots, items, more) and of the effect pools `$1928` (31 x `$20`) and
   `$1D08` (16 x `$20`) whose X is within `-margin-128 .. 256+margin+128`
   and not mid-window (`64..192`). An effect counts only when queued this
   frame; a `$0E68`-table object also when the game stopped queueing it
   after having queued it (`MarginState`, per slot: `id + 1` last seen
   queued), so an enemy past `-32` keeps being drawn and helper objects that
   never draw stay invisible.
2. For each, the ROM's own emitter runs isolated (`call_isolated_ret(..,
   Rts)`, `M=1 X=0 D=0 DB=$8D`, `X` = the object, OAM cursor `$E4/$E5/$E6`
   = 0) with the camera X `$1E4D` moved 256 px towards the object's side,
   so its margin pieces land inside the emitter's window. The entries it
   wrote at `$0700` (and their high-table bits at `$0900` / the partial
   byte in `$E6`) are read back, moved 256 px back, and kept when some of
   the sprite is in a margin and, for a queued object, the game did not
   keep it itself (X outside `-16..=254`): no duplicates at the boundary.
   Priority and palette bits are the emitter's.
3. P2 (co-op, not hidden, away from mid-window): X's emitter on P2's block
   in `$0BA8` with Zero's palette and the P2 shadow page for its window
   tiles, or the X3 Zero emitter (`zero::margin_entries`, pages 0 and 2,
   only frames whose page `emit_x3` already filled).
4. WRAM `$0000..$1FFF` (direct page, stack, OAM buffer, high table, the
   camera, `$0BA8`) is put back byte for byte, and so is the CPU
   (`SnesEmulator::cpu_snapshot`: registers, the 65816's between-cycle
   temporaries and the open bus, which a save state holds). The list goes to
   the PPU (`set_margin_sprites`, at most 128).

No OAM slot is used, so the 128-slot budget, P2's slots and `oam_hide` are
untouched. The trap at `$80:9A86` is the co-op display hook (same address
and body): a netplay session game has it anyway, so a peer turning
widescreen on does not change the trap set; a solo game gets it from
`Game::set_wide_margin`. Paused frames do not reach `$9A86`: the last list
stays (the world is frozen too). Outside a stage the list is cleared.

### What cannot be drawn, and wide gameplay

Objects that do not exist: the spawner fills only the original window plus
one 32-px column, and objects despawn about 64 px past either edge. Showing
the whole margin would mean "wide gameplay": spawn by the wide window
(`$80:DC0B`'s `+256` and row width), queue and despawn by it (`$82:80B4`,
`$82:808F`, `$82:806E`). That changes what enemies do and when (they act and
shoot from further away), so it is gameplay: an opt-in in the netplay
identity, on for both peers, unlike the display-only margins. See "Wide
gameplay".

## Wide gameplay (opt-in, changes the game)

`--wide-gameplay` (native), the launcher's "spawn enemies at the wide edges"
box, the web's checkbox, `Game::set_wide_gameplay(true)`. Off by default; off
registers nothing, so a game without it is byte-identical to one built before
it existed (`xtask verify`, checksums, netplay identity).

### Design

The game's object window is moved out by a fixed margin, independent of the
display preset, so every peer simulates the same world whatever it shows:

- `WIDE_GAMEPLAY_PX` = **136** (the 21:9 margin, the widest preset): the
  draw-queue tests `$82:80B4` and `$82:808F` accept X `-32-136 .. 288+136`
  (`-96-136 .. 352+136`). A 16:10 or 16:9 player simply does not see the
  outer part of it. Y is unchanged (the picture is not taller).
- `WIDE_SPAWN_PX` = **160** (136 rounded up to the spawner's 32-px columns):
  the spawner `$80:DC0B` spawns the column at `cam + 256 + 160` scrolling
  right and `cam - 160` scrolling left, and its row sweep (camera moved
  vertically, which is also how a stage entry or a camera jump fills the
  screen: the camera is stepped into place a row at a time) covers
  `cam - 32 - 160 .. cam + 288 + 160` (20 columns instead of 10). The despawn
  test `$82:806E` accepts X `-64-160 .. 320+160`. Rounding up keeps the
  spawned column wholly outside the 21:9 picture, as the ROM's own column is
  just outside 256 px, and keeps the ROM's 32 px of slack between the spawn
  column and the despawn line (with less, a few px of backing up would
  delete an enemy without the camera re-crossing its column, so it would not
  come back).
- The emitter cull `$80:D67C` is left alone: OAM stays the original 256-px
  window, and the margin sprite pass draws what it culls (now: everything
  out to the margins, since those objects exist and are queued).

Trade-off: enemies engage about 136-160 px earlier on both sides. They
spawn, act, shoot and drop off ledges while X is still that far away, a
walking enemy may reach you sooner, an enemy left behind keeps chasing for
longer, and more objects are alive at once (slot pressure: the ROM refuses a
spawn when its tables are full). That is a different game, so it is opt-in
and part of the netplay identity: the flag and both margins are mixed into
`trapset_id` (the traps' addresses are too), `NET_REVISION` 4. Peers that
disagree refuse the handshake. Each peer still picks its own display preset.

### Implementation (`crates/mmx-core/src/wide_gameplay.rs`)

Traps, registered only with the flag on, every address from `hooks.rs`
(signatures checked on the PAL and USA Rev 1 dumps):

- **Queue / despawn tests** (`$82:806E`, `$82:808F`, `$82:80B4`, all three
  `REP #$20; SEC; LDA $05; SBC $1E4D; CLC; ADC #a; CMP #w; BCS ..; SEC; LDA
  $08; SBC $1E50; CLC; ADC #b; CMP #h; ..`): an `Observe` trap on the entry
  does the two compares in Rust with `a + M` / `w + 2M` (the immediates are
  read from the ROM), sets A and P exactly as the ROM would have them after
  the compare that decides (A = the subtraction, V from the `ADC`, N Z C from
  the `CMP`, M = 16-bit) and moves PC to where the ROM's branches would go:
  the routine's own tail (`STZ $0E` / `LDA #$81; STA $0E` and the draw-list
  push, or `SEP #$20; RTL` with the carry for `$806E`'s callers). With
  decimal mode set the trap does nothing (never seen; the ROM's own test
  runs).
- **Spawner** (`$80:DC0B`): two `Observe` traps on the ROM's own code. At
  `$80:DC56` (a horizontal column is about to be spawned, its X in `$00`):
  `$00 += 160` when it is `cam + 256` (scrolling right), `$00 -= 160` when
  it is `cam`. At `$80:DC9B` (the row sweep, `A = 10` columns from `$00 =
  cam - 32`): `$00 -= 160`, `A = 20`. The column test itself (did the camera
  cross a 32-px line?) stays the ROM's; since 160 is a multiple of 32 the
  moved column is exactly the next one out, no gaps and no repeats at any
  camera speed. Each layout entry has a "spawned" byte (`STA ($14)` = 1;
  the object's `+$0C` points back at it and its despawn clears it), so a
  column spawned twice never doubles an enemy.

Bank `$83` has two private copies of the test for single enemies (`$83:8948`
`-128..384`, `$83:963A` `-32..288`) and bank `$80` one more (`$80:DD89`, no
caller found); they are left alone.

## Verification

- `cargo test -p mmx-core --test widescreen` (ROM-gated): the decoder equals
  VRAM on every on-screen cell (highway-start, highway-mid, octopus-start);
  300 frames walking and jumping with 21:9 on vs off (highway-mid, hw-a):
  WRAM, OAM, CGRAM, VRAM, save state and netplay checksum identical every
  frame, the wide frame's centre equals the normal frame, the margin
  survives a state load. `margin_sprites_are_display_only`: hw-a in a
  netplay session game, solo and co-op, 300 frames walking right, on vs
  off: checksum every frame, WRAM, OAM and save state equal; margin sprites
  in both margins (solo 1463 / 478 sprite-frames left / right, co-op 2154 /
  478), every one in a margin and none equal to an entry of the game's OAM.
- Margin sprites, 21:9, on vs off (`target/widescreen/margin-sprites/
  NAME-{on,off}.png`, dotted lines at the window edges): `hw-a-walk` (Sky
  Claws and their claws leaving left, one entering right), `highway-mid-walk`,
  `highway-mid-shots` (lemons past the right edge), `hw-bee` / `hw-bee2`
  (the Bee Blader's rear half), `hw-car` (road attackers both sides),
  `hw-a-coop-p2` / `hw-a-coop-x3` (P2 left behind in the left margin),
  `hw-c-walk` (camera stops at a wall: nothing spawned, nothing to draw).
  In every run the 256-px centre is pixel-identical on vs off and the
  per-frame status lines (X, camera, HP) are identical.
- `cargo xtask verify --frames 2400 --coop-off --tap 200,400,600,800,1000
  --hold 0100`: OK (widescreen off; the PPU takes the old path).
- `xtask bginfo` on the Rev 1 ROM (`usa11-highway-start`, 400 frames
  walking): decoder 100% on BG1 and BG2, same tables.
- PNGs: `cargo xtask dump --load states/S.state --wide 16:9|21:9 ...`;
  `target/widescreen/{highway-start,highway-mid,octopus-start}-{16x9,21x9}-{static,walk}/`
  (walk = 300 frames holding right, `--refill`). Column-difference at the
  two window boundaries is within the frame's own 90th percentile except
  where real edges sit (highway-start: the stage's first wall at x 0 and a
  pillar edge at x 256).
- Cost: Launch Octopus at 21:9 (two screens + colour math) 3.9 -> 5.8 ms per
  frame on an M-series Mac, about the main renderer's own cost per pixel.
