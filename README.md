# mmxrs

mmxrs rebuilds Mega Man X for the SNES in Rust. Your own cartridge dump runs
inside an SNES emulator core, and routines ported to Rust take over from the
original code one by one. Every ported routine is checked frame by frame
against the unmodified game.

On top of the original game it adds local co-op for up to four players,
online co-op over WebRTC, Zero as a playable character, widescreen, and a
launcher window for choosing the ROM and options.

Download builds from the [Releases](https://github.com/trygentic/mmxrs/releases)
page. There are builds for macOS (Apple Silicon), Windows (x86-64) and Linux
(x86-64). Each zip holds the game and the launcher.

## You need a Mega Man X ROM

No part of the game is included here or in the downloads. You need a dump of
a Mega Man X cartridge you own, as an `.sfc` or `.smc` file. The USA v1.1
(Rev 1) and Europe v1.0 dumps are tested. Other revisions may refuse co-op if
the code the game hooks has moved; the launcher shows the ROM's title,
version and SHA-256 when you pick it.

A Mega Man X3 (USA) dump is optional. With it, Zero gets his full animation
set and saber.

## Play on macOS

You need an Apple Silicon Mac (M1 or newer).

1. Download `mmxrs-<version>-macos-arm64.zip` from the latest release and
   unzip it.
2. Move `mmxrs.app` to your Applications folder.
3. The app isn't signed by Apple, so macOS blocks it the first time.
   Right-click `mmxrs.app`, choose Open, then confirm. If macOS still refuses,
   run this once in Terminal:

   ```
   xattr -dr com.apple.quarantine /Applications/mmxrs.app
   ```

4. Open the app. The launcher opens; choose your ROM and click Launch.

## Play on Windows

1. Download `mmxrs-<version>-windows-x86_64.zip` and unzip it anywhere.
2. Run `mmxrs-launcher.exe`. If SmartScreen warns about an unknown publisher,
   click More info, then Run anyway.
3. Choose your ROM and click Launch.

Keep `mmxrs.exe` in the same folder as the launcher; the launcher starts it
from there.

## Play on Linux

The build targets x86-64 with glibc 2.36 or newer (Debian 12, Ubuntu 23.04,
Fedora 37 and later). It needs ALSA and udev, which desktop distributions
install by default.

1. Download `mmxrs-<version>-linux-x86_64.zip` and unzip it.
2. Run `./mmxrs-launcher` from that folder.
3. Choose your ROM and click Launch.

## The launcher

The launcher sets up a game and starts `mmxrs` with the matching options. It
remembers your choices between runs.

- ROMs: drop the files on the window or browse for them.
- Options: window scale, fullscreen, widescreen (16:10, 16:9, 21:9 or follow
  the window), optional lighting and charge effects.
- Players: one to four local players, each with a character choice, or
  two-player online co-op with a room name.

You can also skip the launcher and run the game from a terminal:

```
mmxrs --rom "Mega Man X (USA).sfc" --coop-local
mmxrs --help
```

## Controls

Player 1 uses the keyboard: arrow keys to move, Z jump, X shoot, C dash, V
saber (with the X3 ROM). Player 2 on the keyboard: WASD, G jump, F shoot,
H dash, T saber. Players 3 and 4 need gamepads. A gamepad joins the first free
slot when you press a button on it.

Esc quits, F11 toggles fullscreen, F10 cycles widescreen, F5 and F7 save and
load a state, and the number keys pick the save slot.

## Online co-op

One player hosts a room and the other joins it with the same room name. Both
need identical ROM dumps and the same mode. The launcher fills in the default
signalling server; you can run your own with `mmx-signal` from this
repository.

## Building from source

You need Rust 1.96 or newer.

```
cargo build --release -p mmx-native -p mmx-launcher
./target/release/mmxrs-launcher
```

On Linux, install the ALSA and udev development packages first (on Debian and
Ubuntu: `libasound2-dev libudev-dev pkg-config`). Tests that need the ROM look
for it in `MMX_ROM` and skip when it is unset.

## Reporting problems

Open an issue on this repository. Say which build you used, which ROM dump
(the launcher shows its SHA-256), what you did and what happened.

## License

mmxrs is licensed under the GPL-3.0 (see [LICENSE](LICENSE)), because it
includes the [jgenesis](https://github.com/jsgroth/jgenesis) emulator core.
See [LEGAL.md](LEGAL.md) for what is and isn't in this repository.

Mega Man is a trademark of Capcom. This project is not affiliated with or
endorsed by Capcom.

## Contributors

Ryan Zaki and Troy Edwards.
