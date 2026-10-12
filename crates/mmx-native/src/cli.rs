//! Hand-rolled argv parser (no clap). See `docs/native.md`.

use mmx_core::{netgame::Mode, Character};

pub const DEFAULT_MOVIE_COOP_DELAY: u16 = 20;

pub const USAGE: &str = "\
usage: mmxrs [--rom PATH] [--x3-rom PATH] [--sprite-pack PATH] [--zero] [--scale N] [--fullscreen] [--show-fps] [--coop-local|--players 1..4] [--vs] [--vs-bots] [--vs-stage ID] [--pads]
             [--p1-character ID] [--p2-character ID] [--p3-character ID] [--p4-character ID]
             [--dynamic-lighting] [--charge-effects] [--bone-sheets DIR] [--vile-sprite PNG]
             [--pad-map 'South=B,East=A,...'] [--timing ntsc|pal]
             [--demo-bots] [--showcase-characters] [--demo-survival]
             [--movie PATH.bk2|PATH.lsmv|PATH.smv] [--movie-coop-delay N] [--movie-smart]
             [--widescreen off|16:10|16:9|21:9|auto|N] [--wide-gameplay] [--overclock off|1.5|2]
       mmxrs (--coop-host ROOM | --coop-join ROOM) [--signal ws://HOST:PORT]
             [--ice 'stun:.. turn:.. username=U credential=C' | none]
             [--mode rollback|lockstep] [--delay N] [--prediction N] [--net-record LOG]
       mmxrs --headless --frames N [--rom PATH] [--dump-at F=out.png,...]
             [--hold P1MASK] [--p2-hold MASK] [--coop-local]
ROM defaults to $MMX_ROM. Zero uses X1 ROM art by default; explicit --x3-rom
(Mega Man X3 USA) selects X3 Zero art and saber hooks; --sprite-pack replaces Zero's
body art with a JSON pack (docs/sprite-packs.md). Masks are
SNES pad words (hex 0x.. or decimal).
--widescreen draws extra scenery left and right (N = 8-px tiles per side, 0-24; auto follows
the window's shape so a maximized or fullscreen window is filled; F10 cycles the presets and
auto; docs/widescreen.md). --wide-gameplay spawns enemies at the wide edges (136/160 px
past the original ones whatever the preset; changes the game, online both players need it).
--overclock runs the SNES CPU 1.5x or 2x as fast, which removes the game's own slowdown with
many enemies (changes the game: online both players need the same); default 2, off restores
the original slowdown. --show-fps (or F2 in game) draws frames per second and % of full speed in the corner.
--pads (or MMX_PAD_DEBUG=1) logs connected gamepads and every pad bit change to stderr.
--pad-map rebinds gamepad buttons: comma-separated PAD=BUTTON with gilrs pad names (South, East,
West, North, LeftTrigger, RightTrigger, LeftTrigger2, RightTrigger2, Start, Select, DPadUp/Down/Left/Right,
LeftThumb, RightThumb) and SNES buttons (A, B, X, Y, L, R, Start, Select, Up, Down, Left, Right, or none);
unlisted pad buttons keep the default positional layout.
--demo-bots enables four local players, drives a repeatable Highway run, and restores critically low health for recording.
--movie replays recorded inputs from power-on (overclock off by default).
--movie-coop-delay N enables four players copying P1 at 0/N/2N/3N frame delays during gameplay.
--movie-smart follows the Highway route with default P2/P3/P4 offsets of 20/40/60 source frames; --movie-coop-delay overrides the base.
It resynchronizes after hits (--movie-route is an alias); safety maneuvers can temporarily bring players together.
--showcase-characters cycles all playable characters during that four-player demo (requires --bone-sheets).
--vs-bots starts the selected local VS arena and roster directly, drives native fighter inputs,
and restarts each round after a 90-frame winner display. Also supports --headless --frames N.
--bone-sheets loads private Blender or bone animation sprites; high-resolution packs preserve game size.
Vile uses the private on-foot PNG at roms/derived/bone_sheets/user_sources/vile_idle.png
when no animated Vile sheet is loaded (ROM poses without it); --vile-sprite overrides its location.
Online: both players pass the same --mode (default rollback). --delay is the input delay
in frames, host's wins: rollback 0-3 (default 2), lockstep 0-8 (default 3). On a
same-region link --mode lockstep --delay 3 plays smoothest; across continents use
--mode rollback --delay 3 --prediction 20 (rollback's window must cover the round
trip in frames, default 8; see docs/netplay.md). --net-record LOG writes a session log
(confirmed inputs, checksums, stalls, rollbacks) for `cargo xtask netreplay LOG`.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub rom: Option<String>,
    /// Mega Man X3 image for P2's Zero art (docs/zero.md "Playing as Zero").
    pub x3_rom: Option<String>,
    /// `--sprite-pack`: custom Zero body art (docs/sprite-packs.md).
    pub sprite_pack: Option<String>,
    /// `--zero`: player 1 plays as Zero (needs the X3 ROM).
    pub zero: bool,
    pub scale: u32,
    pub fullscreen: bool,
    pub coop_local: bool,
    /// Local arena match with character and stage selection (2-4 players).
    pub vs_mode: bool,
    /// Input-driven local fighters; bypass selection and restart rounds.
    pub vs_bots: bool,
    /// Initial arena choice, selected from `vs_stages::ARENAS`.
    pub vs_stage: Option<String>,
    /// Total local players (1-4); online sessions still use two peers.
    pub players: u8,
    /// Per-player art, while movement and attacks follow the X/Zero player code.
    pub characters: [Character; 4],
    pub dynamic_lighting: bool,
    pub charge_effects: bool,
    /// `--show-fps` / F2: fps and % of full speed drawn over the picture.
    pub show_fps: bool,
    /// Private Bone Animation Editor sheet pack root (optional).
    pub bone_sheets: Option<String>,
    /// Override for the private, transparent on-foot Vile idle crop.
    pub vile_sprite: Option<String>,
    /// Drive all four local players with a deterministic Highway demo script.
    pub demo_bots: bool,
    /// Keep living players healthy during an explicitly assisted recording.
    pub demo_survival: bool,
    /// Cycle the full character roster in the four-player demo.
    pub showcase_characters: bool,
    /// `--pads`: log gamepads and pad bit changes to stderr.
    pub pads: bool,
    /// `--pad-map`: gamepad button rebinding spec (see `keys::PadMap::parse`).
    pub pad_map: Option<String>,
    pub headless: bool,
    pub frames: u64,
    /// (frame number after stepping, png path)
    pub dump_at: Vec<(u64, String)>,
    pub hold: u16,
    pub p2_hold: u16,
    pub p3_hold: u16,
    pub p4_hold: u16,
    pub movie: Option<String>,
    /// Copy P1's movie inputs to the other three slots, N frames apart.
    pub movie_coop_delay: Option<u16>,
    pub movie_route: bool,
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
    /// `--net-record LOG`: session log (`mmx_core::netlog`) for `xtask netreplay`.
    pub net_record: Option<String>,
    /// `--widescreen`: margins each side (docs/widescreen.md). Display only.
    pub widescreen: WideSetting,
    /// `--wide-gameplay`: enemies spawn at the wide edges (gameplay; netplay id).
    pub wide_gameplay: bool,
    /// `--overclock`: explicit CPU overclock; None = [`Args::overclock`]'s default.
    pub overclock: Option<mmx_core::Overclock>,
}

