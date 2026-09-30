//! Hand-rolled argv parser (no clap). See `docs/native.md`.

use mmx_core::netgame::Mode;

pub const USAGE: &str = "\
usage: mmxrs [--rom PATH] [--x3-rom PATH] [--zero] [--scale N] [--fullscreen] [--coop-local]
       mmxrs (--coop-host ROOM | --coop-join ROOM) [--signal ws://HOST:PORT]
             [--ice 'stun:.. turn:.. username=U credential=C' | none]
             [--mode rollback|lockstep] [--delay N] [--prediction N]
       mmxrs --headless --frames N [--rom PATH] [--dump-at F=out.png,...]
             [--hold P1MASK] [--p2-hold MASK] [--coop-local]
ROM defaults to $MMX_ROM, --x3-rom (Mega Man X3 USA: P2 is drawn as X3 Zero, X swings
the saber) to $MMX_X3_ROM. --zero: player 1 plays as Zero too (needs the X3 ROM). Masks are SNES pad words (hex 0x.. or decimal).
Online: both players pass the same --mode (default rollback). --delay is the input delay
in frames, host's wins: rollback 0-3 (default 2), lockstep 0-8 (default 3). On a
same-region link --mode lockstep --delay 3 plays smoothest; across continents use
--mode rollback --delay 3 --prediction 20 (rollback's window must cover the round
trip in frames, default 8; see docs/netplay.md).";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub rom: Option<String>,
    /// Mega Man X3 image for P2's Zero art (docs/zero.md "Playing as Zero").
    pub x3_rom: Option<String>,
    /// `--zero`: player 1 plays as Zero (needs the X3 ROM).
    pub zero: bool,
    pub scale: u32,
    pub fullscreen: bool,
    pub coop_local: bool,
    pub headless: bool,
    pub frames: u64,
    /// (frame number after stepping, png path)
    pub dump_at: Vec<(u64, String)>,
    pub hold: u16,
    pub p2_hold: u16,
    // ponytail: --movie is parsed but unused until mmx-verify's bk2 parser lands.
    pub movie: Option<String>,
    /// Online co-op: (host?, room).
    pub net: Option<(bool, String)>,
    pub signal: String,
    pub ice: Option<String>,
    /// Online sync mode (`--mode`); both peers must agree.
    pub mode: Mode,
    /// `--delay`; None = the mode's default ([`Args::net_delay`]).
    pub delay: Option<u8>,
    /// `--prediction`: rollback's prediction window in frames (1-30; default 8).
    /// Long links (intercontinental, ~15-20 frame round trip) need ~20.
    pub prediction: Option<u8>,
    /// `--timing auto|ntsc|pal`: video timing; auto follows the header region.
    pub timing: mmx_core::Timing,
}

impl Args {
    /// Input delay for the session: `--delay` or the mode's default.
    pub fn net_delay(&self) -> u8 {
        self.delay.unwrap_or(self.mode.default_delay())
    }
}

impl Default for Args {
    fn default() -> Self {
        Args {
            rom: None,
            x3_rom: None,
            zero: false,
            scale: 3,
            fullscreen: false,
            coop_local: false,
            headless: false,
            frames: 0,
            dump_at: Vec::new(),
            hold: 0,
            p2_hold: 0,
            movie: None,
            net: None,
            signal: mmx_net::DEFAULT_SIGNAL_URL.into(),
            ice: None,
            mode: Mode::Rollback,
            delay: None,
            prediction: None,
            timing: mmx_core::Timing::Auto,
        }
    }
}

fn parse_u16(s: &str) -> Result<u16, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u16::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("bad pad mask {s:?}"))
}

