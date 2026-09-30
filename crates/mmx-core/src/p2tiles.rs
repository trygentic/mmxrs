//! P2's sprite tiles in a shadow page outside VRAM (docs/zero.md, route A').
//!
//! P2 keeps X's tile window (`+$18`), so the emitter writes X's tile numbers
//! for it. Its per-frame tiles never reach VRAM: when P2's update runs the
//! frame streamer `$84:8FCA`, the entries it appended to the DMA queue `$0500`
//! are taken back out and their ROM source bytes copied into the vendored
//! PPU's shadow page. The display slice then flags P2's OAM slots, and the PPU
//! fetches those sprites' tiles from the page (`tile & 0x3F`). Page and mask
//! go live on the game's next OAM upload, together with the OAM they match.
//!
//! Art hook: [`fill_page`] is the one place the page is written. A Zero art
//! pack would write its own 4bpp tiles there instead of the ROM's X tiles.

use crate::ram::{self, wram_index as ix};
use crate::traps::TrapReturn;
use crate::Game;

/// Frame streamer entry (`JSL` from the player update, `D` = the block).
pub const STREAMER: u32 = 0x848FCA;
/// Streamer's common exit (`PLB; PLP; RTL`): `$A3` is final here. The only
/// other exit (`+$17` bit 7 clear) queues nothing.
pub const STREAMER_EXIT: u32 = 0x849083;
/// DMA queue: 8-byte entries `VMAIN, dest word, size, src24`; `$A3` = next
/// free byte index; `$1F25` = the streamer's vblank budget counter.
const DMA_QUEUE: usize = 0x0500;
const DMA_INDEX: usize = 0x00A3;
const DMA_BUDGET: usize = 0x1F25;
/// Block offset of the tile window base (in tiles).
const OFF_TILE_BASE: usize = 0x18;

/// X's window: 32 tiles (two rows), the part of the page the X1 streamer path fills.
pub const PAGE_TILES: u16 = 32;
pub const PAGE_WORDS: usize = 512;
/// The whole shadow page: 48 tiles (three rows, X3 Zero frames); the PPU
/// indexes it with `(tile & 0x3F) % 48`, so a window must be 64-tile aligned.
pub const SHADOW_TILES: u16 = snes_core::api::P2_PAGE_TILES;
pub const SHADOW_WORDS: usize = snes_core::api::P2_PAGE_WORDS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueEntry {
    /// VRAM word address.
    pub dest: u16,
    /// Bytes.
    pub size: u16,
    /// 24-bit source (ROM).
    pub src: u32,
}

/// One 8-byte `$0500` entry. Pure.
pub fn decode_entry(e: &[u8]) -> QueueEntry {
    QueueEntry {
        dest: u16::from_le_bytes([e[1], e[2]]),
        size: u16::from_le_bytes([e[3], e[4]]),
        src: u32::from(e[5]) | u32::from(e[6]) << 8 | u32::from(e[7]) << 16,
    }
}

/// Word offset of a transfer in the page, or `None` if it does not fit in
/// the 512-word window starting at VRAM word `window`. The page mirrors the
/// window 1:1 (two name-table rows of 16 tiles), so tile `n` of the window is
/// at `n * 16` and the PPU's `tile & 0x3F` finds it (window 64-tile aligned;
/// `+$18 = 0` on every stage seen). Pure.
pub fn page_offset(e: &QueueEntry, window: u16) -> Option<usize> {
    let o = usize::from(e.dest.wrapping_sub(window));
    (o + usize::from(e.size) / 2 <= PAGE_WORDS).then_some(o)
}

/// OAM tile number (9 bits) is one of the window's 32 tiles. Pure.
pub fn in_window(tile: u16, tile_base: u16) -> bool {
    tile.wrapping_sub(tile_base) & 0x1FF < PAGE_TILES
}

pub fn register(game: &mut Game) {
    game.traps.register(STREAMER, "p2tiles_stream", TrapReturn::Observe, stream_enter);
    game.traps.register(STREAMER_EXIT, "p2tiles_steal", TrapReturn::Observe, stream_exit);
}

/// P2's player update is the only streamer call made from an isolated call
/// with `D` = the player block.
fn stream_enter(game: &mut Game) {
    let p2 = game.coop.enabled && game.isolated_depth > 0 && game.regs().d == ram::X_BASE as u16;
    let w = game.wram();
    game.p2_stream = p2.then(|| (w[DMA_INDEX], u16::from_le_bytes([w[DMA_BUDGET], w[DMA_BUDGET + 1]])));
}

