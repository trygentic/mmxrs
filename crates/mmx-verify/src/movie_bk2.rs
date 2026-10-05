//! Movie parsers: BizHawk `.bk2` and lsnes `.lsmv` (both zips) and Snes9x
//! `.smv` (binary). Bytes in, `Vec<Pads>` out (mmx-core auto-read layout),
//! one entry per emulated video frame, lag frames included.
//!
//! `.bk2` `Input Log.txt`:
//! ```text
//! [Input]
//! LogKey:#Reset|Power|#P1 Up|P1 Down|P1 Left|P1 Right|P1 Select|P1 Start|P1 Y|P1 B|P1 X|P1 A|P1 L|P1 R|#P2 Up|...
//! |..|UDLRsSYBXAlr|UDLRsSYBXAlr|
//! [/Input]
//! ```
//! Columns follow `LogKey` (each `#` opens a `|cell|`); a cell char other
//! than `.`/space means pressed. `.lsmv` `input`: `F.|BYsSudlrAXLR|...`,
//! whose 12 pad chars are already in auto-read bit order 15..4.
//!
//! `.smv` (Snes9x 1.43 "version 1"; snes9x-rr adds ROM info): 32-byte header
//! (`SMV\x1A`, version 1, uid, rerecords, frames, controller mask, options
//! bit 0 = from power-on / bit 1 = PAL, sync flags, savestate offset, input
//! offset), then per frame one little-endian `u16` per controller in the
//! mask, already in auto-read layout (`B` = bit 15). The block holds the
//! header's frame count plus one (the power-on frame). A frame whose words
//! are all `$FFFF` is a reset.

use std::io::{Cursor, Read};

use mmx_core::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Movie {
    /// `Platform` from `Header.txt` (bk2) or `gametype` (lsmv).
    pub platform: String,
    pub rerecords: u64,
    /// ROM SHA1 from the header when present (bk2 `SHA1`).
    pub sha1: String,
    /// bk2 `SyncSettings.json`, raw.
    pub sync_settings: String,
    pub pads: Vec<Pads>,
    /// Frames (0-based) at which the movie presses Reset (bk2 `Reset`
    /// column, smv all-`$FFFF` frames). Callers must honour or refuse them.
    pub resets: Vec<usize>,
    /// smv: the ROM's CRC32 and internal name when the movie records them.
    pub rom_crc32: Option<u32>,
    pub rom_name: String,
    /// smv: recorded on a PAL console / starts from a save state instead of
    /// power-on (a snapshot movie cannot be played from power-on).
    pub pal: bool,
    pub from_snapshot: bool,
}

fn button_bit(name: &str) -> u16 {
    match name {
        "B" => BTN_B,
        "Y" => BTN_Y,
        "Select" => BTN_SELECT,
        "Start" => BTN_START,
        "Up" => BTN_UP,
        "Down" => BTN_DOWN,
        "Left" => BTN_LEFT,
        "Right" => BTN_RIGHT,
        "A" => BTN_A,
        "X" => BTN_X,
        "L" => BTN_L,
        "R" => BTN_R,
        _ => 0,
    }
}

const DEFAULT_LOGKEY: &str = "#Reset|Power|#P1 Up|P1 Down|P1 Left|P1 Right|P1 Select|P1 Start|P1 Y|P1 B|P1 X|P1 A|P1 L|P1 R|#P2 Up|P2 Down|P2 Left|P2 Right|P2 Select|P2 Start|P2 Y|P2 B|P2 X|P2 A|P2 L|P2 R";

/// Per cell: per column, `(player 1|2|0 = not a pad, bit)`.
fn parse_logkey(key: &str) -> Vec<Vec<(u8, u16)>> {
    key.split('#')
        .filter(|g| !g.is_empty())
        .map(|g| {
            g.split('|')
                .filter(|b| !b.is_empty())
                .map(|b| match b.split_once(' ') {
                    Some(("P1", n)) => (1, button_bit(n)),
                    Some(("P2", n)) => (2, button_bit(n)),
                    _ if b == "Reset" => (3, 0),
                    _ => (0, 0),
                })
                .collect()
        })
        .collect()
}

