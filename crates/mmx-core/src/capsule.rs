//! Zero's lines in Dr. Light's capsule dialog (docs/zero-upgrades.md
//! "Capsule scene (measured)", "Zero's dialog").
//!
//! The text task (`$80:E686`) loads the message pointer `$68 =
//! $84:C890[$1F32 & $3F]` ([`HookId::TextPtr`]; 16-bit, bank `$84`) and
//! reads every byte through `$80:E953` (`PEA $8684; PLB; LDA ($68),Y; INY;
//! PLB; RTS`, [`HookId::TextRead`]). The newline code adds `Y` into `$68`,
//! so `$68 + Y - start` is always the offset into the message. One
//! `Observe` trap on the reader serves our message's byte at that offset
//! and returns for the ROM; any other message, or a game where nobody is a
//! Zero with the saber, reads the ROM. Pure function of WRAM (`$1F32`, `$68`,
//! `Y`) and the settings (X3 set, co-op, each active player's character),
//! the same on both netplay peers, so rollback replays it.
//!
//! Encoding (the printer `$80:E6BE`, handlers `$80:E78D[code & $0F]`):
//! `$20..$7E` ASCII; `80` newline (lines are double spaced: `80 80`), `81`
//! wait for P1's button with a blinking cursor, `82` end, `83 n` skip n
//! columns, `86 n` scroll n rows (a page turn: `2 * lines + 1`), `87 n`
//! pause n frames, `88 n` speed, `89 lo hi` position, `8A` Light's portrait
//! step, `C0..CF` text colour; `84`/`85 n` two-row glyphs.

use crate::hooks::HookId;
use crate::ram::wram_index as ix;
use crate::traps::TrapReturn;
use crate::zero_data::{Pages, ZERO_ARM, ZERO_ARM_TOO, ZERO_WAVE, ZERO_WAVE_TOO};
use crate::Game;

/// Message id of the running dialog (`$80:E9AD` stores it; `& $3F` indexes the table).
pub const TEXT_ID: u32 = 0x7E1F32;
/// Flame Mammoth's capsule: "This capsule contains a part ... X-Buster."
pub const MSG_ARM: u8 = 0x0E;
/// Armored Armadillo's hidden capsule: "I give you a special present."
pub const MSG_HADOUKEN: u8 = 0x19;
const END: u8 = 0x82;
const WAIT: u8 = 0x81;
const LINE: u8 = 0x80;

/// Who hears the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Only X (or two X): the ROM's text.
    X,
    /// Only Zero (or two Zeros): Zero's text.
    Zero,
    /// X and Zero: X's pages, then Zero's.
    Both,
}

/// From each active player's "is a Zero with the saber" (the X3 set; a Zero
/// with X1 art, and every other character, plays X's moveset and hears
/// X's text). Pure.
pub fn audience(zero: impl IntoIterator<Item = bool>) -> Audience {
    let (mut x, mut z) = (false, false);
    for is_zero in zero {
        *if is_zero { &mut z } else { &mut x } = true;
    }
    match (x, z) {
        (_, false) => Audience::X,
        (false, true) => Audience::Zero,
        (true, true) => Audience::Both,
    }
}

/// The players in the game now: 1, or with co-op the local count (at least 2).
fn players(game: &Game) -> usize {
    if game.coop.enabled { game.local_players.max(2) } else { 1 }
}

/// Player `i` is a Zero: the roster says so (his saber is the arm part
/// whatever art he wears, `saber::armed`).
fn has_saber(game: &Game, i: usize) -> bool {
    game.player_character(i) == Some(crate::Character::Zero)
}

/// Bytes after a control code (`89` takes two).
fn args(code: u8) -> usize {
    match code {
        0x83..=0x88 => 1,
        0x89 => 2,
        _ => 0,
    }
}

/// `pages` encoded: lines joined by `80 80`, a page turn (wait, scroll the
/// page's rows, pause) between pages. Ends after the last line's `80 80`.
fn body(pages: Pages) -> Vec<u8> {
    let mut v = Vec::new();
    for (k, page) in pages.iter().enumerate() {
        if k > 0 {
            v.extend([0x83, 0x09, WAIT, LINE, 0x86, rows(pages[k - 1]), 0x87, 0x1E]);
        }
        for line in page.iter() {
            v.extend(line.bytes());
            v.extend([LINE, LINE]);
        }
    }
    v
}

/// Rows a page of `lines` takes, with the line after the wait.
fn rows(page: &[&str]) -> u8 {
    2 * page.len() as u8 + 1
}