/// Take P2's entries back out of the queue and into the page.
fn stream_exit(game: &mut Game) {
    let Some((start, budget)) = game.p2_stream.take() else { return };
    let zero = game.zero_set.is_some(); // X3 Zero fills the page itself (zero::emit_x3)
    let w = game.wram();
    let end = w[DMA_INDEX];
    let entries: Vec<QueueEntry> = (usize::from(start)..usize::from(end))
        .step_by(8)
        .map(|i| decode_entry(&w[DMA_QUEUE + i..DMA_QUEUE + i + 8]))
        .collect();
    let window = game.obsel().0.wrapping_add(u16::from(w[ix(ram::X_BASE) + OFF_TILE_BASE]) * 16);
    let w = game.wram_mut();
    w[DMA_INDEX] = start;
    w[DMA_BUDGET..DMA_BUDGET + 2].copy_from_slice(&budget.to_le_bytes());
    for e in entries {
        if let Some(o) = page_offset(&e, window).filter(|_| !zero) {
            let bytes: Vec<u8> = (0..u32::from(e.size)).map(|i| game.rom_read(e.src + i)).collect();
            fill_page(game, o, &bytes);
        }
    }
}

/// Write 4bpp tile bytes at word offset `o` of the P2 page. The art hook.
pub fn fill_page(game: &mut Game, o: usize, bytes: &[u8]) {
    let page = game.p2_tile_page_mut();
    for (i, b) in bytes.chunks_exact(2).enumerate() {
        page[o + i] = u16::from_le_bytes([b[0], b[1]]);
    }
}

/// OAM build start (`$80:D544`): no P2 slots until the display slice says so.
pub fn clear_slots(game: &mut Game) {
    if game.coop.enabled {
        game.set_p2_tile_slots(0);
    }
}

/// Display slice, after the emitters: flag P2's slots that use the window's
/// tiles (armour parts with other tiles keep reading VRAM).
pub fn mark_p2_slots(game: &mut Game) {
    let base = u16::from(game.coop.p2[OFF_TILE_BASE]);
    let first = game.coop.p2_oam_base;
    let w = game.wram();
    let mut mask = 0u128;
    for slot in first..(first + game.coop.p2_oam_count).min(128) {
        let e = &w[ix(ram::OAM_BUF) + slot * 4..][..4];
        if in_window(u16::from(e[2]) | u16::from(e[3] & 1) << 8, base) {
            mask |= 1 << slot;
        }
    }
    game.set_p2_tile_slots(mask);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p2tiles_decode_entry() {
        // `$6100 <- $AD:C3A0`, 512 bytes, as the streamer writes it.
        let e = decode_entry(&[0x80, 0x00, 0x61, 0x00, 0x02, 0xA0, 0xC3, 0xAD]);
        assert_eq!(e, QueueEntry { dest: 0x6100, size: 0x200, src: 0xADC3A0 });
    }

    #[test]
    fn p2tiles_page_offset() {
        let e = |dest, size| QueueEntry { dest, size, src: 0 };
        assert_eq!(page_offset(&e(0x6000, 0x200), 0x6000), Some(0));
        assert_eq!(page_offset(&e(0x6100, 0x200), 0x6000), Some(0x100));
        assert_eq!(page_offset(&e(0x6180, 0x100), 0x6000), Some(0x180));
        assert_eq!(page_offset(&e(0x6100, 0x202), 0x6000), None); // spills past the page
        assert_eq!(page_offset(&e(0x5FF0, 0x20), 0x6000), None); // below the window
        assert_eq!(page_offset(&e(0x6400, 0x200), 0x6400), Some(0)); // window at tile $40
    }

    #[test]
    fn p2tiles_layout_matches_16x16() {
        // A 16x16 sprite at tile n reads n, n+1, n+16, n+17 (nibble wrap);
        // the page index (tile & 0x1F) * 16 must land on the word the window
        // holds at (tile - base) * 16.
        for base in [0u16, 0x40, 0x80] {
            for n in 0..0x1F_u16 {
                if n & 0xF == 0xF || n >= 0x10 {
                    continue; // top-left of a 16x16 in the window
                }
                for t in [n, n + 1, n + 16, n + 17] {
                    let tile = base + t;
                    assert!(in_window(tile, base));
                    assert_eq!(usize::from((tile & 0x3F) % SHADOW_TILES) * 16, usize::from(tile - base) * 16);
                }
            }
        }
        assert!(!in_window(0x20, 0));
        assert!(!in_window(0x3F, 0x40));
    }

    #[test]
    fn p2tiles_x3_rows_fit() {
        // X3 frames: tiles $00-$2F, a 16x16 at n (row 0/1) reads n+16/n+17 in
        // rows 1/2; all inside the 48-tile page, row 3 wraps to row 0.
        assert_eq!(SHADOW_WORDS, usize::from(SHADOW_TILES) * 16);
        for n in 0..0x20_u16 {
            if n & 0xF == 0xF {
                continue;
            }
            for t in [n, n + 1, n + 16, n + 17] {
                assert_eq!((t & 0x3F) % SHADOW_TILES, t);
            }
        }
        assert_eq!((0x30 & 0x3F) % SHADOW_TILES, 0);
    }
}
