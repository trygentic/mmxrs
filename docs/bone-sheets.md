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