/// The message `rom` (up to its `82`) for `who`, with Zero's `pages` (alone)
/// or `too` (after X's). `None`: the ROM's own. Keeps the ROM's header
/// (portrait, position, colour, speed) and whatever it puts between the
/// last wait and the last scroll (`8A` in the Hadouken's). Pure.
pub fn build(rom: &[u8], who: Audience, pages: Pages, too: Pages) -> Option<Vec<u8>> {
    // Walk the codes: the first text byte, the last wait, the end.
    let (mut i, mut first, mut last_wait) = (0, None, None);
    loop {
        let b = *rom.get(i)?;
        match b {
            END => break,
            0x20..=0x7E => first = first.or(Some(i)),
            WAIT => last_wait = Some(i),
            _ => {}
        }
        i += 1 + if b >= 0x80 { args(b) } else { 0 };
    }
    let (first, w) = (first?, last_wait?);
    // `81 80 [extras] 86 n 82`
    let tail = &rom[w..=i];
    let scroll = tail.iter().position(|&b| b == 0x86)?;
    let extras = &tail[2..scroll];
    let (mut v, last) = match who {
        Audience::X => return None,
        Audience::Zero => (rom[..first].to_vec(), *pages.last()?),
        Audience::Both => {
            let mut v = rom[..w].to_vec();
            v.extend([WAIT, LINE, 0x86, tail[scroll + 1], 0x87, 0x1E]);
            (v, *too.last()?)
        }
    };
    v.extend(body(if who == Audience::Zero { pages } else { too }));
    v.extend([0x83, 0x09, WAIT, LINE]);
    v.extend(extras);
    v.extend([0x86, rows(last), END]);
    Some(v)
}

/// The message table (`$84:C890` on our dump): the long operand of the
/// text task's `LDA $84xxxx,X` ([`HookId::TextPtr`] + 9).
fn table(game: &Game) -> u32 {
    let a = game.hook(HookId::TextPtr) + 10;
    u32::from_le_bytes([game.rom_read(a), game.rom_read(a + 1), game.rom_read(a + 2), 0])
}

/// Start of message `id` (bank `$84`).
fn start(game: &Game, id: u8) -> u16 {
    let t = table(game) + 2 * u32::from(id);
    u16::from_le_bytes([game.rom_read(t), game.rom_read(t + 1)])
}

/// Message `id` as the game shows it now, `None` when it is the ROM's.
pub fn message(game: &Game, id: u8) -> Option<Vec<u8>> {
    let (pages, too) = match id {
        MSG_ARM => (ZERO_ARM, ZERO_ARM_TOO),
        MSG_HADOUKEN => (ZERO_WAVE, ZERO_WAVE_TOO),
        _ => return None,
    };
    let who = audience((0..players(game)).map(|i| has_saber(game, i)));
    if who == Audience::X {
        return None;
    }
    let s = 0x84_0000 | u32::from(start(game, id));
    // ponytail: messages are < 1 KiB; read until the end code.
    let rom: Vec<u8> = (0..1024).map(|k| game.rom_read(s + k)).collect();
    build(&rom, who, pages, too)
}

pub fn register(game: &mut Game) {
    game.traps.register(game.hook(HookId::TextRead), "capsule_text", TrapReturn::Observe, text_read);
}

