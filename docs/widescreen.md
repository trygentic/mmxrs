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
- **HUD = sprites** (OAM slots 0-6 at X 8, more for the weapon and boss
  bars), not a BG. Widescreen moves it to the wide picture's edges (see HUD).
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
- HUD shift: `set_hud_shift(left, right)` (two slot masks, live from the
  next OAMDATA write like the margin sprites). Those slots are drawn
  `margin` px left / right of their OAM X, on both paths: `offset_sprite`
  adds the shift in the 256-px path (so they leave the centre; clipped at
  X -64 as before), and the margin path places them at their signed X +
  shift, unclipped, so they land in the margins as ordinary OAM (over the
  margin sprites and BGs, lower slot wins, 9-bit X and size bits as usual).
  No effect without a margin or on a hi-res frame.
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

The HUD is OAM, and with a margin it moves to the wide picture's edges,
keeping its inset: X's bar (slots 0..6, X 8) and the weapon bar (7..15,
X 24) end up 8 / 24 px from the new left edge, the boss bar (16..23, X 232)
24 px from the new right edge. Vertical positions are the game's.

`widescreen::update_margin_tiles` sets the PPU's HUD shift after every step
(`hud_slots`): in a stage, left = slots 0..15 (the OAM build never writes
them, its cursor starts at `$10`; hidden pieces are `Y $E0`, and the pause
menu leaves them hidden, so the weapon menu is not shifted) plus P2's bar
(co-op: the seven slots `coop_hud` put it in this step, `MarginState::p2_bar`;
P2's down marker stays where P2 fell), right = 16..23 while `$1F0E` names a
boss (the build then starts at `$18`). A Zero P1's Z icon is slot 0.
Outside a stage, no shift. The step's OAM goes up at the next NMI, which is
when the PPU takes the masks, so they always match the OAM they describe.

Display only, but not invisible to a save state: the state holds the PPU's
last picture and sprite evaluation (STAT77 overflow flags, which MMX never
reads: no `$213E` load in the ROM), and those differ with the HUD moved.
The display-only tests therefore compare save states after one frame with
widescreen off (`same_state_unwidened`); WRAM/OAM/CGRAM/VRAM and the
netplay checksum are compared every frame as before.

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
   never draw stay invisible. **Until its tiles are rewritten**: the pass
   diffs OBJ VRAM (`$6000`, 512 tiles) each frame and keeps per tile the
   frame of its last write and per slot the frame it was last queued; an
   unqueued object one of whose emitted tiles was written after the NMI
   that followed its last queued frame (that NMI is its own last upload) is
   dropped and not drawn again until queued. Its slot then holds someone
   else's graphics: a second mech of the same set streaming into the shared
   slot `$60`, or a set loading there (measured below: "stale").
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

Eight `Observe` traps, registered only with the flag on (`TrapTable::unregister`
takes them off again), every address from `hooks.rs` with signatures that
resolve on the PAL and USA Rev 1 dumps (bank `$82`/`$84` ones in place,
bank `$80` ones `+$2B`). None skips an instruction, so the CPU's timing is
the ROM's; they change a register or a scratch word and let the ROM go on.

| hook | PAL / Rev 1 | what changes |
|---|---|---|
| `queue_test` | `$82:80C3` / same | X `BCS` of `$82:80B4` |
| `queue_test_big` | `$82:809E` / same | X `BCS` of `$82:808F` |
| `despawn_test` | `$82:807D` / same | X `BCS` of `$82:806E` |
| `spawn_column` | `$80:DC56` / `$80:DC81` | none: spawns the wide column, isolated |
| `spawn_row` | `$80:DC9B` / `$80:DCC6` | row sweep start `$00`, count A |
| `kind_count` | `$84:A1E0` / same | Z at the slot test of `$84:A1D5` |
| `spawn_entry` | `$80:DD18` / `$80:DD43` | Z, C at a layout entry's section test |
| `gfx_trigger` | `$83:FDE3` / same | N at the `RTS` of the trigger objects' player test |

(`obj_gfx_init`, `$82:827D` on both, is resolved too but not trapped: its
`LDA $A5E4,X` operand is the id -> graphics set table. Likewise
`gfx_loader`, `$80:B066` / `$80:B091`, the loader thread: `LDA $ACEE,X`, the
per-stage list table; and `gfx_decomp`, `$80:B230` / `$80:B25B`, its
decompressor thread: `LDA $F6F7,Y`, the per-set size and source.)

- **Queue / despawn tests**. All three are `REP #$20; SEC; LDA $05; SBC
  $1E4D; CLC; ADC #a; CMP #w; BCS out; SEC; LDA $08; SBC $1E50; CLC; ADC
  #b; CMP #h; ..`. The trap sits on the X `BCS`: A is `x - cam + a`, P is
  the `CMP`'s. `wide_gameplay::widen` recomputes `ADC #a+M; CMP #w+2M` from
  `A - a` (V from the `ADC`, N Z C from the `CMP`, the immediates read from
  the ROM) and writes A and P back; the ROM's `BCS` then branches the wide
  way and everything after (the Y test, `+$0E = $81`, the draw-list push,
  `$806E`'s carry for its caller) is the ROM's. So the registers are exactly
  those of a ROM whose immediates were widened, and at M = 0 the trap is the
  identity (pinned, below). Decimal mode: no change.
- **Spawner** (`$80:DC0B`). At `$80:DC56` (a horizontal column is about to
  be spawned, its X in `$00`): the column at `$00 + 160` when `$00` is `cam
  + 256` (scrolling right), `$00 - 160` when it is `cam`, is spawned first
  through the ROM's column spawner `$80:DCB0`, isolated (zero emulated time,
  DP `$00..$1F` and the registers put back); then the ROM spawns its own
  column as without wide gameplay (entries already spawned are skipped).
  At `$80:DC9B` (`STA $06`
  of the row sweep, `A = 10` columns from `$00 = cam - 32`): `$00 -= 160`,
  `A = 20`. The column test itself (did the camera cross a 32-px line?)
  stays the ROM's; 160 being a multiple of 32, the moved column is exactly
  the next one out, no gaps and no repeats at any camera speed. Each layout
  entry has a "spawned" byte (`STA ($14)` = 1; the object's `+$0C` points
  back at it and its removal, `$82:8387`, clears it), so a column spawned
  twice never doubles an enemy. Stage entries step the camera into place a
  row at a time through the row sweep, so they fill the wide window too.
- **Graphics not loaded yet** (`spawn_entry`). An enemy's init
  (`$82:827D`, every `$0E68`-table enemy) copies its VRAM slot `+$18` and
  palette `+$11` once from the loaded-set tables `$7F:8200[set]` /
  `$7F:8300[set]` (`set` from `$86:A5E4[(id - 1) * 2 + 1]`; DB is `$86` in
  the object update). The sets are loaded by invisible trigger objects
  (`$83:FD7B`, ids `$15..`) when the player's X passes theirs: the loader
  thread `$80:B091` DMAs the set and fills the tables (palette `| $20`).
  The ROM's spawn line never outruns its triggers; 160 px further out it
  does, and such an enemy lived on with slot 0 and palette 0, i.e. drawn
  from X's streamed tiles `$000-$01F` in palette 0, in the margin and later
  in the window too. Measured on Rev 1 hw-c: the jumping mech (id `$29`,
  set `$30` -> slot `$60`) spawned at frame 207 at cam + 432, its set loaded
  at frame 244 (cam 340); the Sky Claws by the trucks (id `$0F`, set `$1E`,
  slot `$60` after the mech's) likewise (the user's report: Sky Claws at the
  top right in Zero-like fragments). The trap sits on the `BEQ` after the
  entry's section test (`LDA ($18); AND #$F8; CMP $1FFE; BEQ spawn; BCS
  next`; DB `$85`, `($18)` the record: type, Y, id, sub, X): a type-3
  record (the enemy table) in a column outside the ROM's own (`col - cam`
  outside `-63..=256`) whose set has `$7F:8300[set] = 0` gets Z clear, C
  set, the ROM's own "later section" skip. Its spawned byte stays clear, so
  the ROM's own column (or row sweep) spawns it when the ROM would have,
  and the next wide column it reaches spawns it once its set is in. PNGs:
  `target/widescreen/sky-claw-garble/hw-c-sky-claws-16x9.png` (Rev 1 hw-c,
  P1 and P2 as X3 Zero, 16:9, frames 610-760/25; top row before, bottom
  after) and `hw-c-f710-right-3x.png` (frame 710, the right part, before |
  after).
- **Loading graphics sets early** (`gfx_trigger`; how the loader works:
  "Graphics sets and their triggers" below). Holding back only moves the
  pop-in from garbage to the ROM's 256-px line, which is what the user saw
  ("enemies appear suddenly closer to X"). So a graphics-set trigger (type
  2, id `$15`, an X trigger) fires as if the player were `WIDE_SPAWN_PX`
  (160) further right: the set is then in by the time the wide spawn line
  reaches its enemies, as the ROM's is by its own line (the layout keeps
  every first enemy of a set >= 200 px past its trigger, so it is in by
  then with 160 px to spare minus the load's frames). The trap sits on the
  `RTS` of the trigger's player test `$83:FDD3` (`CMP $00,X` of the
  player's X against the trigger's) and only sets N for the caller's
  branch, which it tells from the return address:
  - behind (`$02 = 2`, `BMI` next): "ahead" once `x + 160 >= tx`. When
    that is early (`x < tx`) it must pass `early_ok`, else it is asked
    again next frame (the ROM's own crossing, `x >= tx`, is never refused:
    worst case the ROM's timing). An early fire sets the object's `+$0F`
    (unused by these 16-byte objects; their removal clears it).
  - ahead (`$02 = 0`, `BPL` next): with `+$0F`, "behind" once `x + 160 <
    tx` (backed out of the early band: the previous list reloads), and the
    mark is dropped once `x >= tx`. Without it the ROM's test, so walking
    back left reloads exactly where the ROM does. (A single shifted
    position is what keeps this from flip-flopping: early in both
    directions would need the scroll direction, which WRAM does not keep.)
  - init: the ROM's test (the mark cleared). Y triggers (ids `$18..`) and
    the other loaders' triggers (`$16`, `$17`) are left alone.

  `early_ok` refuses while the loader thread is busy (`$0040 != 0`; a
  second start would restart it, abandoning the first list: so this
  queues), when the list puts a set below VRAM `$1000` (OBJ page 0: X's
  streamed tiles, shots, items; lists 4-6 of the Highway do), and when an
  object in use draws from tiles or a palette row the list gives to another
  set. In use = alive inside the ROM's own object window (X `-64..320`) in
  the enemy, item, `$1628` or effect tables, or an enemy not initialised
  yet (it is about to copy its set's slot). Tiles are ranges: `+$18` plus
  page bit `+$11 & 1`, sized by the set's decompressed size / 32 (enemies
  know their set from the id; other objects are taken as one tile).
  Palette rows clash unless the object's own set is in the list too.
  An object further out exists only with wide gameplay; if a load gives
  its tiles away, it gets the ROM's window back (below) and goes as it
  would have without wide gameplay.
- **Overwritten objects get the ROM's window**. The loader never clears
  `$7F:8200` / `$7F:8300`, so an evicted set still reads as loaded.
  `obj_overwritten` compares an object's tiles and palette row with the
  current list (`$1F08`, the one the loader last started): another set
  there means its graphics are gone. Such an object gets the ROM's window
  in all three window tests (the trap leaves C set when the ROM says
  outside), so it is culled and removed where the ROM would (the mech the
  ROM's own Sky Claw load overwrites while it lives in the left margin,
  the intro cars). `spawn_entry`'s "loaded" test uses the same check, so a
  wide spawn of an evicted set waits for its reload too.
- **Same-kind counts** (`$84:A1D5`, 22 callers: an object counts the live
  objects with its id, e.g. the Sky Claw at `$82:96AF` deletes itself when 2
  are alive). A live object outside the ROM's own despawn window (X
  `-64..320`) counts as an empty slot (Z set at the `BEQ`). Such objects
  only exist with wide gameplay; without this the Sky Claws left alive
  behind the player (out to -224) refused every new one, and a refused
  spawn clears its entry, so it stays away until the camera re-crosses its
  column (measured: no Sky Claw at all over the hw-a and highway-mid walks).
- **Turning it on mid-stage** (a save state made without it, the web's
  checkbox): the columns between the ROM's spawn line and the wide one were
  never spawned and the camera has already crossed them. `wide_gameplay::sweep`
  spawns them once through the ROM's column spawner `$80:DCB0` (isolated,
  5 columns a side, `$00..$1F` and the registers put back; `spawn_entry`
  holds back the enemies of sets not loaded yet). xtask
  (`--load` with `--wide-gameplay`, netsim's frame-0 load, netreplay) and the
  web's checkbox call it; a rollback `load_state` never does (the state
  already has them).

Not widened: Y (the picture is not taller); the emitter cull (OAM stays the
256-px window, the margin sprite pass draws the rest); bank `$83`'s private
copies of the window test for single enemies (`$83:8948` `-128..384`,
`$83:963A` `-32..288`) and bank `$80`'s (`$80:DD89`, no caller found); any
enemy AI that checks its own distance to the camera or to X with its own
constants.

### Graphics sets and their triggers (measured, Rev 1 Highway)

- **Loader**. A trigger object (type 2 record, ids `$15..$1A`, code
  `$83:FD7B`, the 16-byte table `$1D08`) compares the player's X (ids
  `$15..$17`) or Y (`$18..$1A`) with its own (`$86:E4DC[id - $15]`, 5 or
  8) every frame, two-way: crossing forward runs `$FDF9[id - $15]`,
  backward `$FDED[..]`. For id `$15` that stores the list index (sub low
  nibble forward, high nibble backward) in `$1F08` and starts the loader
  thread `$80:B091` (`$80:B071`: thread slot `$10`, status `$0040`; `$16`
  / `$17` start `$80:B416` / `$80:B4DD` with `$1F09` / `$1F0A`). The
  trigger also uses the despawn test, so it lives only while near the
  camera. The thread walks list `$86:ACEE[$ACEE[stage * 2] + idx * 2]`,
  6-byte entries (set, VRAM word address relative to the OBJ base `$6000`,
  palette pointer, palette column), `$FF`-terminated. Per set: a
  decompressor thread (`$80:B25B`, slot `$60`; size and source at
  `$86:F6F7 + set * 5`) unpacks it to WRAM, `$80:B15B` queues the VRAM
  DMA through the `$0500` list, `$1F25`-budgeted (it yields while a frame's
  budget is used up), then `$7F:8200[set] = vram >> 4` (the slot, low
  byte), `$828000` loads the palette and `$7F:8300[set] = $4A | $20`
  (palette bits, page bit 0). One set every 8-9 frames: list 1 (two sets)
  is in 16 frames after the trigger. Nothing is shown or paused while it
  loads; a list started again restarts the thread. The tables are cleared
  only on a stage (re)start.
- **Highway lists** (`stage 0`): 0 = 39 @`$14C0`, 3A @`$17C0`, 3B @`$1BC0`
  (intro cars, effects id `$21`), 1F @`$1000` (id `$15`); 1 = 1F, 30
  @`$1600` (the jumping mech, id `$29`); 2 = 1F, 1E @`$1600` (Sky Claw,
  id `$0F`); 3 = 1E, 30 @`$1900`; 4 = 2D @`$1000`, 2F @`$1AE0`, 23
  @`$0000`; 5 = 21, 22, 1F @`$1600`, 2F, 35 @`$0400`; 6 = 58 @`$1D00`, 35,
  1D @`$1000`; 7 = 1D, 26 @`$1D00`. Slot `$60` (`$1600`) is shared by the
  mech (list 1) and the Sky Claws (list 2); 30 overlaps the cars' 39-3B.
- **Trigger vs enemies** (X; forward list; first enemy of a new set; last
  enemy before it of a set the list evicts):

  | trigger X | list | first new enemy | evicted set's last enemy |
  |---|---|---|---|
  | 448 | 1 | mech 720 (+272) | intro cars (moving) |
  | 896 | 2 | Sky Claw 1096 (+200) | mech 720 (-176) |
  | 1408 | 3 | mech 1904 (+496) | (1E stays; 30 moves to `$1900`) |
  | 2432 | 4 | id `$22` 2804 (+372) | mech 2160 (-272), Sky Claw 2064 |
  | 3568 | 5 | id `$15` 3936 (+368) | id `$22` 3444 (-124) |
  | 5024 | 6 | id `$36` 5344 (+320) | id `$19` 4816 (-208) |
  | 6656 | 7 | - | id `$36` 5632 (-1024) |

  The ROM's spawn line is at about `x + 128`, so each list is in before its
  first enemy spawns; the ROM despawns at `x - 192`, so an evicted set's
  last enemy is usually gone (or dead) by the trigger. With wide gameplay
  the spawn line is `x + 288` (the first enemies of lists 1 and 2 outrun
  their triggers) and the despawn line `x - 352` (the previous section's
  enemies live 160 px longer and hold their slot).
- **Early loading, Rev 1 hw-c walking right with jumps** (`gfx_trigger`):
  list 1 fires at X 377 instead of 448 (the intro cars hold their tiles
  and palette until they leave the ROM's window; 288 at the earliest);
  list 2 at the ROM's 896 unless the mech is dead by X 736 (it holds slot
  `$60`: it is on screen there); list 3 at 1248. Lists 4-6 load into page
  0 and stay at the ROM's timing; their first enemies are >= 320 px
  ahead, so the wide spawn line does not outrun them anyway.

### What it does (measured, 21:9, walking right)

`cargo test -p mmx-core --test wide_gameplay enemies -- --nocapture` prints
every object's X relative to the camera when it first appears and the
leftmost X any object reached (600 frames, `pads()` walk with jumps and
shots):

| state | off: spawn X | on: spawn X | off: leftmost | on: leftmost |
|---|---|---|---|---|
| hw-a | 270..272 | 430..432 | -67 | -226 |
| highway-mid | 255..272 | 414..432 | -67 | -226 |
| hw-a, Rev 1 (`states/usa11`) | 270..272 | 430..432 | -66 | -227 |

(X 153/155 are X's own shots.) So enemies appear 24-40 px past the 21:9
picture's right edge (392) and are removed past -224, 88 px beyond its
left edge (-136). PNG strips (`target/widescreen/wide-gameplay/`, dotted
lines at the 256-px window): `hw-a-{on,off}.png` (frames 100-300: the Sky
Claws and the jumping mech come in from the right margin, the road
attacker car stays alive at the left), `highway-mid-{on,off}.png` (frames
50-250: the car and the Sky Claws are in the right margin 50-100 frames
earlier), `hw-bee2-{on,off}.png` (the Bee Blader fight: camera locked,
identical status lines on vs off over 600 frames).

Difficulty: with the same input X takes more hits (hw-a, holding right:
dead between frames 450 and 500 with it on, alive at 600 with it off), since enemies
come at him from further and in larger numbers.

## Verification

- `cargo test -p mmx-core --test widescreen` (ROM-gated): the decoder equals
  VRAM on every on-screen cell (highway-start, highway-mid, octopus-start);
  300 frames walking and jumping with 21:9 on vs off (highway-mid, hw-a):
  WRAM, OAM, CGRAM, VRAM and netplay checksum identical every frame, the
  save state after one unwidened frame (see HUD), the wide frame's centre
  equals the normal frame outside the HUD, the margin survives a state load. `margin_sprites_are_display_only`: hw-a in a
  netplay session game, solo and co-op, 300 frames walking right, on vs
  off: checksum every frame, WRAM, OAM and (unwidened) save state equal; margin sprites
  in both margins (solo 1463 / 478 sprite-frames left / right, co-op 2154 /
  478), every one in a margin and none equal to an entry of the game's OAM.
- HUD (`hud_shift_is_display_only`): with no margin the masks change
  nothing (hw-spiky with the weapon bar, hw-bee2 with a boss bar: every
  frame pixel-identical with vs without); at 21:9 X's icon is drawn at
  X 8 of the wide frame and not at its old place. 4x crops,
  `target/widescreen/hud/NAME-{16x9,21x9,off}/crop-{left,right}.png`:
  `solo` (hw-spiky f30, weapon bar via `--poke 7E0BDB=02@1-30,7E1F12=00@1`),
  `coop` (P2's bar under X's, both at the edge), `boss` (hw-bee2 f60 co-op,
  `--poke 7E1F0E=68@1-60,7E1F0F=0E@1-60`: boss bar at the right edge),
  `zero` / `coopzero` (the Z icons), `pause` (weapon menu unshifted, the
  HUD back at the edge after it).
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
- Residency check (`MMX_MARGIN_CHECK=1`, or `MarginState::enable_check`;
  `margin_sprites::Check`): for every overlay sprite its object, tiles,
  page and palette, against OBJ VRAM diffed each pass. Findings: `stale`
  (unqueued object drawn with tiles written since it was last queued),
  `slot` (enemy `+$18` is not its set's slot), `page` (shadow page on a
  non-player sprite), `palette` (unqueued object in a palette it never had
  queued) are garbage; `late` (a tile rewritten in the NMI right after it
  was drawn) is informational: streamed objects (`+$17` bit 7) change their
  tiles whenever they animate, and for a queued object the overlay is the
  ROM's own OAM in a wider window. `xtask dump` prints the summary. Rev 1
  `states/usa11` hw-a, hw-c, hw-d, hw-e, hw-f, r1-0..3, co-op (P2 as X3
  Zero), 900 frames each of walk+shoot, walk+jump+shoot (both players),
  idle, P1 jumping with P2 standing (36 runs per row):

  | config | before: garbage (stale / slot) | after |
  |---|---|---|
  | P1 Zero, 16:9, wide gameplay | 774 (0 / 774) | 0 |
  | P1 Zero, 21:9, wide gameplay | 1035 (0 / 1035) | 0 |
  | P1 X, 16:9, wide gameplay | 532 (0 / 532) | 0 |
  | P1 Zero, 16:9 | 81 (81 / 0) | 0 |
  | P1 X, 21:9 | 165 (165 / 0) | 0 |

  `page` and `palette`: 0 everywhere (P2's and Zero's shadow pages never
  reach an enemy sprite; `MarginSprite.page` is only set in the P2 path).
  `slot` only on hw-c (mech `$29`, Sky Claw `$0F`); `stale` on hw-c (two
  mechs sharing slot `$60`, the left-behind one in the left margin).
  `wide_spawns_get_their_graphics` (test, below) pins it.

  Early loading (`gfx_trigger`), same 9 states, 900 frames each, wide
  gameplay on, configs P1 X 16:9 / P1 Zero 16:9 / P1 X 21:9 x scripts
  walk+jump+shoot (right, B 20 of 45 frames, Y 3 of 40) / the same with
  P2 (X3 Zero) given P1's pad / idle, 81 runs, before (`4d548f5`) and
  after: garbage 0 / 0. `late` only on hw-c (23-28 before, 30-34 after,
  all the mech `$29`: one pass drawn the NMI before the ROM's own list-2
  upload overwrites its tiles). A queued object whose tiles the current
  list has given away (`obj_overwritten`) outside the ROM's window: one
  frame per hw-c walk (the mech, the frame the ROM's own list 2 starts;
  the window tests drop it the next frame). Netplay: `xtask netsim` (PAL
  hw-a and Rev 1 hw-c holding right, 2400 frames, `--wide-gameplay`,
  rtt 120, loss 2, prediction 12): no desync, 41 checks ok per peer,
  identical to the plain run. Flag off: `xtask verify --frames 2400
  --coop-off --tap 200,400,600,800,1000 --hold 0100` OK on both dumps.

  First seen (camera-relative X of each enemy the first frame it is
  queued; Rev 1, 1500 frames walking right with jumps, wide gameplay,
  `before` = `4d548f5`). The ROM's own spawn column gives 253-284; the
  wide one 412-432:

  | state | enemy | before | after |
  |---|---|---|---|
  | hw-c (start) | mech `$29` | 270 | 421 |
  | hw-c | id `$15` | 421 | 421 |
  | hw-c | Sky Claws `$0F` | 413 422 281 261 253 253 252 | 413 422 281 261 269 253 422 269 |
  | hw-a | mech | 421 421 | 421 421 |
  | hw-a | Sky Claws | (55 5) 422 269 268 | same |
  | hw-d (trucks) | Sky Claws | (219 199) 413 253 253 420 269 | same |
  | hw-e | Sky Claws | (25 143) 422 269 268 | same |

  (In brackets: alive at the state load.) Still at the ROM's line: the
  first Sky Claws of hw-c (1096-1120: list 2 waits for the mech, which
  holds slot `$60` on screen until X 896 unless killed; with it killed,
  `target/widescreen/early-gfx/hw-c-skyclaw-*`, list 2 loads at 736) and
  every later Sky Claw at 253-272: not graphics but the Sky Claw's own cap
  (`$82:96AF` deletes a new one while 2 are alive in the ROM's window; its
  entry is cleared, the ROM's column respawns it later).
  PNG strips (`target/widescreen/early-gfx/`, 16:9, Rev 1, left column
  before, right after, `NAME-before-after.png`): `hw-c-mech` (frames
  200-300/20: the mech walks in from the right margin, before it is not
  there yet), `hw-c-skyclaw` (600-800/25, mech killed at frame 400: the
  Sky Claws by the trucks enter further right), `hw-d-skyclaw` (60-400/40).
- Wide gameplay (`cargo test -p mmx-core --test wide_gameplay`, ROM-gated;
  `MMX_STATES=states/usa11` with the Rev 1 ROM): `window_test_m0_is_register_exact`
  (observe-only traps on the three X `BCS`s, ROM untouched, 600 frames of
  hw-a and highway-mid walking/jumping/shooting: 6754 and 4871 calls, Rev 1
  hw-a 6949; at every one `widen(.., 0)` equals the ROM's A and P and
  `widen(.., 136)`'s carry is the object's X against the widened window);
  `window_test_m0_matches_rom_run` (the trap body at M = 0 against the
  untrapped ROM, 600 frames of hw-a: `+$0E` of every object, the draw lists
  `$0920..$0A9F`, the counts `$E7..$EC` and all of WRAM equal every frame,
  save states equal at the end); `off_is_untouched` (on then off: trap set,
  `trapset_id`, 2400 frames of checksums and the final save state equal to a
  game that never had it); `enemies_spawn_at_the_wide_edges` (the table
  above); `wide_spawns_get_their_graphics` (PAL highway-start / Rev 1 hw-c,
  16:9, 900 frames: every queued enemy's `+$18` is its set's slot, the
  residency check finds no garbage); `sets_load_ahead_of_the_wide_spawns`
  (PAL highway-start / Rev 1 hw-c, walking right with jumps: the mech
  first appears at X > 400; 270 before `gfx_trigger`). Flag off against main (`8429c79`), 2400 frames, the final save
  state byte for byte and every status line: power-on with taps (PAL and Rev
  1), hw-a `--coop`, hw-a at 21:9: identical. `xtask verify --frames 2400
  --coop-off --tap 200,400,600,800,1000 --hold 0100`: OK on both dumps;
  `scripts/coop-qa.sh check`: output identical to main's. Netplay: `xtask
  netsim --load states/hw-a.state --rtt 120 --jitter 20 --loss 2 --delay 2
  --prediction 12 --frames 2400 --wide-gameplay ...`: no desync, 41 checks
  ok per peer, identical to the plain run; `xtask netreplay` of its log
  (`wide 1` in the header) matches all 40 checksums; `--wide-gameplay-host`
  (host only): the handshake refuses (`mismatch: TrapSet`) in both modes.
- Cost: Launch Octopus at 21:9 (two screens + colour math) 3.9 -> 5.8 ms per
  frame on an M-series Mac, about the main renderer's own cost per pixel.

## Performance

`cargo xtask bench --load S [dump's setup flags and script] --frames N
[--warmup 30] [--csv F]` steps the game in release and prints the wall time
per step (mean / p99 / max), the same on frames with 3+ live enemies
(`$0E68` table), and the **emulated** lag frames (the main loop counter
`$0B9B` did not move). `MMX_PROFILE=1` adds a per-phase table: every trap
body under its trap name, exclusive of nested traps (`mmx_core::profile`,
off until the host installs a clock, so the core stays clock-free for wasm),
`margin sprites`, `wide tiles`, `ppu margins` (`render_wide_margins`,
`snes_core::prof`), `emulator` (the rest of the step: CPU/PPU/APU ticks,
trap dispatch) and `take_audio`.

Rev 1 + X3, script `--hold 0100 --p1tap 4000@1-900/20:6 --p2-hold 0100
--p2tap 4000@5-900/25:6`, 900 frames, M-series Mac. "full" is the user's
launcher set (`--zero --coop --wide 16:9 --wide-gameplay`); each other row
drops one of them. Mean / max ms, before -> after the two fixes below, and
emulated lag frames (identical before and after):

| state | config | mean ms | max ms | lag frames |
|---|---|---|---|---|
| hw-a | full | 4.09 -> 3.92 | 5.26 -> 4.92 | 224 |
| hw-a | no widescreen | 3.15 -> 3.11 | 3.59 -> 3.68 | 224 |
| hw-a | no wide gameplay | 3.95 -> 3.74 | 4.67 -> 5.60 | 83 |
| hw-a | no co-op | 4.05 -> 4.02 | 4.64 -> (noise) | 317 |
| hw-a | no `--zero` | 4.28 -> 3.84 | (noise) -> 6.27 | 256 |
| hw-a | none | 3.00 -> 3.00 | 3.41 -> 3.99 | 85 |
| hw-d | full | 4.03 -> 3.82 | 6.67 -> 4.32 | 170 |
| hw-d | no wide gameplay | 4.08 -> 3.82 | 6.93 -> 4.20 | 31 |
| hw-c | full | 3.75 -> 3.60 | 5.78 -> 4.93 | 42 |
| user-lag | full | 3.58 -> 3.42 | 4.55 -> 5.81 | 39 |

Max is the noisiest column: single 12-17 ms frames show up in random runs
and configs (other processes); they are not tied to a frame or a feature
(`--csv` twice: different frames). Per phase, hw-a full, mean (max) ms:

| phase | before | after |
|---|---|---|
| emulator | 3.11 (3.57) | 3.06 (3.39) |
| ppu margins | 0.69 (1.80) | 0.55 (0.64) |
| margin sprites | 0.22 (0.67) | 0.19 (0.58) |
| co-op update / physics / contact | 0.03 / 0.03 / 0.02 | same |
| wide tiles | 0.03 | 0.03 |
| HUD, effects, Zero, take_audio | < 0.01 each | same |

So the host is not where the lag comes from: the whole step is ~4 ms with
everything on, half the 8 ms budget, the frontend's 50 ms audio ring and
4-step catch-up have room to spare, and the audio target stays at 50 ms
(raising it only adds latency). The lag the player sees is the game's own
slowdown: MMX drops frames when the 65816 cannot finish an enemy-heavy
frame, about 28% of 3+ enemy frames on hw-a with or without our features,
and wide gameplay keeps enemies alive in the margins, so 3+ enemies is the
usual case (hw-a 693 of 900 frames vs 298) and lag frames go from 83 to 224
(hw-d 31 -> 170). Removing that is a simulation change, not a host
performance fix: see "Slowdown and overclock" below.

Checked and left alone: the trap probe `next_tick_fetches_opcode` (CPU
clone) only runs on a trapped address; the margin pass snapshots WRAM and
the CPU once per pass, not per object; `MMX_MARGIN_CHECK` is off unless set;
contact, co-op slices, the wide tile map and `take_audio` are each under
0.03 ms. Fixed: the trap table lookup at every instruction boundary went
through two SipHash lookups (~5% of the step in a sample): now a 64 Kbit
filter on the address's low 16 bits answers untrapped PCs; the margin
renderer fetched the map entry and tile per pixel and evaluated the windows
per pixel: now once per 8-px tile and once per margin side per line. Both
pixel- and state-identical (dumps of 18 state x preset runs, PNGs and
status lines, equal to the previous build).

## Slowdown and overclock

The slowdown above is the 65816 running out of time before the next NMI, so
the fix is a faster CPU: `Game::set_overclock(Overclock::{Off, X1_5, X2})`
(`snes_core::api::CpuOverclock`, `// mmxrs patch` in `api.rs`).

- **What changes.** Every CPU bus cycle costs fewer master cycles:
  `mclk * 2 / 3` at 1.5x, `mclk / 2` at 2x. Fast (B bus, I/O, idle) 6 -> 4 /
  3, slow (ROM, WRAM) 8 -> 5 / 4, joypad ports 12 -> 8 / 6. That is the only
  change: the cycle the CPU just ran is scaled in `SnesEmulator::tick`, and
  the DMA unit's end-of-DMA alignment ("a whole number of CPU cycles since
  DMA began", `next_cpu_cycle_mclk`) uses the same scaled length.
- **What stays at hardware timing** (all in master cycles): the PPU and its
  frame rate, HDMA and general DMA (8 master cycles per byte, start on a
  multiple of 8), the 40-cycle WRAM refresh per line, H/V IRQ and NMI
  positions, auto-joypad read, the APU/SPC700 (own clock, fed the same master
  cycles) and so the audio rate. The multiply/divide unit counts CPU cycles,
  so it finishes in the same number of instructions. The CPU simply gets
  ~1.6x / 2x the instructions per scanline (synthetic ROM loop: 8626 ->
  13654 / 17255 iterations per 10 frames, `overclock_runs_more_instructions_per_frame`).
- One vendored assert went: `update_irq_line` asserted that every tick moves
  the H counter, which a 3-master-cycle tick need not; the checks themselves
  already treat "no dot crossed" as no edge.
- **Deterministic and outside save states.** The factor is a host setting
  like the widescreen margin: it encodes to nothing (old and new states are
  byte-identical), `load_state` and `hard_reset` keep the running game's
  factor, so a state runs at whatever the loading game is set to. Netplay
  puts it in the trap-set id (docs/netplay.md), so both peers always agree.
- **Defaults.** `mmxrs --overclock off|1.5|2`; **default 2x** (nobody wants
  the slowdown; `off` is for purists and slow hosts). The launcher has no
  control for it; web: "CPU overclock", auto = 2x. `xtask dump/bench/verify/netsim/bginfo/trace --overclock F`
  default to off whatever `--wide-gameplay` says (parity runs stay plain).

Measured (`cargo xtask bench`, the user's set: Rev 1 + X3, `--zero --coop
--wide 16:9 --wide-gameplay`, the script above, 900 frames). Lag frames
(`$0B9B` did not move) and host mean / max ms per step:

| state | off | 1.5x | 2x |
|---|---|---|---|
| hw-a | 224 lag, 3.9 / 10.9 ms | 1 lag, 4.9 / 13.7 ms | 0 lag, 5.4 / 6.0 ms |
| hw-d | 170 lag, 3.7 / 4.2 ms | 1 lag, 4.6 / 5.2 ms | 1 lag, 5.2 / 7.6 ms |
| hw-c | 42 lag, 3.5 / 4.9 ms | 26 lag, 4.3 / 5.7 ms | 22 lag, 5.0 / 8.2 ms |

The hw-c remainder (and the single hw-a/hw-d frame) is not slowdown: they
come in runs (`--csv` has a `lag` column), e.g. frames 855-872 are a black
death fade where the game holds its main loop on purpose. The "3+ enemies"
count drops (hw-a 693 -> 395 frames) because without slowdown the same 900
frames cover more of the stage. Host cost grows ~40% (the CPU spins its
idle loop twice as often), still well under a 16.7 ms frame natively; on
the web build that margin is smaller.

Checked (PAL dump and its states unless noted):

- Parity: `--overclock off` against the previous build, both dumps (PAL
  hw-a, octopus-start + wide gameplay; Rev 1 hw-a with the full set, hw-d):
  PNGs, WRAM, status lines and the save state after 600 frames identical.
  `xtask verify --frames 2400 --coop-off` with oracle and DUT both at off /
  1.5 / 2 (PAL) and 2 (Rev 1): identical (deterministic). `netsim` 2400
  frames, rollback (wide gameplay) and lockstep (1.5x), both peers
  overclocked: no desync, equal to the plain run; `--overclock-host 2`
  alone: `closed (mismatch: TrapSet)`.
- Off vs 2x, same scripts: Highway start to the pit (545 frames right, the
  fall, death, respawn), Octopus water HDMA (300 frames walking), the pause
  menu, the game-over continue: same positions, HP and modes; PNGs equal
  except a frame or two where a CPU-bound load or fade finishes a little
  sooner (game over -> stage: the stage is up by frame 150 instead of
  later). Bee Blader 1 with steady fire: the fight runs ahead at 2x (the
  road collapses earlier), no visual errors.
- Audio: 600 frames from highway-start and hw-a give the same sample count
  (32 kHz, 383946/383947), no clipping, music level per second within
  noise of the off run (identical while the gameplay matches); the SPC
  handshake and music are unaffected.
