//! Movie parsers: BizHawk `.bk2` and lsnes `.lsmv`, both zips. Bytes in,
//! `Vec<Pads>` out (mmx-core auto-read layout).
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
    let mut cells = parse_logkey(DEFAULT_LOGKEY);
    let mut pads = Vec::new();
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
                        _ => {} // ponytail: Reset/Power ignored
                    }
                }
            }
        }
        pads.push(p);
    }
    Ok(pads)
}

/// Parse a BizHawk `.bk2` archive.
pub fn parse_bk2(bytes: &[u8]) -> Result<Movie, String> {
    let mut zip = open(bytes)?;
    let log = read_member(&mut zip, "Input Log.txt").ok_or("bk2: no `Input Log.txt`")?;
    let mut m = Movie { pads: parse_bk2_input_log(&log)?, ..Movie::default() };
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
