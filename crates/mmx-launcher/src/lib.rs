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
pub use mmx_core::Character;

/// Public signalling server used in playtests.
pub const DEFAULT_SIGNAL_URL: &str = "wss://signal.z2rs.com";
/// Game output (stdout + stderr) goes here, inside the data dir: a file, not
/// a pipe, so the game keeps printing safely after the launcher closes.
pub const LOG_FILE_NAME: &str = "last-game.log";
pub const DEFAULT_BONE_SHEETS_PATH: &str = "roms/derived/bone_sheets";
pub const DEFAULT_VILE_SPRITE_PATH: &str = "roms/derived/bone_sheets/user_sources/vile_idle.png";

fn default_characters() -> [String; 4] {
    ["x".into(), "zero".into(), "x".into(), "x".into()]
}

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
    /// Custom Zero sprite pack path (`--sprite-pack`), empty for none.
    #[serde(default)]
    pub sprite_pack: String,
    pub scale: u32,
    pub fullscreen: bool,
    pub zero: bool,
    pub coop_local: bool,
    /// Local arena match; the in-game screen selects fighters and an arena.
    #[serde(default)]
    pub vs_mode: bool,
    /// Number of local players, 1-4. `coop_local` is retained for older saves.
    #[serde(default = "default_local_players")]
    pub local_players: u8,
    #[serde(default = "default_characters")]
    pub characters: [String; 4],
    #[serde(default)]
    pub dynamic_lighting: bool,
    #[serde(default)]
    pub charge_effects: bool,
    /// Optional private Bone Animation Editor sheet pack.
    #[serde(default)]
    pub bone_sheets: bool,
    #[serde(default = "default_bone_sheets_path")]
    pub bone_sheets_path: String,
    /// Correct on-foot Vile crop; loaded even when animated sheets are off.
    #[serde(default = "default_vile_sprite_path")]
    pub vile_sprite_path: String,
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
    /// `--widescreen`: "" / "off", "16:10", "16:9", "21:9" or "auto" (the
    /// window's aspect picks the margin; docs/widescreen.md).
    #[serde(default)]
    pub widescreen: String,
    /// `--wide-gameplay`: enemies spawn at the wide edges (changes the game;
    /// online, both players need it).
    #[serde(default)]
    pub wide_gameplay: bool,
    /// `--pad-map`: gilrs pad button → SNES button (or `none`); unlisted pad
    /// buttons keep the runner's positional default. Empty = send nothing.
    #[serde(default)]
    pub pad_map: std::collections::BTreeMap<String, String>,
}

/// Pad buttons the mapping table offers, in UI order (gilrs names; positional,
/// `South` is the bottom face button).
pub const PAD_BUTTONS: [&str; 10] = [
    "South", "East", "West", "North", "LeftTrigger", "RightTrigger", "LeftTrigger2", "RightTrigger2", "Start", "Select",
];
/// SNES buttons a pad button may be bound to.
pub const GAME_BUTTONS: [&str; 8] = ["A", "B", "X", "Y", "L", "R", "Start", "Select"];
/// Named layouts: positional is the runner's own default (Switch Pro / SNES
/// labels), Xbox labels makes the printed A B X Y match.
pub fn pad_map_presets() -> Vec<(&'static str, std::collections::BTreeMap<String, String>)> {
    let xbox = [("South", "A"), ("East", "B"), ("West", "X"), ("North", "Y")]
        .into_iter()
        .map(|(p, b)| (p.to_string(), b.to_string()))
        .collect();
    vec![("Positional (Switch Pro / SNES)", Default::default()), ("Xbox labels", xbox)]
}
/// The `--pad-map` value for a settings table, or None when it is empty.
pub fn pad_map_spec(map: &std::collections::BTreeMap<String, String>) -> Result<Option<String>, String> {
    let mut pairs = Vec::new();
    for (pad, btn) in map {
        if btn.is_empty() {
            continue;
        }
        if !PAD_BUTTONS.contains(&pad.as_str()) {
            return Err(format!("unknown pad button {pad:?} in the controller map"));
        }
        if btn != "none" && !GAME_BUTTONS.contains(&btn.as_str()) {
            return Err(format!("unknown SNES button {btn:?} in the controller map"));
        }
        pairs.push(format!("{pad}={btn}"));
    }
    Ok((!pairs.is_empty()).then(|| pairs.join(",")))
}