fn read_member(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut s = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
    Some(s)
}

fn open(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, String> {
    zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("movie: not a zip: {e}"))
}

/// Parse `Input Log.txt` text (exposed for tests and for loose logs).
pub fn parse_bk2_input_log(text: &str) -> Result<Vec<Pads>, String> {
    parse_bk2_log(text).map(|(p, _)| p)
}

/// `Input Log.txt` -> (pads, frames with Reset pressed).
fn parse_bk2_log(text: &str) -> Result<(Vec<Pads>, Vec<usize>), String> {
    let mut cells = parse_logkey(DEFAULT_LOGKEY);
    let mut pads = Vec::new();
    let mut resets = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim_end();
        if let Some(key) = line.strip_prefix("LogKey:") {
            if !key.is_empty() {
                cells = parse_logkey(key);
            }
            continue;
        }
        let Some(body) = line.strip_prefix('|') else { continue }; // [Input], blanks
        let fields: Vec<&str> = body.trim_end_matches('|').split('|').collect();
        // ponytail: a leading frame counter cell is recognised by count only.
        let fields = if fields.len() == cells.len() + 1 { &fields[1..] } else { &fields[..] };
        if fields.len() != cells.len() {
            return Err(format!("bk2: line {}: {} cells, LogKey has {}", n + 1, fields.len(), cells.len()));
        }
        let mut p = Pads::default();
        for (cell, cols) in fields.iter().zip(&cells) {
            for (ch, &(player, bit)) in cell.chars().zip(cols) {
                if ch != '.' && ch != ' ' {
                    match player {
                        1 => p.p1 |= bit,
                        2 => p.p2 |= bit,
                        3 => resets.push(pads.len()),
                        _ => {} // ponytail: Power ignored
                    }
                }
            }
        }
        pads.push(p);
    }
    resets.dedup();
    Ok((pads, resets))
}

/// Parse a BizHawk `.bk2` archive.
pub fn parse_bk2(bytes: &[u8]) -> Result<Movie, String> {
    let mut zip = open(bytes)?;
    let log = read_member(&mut zip, "Input Log.txt").ok_or("bk2: no `Input Log.txt`")?;
    let (pads, resets) = parse_bk2_log(&log)?;
    let mut m = Movie { pads, resets, ..Movie::default() };
    for line in read_member(&mut zip, "Header.txt").unwrap_or_default().lines() {
        let (k, v) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        match k {
            "Platform" => m.platform = v.to_string(),
            "rerecordCount" => m.rerecords = v.trim().parse().unwrap_or(0),
            "SHA1" => m.sha1 = v.to_string(),
            _ => {}
        }
    }
    m.sync_settings = read_member(&mut zip, "SyncSettings.json").unwrap_or_default();
    Ok(m)
}

/// Parse lsnes `input` text. Only `F` lines (frame starts) count;
/// subframe lines are ignored (ponytail: lsnes subframe polls unsupported).
pub fn parse_lsmv_input(text: &str) -> Vec<Pads> {
    text.lines()
        .filter(|l| l.starts_with('F'))
        .map(|l| {
            let mut ports = l.split('|').skip(1).map(|f| {
                f.chars().take(12).enumerate().fold(0u16, |acc, (i, c)| {
                    if c != '.' && c != ' ' { acc | 1 << (15 - i) } else { acc }
                })
            });
            Pads { p1: ports.next().unwrap_or(0), p2: ports.next().unwrap_or(0) }
        })
        .collect()
}

/// Parse an lsnes `.lsmv` archive.
pub fn parse_lsmv(bytes: &[u8]) -> Result<Movie, String> {
    let mut zip = open(bytes)?;
    let input = read_member(&mut zip, "input").ok_or("lsmv: no `input` member")?;
    Ok(Movie {
        platform: read_member(&mut zip, "gametype").unwrap_or_default().trim().to_string(),
        rerecords: read_member(&mut zip, "rerecords").and_then(|s| s.trim().parse().ok()).unwrap_or(0),
        pads: parse_lsmv_input(&input),
        ..Movie::default()
    })
}

