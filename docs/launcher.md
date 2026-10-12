# Launcher (`crates/mmx-launcher`, binary `mmxrs-launcher`)

A small egui window (eframe, glow backend) that picks the ROMs and options and
starts the game with the matching flags. Shape copied from z2rs's
z2-launcher, shrunk. It does not link the game; `crates/mmx-native/src/cli.rs`
stays the source of truth for the flags (docs/native.md, docs/netplay.md).

```sh
cargo build --release -p mmx-native -p mmx-launcher && ./target/release/mmxrs-launcher
./target/release/mmxrs-launcher --print-args   # the command it would run, from the saved config
```

The window:

* **ROMs**: Mega Man X (USA) and, optionally, Mega Man X3 (USA). Browse, type
  the path, or drop the file on the window (an X3 header goes to the X3 slot).
  Each shows its header: title, version, SHA-256 of the image with any 512-byte
  copier header stripped (same hash as `cargo xtask rominfo`).
* **Options**: window scale 1-8 (`--scale`), `--fullscreen`, Widescreen off/16:10/16:9/21:9/auto (`--widescreen`; auto
  follows the window's shape, so maximized or fullscreen fills the screen), "spawn enemies at the
  wide edges (changes gameplay; both players)" (`--wide-gameplay`, docs/widescreen.md
  "Wide gameplay"; online both players must tick it), optional multiply
  lighting (`--dynamic-lighting`), and optional X/Zero charge glow and
  particles (`--charge-effects`). The optional 60 FPS bone animation sheet
  toggle selects a private `bae-sheet` folder (`--bone-sheets DIR`); it starts
  at `roms/derived/bone_sheets` and missing action clips use idle or the ROM
  sprite. Bass and Proto Man require sheets from that folder. The pack is
  private art, not part of a release download: with the option ticked the
  launcher says whether the folder is a usable pack (and which characters it
  has) and refuses to launch with a missing or empty one; the game itself keeps
  the ROM sprites when the folder or the selected characters' sheets are
  missing, and only stops for Bass or Proto Man. Four-player HP
  meters run vertically along the screen sides. No overclock control: the game's default
  (2x) applies; `mmxrs --overclock off` from a terminal restores the slowdown.
* **Sprite pack**: shown when a player is Zero: a path to a custom Zero
  sprite pack (`--sprite-pack`, docs/sprite-packs.md), with Browse and Clear.
* **Players**: 1–4 local controller slots (`--players N`) or two-player online
  co-op, with a character dropdown for each active player. X and Zero have
  playable art from the X1 ROM. Zero uses his X1 standing and dash poses by
  default; an optional X3 ROM supplies his full animation set and saber.
  Every listed boss uses graphics and a
  palette extracted from the selected X1 ROM at startup. Sigma and Vile cycle
  ROM poses while moving; other bosses use their idle pose when a matching
  action pose has not been mapped. Wolf Sigma's large face is reduced to a
  player-sized sprite, and Rangda Bangda uses its ROM sprite component.
  Selecting Vile loads the private on-foot PNG from
  `roms/derived/bone_sheets/user_sources/vile_idle.png` even when animated
  sheets are off; its path can be changed in the Vile sprite field. The PNG is
  optional and not in a release: without it Vile uses his ROM poses.
  Each local player has its own sprite page and palette. In co-op (two or more
  players, or online) the launcher warns, without blocking, when a player is
  anyone but X or X3-ROM Zero and the enabled 60 FPS pack has no sheet for
  them: X1 Zero and the bosses use rough X1 sprite art.
  For online co-op,
  the online controls show only for Online co-op: Host/Join (`--coop-host`/`--coop-join ROOM`), room with a
  random 8-char button, signal URL (default `wss://signal.z2rs.com`), mode,
  delay (auto = the mode's default; rollback 0-3, lockstep 0-8), ICE (empty =
  game default, `none` allowed). Host and guest need identical ROM dumps and
  the same mode.
* **Launch / Stop**: Launch is disabled while the X ROM is missing or has the
  wrong header, or while a file the game would refuse is missing (X3 ROM,
  sprite pack, bone sheet folder); the reason shows under the
  button. Bass and Proto Man have no ROM art: their dropdown entries are greyed
  out (hover for why) until the bone sheet folder holds
  `<id>/<id>.sheet.json` for them, and VS mode stays off until every fighter
  has its art. The game's stdout +
  stderr go to `<data-dir>/last-game.log` (a file, so the game survives the
  launcher closing); the last lines show under the button. When the game
  exits with an error, a red panel names the exit code and, for the known
  causes (no ROM, unreadable ROM, missing bone sheets or Vile sprite, netplay
  connect failure, launcher/game version mismatch), says what to fix; the raw
  output stays visible below it.

Game program: `mmxrs` next to the launcher, else `./target/release/mmxrs`,
else `mmxrs` on PATH. Settings: `<data-dir>/launcher.json`, where data-dir is
`$XDG_DATA_HOME/mmxrs`, `~/Library/Application Support/mmxrs` (macOS),
`%APPDATA%\mmxrs` (Windows) or `~/.local/share/mmxrs`.
