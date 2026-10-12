# Zero sprite packs

A sprite pack replaces Zero's body art with your own, in every frontend:

- native: `mmxrs --sprite-pack pack.json` (with or without `--x3-rom`);
- launcher: "Custom Zero sprite pack" under the players, shown when a
  player is Zero;
- web: the "Custom Zero sprite pack" picker (`Emu::import_sprite_pack`).

It takes the place of the X3 set (`Game::set_zero_set`): every player whose
roster character is Zero wears it, drawn by the X3 Zero path
(docs/zero.md "Drawing P2 as X3 Zero"). A pack is **body art only**: no
slash arc, wave, sounds or HUD icon. The saber itself is Zero's whatever he
wears (`saber::armed`: the arm part, docs/zero-upgrades.md), so a packed
Zero swings it without X3's effects, as X1-art Zero does. Without a pack or
X3, Zero uses his X1 ROM sprites with a standing fallback (the roster's
boss-set path).

Netplay: the loaded art is part of the handshake id (`netgame::trapset_id`),
so both peers must load the same pack, and the same X3 ROM, before
connecting; a mismatch refuses to start. No sprite assets are bundled.

## Format

A UTF-8 JSON object, at most 8 MiB, unknown fields rejected:

- `version`: `1`.
- `tiles`: array of bytes, SNES 4bpp planar tiles, 32 bytes per 8x8 tile.
- `palette`: exactly 16 BGR555 integers (0-32767); slot 0 is transparent.
- `frames`: exactly 138 frame objects in the playable X3 numbering
  (docs/zero.md "X3 Zero data" lists the poses; repeat the standing frame
  for animations you do not draw).

Each frame has `tiles_used`, exactly 48 integers mapping the tile page's
slots (three rows of 16) to indices in `tiles` (65535 = blank), and
`sprites`, up to 64 objects:

```json
{"x": -8, "y": -24, "big": true, "tile": 0, "hflip": false, "vflip": false}
```

`x`, `y` are signed bytes from the player's origin, facing left (the game
mirrors for right); `big` is 16x16 (else 8x8); `tile` is a page slot 0-47,
and a big sprite also uses `tile + 1`, `tile + 16` and `tile + 17`, so it
cannot sit in column 15 or in the last row. The importer
(`zero_data::import_sprite_pack`) rejects anything outside these bounds
(test `sprite_pack_validates_before_use`).

`cargo xtask zerosheet --x3 PATH --out DIR` dumps the X3 frames as a
reference for the numbering and the origin.