/// `--widescreen`: a fixed margin, or `auto` (the window's aspect picks it,
/// `video::auto_margin`; the frontend re-evaluates it on every resize).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WideSetting {
    Fixed(mmx_core::Widescreen),
    Auto,
}

impl WideSetting {
    pub fn parse(s: &str) -> Option<Self> {
        if s.trim().eq_ignore_ascii_case("auto") {
            return Some(Self::Auto);
        }
        mmx_core::Widescreen::parse(s).map(Self::Fixed)
    }

    /// F10: off, 16:10, 16:9, 21:9, auto, off ... (a custom N goes to off).
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Fixed(mmx_core::Widescreen::OFF),
            Self::Fixed(w) if w == mmx_core::Widescreen::W21_9 => Self::Auto,
            Self::Fixed(w) => Self::Fixed(w.next()),
        }
    }

    /// The margin to start with (auto: none until the window exists).
    pub fn initial(self) -> mmx_core::Widescreen {
        match self {
            Self::Fixed(w) => w,
            Self::Auto => mmx_core::Widescreen::OFF,
        }
    }
}

impl Args {
    /// Input delay for the session: `--delay` or the mode's default.
    pub fn net_delay(&self) -> u8 {
        self.delay.unwrap_or(self.mode.default_delay())
    }

    /// CPU overclock: `--overclock`, else 2x (no reason to keep the SNES's
    /// slowdown; `--overclock off` is for purists and slow hosts).
    pub fn overclock(&self) -> mmx_core::Overclock {
        self.overclock.unwrap_or(if self.movie.is_some() { mmx_core::Overclock::Off } else { mmx_core::Overclock::X2 })
    }
}

