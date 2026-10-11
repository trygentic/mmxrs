# Private bone animation sheets

`tools/bone_sheets.py` extracts transparent character poses from a user-owned
Mega Man X ROM, cuts each idle pose into pixel-grid parts, and authors ten bone
clips: `idle`, `run`, `dash`, `jump`, `fall`, `climb`, `wall_jump`, `shoot`,
`attack`, and `hurt`. The source ROM never changes. X1 Zero is the default Zero
source. Bass, Proto Man, and on-foot Vile use supplied PNG sheets staged under
the private `roms/derived/bone_sheets/user_sources/` directory. Zero's supplied
extra sheet provides movement poses as references only; no pixels from it
appear in the playable Zero sheet.
The Vile source image credits QuadFactor for the rip and Capcom for the
original sprites. It is kept in the private asset folder with that credit.

The source PNGs, part textures, editor rigs, previews and exported sheets are
copyrighted or derived art. Keep them under `roms/` or the editor's ignored
`assets/anims/mmxrs_*`, `assets/img/mmxrs_*`, and `assets/rigs/mmxrs_*` paths;
never commit them. Only pipeline scripts and documentation belong in Git.

```sh
python3 tools/bone_sheets.py --rom roms/mega-man-x.smc --out roms/derived/bone_sheets
python3 tools/bone_sheets.py --build-work --out roms/derived/bone_sheets
python3 tools/bone_sheets.py --import-all --out roms/derived/bone_sheets
python3 tools/bone_sheets.py --render-all --out roms/derived/bone_sheets
python3 tools/bone_sheets.py --validate --out roms/derived/bone_sheets
```

The output pack has `manifest.json` and one `bae-sheet` v1 JSON plus PNG atlas
per character at 60.0988 fps. The bone editor's file-mode checker validates the
editable `.anim` rigs. `--render-all` samples those rigs with the skill's
pixel-grid preview renderer, checks for clipping and ground contact, and packs
the frames. `review/run-contact.png` shows eight run footfalls for every
selectable character. `tools/x_bone_rig.py` cuts each original X1 ROM run pose
05–12 into helmet, torso, arm, buster and leg textures. `tools/zero_bone_rig.py`
cuts the actual X1 ROM Zero idle, sprint, and airborne art into independent
hair, head, torso, arm, and leg textures. Each 60.0988 fps frame poses those
parts at native pixel resolution, with small hand-authored occlusion repairs.
No complete source pose is pasted into the export. Its
`work/zero/run_composition.json` records each part's transform for every
output run frame. Grounded run frames share one foot line.

## Blender-rendered replacements

The same optional `--bone-sheets DIR` pack can contain high-resolution Blender
renders. In the launcher this is **60 FPS generated character sprites**. A
sheet remains `bae-sheet` v1 with `pages` and `animations`; the runtime uses
these additional, backward-compatible fields:

```json
{
  "format": "bae-sheet",
  "version": 1,
  "render_scale": 4,
  "source_facing": "right",
  "pages": [{"file": "x_000.png", "size": [4096, 4096]}],
  "animations": [{
    "name": "run",
    "fps": 60,
    "looped": true,
    "frames": [{"page": 0, "rect": [2, 2, 128, 144], "pivot": [64, 144]}]
  }]
}
```

`render_scale` is source sprite pixels per ROM/game pixel; supported values
are 1, 2, 4 and 8. It defaults to 1. High-resolution pixels are decoded intact:
the completed PPU frame is nearest-neighbour enlarged to the largest scale of
the active characters, then sprites are composited at their original density.
Legacy sprites and the HUD keep their original game size and pixel grid.
This changes presentation only, never character physics, camera coordinates,
collision, projectile timing or the emulator clock. `Game::frame_scale()`
reports the output density to frontends so widescreen and optional effects
continue to use game-pixel coordinates.

Every frame's `pivot` is measured in **source pixels**, local to its cropped
rectangle. It marks the fixed character origin at the standing foot line,
which is anchored to the ROM object screen origin plus 16 game pixels in Y.
Do not normalize pivots to the current visible feet or bake a jumping root's
travel into the drawing: compensate any authored root travel in the pivot.
Trimming changes the pivot by the same crop offset. Pivots may lie outside
the crop. Mirroring happens around that origin. `source_facing` is `right` or
`left`; omitting it preserves each legacy character's source direction.

The sample index is `floor(elapsed_emulated_frames * clip.fps / game.fps())`.
Clips restart when the selected clip changes or a save state is loaded. Pausing
holds the frame. `looped: false` holds the final exported frame, while `true`
wraps it. Omitting `looped` retains the former looping behavior. A 60 FPS
export therefore follows real emulated time on both NTSC and PAL; its rate is
independent of the monitor refresh rate.

