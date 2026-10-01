//! `mmxrs-launcher`: pick the ROMs and options in a window, start `mmxrs`
//! with the matching flags (docs/launcher.md). Shape copied from z2rs's
//! z2-launcher, shrunk. It does not link the game: the game's `cli.rs` stays
//! the single source of truth for what the flags mean.
//!
//! Everything window-free lives here and is unit tested: [`Settings`] (saved
//! as `<data-dir>/launcher.json`), [`build_args`], [`rom_info`], and finding
//! and spawning the game.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde::{Deserialize, Serialize};

/// Public signalling server used in playtests.
pub const DEFAULT_SIGNAL_URL: &str = "wss://signal.z2rs.com";
/// Game output (stdout + stderr) goes here, inside the data dir: a file, not
/// a pipe, so the game keeps printing safely after the launcher closes.
pub const LOG_FILE_NAME: &str = "last-game.log";

/// `$XDG_DATA_HOME/mmxrs`, `~/Library/Application Support/mmxrs` (macOS),
/// `%APPDATA%/mmxrs` (Windows), else `~/.local/share/mmxrs`, else
/// `./.mmxrs-data`. Same rules as z2-launcher; no `dirs` crate.
pub fn data_dir() -> PathBuf {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(x) = env("XDG_DATA_HOME") {
        return x.join("mmxrs");
    }
    if cfg!(windows) {
        if let Some(a) = env("APPDATA") {
            return a.join("mmxrs");
        }
    }
    match env("HOME") {
        Some(h) if cfg!(target_os = "macos") => h.join("Library/Application Support/mmxrs"),
        Some(h) => h.join(".local/share/mmxrs"),
        None => PathBuf::from(".mmxrs-data"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Rollback,
    Lockstep,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Rollback => "rollback",
            Mode::Lockstep => "lockstep",
        }
    }
    /// Same ranges as `mmx_core::netgame::Mode`.
    pub fn max_delay(self) -> u8 {
        match self {
            Mode::Rollback => 3,
            Mode::Lockstep => 8,
        }
    }
}

/// Everything the window shows, saved as JSON. Missing fields take defaults,
/// so old files keep loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub rom: String,
    pub x3_rom: String,
    pub scale: u32,
    pub fullscreen: bool,
    pub zero: bool,
    pub coop_local: bool,
    pub online: bool,
    pub host: bool,
    pub room: String,
    pub signal: String,
    pub mode: Mode,
    /// None = the mode's default ("auto").
    pub delay: Option<u8>,
    /// Rollback prediction window in frames (None = the game's default 8).
    #[serde(default)]
    pub prediction: Option<u8>,
    /// Video timing: "auto" (header region), "ntsc" or "pal".
    #[serde(default)]
    pub timing: String,
    /// Empty = the game's default ICE servers; `none` = no ICE servers.
    pub ice: String,
    /// `--widescreen`: "" / "off", "16:10", "16:9" or "21:9" (docs/widescreen.md).
    #[serde(default)]
    pub widescreen: String,
    /// `--wide-gameplay`: enemies spawn at the wide edges (changes the game;
    /// online, both players need it).
    #[serde(default)]
    pub wide_gameplay: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rom: String::new(),
            x3_rom: String::new(),
            scale: 3,
            fullscreen: false,
            zero: false,
            coop_local: false,
            online: false,
            host: true,
            room: String::new(),
            signal: DEFAULT_SIGNAL_URL.into(),
            mode: Mode::Rollback,
            delay: None,
            prediction: None,
            timing: String::new(),
            ice: String::new(),
            widescreen: String::new(),
            wide_gameplay: false,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        data_dir().join("launcher.json")
    }
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
}

