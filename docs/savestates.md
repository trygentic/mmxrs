# Save states

`Game::save_state` (native F5, web F5, `xtask dump --save`, rollback netplay)
writes, in order:

1. bincode `(SnesEmulator, frame_count, fired_at)`: the whole console.
2. `P2T7` + bincode `P2Tiles`: the PPU's six shadow tile pages and three
   independent local co-op sprite palettes.
3. `COOP` + `u32` LE length + bincode `CoopState`: P2's shadow block
   (`$0BA8..`), scratch, hidden/down/revive, saber, shots/effects/pickup
   bookkeeping, P1-as-Zero swing.
4. Optional `XTRA` + bincode `[CoopState; 2]` for players 3 and 4.
5. Optional `VS01` + bincode `VsState` for local versus mode.
6. `SHP1` + bincode `(bool, Option<u8>)`: whether the current shot was
   initializing and which extra player owns its pending callback. A frame
   can end between the ROM shot update and its completion hook; these fields
   keep shoulder launches and owner-block restoration deterministic after load.

Loading keeps the frontend's settings in `CoopState` (co-op on/off, P1 as
Zero, P2's palette) and takes everything else from the state. Not saved: the
X3 Zero art (`--x3`, loaded per run), `Game.oam_slots` and the display
offsets (display only, rebuilt by the next frame's OAM build).

Compatibility: every tail is optional. A state without `COOP` (saved before
it) loads with a default `CoopState`: P2 is hidden and re-anchors beside P1
when it next shows. Older tile markers still load: `P2T6` (four pages and one
extra palette), `P2T5` (no P2 palette: none until the next
OAM build fills it), `P2T4`..`P2T2`, none.
States without `SHP1` reset pending shot callbacks to `(false, None)`.
A truncated `COOP` or `SHP1` tail is an error. Rollback states (`net_save`) are the
same bytes; `NET_REVISION` 3 marks the change (6 the P2 palette).

## Sending a slot with a bug report

Native: press F5 when the bug shows (the title bar's `[SLOT n]` is the slot;
1-9 picks another). The file is `~/.mmxrs/slotN.state`; attach it with the
ROM revision and the flags you ran with (`--coop-local`, `--x3-rom`, `--zero`).
Web keeps its slot in memory only. Inspect a slot without a ROM:

    cargo xtask stateinfo ~/.mmxrs/slot1.state

(frame count, tile pages, whether it has co-op data, P2's position, HP and
hidden flag). Replay it with `cargo xtask dump --load PATH --coop ...`.
