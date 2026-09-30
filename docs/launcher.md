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
* **Options**: window scale 1-8 (`--scale`), `--fullscreen`, Play as Zero
  (`--zero`, needs the X3 ROM), Local co-op (`--coop-local`, ignored online).
* **Play online**: Host/Join (`--coop-host`/`--coop-join ROOM`), room with a
  random 8-char button, signal URL (default `wss://signal.z2rs.com`), mode,
  delay (auto = the mode's default; rollback 0-3, lockstep 0-8), ICE (empty =
  game default, `none` allowed). Host and guest need identical ROM dumps and
  the same mode.
* **Launch / Stop**: Launch is disabled while the X ROM is missing or has the
  wrong header, or Zero is ticked without a valid X3 ROM. The game's stdout +
  stderr go to `<data-dir>/last-game.log` (a file, so the game survives the
  launcher closing); the last lines show under the button.

Game program: `mmxrs` next to the launcher, else `./target/release/mmxrs`,
else `mmxrs` on PATH. Settings: `<data-dir>/launcher.json`, where data-dir is
`$XDG_DATA_HOME/mmxrs`, `~/Library/Application Support/mmxrs` (macOS),
`%APPDATA%\mmxrs` (Windows) or `~/.local/share/mmxrs`.