fn default_local_players() -> u8 { 1 }
fn default_bone_sheets_path() -> String {
    let relative = PathBuf::from(DEFAULT_BONE_SHEETS_PATH);
    if let Ok(found) = relative.canonicalize() {
        return found.to_string_lossy().into_owned();
    }
    if let Some(binary) = std::env::current_exe().ok() {
        for parent in binary.ancestors().skip(1).take(4) {
            if let Ok(found) = parent.join(DEFAULT_BONE_SHEETS_PATH).canonicalize() {
                return found.to_string_lossy().into_owned();
            }
        }
    }
    DEFAULT_BONE_SHEETS_PATH.into()
}
fn default_vile_sprite_path() -> String {
    let relative = PathBuf::from(DEFAULT_VILE_SPRITE_PATH);
    if let Ok(found) = relative.canonicalize() {
        return found.to_string_lossy().into_owned();
    }
    if let Some(binary) = std::env::current_exe().ok() {
        for parent in binary.ancestors().skip(1).take(4) {
            if let Ok(found) = parent.join(DEFAULT_VILE_SPRITE_PATH).canonicalize() {
                return found.to_string_lossy().into_owned();
            }
        }
    }
    DEFAULT_VILE_SPRITE_PATH.into()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rom: String::new(),
            x3_rom: String::new(),
            sprite_pack: String::new(),
            scale: 3,
            fullscreen: false,
            zero: false,
            coop_local: false,
            vs_mode: false,
            local_players: 1,
            characters: default_characters(),
            dynamic_lighting: false,
            charge_effects: false,
            bone_sheets: false,
            bone_sheets_path: default_bone_sheets_path(),
            vile_sprite_path: default_vile_sprite_path(),
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
            pad_map: Default::default(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        data_dir().join("launcher.json")
    }
    pub fn load(path: &Path) -> Settings {
        let mut settings: Settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if settings.zero && settings.characters[0] == "x" {
            settings.characters[0] = "zero".into();
            settings.zero = false;
        }
        if settings.coop_local && settings.local_players == 1 {
            settings.local_players = 2;
        }
        settings
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
}

/// The game's argv (without the program name), or why it cannot start.
/// Checks what the game would refuse. X1 Zero uses the X1 ROM by default. File
/// contents are checked separately ([`rom_info`]).
pub fn build_args(s: &Settings) -> Result<Vec<String>, String> {
    let rom = s.rom.trim();
    let x3 = s.x3_rom.trim();
    if rom.is_empty() {
        return Err(NO_ROM_HINT.into());
    }
    let p1 = if s.zero && s.characters[0] == "x" { "zero" } else { s.characters[0].as_str() };
    let local_players = if s.coop_local { s.local_players.max(2) } else { s.local_players };
    if !(1..=4).contains(&local_players) {
        return Err("local players must be 1-4".into());
    }
    let player_count = if s.online { 2 } else { local_players };
    if s.vs_mode && (s.online || local_players < 2) {
        return Err("VS mode needs 2-4 local players".into());
    }
    if s.vs_mode && !s.bone_sheets {
        return Err("VS mode needs 60 FPS bone animation sheets for the full fighter roster".into());
    }
    if s.online && s.characters[1] != "zero" {
        return Err("online co-op currently requires player 2 to be Zero".into());
    }
    for (i, character) in s.characters.iter().enumerate().take(usize::from(player_count)) {
        if Character::parse(character).is_none() {
            return Err(format!("unknown player {} character {character:?}", i + 1));
        }
        if matches!(Character::parse(character), Some(Character::Bass | Character::ProtoMan)) && !s.bone_sheets {
            return Err(format!(
                "{} has no ROM sprites: tick \"60 FPS generated character sprites\" and pick a bone sheet pack",
                Character::parse(character).map_or_else(|| character.clone(), |c| c.label().to_string())
            ));
        }
    }
    if !(1..=8).contains(&s.scale) {
        return Err("scale must be 1-8".into());
    }
    let mut a: Vec<String> = vec!["--rom".into(), rom.into()];
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
    if !x3.is_empty() {
        push(&["--x3-rom", x3]);
    }
    if !s.sprite_pack.trim().is_empty() {
        push(&["--sprite-pack", s.sprite_pack.trim()]);
    }
    push(&["--scale", &s.scale.to_string()]);
    if s.fullscreen {
        push(&["--fullscreen"]);
    }
    if s.vs_mode { push(&["--vs"]); }
    if s.zero && s.characters[0] == "x" {
        push(&["--zero"]);
    }
    for i in 0..usize::from(player_count) {
        let id = if i == 0 { p1 } else { &s.characters[i] };
        let default = if i == 1 { "zero" } else { "x" };
        if id != default && !(i == 0 && s.zero && s.characters[0] == "x") {
            push(&[&format!("--p{}-character", i + 1), id]);
        }
    }
    if s.dynamic_lighting { push(&["--dynamic-lighting"]); }
    if s.charge_effects { push(&["--charge-effects"]); }
    if s.bone_sheets {
        let path = s.bone_sheets_path.trim();
        if path.is_empty() { return Err("choose a bone animation sheet folder".into()); }
        push(&["--bone-sheets", path]);
    }
    if s.characters.iter().take(usize::from(player_count)).any(|id| id == "vile") {
        // Optional: without the PNG the game keeps Vile's ROM poses.
        let path = s.vile_sprite_path.trim();
        if !path.is_empty() { push(&["--vile-sprite", path]); }
    }
    match s.widescreen.as_str() {
        "" | "off" => {}
        w @ ("16:10" | "16:9" | "21:9" | "auto") => push(&["--widescreen", w]),
        other => return Err(format!("bad widescreen {other:?}")),
    }
    if s.wide_gameplay {
        push(&["--wide-gameplay"]);
    }
    if let Some(spec) = pad_map_spec(&s.pad_map)? {
        push(&["--pad-map", &spec]);
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
    } else if local_players > 1 {
        if local_players == 2 && s.coop_local {
            push(&["--coop-local"]);
        } else {
            push(&["--players", &local_players.to_string()]);
        }
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

/// Characters the game refuses to start without a bone sheet for them
/// (`main.rs`: no ROM art to fall back on).
pub fn needs_bone_sheet(character: Character) -> bool {
    matches!(character, Character::Bass | Character::ProtoMan)
}

/// Whether the sheet folder holds `<id>/<id>.sheet.json` for `character`, the
/// file the game's `bone_sheets::install` loads.
pub fn has_bone_sheet(folder: &str, character: Character) -> bool {
    let folder = folder.trim();
    !folder.is_empty() && Path::new(folder).join(character.id()).join(format!("{}.sheet.json", character.id())).is_file()
}

/// Characters the sheet folder has sheets for, or why it is not a usable
/// pack. The pack is private art and is not part of a release download.
pub fn bone_pack(folder: &str) -> Result<Vec<Character>, String> {
    let folder = folder.trim();
    if folder.is_empty() {
        return Err("choose a bone animation sheet folder".into());
    }
    if !Path::new(folder).is_dir() {
        return Err(format!(
            "No 60 FPS sprite pack at {folder}. The pack is not included in the download: untick this option, \
             or pick a folder that holds <character>/<character>.sheet.json files."
        ));
    }
    let found: Vec<Character> = Character::ALL.iter().copied().filter(|&c| has_bone_sheet(folder, c)).collect();
    if found.is_empty() {
        return Err(format!(
            "{folder} has no character sheets (<character>/<character>.sheet.json). Untick this option or pick the pack folder."
        ));
    }
    Ok(found)
}

/// Why `character` cannot be picked with these settings, or None. Used to grey
/// out the dropdown entry and to refuse the launch.
pub fn character_unavailable(s: &Settings, character: Character) -> Option<String> {
    if needs_bone_sheet(character) && !has_bone_sheet(&s.bone_sheets_path, character) {
        return Some(format!(
            "{} has no ROM sprites: he needs a 60 FPS bone sheet pack with {}/{}.sheet.json. \
             Tick \"60 FPS generated character sprites\" and pick the pack folder.",
            character.label(),
            character.id(),
            character.id()
        ));
    }
    None
}

/// Co-op players whose art is rough: anyone but X, or Zero with the X3 ROM,
/// who has no sheet in an enabled 60 FPS pack (X1 Zero and the bosses use
/// partial X1 ROM sprites). Empty for single player.
pub fn coop_rough_art(s: &Settings) -> Vec<Character> {
    let players = if s.online { 2 } else if s.coop_local { s.local_players.max(2) } else { s.local_players };
    if players < 2 {
        return Vec::new();
    }
    let mut rough = Vec::new();
    for c in s.characters.iter().take(usize::from(players)).filter_map(|c| Character::parse(c)) {
        let fine = c == Character::X
            || (c == Character::Zero && !s.x3_rom.trim().is_empty())
            || (s.bone_sheets && has_bone_sheet(&s.bone_sheets_path, c));
        if !fine && !rough.contains(&c) {
            rough.push(c);
        }
    }
    rough
}

/// The co-op art warning shown near the character pickers, or None. A
/// warning only: these characters still launch.
pub fn coop_art_warning(s: &Settings) -> Option<String> {
    let rough = coop_rough_art(s);
    if rough.is_empty() {
        return None;
    }
    let names: Vec<_> = rough.iter().map(|c| if *c == Character::Zero { "Zero (without the X3 ROM)" } else { c.label() }).collect();
    Some(format!(
        "Co-op looks best with X, and Zero using the Mega Man X3 ROM. {} {} rough X1 sprite art without the \
         60 FPS character sprite pack and may look glitchy.",
        names.join(", "),
        if names.len() == 1 { "uses" } else { "use" }
    ))
}

/// File checks the game would otherwise fail on after the window closes:
/// paths exist, the selected characters have their art. [`build_args`] stays
/// pure so `--print-args` and its tests need no files.
pub fn preflight(s: &Settings) -> Result<(), String> {
    let file = |path: &str, what: &str| -> Result<(), String> {
        let p = Path::new(path.trim());
        if p.is_file() { Ok(()) } else { Err(format!("{what} not found: {}", path.trim())) }
    };
    if s.rom.trim().is_empty() {
        return Err(NO_ROM_HINT.into());
    }
    file(&s.rom, "Mega Man X ROM")?;
    if !s.x3_rom.trim().is_empty() {
        file(&s.x3_rom, "Mega Man X3 ROM")?;
    }
    if !s.sprite_pack.trim().is_empty() {
        file(&s.sprite_pack, "Zero sprite pack")?;
    }
    let players = if s.online { 2 } else if s.coop_local { s.local_players.max(2) } else { s.local_players };
    let chosen: Vec<Character> = s.characters.iter().take(usize::from(players)).filter_map(|c| Character::parse(c)).collect();
    if s.bone_sheets {
        bone_pack(&s.bone_sheets_path)?;
    }
    let roster: &[Character] = if s.vs_mode { Character::ALL } else { &chosen };
    for &c in roster {
        if let Some(why) = character_unavailable(s, c) {
            return Err(if s.vs_mode { format!("VS mode offers every fighter. {why}") } else { why });
        }
    }
    Ok(())
}

pub const NO_ROM_HINT: &str =
    "Select your Mega Man X (USA) .sfc ROM. It isn't bundled: browse to your own dump or drop it on this window.";

/// A plain explanation of a failed game run from its log (stdout + stderr),
/// or None when nothing known matched. The raw lines are still shown.
pub fn diagnose(log: &str, s: &Settings) -> Option<String> {
    let rom = s.rom.trim();
    let x3 = s.x3_rom.trim();
    for line in log.lines().rev().map(str::trim).filter(|l| !l.is_empty()) {
        let msg = if line.starts_with("no ROM:") {
            NO_ROM_HINT.to_string()
        } else if line.starts_with("netplay:") {
            format!(
                "Could not connect online ({}). Check the room name and signal URL, that both players picked the \
                 same mode, and that the host started first.",
                line.trim_start_matches("netplay:").trim()
            )
        } else if line.starts_with("bone sheets:") || line.contains("need a 60 FPS bone sheet pack") {
            format!(
                "The 60 FPS bone sheet pack is missing or incomplete ({}). Pick a pack folder that has every \
                 selected character, or choose X or Zero.",
                line.trim_start_matches("bone sheets:").trim()
            )
        } else if line.starts_with("Vile's on-foot sprite is required") {
            "This mmxrs build requires Vile's on-foot PNG. Set the Vile sprite path, choose another character, \
             or update the game: newer builds use his ROM poses without it."
                .to_string()
        } else if line.contains(" sprites: ") {
            format!("Could not read a boss's sprites from the X1 ROM ({line}). Check it is the Mega Man X (USA) dump.")
        } else if !x3.is_empty() && line.starts_with(x3) {
            format!("The X3 ROM could not be used ({}). Clear it or pick Mega Man X3 (USA).", &line[x3.len()..].trim_start_matches(':').trim())
        } else if !rom.is_empty() && line.starts_with(rom) {
            format!("The Mega Man X ROM could not be loaded ({}). Pick the Mega Man X (USA) .sfc dump.", &line[rom.len()..].trim_start_matches(':').trim())
        } else if line.starts_with("unknown argument") {
            "The game did not understand an option from the launcher. The launcher and the mmxrs program are \
             probably from different releases: use the pair from the same download."
                .to_string()
        } else {
            continue;
        };
        return Some(msg);
    }
    None
}

/// Last `n` non-empty lines of `text`.
pub fn last_lines(text: &str, n: usize) -> Vec<&str> {
    let v: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    v[v.len().saturating_sub(n)..].to_vec()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pad_map_is_sent_only_when_set() {
        let mut s = Settings { rom: "/x.sfc".into(), ..Settings::default() };
        assert!(!build_args(&s).unwrap().contains(&"--pad-map".to_string()));
        s.pad_map = pad_map_presets()[1].1.clone();
        let a = build_args(&s).unwrap();
        assert!(a.windows(2).any(|w| w == ["--pad-map", "East=B,North=Y,South=A,West=X"]), "{a:?}");
        s.pad_map.insert("Start".into(), "none".into());
        assert!(build_args(&s).unwrap().iter().any(|x| x.contains("Start=none")));
        s.pad_map.insert("Bogus".into(), "A".into());
        assert!(build_args(&s).is_err());
    }
    use super::*;

    fn base() -> Settings {
        Settings { rom: "x.sfc".into(), ..Default::default() }
    }

    #[test]
    fn argv_every_option() {
        assert_eq!(build_args(&base()).unwrap(), ["--rom", "x.sfc", "--scale", "3"]);
        let s = Settings {
            x3_rom: "x3.sfc".into(),
            sprite_pack: "zero.json".into(),
            scale: 5,
            fullscreen: true,
            zero: true,
            coop_local: true,
            ..base()
        };
        assert_eq!(
            build_args(&s).unwrap().join(" "),
            "--rom x.sfc --x3-rom x3.sfc --sprite-pack zero.json --scale 5 --fullscreen --zero --coop-local"
        );
    }

    #[test]
    fn argv_refusals() {
        assert!(build_args(&Settings::default()).is_err());
        let zero = Settings { zero: true, ..base() };
        assert!(build_args(&zero).unwrap().contains(&"--zero".to_string()));
        assert!(build_args(&Settings { scale: 9, ..base() }).is_err());
        let ws = Settings { widescreen: "16:9".into(), ..base() };
        assert_eq!(build_args(&ws).unwrap()[4..], ["--widescreen", "16:9"]);
        assert!(build_args(&Settings { widescreen: "4:3".into(), ..base() }).is_err());
        let auto = Settings { widescreen: "auto".into(), ..base() };
        assert_eq!(build_args(&auto).unwrap()[4..], ["--widescreen", "auto"]);
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
    fn character_players_and_effects_args() {
        let s = Settings {
            local_players: 4,
            characters: ["storm-eagle".into(), "sigma".into(), "vile".into(), "x".into()],
            dynamic_lighting: true,
            charge_effects: true,
            ..base()
        };
        let args = build_args(&s).unwrap().join(" ");
        assert!(args.contains("--p1-character storm-eagle"));
        assert!(args.contains("--p2-character sigma"));
        assert!(args.contains("--p3-character vile"));
        assert!(args.contains("--players 4"));
        assert!(args.contains("--dynamic-lighting --charge-effects"));
        assert!(!args.contains("--p4-character"));
    }

    #[test]
    fn vs_requires_local_opponents_and_full_roster_art() {
        let mut s = base();
        s.vs_mode = true;
        assert!(build_args(&s).is_err());
        s.local_players = 4;
        assert!(build_args(&s).unwrap_err().contains("bone animation"));
        s.bone_sheets = true;
        let args = build_args(&s).unwrap();
        assert!(args.contains(&"--vs".to_string()));
        assert!(args.windows(2).any(|pair| pair == ["--players", "4"]));
        s.online = true;
        assert!(build_args(&s).is_err());
    }

    #[test]
    fn bone_sheet_option_and_external_characters() {
        let mut s = base();
        s.characters[0] = "bass".into();
        assert!(build_args(&s).unwrap_err().contains("bone sheet pack"));
        s.bone_sheets = true;
        s.bone_sheets_path = "/private/sheets".into();
        assert!(build_args(&s).unwrap().windows(2).any(|w| w == ["--bone-sheets", "/private/sheets"]));
        s.bone_sheets_path.clear();
        assert!(build_args(&s).is_err());
    }

    #[test]
    fn vile_static_sprite_is_passed_without_bone_sheets() {
        let mut s = base();
        s.characters[0] = "vile".into();
        s.vile_sprite_path = "/private/vile.png".into();
        let args = build_args(&s).unwrap();
        assert!(args.windows(2).any(|w| w == ["--vile-sprite", "/private/vile.png"]));
        assert!(!args.contains(&"--bone-sheets".to_string()));
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
        let loaded = Settings::load(&p);
        assert_eq!(loaded.characters[0], "zero");
        assert!(!loaded.zero);
        assert_eq!(loaded.x3_rom, s.x3_rom);
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
    fn diagnose_maps_game_errors() {
        let s = Settings { rom: "/r/mmx.sfc".into(), x3_rom: "/r/x3.sfc".into(), ..base() };
        let d = |log: &str| diagnose(log, &s).unwrap_or_default();
        assert!(d("no ROM: pass --rom PATH or set MMX_ROM\nusage: mmxrs --rom PATH").contains("isn't bundled"));
        assert!(d("netplay: room full").contains("connect online (room full)"));
        assert!(d("Bass and Proto Man need a 60 FPS bone sheet pack (--bone-sheets DIR)").contains("bone sheet pack"));
        assert!(d("bone sheets: bone sheet folder missing: /x").contains("folder missing"));
        assert!(d("Vile's on-foot sprite is required at a.png: No such file").contains("on-foot PNG"));
        assert!(d("/r/mmx.sfc: No such file or directory (os error 2)").contains("X ROM could not be loaded (No such file"));
        assert!(d("/r/x3.sfc: not an X3 ROM").contains("X3 ROM could not be used (not an X3 ROM)"));
        assert!(d("unknown argument \"--foo\"").contains("different releases"));
        assert_eq!(diagnose("audio: no output device\nsomething odd", &s), None);
    }

    /// The window's failure path: spawn, wait, read the log, explain it.
    #[cfg(unix)]
    #[test]
    fn failed_run_is_explained_from_its_log() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mmx-launcher-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("mmxrs");
        std::fs::write(&game, "#!/bin/sh\necho 'audio: ok'\necho 'no ROM: pass --rom PATH or set MMX_ROM' >&2\necho 'usage: mmxrs' >&2\nexit 2\n").unwrap();
        std::fs::set_permissions(&game, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log = dir.join(LOG_FILE_NAME);
        let status = spawn(&game, &[], &log).unwrap().wait().unwrap();
        assert_eq!(status.code(), Some(2));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(diagnose(&text, &base()).unwrap().contains("isn't bundled"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preflight_checks_files_and_sheet_characters() {
        let dir = std::env::temp_dir().join(format!("mmx-launcher-pre-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bass")).unwrap();
        let rom = dir.join("x.sfc");
        std::fs::write(&rom, b"rom").unwrap();
        let mut s = Settings { rom: rom.to_string_lossy().into(), bone_sheets_path: dir.to_string_lossy().into(), ..base() };
        assert_eq!(preflight(&s), Ok(()));
        // The 60 FPS option with no pack (a release download) or an empty one.
        let missing = Settings { bone_sheets: true, bone_sheets_path: "/nope/bone_sheets".into(), ..s.clone() };
        assert!(preflight(&missing).unwrap_err().contains("not included in the download"));
        let empty = Settings { bone_sheets: true, ..s.clone() };
        assert!(preflight(&empty).unwrap_err().contains("has no character sheets"));
        assert!(preflight(&Settings { rom: String::new(), ..s.clone() }).unwrap_err().contains("isn't bundled"));
        assert!(preflight(&Settings { rom: "/nope.sfc".into(), ..s.clone() }).unwrap_err().contains("not found"));
        s.characters[0] = "bass".into();
        s.bone_sheets = true;
        assert!(character_unavailable(&s, Character::Bass).is_some());
        assert!(preflight(&s).unwrap_err().contains("has no character sheets"));
        assert!(preflight(&Settings { bone_sheets: false, ..s.clone() }).unwrap_err().contains("Bass has no ROM sprites"));
        std::fs::write(dir.join("bass/bass.sheet.json"), b"{}").unwrap();
        assert_eq!(character_unavailable(&s, Character::Bass), None);
        assert_eq!(preflight(&s), Ok(()));
        assert_eq!(bone_pack(&s.bone_sheets_path), Ok(vec![Character::Bass]));
        assert!(character_unavailable(&s, Character::ProtoMan).is_some());
        assert_eq!(character_unavailable(&s, Character::Zero), None);
        s.characters[0] = "vile".into();
        // The on-foot PNG is optional art: Vile launches on ROM poses without it.
        s.vile_sprite_path = "/nope/vile.png".into();
        assert_eq!(preflight(&s), Ok(()));
        s.vile_sprite_path.clear();
        assert!(!build_args(&s).unwrap().contains(&"--vile-sprite".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn coop_art_warning_only_for_rough_coop_characters() {
        let dir = std::env::temp_dir().join(format!("mmx-launcher-coop-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("zero")).unwrap();
        std::fs::write(dir.join("zero/zero.sheet.json"), b"{}").unwrap();
        // Default co-op: X + X1 Zero warns; the X3 ROM fixes Zero.
        let mut s = Settings { local_players: 2, ..base() };
        assert_eq!(coop_rough_art(&s), [Character::Zero]);
        assert!(coop_art_warning(&s).unwrap().contains("Zero (without the X3 ROM)"));
        s.x3_rom = "x3.sfc".into();
        assert_eq!(coop_art_warning(&s), None);
        // Single player never warns.
        s.characters[0] = "sigma".into();
        assert_eq!(coop_art_warning(&Settings { local_players: 1, ..s.clone() }), None);
        assert_eq!(coop_rough_art(&s), [Character::Sigma]);
        // An enabled pack with the character's sheet clears it; Sigma has none here.
        s.x3_rom.clear();
        s.characters[0] = "x".into();
        s.bone_sheets = true;
        s.bone_sheets_path = dir.to_string_lossy().into();
        assert_eq!(coop_art_warning(&s), None);
        s.characters[1] = "vile".into();
        assert_eq!(coop_rough_art(&s), [Character::Vile]);
        // Online is co-op too.
        let online = Settings { online: true, local_players: 1, characters: ["x".into(), "zero".into(), "x".into(), "x".into()], ..base() };
        assert_eq!(coop_rough_art(&online), [Character::Zero]);
        let _ = std::fs::remove_dir_all(&dir);
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