/// The game's argv (without the program name), or why it cannot start.
/// Checks what the game would refuse, plus Zero without an X3 ROM. File
/// contents are checked separately ([`rom_info`]).
pub fn build_args(s: &Settings) -> Result<Vec<String>, String> {
    let rom = s.rom.trim();
    let x3 = s.x3_rom.trim();
    if rom.is_empty() {
        return Err("choose the Mega Man X ROM".into());
    }
    if s.zero && x3.is_empty() {
        return Err("playing as Zero needs the Mega Man X3 ROM".into());
    }
    if !(1..=8).contains(&s.scale) {
        return Err("scale must be 1-8".into());
    }
    let mut a: Vec<String> = vec!["--rom".into(), rom.into()];
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
    if !x3.is_empty() {
        push(&["--x3-rom", x3]);
    }
    push(&["--scale", &s.scale.to_string()]);
    if s.fullscreen {
        push(&["--fullscreen"]);
    }
    if s.zero {
        push(&["--zero"]);
    }
    match s.widescreen.as_str() {
        "" | "off" => {}
        w @ ("16:10" | "16:9" | "21:9") => push(&["--widescreen", w]),
        other => return Err(format!("bad widescreen {other:?}")),
    }
    if s.wide_gameplay {
        push(&["--wide-gameplay"]);
    }
    if s.online {
        let room = s.room.trim();
        let ok = (1..=32).contains(&room.len())
            && room.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !ok {
            return Err("room name: 1-32 of A-Z a-z 0-9 _ -".into());
        }
        push(&[if s.host { "--coop-host" } else { "--coop-join" }, room]);
        let signal = s.signal.trim();
        if !(signal.starts_with("ws://") || signal.starts_with("wss://")) {
            return Err("signal URL must start with ws:// or wss://".into());
        }
        push(&["--signal", signal, "--mode", s.mode.name()]);
        if let Some(d) = s.delay {
            if d > s.mode.max_delay() {
                return Err(format!("{} delay is 0-{}", s.mode.name(), s.mode.max_delay()));
            }
            push(&["--delay", &d.to_string()]);
        }
        match s.timing.as_str() {
            "" | "auto" => {}
            "ntsc" | "pal" => push(&["--timing", &s.timing]),
            other => return Err(format!("bad timing {other:?}")),
        }
        if let Some(p) = s.prediction {
            if s.mode != Mode::Rollback {
                return Err("prediction window applies to rollback only".into());
            }
            if !(1..=30).contains(&p) {
                return Err("prediction window is 1-30 frames".into());
            }
            push(&["--prediction", &p.to_string()]);
        }
        if !s.ice.trim().is_empty() {
            push(&["--ice", s.ice.trim()]);
        }
    } else if s.coop_local {
        push(&["--coop-local"]);
    }
    Ok(a)
}

/// Shell-ish rendering for `--print-args` and the window.
pub fn format_command(program: &str, args: &[String]) -> String {
    let q = |a: &str| {
        if !a.is_empty() && a.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./:=".contains(&b)) {
            a.to_string()
        } else {
            format!("'{}'", a.replace('\'', r"'\''"))
        }
    };
    std::iter::once(q(program)).chain(args.iter().map(|a| q(a))).collect::<Vec<_>>().join(" ")
}

/// Header facts of a SNES dump, as `cargo xtask rominfo` prints them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomInfo {
    pub title: String,
    pub version: u8,
    /// `$FFD9`: 0/1 NTSC (Japan/USA), 2+ PAL.
    pub region: u8,
    pub sha256: String,
}

/// LoROM header at $7FC0 after dropping a 512-byte copier header; SHA-256 of
/// the stripped image (matches mmx-core's `RomInfo`).
pub fn rom_info(bytes: &[u8]) -> Result<RomInfo, String> {
    let rom = if bytes.len() % 0x8000 == 512 { &bytes[512..] } else { bytes };
    if rom.len() < 0x8000 {
        return Err(format!("too small for a SNES ROM ({} bytes)", bytes.len()));
    }
    Ok(RomInfo {
        title: String::from_utf8_lossy(&rom[0x7FC0..0x7FD5]).trim_end().to_string(),
        version: rom[0x7FDB],
        region: rom[0x7FD9],
        sha256: sha256(rom).iter().map(|b| format!("{b:02x}")).collect(),
    })
}

/// Read `path` and check its title: `MEGAMAN X` exactly (X3's title starts
/// with it too), or `MEGAMAN X3` when `x3`.
pub fn check_rom(path: &str, x3: bool) -> Result<RomInfo, String> {
    let bytes = std::fs::read(path.trim()).map_err(|e| e.to_string())?;
    let info = rom_info(&bytes)?;
    let want = if x3 { "MEGAMAN X3" } else { "MEGAMAN X" };
    if info.title != want {
        return Err(format!("header title {:?}, want {want:?}", info.title));
    }
    Ok(info)
}

/// Plain SHA-256 (FIPS 180-4); a ROM is at most a few MiB, speed is moot.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (o, x) in out.chunks_mut(4).zip(h) {
        o.copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// 8 random chars of `A-Za-z0-9`, seeded from std's per-process random
/// hasher keys (no `rand` dep).
pub fn random_room() -> String {
    use std::hash::{BuildHasher, Hasher};
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let mut x = h.finish();
    (0..8)
        .map(|_| {
            let c = CHARS[(x % 62) as usize] as char;
            x /= 62;
            c
        })
        .collect()
}