The main runtime names are `idle`, `run`, `dash`, `jump`, `fall`, `climb`,
`wall_jump`, `wall_slide`, `shoot`, `attack`, `hurt`, and `death`. Combined
firing poses use `idle_shoot`, `run_shoot`, `jump_shoot`, `fall_shoot`,
`dash_shoot`, `climb_shoot`, `wall_slide_shoot`, and `wall_jump_shoot`.
New packs can also provide `land`, `land_shoot`, `ready`, `spawn`, `beam_out`,
`hold`, `captured`, `captured_down`, and `item_get` for their explicit ROM
states; landing retains its authored pose while game physics moves horizontally.
`attack` can follow an active optional Zero saber swing. These names
change visuals only; they do not add a saber move to X1 Zero or Sigma.

Hurt/death take precedence. Firing variants use the ROM's actual pose IDs,
not simply a held fire button. If a high-resolution or explicitly oriented
pack lacks a combined firing pose, it keeps the corresponding locomotion
pose. Old packs retain their prior selection. Optional `{base}_charge`
clips can use the existing charge state, but charge lights, sparks and shots
remain runtime effects and should not be baked into character textures.
`action_XX` (lowercase hexadecimal) overrides a rare action if supplied.
Unrecognized or absent animations retain the legacy alias/idle fallback;
aliases are `shoot` → `attack`, `climb` → `ladder`, `wall_jump` → `wall_kick`.

Install only the intended character subfolders. The loader selects the
requested character IDs and leaves other sheet assets alone. A complete
animated Vile sheet takes precedence over his old static PNG and does not
require that fallback PNG to be present. Pages remain within the character's
sheet folder and have PNG edges at most 4096 pixels; a declared page `size`
must match its actual PNG. Rendered sprites, atlases and pack receipts remain
in ignored private asset folders.

## Legacy generated sheets

The runtime mirrors each sheet relative to its source-facing direction;
X and X1 Zero's ROM source faces left, while Vile, Sigma, D-Rex and Proto Man
source art faces right. Bass and the other directional boss art face left.
X's `jump`, `fall`, and `wall_jump` use six independently moved layers cut
from native ROM poses 23–25. Zero's corresponding clips use transformed
cutouts from X1 ROM airborne pose 21. His dash, climb, and other actions use
the X1 ROM standing pose rig.
The neutral rig reconstructs the source pixel for pixel. `tools/vile_bone_rig.py`
cuts Vile's supplied on-foot idle sprite into 16 pieces, including his shoulder
cannon, arms, legs, and boots. His run also cuts six supplied motion drawings
into eight separate layers each, then moves those layers between authored
poses at 60.0988 fps. Vile's jump, fall, and wall jump reuse raised-knee cutouts from those clean
sprint drawings; the spark-covered poses in the source are damage art.
His remaining clips use the 16-part idle rig.
`tools/sigma_bone_rig.py` cuts Sigma's ROM sword stance and lunge drawings
into separate armor, arm, leg, head, and sword textures for his run and
airborne clips. His ROM art has combat strides but no authored run cycle.
The renderer selects run while a player moves horizontally, including when
the ROM marks the current frame as shooting. No clip pastes
a whole source frame. A live
Bevy editor GPU export of X was also verified at 60.0988 fps; its bilinear
texture filtering softened the original pixel art, so the preview renderer's
sharper frames are used by the game.

`tools/bass_proto_bone_rig.py` uses eight run silhouettes from the supplied
Bass sheet. Each 60 Hz frame composes independently moved native-pixel body,
head, arm and leg parts. The supplied Proto Man sheet has no run strip; his
run uses its forward-leaning dash and firing poses, with separate scarf,
helmet, arm and boot parts following X's eight-beat stride.

These rigs are an automated first pass. Native pose coverage is strongest for
X and X1 Zero. The X1 bosses have no playable walk or ladder sets, so those
motions are generated from their ROM idle art. Wolf Sigma's original sprite is
a stage-scale composite, and Rangda Bangda contributes an eye sprite rather
than a whole body. Hand-cut joints and per-character pose work would improve
their silhouettes and movement.

## Saber contacts

Zero and Sigma use the loaded `attack` clip's timing. Without authored
contacts, the runtime derives each frame's contact rectangle from its long
green blade component; the body, cape and boots do not expand the hitbox.
The rendered swing and collision select the same frame from the saber timer,
including after restoring a game state.

An optional `<character>/<character>.saber_contacts.json` provides precise
blade contacts from a separate weapon render. Its format is
`mmxrs-saber-contacts-v1`, with `character`, `clip: "attack"`, `fps`,
`coordinate_origin: "foot_pivot"`, and the loaded sheet's `sheet_sha256`.
`frames` contains ordered `{index, bounds}` entries. `bounds` is null when
no blade is visible, otherwise `[left, top, right, bottom]` in integer logical
game pixels relative to the foot pivot, normalized to face right. Right and
bottom are exclusive. The loader adds 16 game pixels to Y to convert to the
ROM object origin. Frame count and FPS must match the attack clip; stale
sheet hashes and invalid rectangles fail loading rather than silently using
misaligned contacts. HD render scale never enlarges the gameplay rectangle.