/// Parse a Snes9x `.smv` (version 1, as written by Snes9x 1.43 and
/// snes9x-rr 1.43 v15+; later versions use other layouts and are refused).
pub fn parse_smv(bytes: &[u8]) -> Result<Movie, String> {
    let u32_at = |o: usize| bytes.get(o..o + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.len() < 0x20 || &bytes[..4] != b"SMV\x1a" {
        return Err("smv: no `SMV\\x1A` signature".into());
    }
    let version = u32_at(4);
    if version != 1 {
        return Err(format!("smv: version {version} (only Snes9x 1.43's version 1 is supported)"));
    }
    let (mask, options, sync) = (bytes[0x14], bytes[0x15], bytes[0x17]);
    let input = u32_at(0x1C) as usize;
    let ports: Vec<u8> = (0..5).filter(|k| mask >> k & 1 != 0).collect();
    if ports.is_empty() || input < 0x20 || input > bytes.len() {
        return Err(format!("smv: controller mask {mask:#04x}, input offset {input:#x}"));
    }
    let mut m = Movie {
        platform: "SNES (Snes9x 1.43 smv)".into(),
        rerecords: u64::from(u32_at(0x0C)),
        pal: options & 2 != 0,
        from_snapshot: options & 1 == 0,
        ..Movie::default()
    };
    // snes9x-rr: sync flag $40 = 30 bytes of ROM info (3 zero bytes, CRC32,
    // 23-byte name) right before the input block.
    if sync & 0x40 != 0 && input >= 0x20 + 30 {
        let r = &bytes[input - 30..input];
        m.rom_crc32 = Some(u32::from_le_bytes([r[3], r[4], r[5], r[6]]));
        m.rom_name = String::from_utf8_lossy(&r[7..]).trim_end_matches(['\0', ' ']).to_string();
    }
    for (f, chunk) in bytes[input..].chunks_exact(2 * ports.len()).enumerate() {
        let words: Vec<u16> = chunk.chunks_exact(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect();
        if words.iter().all(|&w| w == 0xFFFF) {
            m.resets.push(f);
            m.pads.push(Pads::default());
            continue;
        }
        let pad = |port: u8| ports.iter().position(|&p| p == port).map_or(0, |i| words[i] & 0xFFF0);
        m.pads.push(Pads { p1: pad(0), p2: pad(1) });
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in files {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    const LOG: &str = "[Input]\r\n\
LogKey:#Reset|Power|#P1 Up|P1 Down|P1 Left|P1 Right|P1 Select|P1 Start|P1 Y|P1 B|P1 X|P1 A|P1 L|P1 R|#P2 Up|P2 Down|P2 Left|P2 Right|P2 Select|P2 Start|P2 Y|P2 B|P2 X|P2 A|P2 L|P2 R|\r\n\
|..|............|............|\r\n\
|..|.....S......|............|\r\n\
|..|...R...B...r|U..........r|\r\n\
|..|UDLRsSYBXAlr|............|\r\n\
[/Input]\r\n";

    #[test]
    fn bk2_roundtrip() {
        let zip = make_zip(&[
            ("Header.txt", "MovieVersion BizHawk v2.0\nPlatform SNES\nrerecordCount 42\nSHA1 ABCDEF\n"),
            ("Input Log.txt", LOG),
            ("SyncSettings.json", "{\"x\":1}"),
        ]);
        let m = parse_bk2(&zip).unwrap();
        assert_eq!((m.platform.as_str(), m.rerecords, m.sha1.as_str()), ("SNES", 42, "ABCDEF"));
        assert_eq!(m.sync_settings, "{\"x\":1}");
        assert_eq!(
            m.pads,
            vec![
                Pads::default(),
                Pads { p1: BTN_START, p2: 0 },
                Pads { p1: BTN_RIGHT | BTN_B | BTN_R, p2: BTN_UP | BTN_R },
                Pads { p1: 0xFFF0, p2: 0 },
            ]
        );
    }

    #[test]
    fn bk2_logkey_reorders_columns() {
        // a hypothetical core logging B first and A second
        let log = "LogKey:#Power|#P1 B|P1 A|\n|.|B.|\n|.|.A|\n";
        let p = parse_bk2_input_log(log).unwrap();
        assert_eq!(p.iter().map(|p| p.p1).collect::<Vec<_>>(), vec![BTN_B, BTN_A]);
        assert!(parse_bk2_input_log("LogKey:#Power|#P1 B|\n|.|B|extra|junk|\n").is_err());
    }

    #[test]
    fn bk2_missing_log_is_an_error() {
        assert!(parse_bk2(&make_zip(&[("Header.txt", "")])).is_err());
        assert!(parse_bk2(b"not a zip").is_err());
    }

    fn smv(mask: u8, options: u8, words: &[u16]) -> Vec<u8> {
        let mut v = b"SMV\x1a".to_vec();
        v.extend(1u32.to_le_bytes());
        v.extend(0x1234u32.to_le_bytes()); // uid
        v.extend(99u32.to_le_bytes()); // rerecords
        v.extend((words.len() as u32).to_le_bytes());
        v.extend([mask, options, 0, 0x41]); // sync: data exists + ROM info
        v.extend(0u32.to_le_bytes()); // savestate offset
        v.extend(((0x20 + 30) as u32).to_le_bytes());
        v.extend([0, 0, 0]);
        v.extend(0xDED5_3C64u32.to_le_bytes());
        let mut name = b"MEGAMAN X".to_vec();
        name.resize(23, 0);
        v.extend(name);
        for w in words {
            v.extend(w.to_le_bytes());
        }
        v
    }

    #[test]
    fn smv_input() {
        // one controller: Start, B+Right, a reset frame, nothing
        let m = parse_smv(&smv(1, 1, &[0x1000, 0x8100, 0xFFFF, 0])).unwrap();
        assert_eq!(m.pads.iter().map(|p| p.p1).collect::<Vec<_>>(), vec![BTN_START, BTN_B | BTN_RIGHT, 0, 0]);
        assert_eq!((m.resets.clone(), m.rerecords, m.rom_crc32, m.rom_name.as_str()), (vec![2], 99, Some(0xDED5_3C64), "MEGAMAN X"));
        assert!(!m.pal && !m.from_snapshot);
        // two controllers interleaved per frame; the low nibble (not buttons) dropped
        let m = parse_smv(&smv(3, 3, &[BTN_A, BTN_Y | 0x000F, 0, BTN_L])).unwrap();
        assert_eq!(m.pads, vec![Pads { p1: BTN_A, p2: BTN_Y }, Pads { p1: 0, p2: BTN_L }]);
        assert!(m.pal && !m.from_snapshot);
        assert!(parse_smv(b"SMV\x1a\x04\0\0\0").is_err());
        assert!(parse_smv(b"not a movie at all, not at all, no").is_err());
    }

    #[test]
    fn bk2_reset_column() {
        let log = "LogKey:#Reset|Power|#P1 B|\n|..|.|\n|r.|B|\n|..|.|\n";
        assert_eq!(parse_bk2_log(log).unwrap(), (vec![Pads::default(), Pads { p1: BTN_B, p2: 0 }, Pads::default()], vec![1]));
    }

    #[test]
    fn lsmv_input() {
        let zip = make_zip(&[
            ("gametype", "snes_ntsc\n"),
            ("rerecords", "7"),
            ("input", "F.|............|............\nF.|B..S........|...........R\n.|B...........|\nF.|............\n"),
        ]);
        let m = parse_lsmv(&zip).unwrap();
        assert_eq!((m.platform.as_str(), m.rerecords), ("snes_ntsc", 7));
        assert_eq!(m.pads, vec![Pads::default(), Pads { p1: BTN_B | BTN_START, p2: BTN_R }, Pads::default()]);
    }
}
