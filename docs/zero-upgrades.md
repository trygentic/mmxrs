# Zero's upgrades (planned 2026-10-11, shipped the same day)

Goal: the armour capsules work for a Zero player, in every player mix
(1P X, 1P Zero, co-op X+Zero, X+X, Zero+Zero, up to four local players
with the roster), with Zero's saber turned into one of the parts.
Passwords and netplay keep working.

Measured on the PAL v1.0 dump (`roms/mega-man-x.smc`, sha256 `1c41bb11…`,
our default `MMX_ROM`) unless marked; hook signatures checked on USA v1.0
and v1.1 as well. "Zero" below means a Zero with the X3 ROM loaded (the
saber); in the roster (`--pN-character`, `Game::player_characters`) that is
a player whose character is Zero while the X3 set is loaded
(`Game::current_shadow_is_zero` for players 2-4, `--zero` for player 1).
Zero without the X3 ROM is X1 art with a standing fallback
(`Game::set_boss_set`) on X's moveset, and gets every capsule exactly as X,
dialog included; so does every boss character.

## What each capsule gives

| capsule | `$1F99` bit | X | Zero (X3) | dialog |
|---|---|---|---|---|
| Chill Penguin (legs) | `$08` | dash | dash (identical) | X's (ROM), both players |
| Storm Eagle (head) | `$01` | head-butt blocks | identical (ROM terrain code) | X's |
| Sting Chameleon (body) | `$04` | half damage | identical (ROM damage code) | X's |
| Flame Mammoth (arm) | `$02` | charge level 3, charged weapons | **the Z-Saber** (no saber before it); X's level 3 charge too | Zero's own (below) |
| Armored Armadillo's hidden one | `$1F7E` bit 7 | Hadouken | **the green wave** on a full-charge swing; no Hadouken | Zero's own |

**The bit map differs from the plan's guess** (head 01, body 02, arm 04):
the ROM has arm `$02`, body `$04`.

- arm `$02`: poke `7E1F99=02`, hold Y 260 frames from `highway-start`:
  `+5B` (`$0C03`) reaches 1 (the third level) at frame 200; with `00` or
  `04` it stays 2. The charge code reads it at `$81:97DD` (`BIT #$02`);
  Zero's arm-part gift in Sigma's fortress (`$88:D7D3`) sets `$02` too.
- body `$04`: poke, walk into the Spiky from `hw-spiky`: HP 16 -> 15 with
  `04`, 16 -> 14 with `00`, `01`, `02` (damage halved rounding up,
  `$84:9D44 BIT #$04`).
- legs `$08`: dash (docs/zero.md).
- head `$01`: unverified by poke (no breakable block on the Highway). From
  the capsule's level -> bit table `$86:D306`/`$86:D379` (level 5, whose
  capsule text is the helmet's, message `$0C`, gives `$01`) and the terrain
  code's `BIT #$01` (`$81:9EA5`).
- Hadouken: `$1F7E` bit 7, set by the capsule at `$87:D0D9` (`TSB`, level
  3); bits 0-6 are that capsule's appearance counter (`$87:CA8D`: it
  shows from the fifth visit with all parts and hearts `$1F99 & $1F9C =
  $FF`, `$1F7C` bit 6, HP `$20`). The motion check is `$81:A0EA` (no
  weapon, HP exactly `$20`, the flag, standing/walking); X's shot from it
  has id 4 (poke `1F7E=80 1F9A=20 0BCF=20`, down, down-forward, forward+Y:
  action `$42` at frame 28, shot `+0A = 4` at 45).

Constants: `ram::ARMOR_HEAD/ARM/BODY/LEGS`, `ram::HADOUKEN`, `HADOUKEN_BIT`.

## Capsule scene (measured)

State: `states/cap-legs.state` (gitignored), the 100% TAS 3197M
(`movies/hetfield90v3-megamanx-100p.bk2`) at frame 12040, played on the PAL
dump at NTSC timing (`MMX_TIMING=ntsc`: it syncs there exactly as on the
USA v1.0 dump), 197 frames before the Leg Parts scene; and
`states/cap-text.state`, the same run with `--coop --x3` saved at frame
390 (4 frames before the text starts; P2 present, far below). Both load
with `MMX_TIMING=ntsc` (tests use `Timing::Ntsc`).

```
MMX_TIMING=ntsc xtask dump --movie movies/hetfield90v3-megamanx-100p.bk2 --frames 12040 --out DIR --save 12040=states/cap-legs.state
MMX_TIMING=ntsc xtask dump --x3 X3 --load states/cap-legs.state --movie movies/hetfield90v3-megamanx-100p.bk2 --movie-offset -12040 --frames 390 --out DIR --coop --save 390=states/cap-text.state
```

Timeline from `cap-legs` with the movie continued (`--movie-offset
-12040`; movie frame = +12040):