/// Parse argv without the program name. `env_rom` is `$MMX_ROM`; `$MMX_X3_ROM`
/// is read by `main` (fallback for `--x3-rom`).
pub fn parse<I: IntoIterator<Item = String>>(
    argv: I,
    env_rom: Option<String>,
) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--rom" => a.rom = Some(val("--rom")?),
            "--x3-rom" => a.x3_rom = Some(val("--x3-rom")?),
            "--scale" => {
                let v = val("--scale")?;
                a.scale = v.parse().map_err(|_| format!("bad --scale {v:?}"))?;
                if !(1..=8).contains(&a.scale) {
                    return Err("--scale must be 1-8".into());
                }
            }
            "--fullscreen" => a.fullscreen = true,
            "--zero" => a.zero = true,
            "--coop-local" => a.coop_local = true,
            "--headless" => a.headless = true,
            "--frames" => {
                let v = val("--frames")?;
                a.frames = v.parse().map_err(|_| format!("bad --frames {v:?}"))?;
            }
            "--dump-at" => {
                for item in val("--dump-at")?.split(',').filter(|s| !s.is_empty()) {
                    let (f, path) = item
                        .split_once('=')
                        .ok_or_else(|| format!("bad --dump-at item {item:?}, want F=path.png"))?;
                    let f = f
                        .parse()
                        .map_err(|_| format!("bad --dump-at frame {f:?}"))?;
                    a.dump_at.push((f, path.to_string()));
                }
            }
            "--hold" => a.hold = parse_u16(&val("--hold")?)?,
            "--p2-hold" => a.p2_hold = parse_u16(&val("--p2-hold")?)?,
            "--movie" => a.movie = Some(val("--movie")?),
            "--coop-host" | "--coop-join" => {
                let room = val(&flag)?;
                if !mmx_net::room_name_valid(&room) {
                    return Err(format!("bad room {room:?}: 1-32 of A-Z a-z 0-9 _ -"));
                }
                a.net = Some((flag == "--coop-host", room));
            }
            "--signal" => a.signal = val("--signal")?,
            "--ice" => a.ice = Some(val("--ice")?),
            "--mode" => {
                let v = val("--mode")?;
                a.mode = Mode::parse(&v).ok_or_else(|| format!("bad --mode {v:?}: rollback or lockstep"))?;
            }
            "--delay" => {
                let v = val("--delay")?;
                a.delay = Some(v.parse().map_err(|_| format!("bad --delay {v:?}"))?);
            }
            "--timing" => {
                let v = val("--timing")?;
                a.timing = mmx_core::Timing::parse(&v).ok_or_else(|| format!("bad --timing {v:?}: auto, ntsc or pal"))?;
            }
            "--prediction" => {
                let v = val("--prediction")?;
                let n: u8 = v.parse().map_err(|_| format!("bad --prediction {v:?}"))?;
                if !(1..=30).contains(&n) {
                    return Err("--prediction is 1-30 frames".into());
                }
                a.prediction = Some(n);
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if a.rom.is_none() {
        a.rom = env_rom.filter(|s| !s.is_empty());
    }
    if a.net.is_some() && (a.headless || a.coop_local) {
        return Err("--coop-host/--coop-join cannot be combined with --headless or --coop-local".into());
    }
    if a.net_delay() > a.mode.max_delay() {
        return Err(format!("--delay must be 0-{} with --mode {}", a.mode.max_delay(), a.mode.name()));
    }
    if a.headless && a.frames == 0 {
        return Err("--headless needs --frames N".into());
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        parse(s.split_whitespace().map(String::from), None)
    }

    #[test]
    fn defaults() {
        let a = p("").unwrap();
        assert_eq!(a, Args::default());
        assert_eq!(a.scale, 3);
    }

    #[test]
    fn full_windowed() {
        let a = p("--rom x.sfc --x3-rom x3.sfc --scale 5 --fullscreen --coop-local --zero").unwrap();
        assert!(a.zero);
        assert_eq!(a.rom.as_deref(), Some("x.sfc"));
        assert_eq!(a.x3_rom.as_deref(), Some("x3.sfc"));
        assert_eq!(a.scale, 5);
        assert!(a.fullscreen && a.coop_local && !a.headless);
    }

    #[test]
    fn headless_everything() {
        let a = p("--headless --frames 600 --dump-at 10=a.png,600=b.png --hold 0x8000 --p2-hold 4096 --movie m.bk2").unwrap();
        assert!(a.headless);
        assert_eq!(a.frames, 600);
        assert_eq!(a.dump_at, vec![(10, "a.png".into()), (600, "b.png".into())]);
        assert_eq!(a.hold, 0x8000);
        assert_eq!(a.p2_hold, 0x1000);
        assert_eq!(a.movie.as_deref(), Some("m.bk2"));
    }

    #[test]
    fn env_rom_is_fallback_only() {
        let env = Some("env.sfc".to_string());
        assert_eq!(
            parse(vec![], env.clone()).unwrap().rom.as_deref(),
            Some("env.sfc")
        );
        let a = parse(vec!["--rom".into(), "cli.sfc".into()], env).unwrap();
        assert_eq!(a.rom.as_deref(), Some("cli.sfc"));
    }

    #[test]
    fn netplay_flags() {
        let a = p("--coop-join room_1 --signal ws://h:1 --ice none --delay 0").unwrap();
        assert_eq!(a.net, Some((false, "room_1".into())));
        assert_eq!((a.signal.as_str(), a.ice.as_deref(), a.net_delay()), ("ws://h:1", Some("none"), 0));
        let a = p("--coop-host r").unwrap();
        assert_eq!(a.net, Some((true, "r".into())));
        assert_eq!((a.signal.as_str(), a.mode, a.net_delay()), ("ws://127.0.0.1:3536", Mode::Rollback, 2));
        assert!(p("--coop-host bad/room").is_err());
        assert!(p("--coop-host r --delay 4").is_err());
        assert!(p("--coop-host r --mode bogus").is_err());
        assert!(p("--coop-host r --coop-local").is_err());
    }

    #[test]
    fn mode_and_delay() {
        let a = p("--coop-host r --mode lockstep").unwrap();
        assert_eq!((a.mode, a.net_delay()), (Mode::Lockstep, 3));
        let a = p("--coop-join r --mode lockstep --delay 8").unwrap();
        assert_eq!((a.mode, a.net_delay()), (Mode::Lockstep, 8));
        assert!(p("--coop-join r --mode lockstep --delay 9").is_err());
        assert!(p("--coop-join r --delay 5 --mode rollback").is_err());
        let a = p("--coop-host r --mode rollback --delay 0").unwrap();
        assert_eq!((a.mode, a.net_delay()), (Mode::Rollback, 0));
    }

    #[test]
    fn errors() {
        assert!(p("--scale 0").is_err());
        assert!(p("--scale 9").is_err());
        assert!(p("--scale").is_err());
        assert!(p("--bogus").is_err());
        assert!(p("--headless").is_err());
        assert!(p("--headless --frames 1 --dump-at nope").is_err());
        assert!(p("--hold 0xZZ").is_err());
    }
}