impl Default for Args {
    fn default() -> Self {
        Args {
            rom: None,
            x3_rom: None,
            sprite_pack: None,
            zero: false,
            scale: 3,
            fullscreen: false,
            coop_local: false,
            vs_mode: false,
            vs_bots: false,
            vs_stage: None,
            players: 1,
            characters: [Character::X, Character::Zero, Character::X, Character::X],
            dynamic_lighting: false,
            charge_effects: false,
            show_fps: false,
            bone_sheets: None,
            vile_sprite: None,
            demo_bots: false,
            demo_survival: false,
            showcase_characters: false,
            pads: false,
            pad_map: None,
            headless: false,
            frames: 0,
            dump_at: Vec::new(),
            hold: 0,
            p2_hold: 0,
            p3_hold: 0,
            p4_hold: 0,
            movie: None,
            movie_coop_delay: None,
            movie_route: false,
            net: None,
            signal: mmx_net::DEFAULT_SIGNAL_URL.into(),
            ice: None,
            mode: Mode::Rollback,
            delay: None,
            prediction: None,
            timing: mmx_core::Timing::Auto,
            net_record: None,
            widescreen: WideSetting::Fixed(mmx_core::Widescreen::OFF),
            wide_gameplay: false,
            overclock: None,
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
            "--sprite-pack" => a.sprite_pack = Some(val("--sprite-pack")?),
            "--scale" => {
                let v = val("--scale")?;
                a.scale = v.parse().map_err(|_| format!("bad --scale {v:?}"))?;
                if !(1..=8).contains(&a.scale) {
                    return Err("--scale must be 1-8".into());
                }
            }
            "--fullscreen" => a.fullscreen = true,
            "--zero" => a.zero = true,
            "--coop-local" => { a.coop_local = true; a.players = a.players.max(2); }
            "--vs" => a.vs_mode = true,
            "--vs-bots" => a.vs_bots = true,
            "--vs-stage" => a.vs_stage = Some(val("--vs-stage")?),
            "--players" => {
                let v = val("--players")?;
                a.players = v.parse().map_err(|_| format!("bad --players {v:?}: use 1-4"))?;
                if !(1..=4).contains(&a.players) { return Err("--players must be 1-4".into()); }
                a.coop_local = a.players > 1;
            }
            "--p1-character" | "--p2-character" | "--p3-character" | "--p4-character" => {
                let i = usize::from(flag.as_bytes()[3] - b'1');
                let v = val(&flag)?;
                a.characters[i] = Character::parse(&v).ok_or_else(|| format!("unknown character {v:?}"))?;
            }
            "--dynamic-lighting" => a.dynamic_lighting = true,
            "--charge-effects" => a.charge_effects = true,
            "--show-fps" => a.show_fps = true,
            "--bone-sheets" => a.bone_sheets = Some(val("--bone-sheets")?),
            "--vile-sprite" => a.vile_sprite = Some(val("--vile-sprite")?),
            "--demo-bots" => a.demo_bots = true,
            "--demo-survival" => a.demo_survival = true,
            "--showcase-characters" => a.showcase_characters = true,
            "--pads" => a.pads = true,
            "--pad-map" => a.pad_map = Some(val("--pad-map")?),
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
            "--p3-hold" => a.p3_hold = parse_u16(&val("--p3-hold")?)?,
            "--p4-hold" => a.p4_hold = parse_u16(&val("--p4-hold")?)?,
            "--movie" => a.movie = Some(val("--movie")?),
            "--movie-route" | "--movie-smart" => a.movie_route = true,
            "--movie-coop-delay" => {
                let v = val("--movie-coop-delay")?;
                let n: u16 = v.parse().map_err(|_| "--movie-coop-delay needs 0-120 frames".to_string())?;
                if n > 120 { return Err("--movie-coop-delay needs 0-120 frames".into()); }
                a.movie_coop_delay = Some(n);
            }
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
            "--net-record" => a.net_record = Some(val("--net-record")?),
            "--widescreen" => {
                let v = val("--widescreen")?;
                a.widescreen = WideSetting::parse(&v)
                    .ok_or_else(|| format!("bad --widescreen {v:?}: off, 16:10, 16:9, 21:9, auto or 0-24"))?;
            }
            "--wide-gameplay" => a.wide_gameplay = true,
            "--overclock" => a.overclock = Some(val("--overclock")?.parse()?),
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
    if a.zero { a.characters[0] = Character::Zero; }
    if a.movie_route {
        if a.movie.is_none() { return Err("--movie-route requires --movie PATH".into()); }
        a.movie_coop_delay.get_or_insert(DEFAULT_MOVIE_COOP_DELAY);
    }
    if a.demo_survival && !a.movie_route && !a.demo_bots && !a.showcase_characters {
        return Err("--demo-survival requires --movie-smart, --demo-bots or --showcase-characters".into());
    }
    if a.movie_coop_delay.is_some() {
        if a.movie.is_none() { return Err("--movie-coop-delay requires --movie PATH".into()); }
        a.players = 4;
        a.coop_local = true;
    }
    if a.movie.is_some() && (a.demo_bots || a.showcase_characters || a.vs_mode || a.net.is_some() || a.wide_gameplay) {
        return Err("--movie cannot combine with bots, showcase, VS, netplay, or wide gameplay".into());
    }
    if a.showcase_characters {
        if a.bone_sheets.is_none() {
            return Err("--showcase-characters requires --bone-sheets DIR".into());
        }
        a.demo_bots = true;
    }
    if a.demo_bots {
        a.players = 4;
        a.coop_local = true;
    }
    if a.vs_bots && !a.vs_mode {
        return Err("--vs-bots requires --vs".into());
    }
    if a.vs_mode {
        if a.net.is_some() || a.demo_bots || (a.headless && !a.vs_bots) {
            return Err("--vs requires local play; headless VS requires --vs-bots".into());
        }
        if a.players < 2 {
            return Err("--vs needs --players 2..4 (or --coop-local)".into());
        }
        if a.bone_sheets.is_none() {
            return Err("--vs needs --bone-sheets DIR for the full fighter roster".into());
        }
        a.coop_local = true;
    }
    if a.vs_stage.is_some() && !a.vs_mode {
        return Err("--vs-stage requires --vs".into());
    }
    if let Some(id) = a.vs_stage.as_deref() {
        if mmx_core::vs_stages::arena(id).is_none() {
            return Err(format!("unknown VS stage {id:?}"));
        }
    }
    a.zero = a.characters[0] == Character::Zero;
    if a.net.is_some() && (a.headless || a.coop_local) {
        return Err("--coop-host/--coop-join cannot be combined with --headless or --coop-local".into());
    }
    if a.net.is_some() && a.characters[1] != Character::Zero {
        return Err("online co-op currently requires player 2 to be Zero".into());
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
    fn movie_coop_options() {
        let a = p("--movie run.bk2 --movie-coop-delay 8").unwrap();
        assert_eq!(a.players, 4);
        assert!(a.coop_local);
        assert_eq!(a.movie_coop_delay, Some(8));
        assert_eq!(a.overclock(), mmx_core::Overclock::Off);
        let route = p("--movie run.bk2 --movie-route").unwrap();
        assert!(route.movie_route && route.coop_local);
        assert_eq!(route.movie_coop_delay, Some(20));
        let explicit = p("--movie run.bk2 --movie-smart --movie-coop-delay 8").unwrap();
        assert_eq!(explicit.movie_coop_delay, Some(8));
        let synchronized = p("--movie run.bk2 --movie-smart --movie-coop-delay 0").unwrap();
        assert_eq!(synchronized.movie_coop_delay, Some(0));
        assert!(p("--movie run.bk2 --movie-smart").unwrap().movie_route);
        for bad in ["--movie-route", "--movie-coop-delay 8", "--movie a.bk2 --demo-bots", "--movie a.bk2 --wide-gameplay", "--movie a.bk2 --coop-host room", "--movie a.bk2 --movie-coop-delay 121"] {
            assert!(p(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn players_characters_and_effects() {
        let a = p("--players 4 --p1-character storm-eagle --p2-character sigma --p3-character vile --dynamic-lighting --charge-effects").unwrap();
        assert_eq!(a.players, 4);
        assert_eq!(a.characters[0], Character::StormEagle);
        assert_eq!(a.characters[1], Character::Sigma);
        assert_eq!(a.characters[2], Character::Vile);
        assert!(a.dynamic_lighting && a.charge_effects);
        assert!(p("--players 5").is_err());
        assert!(p("--p1-character unknown").is_err());
    }

    #[test]
    fn vs_is_local_and_needs_opponents() {
        let a = p("--vs --players 4 --bone-sheets sheets").unwrap();
        assert!(a.vs_mode && a.coop_local);
        assert_eq!(a.players, 4);
        assert!(p("--vs").is_err());
        assert!(p("--vs --players 2 --bone-sheets sheets --coop-host room").is_err());
    }

    #[test]
    fn vs_bots_preserve_player_count_and_allow_headless_rounds() {
        for players in [2, 4] {
            let a = p(&format!("--vs --vs-bots --players {players} --bone-sheets sheets --vs-stage penguin --headless --frames 600")).unwrap();
            assert!(a.vs_mode && a.vs_bots && a.coop_local && a.headless);
            assert_eq!(a.players, players);
            assert!(!a.demo_bots && !a.demo_survival);
            assert_eq!(a.vs_stage.as_deref(), Some("penguin"));
        }
        for invalid in [
            "--vs-bots --players 2 --bone-sheets sheets",
            "--vs --vs-bots --players 1 --bone-sheets sheets",
            "--vs --vs-bots --players 2 --bone-sheets sheets --demo-bots",
            "--vs --vs-bots --players 2 --bone-sheets sheets --demo-survival",
            "--vs --players 2 --bone-sheets sheets --headless --frames 600",
        ] {
            assert!(p(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn demo_bots_enable_four_local_players() {
        let a = p("--demo-bots").unwrap();
        assert!(a.demo_bots && a.coop_local);
        assert_eq!(a.players, 4);
        assert!(p("--demo-bots --coop-host room").is_err());
    }

    #[test]
    fn online_rejects_unsupported_p2_character() {
        assert!(p("--coop-host room --p2-character x").unwrap_err().contains("player 2"));
    }

    #[test]
    fn widescreen() {
        use mmx_core::Widescreen;
        assert_eq!(p("--widescreen 16:9").unwrap().widescreen, WideSetting::Fixed(Widescreen::W16_9));
        assert_eq!(p("--widescreen 3").unwrap().widescreen.initial().width(), 304);
        assert_eq!(p("--widescreen auto").unwrap().widescreen, WideSetting::Auto);
        assert_eq!(p("--widescreen AUTO").unwrap().widescreen.initial(), Widescreen::OFF);
        // F10: the presets, then auto, then back to off.
        let mut w = WideSetting::Fixed(Widescreen::OFF);
        let mut seen = vec![];
        for _ in 0..5 {
            w = w.next();
            seen.push(w);
        }
        assert_eq!(
            seen,
            [
                WideSetting::Fixed(Widescreen::W16_10),
                WideSetting::Fixed(Widescreen::W16_9),
                WideSetting::Fixed(Widescreen::W21_9),
                WideSetting::Auto,
                WideSetting::Fixed(Widescreen::OFF)
            ]
        );
        assert_eq!(WideSetting::Fixed(Widescreen(3)).next(), WideSetting::Fixed(Widescreen::OFF));
        assert!(p("--widescreen 4:3").is_err());
        assert!(p("--wide-gameplay").unwrap().wide_gameplay);
        assert!(!p("--widescreen 21:9").unwrap().wide_gameplay);
    }

    #[test]
    fn overclock() {
        use mmx_core::Overclock;
        assert_eq!(p("").unwrap().overclock(), Overclock::X2);
        assert_eq!(p("--overclock off").unwrap().overclock(), Overclock::Off);
        assert_eq!(p("--overclock 1.5").unwrap().overclock(), Overclock::X1_5);
        assert!(p("--overclock 3").is_err());
    }

    #[test]
    fn full_windowed() {
        let a = p("--rom x.sfc --x3-rom x3.sfc --sprite-pack zero.json --scale 5 --fullscreen --coop-local --zero --pads").unwrap();
        assert_eq!(a.sprite_pack.as_deref(), Some("zero.json"));
        assert!(a.zero && a.pads);
        assert_eq!(a.rom.as_deref(), Some("x.sfc"));
        assert_eq!(a.x3_rom.as_deref(), Some("x3.sfc"));
        assert_eq!(a.scale, 5);
        assert!(a.fullscreen && a.coop_local && !a.headless);
    }

    #[test]
    fn bone_sheet_path_and_external_character() {
        let args = p("--p1-character bass --bone-sheets /private/sheets").unwrap();
        assert_eq!(args.characters[0], Character::Bass);
        assert_eq!(args.bone_sheets.as_deref(), Some("/private/sheets"));
        assert_eq!(p("--p2-character proto-man").unwrap().characters[1], Character::ProtoMan);
        assert!(p("--showcase-characters").is_err());
        let showcase = p("--showcase-characters --bone-sheets sheets").unwrap();
        assert!(showcase.showcase_characters && showcase.demo_bots && showcase.players == 4);
        assert_eq!(p("--p1-character vile --vile-sprite /private/vile.png").unwrap().vile_sprite.as_deref(), Some("/private/vile.png"));
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
        let a = p("--coop-host r --net-record s.log").unwrap();
        assert_eq!(a.net, Some((true, "r".into())));
        assert_eq!(a.net_record.as_deref(), Some("s.log"));
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