| frame | what |
|---|---|
| 197 | X jumps into the trigger box: action `$1E` (hold), `$0C16 = $1E` |
| 303 | the dialog object (id `$2A`) spawns |
| 394 | the text starts (`$1F32 = $0B`) |
| 995 | text done, X free (action 0); the TAS walks into the capsule |
| 1018 | X in the capsule: action `$1E` again |
| 1337 | `$1F99 |= $08` |
| 1481-1540 | action `$38` (the powered-up pose), then free |

`$D3` stays 4 and `$1F24` 0 throughout; the scene is X's actions.

**The capsule** is an object in the enemy table, update `$87:CA7B`
(dispatch on `+01`, then `+02`). Init `$87:CA86`: level 3 is the
Hadouken's (conditions above); every other level despawns if its bit is
owned (`$86:D379[$86:D306[$1F7A]] & $1F99`). State 2 (`$87:CBBB`): the
trigger box `+20 = $86:D2DA` through **the enemy-contact routine
`$84:9B03`** (box test `$84:9C0E` with `X = $0BA8`, absolute; contact damage
`+26 = 0`, so `$84:9D28` skips the damage). On a hit: sound `$F1`, state 4,
`$1F49 = 4`, `JSL $84:9FEB` (X's action `$1E`, `$0C16`), the camera
limits saved and locked. State 4 waits for the camera, then Dr. Light's
hologram (bank `$88`, `$88:A793`) spawns the dialog object (`$83:F74A`)
with the message `$86:D487[$1F7A]`: level 2 `$0D` (body), 3 `$19`
(Hadouken), 4 `$0E` (arm), 5 `$0C` (head), 8 `$0B` (legs). State 6 (`$87:CC84`)
waits for X in the capsule box `$86:D2DE` (the same contact routine); state 8
centres X (`STA $0BAD`), clears pad 1 (`$A7/$A9/$AB/$0BDE/$0BE0`), loads
the palettes and holds X; the grant is `$87:D0AF` (`LDA $D379,X; TSB
$1F99`, `X = +0B`), the Hadouken `$87:D0D9`.

**The text.** The dialog object `$83:F51A` opens the window and calls
`$80:E9AD` with the message id: `$1F32 = id`, and the text task `$80:E686`
starts. It loads `$68 = $84:C890[id & $3F]` (`$80:E692`, a 16-bit pointer,
bank `$84` fixed) and reads every byte through `$80:E953`: `PEA $8684; PLB;
LDA ($68),Y; INY; PLB; RTS` (DB = `$84` for the read). Encoding: ASCII
`$20-$7E` (the character is the BG3 tile number); control codes (handlers
`$80:E78D[code & $0F]`): `80` newline (`$6A += $20`, `$68 += Y`, `Y = 0`;
lines are double spaced, `80 80`), `81` wait for P1's button with a
blinking cursor (`$80:E9A4`: `$0BDE | $0BDF & $F0`, X's remapped input, so
P2 can never advance it), `82` end, `83 n` skip n columns, `84`/`85 n`
two-row glyphs, `86 n` scroll the box up n rows (**the page turn**: `2 *
lines + 1`), `87 n` pause n frames (a button cuts it), `88 n` speed, `89 lo
hi` position, `8A` Light's portrait step (`$1F3C++`), `C0-CF` colour.
A page ends `80 80 83 09 81 80 86 nn 87 1E`, a message `... 81 80 [8A] 86
nn 82`. The engine pages, so X+Zero is X's pages plus one.

**Cutscene lock.** X's actions `$1E` (text, capsule) and `$38` (pose) are in
`coop_stages::p1_scripted`, so P2's pad is dropped for the whole scene
(docs/coop.md "Cutscenes").

Debug tooling added: `xtask trace` takes `--load`, `--movie
[--movie-offset]`, `--poke` and `--watch ADDR[:LEN],..` (every instruction
that changed those WRAM bytes, `Game::trace_frame_each`); that is how
`$84:9FED` (action), `$83:F758` (dialog), `$80:E69F` (pointer) and
`$80:E9B3` (message id) were found.

## Zero's dialog

`capsule.rs`: one `Observe` trap on the reader `$80:E953` (hook
`text_read`; the table address comes from the operand at hook `text_ptr`).
For message `$0E` (arm) and `$19` (Hadouken) it serves our message's byte
at `$68 + Y - start` and returns for the ROM (A, `Y + 1`, DB = `$86`, N set,
Z clear, as the routine leaves them); any other message reads the ROM. The
audience is the active players (1, or with co-op the local count, at least
2): Zero = a player whose roster character is Zero with the X3 set and no
X1 art set for him; X = everyone else.

- only X (X alone, X+X, ...): the ROM's text;
- only Zeros (Zero alone, Zero+Zero, ...): the ROM header + Zero's pages
  (`zero_data::ZERO_ARM`, `ZERO_WAVE`) + the ROM's last-page codes;
- a mix (X+Zero either way round, or any mix of up to four): the ROM's
  pages, a page turn, Zero's page
  (`ZERO_ARM_TOO`, `ZERO_WAVE_TOO`).

Built per read from the ROM's message (pure), so it is a function of WRAM
(`$1F32`, `$68`, `Y`) and the handshake's settings: rollback replays it.
Lines are at most 16 characters (the capsule's top sprite covers the box's
first columns, as it does on the ROM's own text: "capabilities" shows as
"abilities").

Checked from `cap-text` with the arm message forced (`--poke
7E1F32=0E@1-1100`; the legs capsule still grants the legs), Right held, B
for 12 frames every 30 (`xtask dump --x3 X3 --load states/cap-text.state
--frames 1300 --out DIR --at 1-1300/1 --wram --hold 0100 --p1tap
8000@10-1100/30:12 --poke 7E1F32=0E@1-1100` plus the flags), `MMX_TIMING=ntsc`:

| run | flags | text (`$1F3C = 1`) | in the capsule | `$1F99 = 08` | free |
|---|---|---|---|---|---|
| X | none | 5-410 | 563 | 882 | 1085 |
| X+X | `--players 2 --p2-character x` | 5-410 (same frames) | 563 | 882 | 1085 |
| X+X+Zero | `--players 3 --p2-character x --p3-character zero` | 5-564 | | | |
| Zero | `--zero` | 5-331 | 483 | 802 | 1005 |
| Zero+Zero | `--coop --zero` | 5-331 | 483 | 802 | 1005 |
| X+Zero | `--coop` | 5-564 | 716 | 1035 | 1238 |

The Hadouken message with X+Zero: `--poke 7E1F32=19@1-500 --coop` shows
"I give you a special present. Now, enter the capsule, please!" then
"Zero, charge your Z-Saber, then swing it to hurl a wave!". PNG strips
(`target/zu/`, not committed): `legs_rom_text.png` (the TAS, ROM text),
`arm_x.png`, `arm_zero.png`, `arm_x_zero.png`, `hadouken_x_zero.png`.
Tests `capsule_audience`, `capsule_build`, `capsule_dialog_in_game` (reads
the first page back from the BG3 tilemap for each audience).

unverified: on the real arm and Armadillo capsules (no state reaches
Flame Mammoth or the Hadouken capsule; the message is forced at the legs
capsule). The ids `$0E`/`$19` and the table are the same on USA v1.0
(`$84:C886`) and v1.1 (`$84:C890`).

## Co-op trigger (v1)

Only P1 triggers a capsule, as planned, and nothing was changed: the
capsule tests X through `$84:9B03`, which co-op already runs a second time
for P2 (hook `contact`), but that pass's result is thrown away and the
capsule's contact damage is 0, so P2 inside the boxes does nothing (static
reading; P2 stood at (2524,655), in the capsule, through the X+Zero run
above with no effect). In that run (`--p2-hold 0100 --to 1238 --p2tap
0080@1300:20,0040@1360:2 --poke 7E1F99=0A@1350-1420`): P2's input is `0000`
from the trigger to frame 1238 while its pad holds Right (locked), it
dashes at 1300 with the legs the capsule gave (`$1F99` is shared), and
with the arm bit poked swings at 1360 (frames 87, 88, 89). P2 cannot skip
the text (the wait reads X's input).

## A second X (`--p2-character x`)

The roster already makes P2 X (`--p2-character x`, the launcher's
character slots; netgame copies each player's character into the session
game). Under it a P2 that is X took the no-X3 path with `palette_for`, i.e.
X's tiles in Zero's colours, and P2's HUD icon was the Z glyph. Now a
shadow player that is X (`zero.rs` `shadow_is_x`): `zero::p2_x_palette`
(X's blue ramp 7-9 turned green, the rest X's; X's own colours from
`zero::X_ROM` while P1 wears Zero's slot 1), its own charge flash (its loads
into slot 1 skipped as for an X3 Zero), and X's emblem in the HUD palette;
no saber and no wave (the roster's `current_shadow_is_zero` gate); the
widescreen margins draw X3 Zero art only for a Zero shadow. The capsule
dialog counts it as X. Test `p2_x_tints_the_blue_ramp`. Strip
`target/zu/p2_x.png` (`highway-start --players 2 --p2-character x`, with
and without `--x3`, and with `--zero`). ponytail: one fixed tint (P2's
palette; players 3/4 keep the roster's own palettes); a picker is the
upgrade. Its third charge level flashes X3's purple (`CHARGE_FLASH_3`), not
X1's own (unverified which X1 shows). Online co-op still requires P2 to be
Zero (`mmx-native/src/cli.rs`), as on `main`.

## The saber is the arm part

`saber::update` takes `$1F99` and starts a swing only with `$02` (the one
place the gate lives), for P1 (`--zero`) and every shadow player the roster
makes a Zero with the X3 set (`current_shadow_is_zero`, so players 3 and 4
in their slices too). Test
`saber_needs_arm_part`; the ROM tests give the part
(`saber::tests::give_arm`). Scripted runs poke `7E1F99=02`.

## The wave

Reverted from `edd27cf`/`92f667d`/`1cdb347` and gated (`saber.rs`,
`zero.rs`):

- **Trigger**: a swing whose first frame finds a full charge (`+58 = 4`,
  `+5B` 2 or 1) while `$1F7E` bit 7 is set. The charge is consumed the
  ROM's way at once (`$81:97EE`: stop sound `$17` if `+87` bit 6, `+58/+5A/
  +5B = 0`), so the swing's masked pad does not fire the charged shot; the
  wave is thrown on the arc pose's first frame and flies 31 frames
  (`x3_wave_step`). Without the flag, or without a full charge, the swing
  is the plain slash and the masked release fires the charged shot as
  before. Full HP is not required (ponytail: the Hadouken's rule; add it if
  the wave reads as too cheap).
- **Damage**: its own shot, id 4 (the Hadouken's), box in its slot
  (`BOX_AT`, as the blade's), put in fresh every frame so it pierces like
  the Hadouken's shot. Measured: X's Hadouken takes the `hw-spiky` Spiky 2
  -> 1 -> dead on two frames running; the wave does the same from 101 px.
- **Drawing**: `zero::wave_entries`, shadow page 3 (P1 columns 0-5, P2 8-13,
  around the HUD icon's cells), `WAVE_ON_ZERO` colours, refilled each frame.
- **State**: one `Wave` per player, `Game::waves` (P1, P2, 3, 4), saved as
  the state's last tail (the bincode waves, their length, `WAVE`), so
  `CoopState` and the `XTRA` tail keep their encoding and every older state
  loads (with no wave). Rollback carries it (`net_save` is the save state).
  ponytail: players 3 and 4 throw no wave yet (shadow page 3 holds two, and
  their swings are not inserted as shots by `shots_done` either).
- **No Hadouken for a Zero**: an `Observe` trap on the motion check
  `$81:A0EA` (hook `hadouken_check`) returns "no" (A = 1, Z clear) for a Zero
  P1's block or P2's slice when the X3 set is loaded.

Tests `saber_wave_spawns_on_arc_and_flies`, `saber_wave_hits_spiky_from_afar`,
`saber_p1_throws_the_wave`, `saber_no_wave_without_hadouken`,
`saber_wave_in_save_state`, `zero_has_no_hadouken`. `NET_REVISION` 8. Strips
`target/zu/wave_p1.png` (`highway-start --zero --poke
7E1F99=02@1-175,7E1F7E=80@1-175 --p1tap 4000@10:140,0040@130:2`),
`wave_p2.png` (`--coop --p2tap 0100@1:12,4000@20:140,0040@140:2`).

## Password check

Game over with the parts and the Hadouken (`highway-start`, poke
`1F99=0F 1F7E=80 1F9B=10` (intro cleared: before it, a password with items
is invalid, `$80:F1A7`), lives 0, HP 1 into the bee): password digits
`$FFCB` = `04 05 00 07 00 00 04 06 03 02 07 06` ("5618 1157 4387"). Typed
in at the title (`MMX_TIMING=ntsc`, `--tap 200,400,600,800 --p1tap
0400@960:2,1000@980:2,4000@1200:2`, digits poked into `$1E60..$1E6B`,
cursor `$1E4F = 3`; the confirm button is Y, `$80:EF40`):

- `$1F99 = $0F`, `$1F9B = $10`: **the parts come back**;
- `$1F7E = 0`: **the Hadouken does not**, unless Down+X+L+R are held while
  confirming (`$80:F28C`: `$A7 & $0470 = $0470`, else `STZ $1F7E`); with
  them held (`--p1tap 0470@1185:20`) `$1F7E = $80`. The bit is in the
  password (`$80:F082`), the ROM only restores it on that button combo.

So the arm part (the saber) survives a password; the Hadouken (the wave)
needs the ROM's own combo. Not extended (the plan: document and stop).

## Not doing

- A separate per-player parts byte: nothing needs it, and it would break the
  password and the ROM's readers.
- A new save format: `$1F99`/`$1F7E` are WRAM, carried by states, rollback
  and passwords.
- A P2 capsule trigger (v2 if playtest asks: rerun the capsule's contact
  test with P2's block, `coop_objects` style).
- Suppressing X's level-3 charge for a Zero with the saber (one `Observe`
  trap on `$81:97DA` if it plays too strong).