/// [`HookId::TextRead`]: serve our message's byte and return as the ROM
/// would (A low = byte, `Y + 1`, `DB` = the `PEA`'s high byte, N set and Z
/// clear by the last `PLB`).
fn text_read(game: &mut Game) {
    let id = game.wram()[ix(TEXT_ID)] & 0x3F;
    let r = game.regs();
    // The text task runs with 8-bit A and index (`$80:E686 SEP #$30`).
    if r.p & 0x30 != 0x30 {
        return;
    }
    let Some(text) = message(game, id) else { return };
    let w = game.wram();
    let dp = usize::from(r.d.wrapping_add(0x68));
    let ptr = u16::from_le_bytes([w[dp], w[dp + 1]]);
    let off = usize::from(ptr.wrapping_add(r.y & 0xFF).wrapping_sub(start(game, id)));
    let mut n = r;
    n.a = r.a & 0xFF00 | u16::from(text.get(off).copied().unwrap_or(END));
    n.y = r.y.wrapping_add(1) & 0xFF;
    n.dbr = game.rom_read(game.hook(HookId::TextRead) + 2);
    n.p = (r.p | 0x80) & !0x02;
    game.set_regs(n);
    game.emulate_return(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_audience() {
        use Audience::*;
        // Each active player: a Zero with the saber?
        assert_eq!(audience([false]), X, "X alone (or Zero without X3)");
        assert_eq!(audience([false, false]), X, "X + X");
        assert_eq!(audience([true]), Zero, "Zero alone");
        assert_eq!(audience([true, true, true, true]), Zero, "four Zeros");
        assert_eq!(audience([false, true]), Both, "X + Zero");
        assert_eq!(audience([true, false, true]), Both, "Zero + X + Zero");
    }

    /// `states/cap-text.state` (the Leg Parts capsule, 4 frames before the
    /// dialog starts; made on the PAL dump at NTSC timing from 3197M, see
    /// docs/zero-upgrades.md) with the message id held at the arm capsule's:
    /// the first page as typed into the BG3 tilemap (VRAM words `$0800..`,
    /// low byte = the character; spaces dropped) after 160 frames.
    fn first_page(p1_zero: bool, coop: bool, p2: crate::Character) -> Option<String> {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return None;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/cap-text.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return None;
        };
        let mut g = Game::from_rom_timing(std::fs::read(x1).unwrap(), crate::Timing::Ntsc).unwrap();
        g.set_x3_rom(&std::fs::read(x3).unwrap()).unwrap();
        if g.load_state(&state).is_err() {
            eprintln!("{st} is from another dump; skipping");
            return None;
        }
        g.coop.enabled = coop;
        crate::coop::register_coop_traps(&mut g).unwrap();
        g.set_p1_zero(p1_zero).unwrap();
        g.set_player_character(1, p2);
        for _ in 0..160 {
            g.wram_mut()[ix(TEXT_ID)] = MSG_ARM;
            g.step(crate::Pads::default());
        }
        let v = g.vram();
        Some((0x0800..0x0C00).map(|a| v[a] as u8).filter(|c| (0x21..0x7F).contains(c)).map(char::from).collect())
    }

    #[test]
    fn capsule_dialog_in_game() {
        use crate::Character::{Zero, X};
        let Some(x) = first_page(false, false, Zero) else { return };
        assert!(x.contains("Thiscapsule") && x.contains("containsapart"), "X alone: {x:?}");
        let xx = first_page(false, true, X).unwrap();
        assert!(xx.contains("Thiscapsule"), "X + X: {xx:?}");
        for (p1, coop) in [(true, false), (true, true)] {
            let z = first_page(p1, coop, Zero).unwrap();
            assert!(z.contains("YouarenotX...") && !z.contains("capsule"), "Zero, co-op {coop}: {z:?}");
        }
        let both = first_page(false, true, Zero).unwrap();
        assert!(both.contains("Thiscapsule"), "X + Zero starts with X's: {both:?}");
    }

    /// The ROM's arm message (PAL / USA, `$84:CFAC`): built around Zero's
    /// pages, the header and the end codes survive and every page turn
    /// scrolls its own rows.
    #[test]
    fn capsule_build() {
        let rom: Vec<u8> = [
            &[0x8A, 0x89, 0x86, 0x08, 0xC0, 0x88, 0x04][..],
            b"This capsule\x80\x80contains a part\x80\x80\x83\x09\x81\x80\x86\x05\x87\x1E",
            b"You can use it\x80\x80to fire all\x80\x80types of weapons.\x80\x80\x83\x09\x81\x80\x86\x07\x82",
            b"NOT THIS",
        ]
        .concat();
        assert_eq!(build(&rom, Audience::X, ZERO_ARM, ZERO_ARM_TOO), None);
        let z = build(&rom, Audience::Zero, ZERO_ARM, ZERO_ARM_TOO).unwrap();
        assert!(z.starts_with(&[0x8A, 0x89, 0x86, 0x08, 0xC0, 0x88, 0x04, b'Y']));
        assert!(z.ends_with(b"guide you, Zero!\x80\x80\x83\x09\x81\x80\x86\x05\x82"));
        assert!(z.windows(9).any(|w| w == b"\x80\x80\x83\x09\x81\x80\x86\x07\x87"), "3-line page turn");
        let b = build(&rom, Audience::Both, ZERO_ARM, ZERO_ARM_TOO).unwrap();
        let x_end = rom.iter().position(|&c| c == END).unwrap() - 2; // before `86 07 82`
        assert_eq!(b[..x_end], rom[..x_end], "X's pages first");
        assert_eq!(b[x_end..x_end + 5], [0x86, 0x07, 0x87, 0x1E, b'A']);
        assert!(b.ends_with(b"of your Z-Saber.\x80\x80\x83\x09\x81\x80\x86\x09\x82"));
        // The Hadouken's `8A` between the last wait and the scroll stays last.
        let had = b"\x8A\x89\x86\x08\xC1\x88\x04I give you a\x80\x80\x83\x09\x81\x80\x8A\x86\x09\x82";
        let z = build(had, Audience::Zero, ZERO_WAVE, ZERO_WAVE_TOO).unwrap();
        assert!(z.ends_with(b"\x81\x80\x8A\x86\x05\x82"));
        for p in [ZERO_ARM, ZERO_ARM_TOO, ZERO_WAVE, ZERO_WAVE_TOO] {
            for line in p.iter().flat_map(|p| p.iter()) {
                assert!(line.len() <= 16 && line.bytes().all(|c| (0x20..0x7F).contains(&c)), "{line:?}");
            }
            assert!(p.iter().all(|p| p.len() <= 5));
        }
    }
}