/// The game program: `mmxrs` next to the launcher, else
/// `./target/release/mmxrs`, else `mmxrs` on PATH.
pub fn game_binary() -> PathBuf {
    let name = format!("mmxrs{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.join(&name)));
    let dev = PathBuf::from("target/release").join(&name);
    beside
        .filter(|p| p.is_file())
        .or_else(|| dev.is_file().then_some(dev))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Start the game with stdout + stderr going to `log`.
pub fn spawn(program: &Path, args: &[String], log: &Path) -> std::io::Result<Child> {
    if let Some(d) = log.parent() {
        std::fs::create_dir_all(d)?;
    }
    let out = std::fs::File::create(log)?;
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out)
        .spawn()
}

/// Last `n` non-empty lines of `text`.
pub fn last_lines(text: &str, n: usize) -> Vec<&str> {
    let v: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    v[v.len().saturating_sub(n)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Settings {
        Settings { rom: "x.sfc".into(), ..Default::default() }
    }

    #[test]
    fn argv_every_option() {
        assert_eq!(build_args(&base()).unwrap(), ["--rom", "x.sfc", "--scale", "3"]);
        let s = Settings {
            x3_rom: "x3.sfc".into(),
            scale: 5,
            fullscreen: true,
            zero: true,
            coop_local: true,
            ..base()
        };
        assert_eq!(
            build_args(&s).unwrap().join(" "),
            "--rom x.sfc --x3-rom x3.sfc --scale 5 --fullscreen --zero --coop-local"
        );
    }

    #[test]
    fn argv_refusals() {
        assert!(build_args(&Settings::default()).is_err());
        let zero = Settings { zero: true, ..base() };
        assert!(build_args(&zero).unwrap_err().contains("X3"));
        assert!(build_args(&Settings { scale: 9, ..base() }).is_err());
        let ws = Settings { widescreen: "16:9".into(), ..base() };
        assert_eq!(build_args(&ws).unwrap()[4..], ["--widescreen", "16:9"]);
        assert!(build_args(&Settings { widescreen: "4:3".into(), ..base() }).is_err());
        let wg = Settings { widescreen: "21:9".into(), wide_gameplay: true, ..base() };
        assert_eq!(build_args(&wg).unwrap()[4..], ["--widescreen", "21:9", "--wide-gameplay"]);
        let net = Settings { online: true, room: "bad/room".into(), ..base() };
        assert!(build_args(&net).is_err());
        let net = Settings { online: true, room: "r".into(), delay: Some(4), ..base() };
        assert!(build_args(&net).is_err());
        let net = Settings { online: true, room: "r".into(), signal: "http://x".into(), ..base() };
        assert!(build_args(&net).is_err());
    }

    #[test]
    fn argv_host_vs_join() {
        let host = Settings { online: true, room: "abc".into(), coop_local: true, ..base() };
        assert_eq!(
            build_args(&host).unwrap().join(" "),
            "--rom x.sfc --scale 3 --coop-host abc --signal wss://signal.z2rs.com --mode rollback"
        );
        let join = Settings {
            online: true,
            host: false,
            room: "abc".into(),
            mode: Mode::Lockstep,
            delay: Some(8),
            ice: "none".into(),
            signal: "ws://h:1".into(),
            ..base()
        };
        assert_eq!(
            build_args(&join).unwrap().join(" "),
            "--rom x.sfc --scale 3 --coop-join abc --signal ws://h:1 --mode lockstep --delay 8 --ice none"
        );
    }

    #[test]
    fn config_round_trip() {
        let p = std::env::temp_dir().join(format!("mmx-launcher-{}/launcher.json", std::process::id()));
        let s = Settings { x3_rom: "y".into(), zero: true, mode: Mode::Lockstep, delay: Some(2), ..base() };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
        std::fs::write(&p, r#"{"rom":"old.sfc"}"#).unwrap();
        assert_eq!(Settings::load(&p), Settings { rom: "old.sfc".into(), ..Default::default() });
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        assert_eq!(Settings::load(&p), Settings::default());
    }

    #[test]
    fn header_on_synthetic_rom() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x7FC0..0x7FD5].copy_from_slice(b"MEGAMAN X            ");
        rom[0x7FDB] = 1;
        let i = rom_info(&rom).unwrap();
        assert_eq!((i.title.as_str(), i.version), ("MEGAMAN X", 1));
        let mut copier = vec![0xAA; 512];
        copier.extend_from_slice(&rom);
        assert_eq!(rom_info(&copier).unwrap(), i);
        assert!(rom_info(&rom[..0x4000]).is_err());
    }

    #[test]
    fn sha256_vectors() {
        let hex = |d: &[u8]| sha256(d).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn room_and_quoting() {
        let r = random_room();
        assert_eq!(r.len(), 8);
        assert!(r.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_eq!(format_command("mmxrs", &["--ice".into(), "stun:a b".into()]), "mmxrs --ice 'stun:a b'");
        assert_eq!(last_lines("a\n\nb\nc\n", 2), ["b", "c"]);
    }
}
