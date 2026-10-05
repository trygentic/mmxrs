//! `cargo xtask movie`: play an input movie (`.bk2` / `.lsmv` / `.smv`) and
//! watch for trouble: panics, widescreen changing the simulation (`--twin`),
//! the margin residency check, cheap picture checks on the wide frame, and
//! PNGs at mode changes. docs/widescreen-coverage.md.

use std::fmt::Write as _;
use std::path::PathBuf;

use mmx_core::{ram, Game, Widescreen};
use mmx_verify::hash_state;

use crate::{die, flag, game_scripted, num, scripted, write_png};

/// Level names by `LEVEL_ID` (`$1F7A`), checked against the screenshots.
pub fn level_name(id: u8) -> &'static str {
    match id {
        0x00 => "Intro highway",
        0x01 => "Launch Octopus",
        0x02 => "Sting Chameleon",
        0x03 => "Armored Armadillo",
        0x04 => "Flame Mammoth",
        0x05 => "Storm Eagle",
        0x06 => "Spark Mandrill",
        0x07 => "Boomer Kuwanger",
        0x08 => "Chill Penguin",
        0x09 => "Sigma 1",
        0x0A => "Sigma 2",
        0x0B => "Sigma 3",
        0x0C => "Sigma 4",
        _ => "?",
    }
}

/// What the game is doing: `$D1 $D2 $D3 $D4`, the level, lives, pause, boss bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Mode {
    d: [u8; 4],
    level: u8,
    lives: u8,
    pause: u8,
    boss: bool,
}

fn mode(g: &Game) -> Mode {
    let w = g.wram();
    let b = |a: u32| w[ram::wram_index(a)];
    Mode {
        d: [b(ram::MODE_D1), b(ram::MODE_D2), b(ram::STAGE_PHASE), b(ram::MODE_D4)],
        level: b(ram::LEVEL_ID),
        lives: b(ram::LIVES),
        pause: b(ram::PAUSE),
        boss: b(ram::BOSS_PTR) | b(ram::BOSS_PTR + 1) != 0,
    }
}

fn status(g: &Game) -> String {
    let w = g.wram();
    let b = |a: u32| w[ram::wram_index(a)];
    let w16 = |a: u32| u16::from_le_bytes([b(a), b(a + 1)]);
    format!(
        "mode {:02X}/{:02X}/{:02X}/{:02X} level {:02X} X=({},{}) act {:02X} hp {} lives {} cam ({},{})",
        b(ram::MODE_D1), b(ram::MODE_D2), b(ram::STAGE_PHASE), b(ram::MODE_D4), b(ram::LEVEL_ID),
        w16(ram::X_POS_X), w16(ram::X_POS_Y), b(ram::X_ACTION), b(ram::X_HP) & 0x7F, b(ram::LIVES), w16(ram::CAMERA_X), w16(ram::CAMERA_Y)
    )
}

/// One `--trace` line, the same format as `tools/oracle-mesen` writes:
/// frame, FNV-1a 64 of WRAM, `$0B9B` main-loop counter, `$0B9E` NMI counter,
/// RNG `$0BA6`, X's x/y, pad held `$A7`, `$D1-$D4`, level, lives, X's HP,
/// `$02FD` (the low byte of the PC the NMI pushed: `A1`/`A4` = which idle-loop
/// instruction it interrupted, so where in the loop the CPU was at NMI).
fn trace_line(f: usize, g: &Game) -> String {
    let w = g.wram();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in &w[..0x20000] {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    let w16 = |a: usize| u16::from_le_bytes([w[a], w[a + 1]]);
    format!(
        "{f} {h:016x} {:02x} {:02x} {:04x} {:04x} {:04x} {:04x} {:02x}{:02x}{:02x}{:02x} {:02x} {:02x} {:02x} {:02x}",
        w[0x0B9B], w[0x0B9E], w16(0x0BA6), w16(0x0BAD), w16(0x0BB0), w16(0x00A7),
        w[0xD1], w[0xD2], w[0xD3], w[0xD4], w[0x1F7A], w[0x1F80], w[0x0BCF] & 0x7F, w[0x02FD]
    ) + &format!(" | pc {:06x} mclk {} at {:?}", g.regs().pbr_pc(), g.master_cycles(), g.ppu_position())
}

/// `F` or `A-B/S` (every `S` frames from `A` to `B`).
fn dump_frames(item: &str) -> Vec<usize> {
    let n = |v: &str| v.trim().parse::<usize>().unwrap_or_else(|_| die("--dump-wram F|A-B/S,.."));
    match item.split_once('-') {
        Some((a, rest)) => {
            let (b, s) = rest.split_once('/').unwrap_or((rest, "1"));
            (n(a)..=n(b)).step_by(n(s).max(1)).collect()
        }
        None => vec![n(item)],
    }
}

/// WRAM + OAM + CGRAM + VRAM (what `--twin` compares).
fn sim_hash(g: &Game) -> u64 {
    let c: Vec<u8> = g.cgram().iter().flat_map(|w| w.to_le_bytes()).collect();
    let v: Vec<u8> = g.vram().iter().flat_map(|w| w.to_le_bytes()).collect();
    hash_state(&[&g.wram()[..], &g.oam(), &c, &v])
}

/// Where widescreen-off `a` and widescreen-on `b` differ (WRAM: every byte, up to 12 listed).
fn first_difference(a: &Game, b: &Game) -> String {
    let w: Vec<usize> = (0..0x20000).filter(|&i| a.wram()[i] != b.wram()[i]).collect();
    if !w.is_empty() {
        let list: Vec<String> = w.iter().take(12).map(|&i| format!("${:06X} {:02X}/{:02X}", 0x7E0000 + i, a.wram()[i], b.wram()[i])).collect();
        return format!("WRAM {} bytes (off/on): {}", w.len(), list.join(" "));
    }
    if let Some(i) = (0..544).find(|&i| a.oam()[i] != b.oam()[i]) {
        return format!("OAM {i} {:02X} vs {:02X}", a.oam()[i], b.oam()[i]);
    }
    if let Some(i) = (0..256).find(|&i| a.cgram()[i] != b.cgram()[i]) {
        return format!("CGRAM {i} {:04X} vs {:04X}", a.cgram()[i], b.cgram()[i]);
    }
    if let Some(i) = (0..0x8000).find(|&i| a.vram()[i] != b.vram()[i]) {
        return format!("VRAM word ${i:04X} {:04X} vs {:04X}", a.vram()[i], b.vram()[i]);
    }
    "no difference".into()
}

/// Cheap checks on a wide frame (margin `m` px per side). Returns finding
/// kinds: `flat-left`/`flat-right` (a margin one colour while the centre strip
/// next to it is not), `seam-left`/`seam-right` (the column step at a window
/// boundary above the centre's own 99th percentile and 3x its median + 24),
/// `tilecol` (an 8-px column boundary inside a margin stepping more than
/// 1.5x the centre's 99th-percentile tile-boundary step).
pub fn picture_checks(px: &[u32], w: usize, h: usize, m: usize) -> Vec<&'static str> {
    let mut out = Vec::new();
    if m == 0 || w != 256 + 2 * m || px.len() < w * h {
        return out;
    }
    let ch = |p: u32| [(p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF];
    let d = |a: u32, b: u32| -> u32 { ch(a).iter().zip(ch(b)).map(|(x, y)| x.abs_diff(y)).sum() };
    // Mean step between column x-1 and x.
    let col = |x: usize| -> f64 { (0..h).map(|y| f64::from(d(px[y * w + x - 1], px[y * w + x]))).sum::<f64>() / h as f64 };
    let centre: Vec<f64> = (m + 1..m + 256).map(col).collect();
    let mut s = centre.clone();
    s.sort_by(f64::total_cmp);
    if s[s.len() - 1] == 0.0 {
        return out; // blank / single-colour picture (forced blank, fades)
    }
    let (median, p99) = (s[s.len() / 2], s[s.len() * 99 / 100]);
    let seam = |x: usize| {
        let v = col(x);
        v > p99 && v > 3.0 * median + 24.0
    };
    if seam(m) {
        out.push("seam-left");
    }
    if seam(m + 256) {
        out.push("seam-right");
    }
    // Tile boundaries: the centre's at x % 8 == 0 relative to the margin.
    let mut tb: Vec<f64> = (1..32).map(|k| centre[8 * k - 1]).collect();
    tb.sort_by(f64::total_cmp);
    let tb99 = tb[tb.len() - 1].max(p99);
    let inside: Vec<usize> = (1..m / 8).map(|k| 8 * k).chain((1..m / 8).map(|k| m + 256 + 8 * k)).collect();
    if inside.iter().any(|&x| {
        let v = col(x);
        v > 1.5 * tb99 && v > 3.0 * median + 24.0
    }) {
        out.push("tilecol");
    }
    let flat = |x0: usize, x1: usize| -> f64 {
        let mut counts = std::collections::HashMap::new();
        for y in 0..h {
            for x in x0..x1 {
                *counts.entry(px[y * w + x]).or_insert(0usize) += 1;
            }
        }
        *counts.values().max().unwrap_or(&0) as f64 / ((x1 - x0) * h) as f64
    };
    let strip = m.min(64);
    if flat(0, m) >= 0.97 && flat(m, m + strip) < 0.9 {
        out.push("flat-left");
    }
    if flat(m + 256, w) >= 0.97 && flat(m + 256 - strip, m + 256) < 0.9 {
        out.push("flat-right");
    }
    out
}

const KINDS: [&str; 13] = ["late", "stale", "slot", "page", "palette", "unseen", "flat-left", "flat-right", "seam-left", "seam-right", "tilecol", "twin", "centre"];

/// Pixels of the wide frame's centre (`m` px margins) that differ from the
/// unwidened frame, columns 32..224 only (the HUD moves to the margins).
fn centre_diff(wide: (&[u32], usize, usize), off: (&[u32], usize, usize), m: usize) -> usize {
    let ((a, aw, ah), (b, bw, bh)) = (wide, off);
    if aw != 256 + 2 * m || bw != 256 || ah != bh {
        return 0;
    }
    (0..ah).map(|y| (32..224).filter(|&x| a[y * aw + m + x] != b[y * bw + x]).count()).sum()
}

struct Segment {
    level: u8,
    in_stage: bool,
    from: usize,
    to: usize,
    deaths: usize,
    counts: [usize; KINDS.len()],
    /// First frame of each kind (PNG saved there).
    first: [Option<usize>; KINDS.len()],
}

impl Segment {
    fn label(&self) -> String {
        if self.in_stage {
            format!("{} (${:02X})", level_name(self.level), self.level)
        } else {
            format!("menus/cutscenes (level ${:02X})", self.level)
        }
    }
}

pub fn movie(args: &[String]) -> i32 {
    // Without `--movie` the input script alone drives it (fallback coverage).
    let inputs = crate::movie(args);
    if inputs.is_empty() && num(args, "--frames").is_none() {
        die("movie needs --movie F.bk2|F.lsmv|F.smv or --frames N with an input script");
    }
    if args.iter().any(|a| a == "--wide-gameplay") {
        die("movie: --wide-gameplay changes the game and desyncs any movie");
    }
    let frames = num(args, "--frames").unwrap_or(inputs.len());
    let out = PathBuf::from(flag(args, "--out").unwrap_or("target/movie/run"));
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    let script = scripted(args);
    let pokes = crate::Pokes::parse(args);
    let png_every = num(args, "--png-every").unwrap_or(600);
    let pict_every = num(args, "--pict-every").unwrap_or(30);
    let log_every = num(args, "--log-every").unwrap_or(0);
    let at: Vec<usize> = flag(args, "--at").map(|s| s.split(',').map(|f| f.trim().parse().unwrap_or_else(|_| die("--at"))).collect()).unwrap_or_default();
    let save_every = num(args, "--save-every").unwrap_or(0);
    let save_dir = PathBuf::from(flag(args, "--save-dir").unwrap_or("target/movie/states"));
    // `--trace PATH`: one line per frame for diffing against tools/oracle-mesen.
    let mut trace = flag(args, "--trace").map(|p| {
        std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| die(&format!("{p}: {e}"))))
    });
    let mut cpu_trace = args.iter().position(|a| a == "--cpu-trace").map(|i| {
        let (r, p) = (args.get(i + 1).map(String::as_str).unwrap_or(""), args.get(i + 2).map(String::as_str).unwrap_or(""));
        let (a, b) = r.split_once('-').unwrap_or_else(|| die("--cpu-trace A-B PATH"));
        let n = |v: &str| v.parse::<usize>().unwrap_or_else(|_| die("--cpu-trace A-B PATH"));
        (n(a), n(b), std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| die(&format!("{p}: {e}")))))
    });
    let mut spc_trace = flag(args, "--spc-trace").map(|p| {
        std::io::BufWriter::new(std::fs::File::create(p).unwrap_or_else(|e| die(&format!("{p}: {e}"))))
    });
    // `--dump-wram F|A-B/S,..`: WRAM after those frames to `OUT/wram-F.bin`.
    let dump_wram: Vec<usize> = flag(args, "--dump-wram").map(|s| s.split(',').flat_map(dump_frames).collect()).unwrap_or_default();
    if save_every > 0 {
        std::fs::create_dir_all(&save_dir).unwrap_or_else(|e| die(&format!("{}: {e}", save_dir.display())));
    }
    let mut g = game_scripted(args);
    // `--twin`: a second game with widescreen off, compared every frame.
    let mut twin = args.iter().any(|a| a == "--twin").then(|| {
        let mut t = game_scripted(args);
        t.set_widescreen(Widescreen::parse("off").unwrap());
        t
    });
    // Movies press Left+Right (3197M at frame 14258); the console passes it through.
    g.set_allow_opposing_directions(true);
    if let Some(t) = twin.as_mut() {
        t.set_allow_opposing_directions(true);
    }
    let twin_keep = args.iter().any(|a| a == "--twin-keep");
    let wide = g.widescreen();
    if g.wide_margin() > 0 {
        g.margin.enable_check();
    }
    // `--wram-fill HEX`: power-on WRAM contents (our core zeroes it; bsnes v0.87 uses $55).
    if let Some(v) = flag(args, "--wram-fill") {
        let v = u8::from_str_radix(v, 16).unwrap_or_else(|_| die("--wram-fill HEX"));
        g.wram_mut().fill(v);
        if let Some(t) = twin.as_mut() {
            t.wram_mut().fill(v);
        }
    }
    eprintln!(
        "movie: {frames} frames, widescreen {} (margin {} px), wide gameplay {}, overclock {:?}, twin {}",
        wide.name(), g.wide_margin(), g.wide_gameplay(), g.overclock(), twin.is_some()
    );
    let mut segs: Vec<Segment> = Vec::new();
    let mut events = String::new();
    let mut findings = String::new();
    let mut prev: Option<Mode> = None;
    let mut last_state: (usize, Vec<u8>) = (0, g.save_state());
    let mut reports_seen = 0;
    let mut twin_bad = 0usize;
    let mut centre_bad = 0usize;
    let shot = |g: &Game, name: String| {
        let (px, w, h) = g.frame();
        let p = out.join(name);
        write_png(&p, px, w, h).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    };
    for f in 1..=frames {
        let pads = script(f, inputs.get(f - 1).copied().unwrap_or_default());
        if f % 60 == 0 {
            last_state = (f - 1, g.save_state());
        }
        pokes.apply(&mut g, f);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match cpu_trace.as_mut() {
            // `--cpu-trace A-B PATH`: every opcode fetch of frames A..=B as
            // `frame PC scanline H` (H in master clocks), like tools/oracle-mesen.
            Some((a, b, out)) if (*a..=*b).contains(&f) => {
                use std::io::Write as _;
                if spc_trace.is_some() && f == *a {
                    Game::spc_trace(true);
                }
                for (pc, line, h) in g.trace_frame_pos(pads) {
                    writeln!(out, "{f} {pc:06x} {line} {h}").unwrap_or_else(|e| die(&format!("--cpu-trace: {e}")));
                }
                // `--spc-trace PATH` (with --cpu-trace): SPC700 instruction starts, `frame PC cycle`.
                if let Some(st) = spc_trace.as_mut() {
                    for (pc, cyc) in Game::spc_trace_take() {
                        writeln!(st, "{f} {pc:04x} {cyc}").unwrap_or_else(|e| die(&format!("--spc-trace: {e}")));
                    }
                }
            }
            _ => g.step(pads),
        }));
        if r.is_err() {
            let p = out.join(format!("panic-before-{:06}.state", last_state.0 + 1));
            let _ = std::fs::write(&p, &last_state.1);
            println!(
                "PANIC at movie frame {f} ({}); state after frame {} in {}; repro: cargo xtask movie {} --frames {f} (or --load {} --movie-offset -{}@0)",
                status(&g), last_state.0, p.display(), args.join(" "), p.display(), last_state.0
            );
            return 3;
        }
        let mut twin_now = false;
        let mut centre_now = false;
        if let Some(t) = twin.as_mut() {
            pokes.apply(t, f);
            t.step(pads);
            let n = centre_diff(g.frame(), t.frame(), usize::from(g.wide_margin()));
            if n > 0 {
                centre_now = true;
                centre_bad += 1;
                if centre_bad <= 200 {
                    let _ = writeln!(findings, "centre f={f} {n} pixels differ from the unwidened frame (x 32..224) ({})", status(&g));
                }
            }
            if sim_hash(t) != sim_hash(&g) {
                twin_now = true;
                twin_bad += 1;
                if twin_bad <= 200 {
                    let _ = writeln!(findings, "twin f={f} {} ({})", first_difference(t, &g), status(&g));
                }
                // Re-align so one difference is not reported every frame after
                // it (`--twin-keep`: let the two runs drift apart instead).
                if !twin_keep {
                    let s = g.save_state();
                    t.load_state(&s).unwrap_or_else(|e| die(&e));
                }
            }
        }
        if let Some(t) = trace.as_mut() {
            use std::io::Write as _;
            writeln!(t, "{}", trace_line(f, &g)).unwrap_or_else(|e| die(&format!("--trace: {e}")));
        }
        if dump_wram.contains(&f) {
            let p = out.join(format!("wram-{f}.bin"));
            std::fs::write(&p, &g.wram()[..0x20000]).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
        }
        let m = mode(&g);
        let in_stage = m.d[1] == ram::D2_IN_STAGE;
        if segs.last().is_none_or(|s| s.level != m.level || s.in_stage != in_stage) {
            if let Some(s) = segs.last_mut() {
                s.to = f - 1;
            }
            segs.push(Segment { level: m.level, in_stage, from: f, to: f, deaths: 0, counts: [0; KINDS.len()], first: [None; KINDS.len()] });
        }
        let seg = segs.last_mut().unwrap();
        seg.to = f;
        let mut tags: Vec<String> = Vec::new();
        if let Some(p) = prev {
            if p.d != m.d || p.level != m.level {
                tags.push(format!("mode{:02X}{:02X}{:02X}{:02X}-L{:02X}", m.d[0], m.d[1], m.d[2], m.d[3], m.level));
            }
            if m.lives < p.lives && in_stage {
                seg.deaths += 1;
                tags.push("death".into());
            }
            if m.boss != p.boss {
                tags.push(if m.boss { "boss-bar-on".into() } else { "boss-bar-off".into() });
            }
            if (m.pause != 0) != (p.pause != 0) && m.pause != 0 {
                tags.push("pause".into());
            }
        }
        prev = Some(m);
        if !tags.is_empty() {
            let _ = writeln!(events, "f={f} {} | {}", tags.join(" "), status(&g));
            shot(&g, format!("f{f:06}_{}.png", tags.join("_")));
        }
        if png_every > 0 && f % png_every == 0 || at.contains(&f) {
            shot(&g, format!("f{f:06}.png"));
        }
        if log_every > 0 && f % log_every == 0 {
            println!("{f} {}", status(&g));
        }
        if save_every > 0 && f % save_every == 0 {
            let p = save_dir.join(format!("movie-{f:06}.state"));
            std::fs::write(&p, g.save_state()).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
        }
        // Margin residency findings since the last frame.
        let mut kinds_now: Vec<&str> = Vec::new();
        if let Some(c) = &g.margin.check {
            for r in &c.reports[reports_seen..] {
                let k = r.split(' ').next().unwrap_or("");
                if k != "late" {
                    let _ = writeln!(findings, "margin f={f} {r}");
                }
                if let Some(k) = KINDS.iter().find(|&&x| x == k) {
                    kinds_now.push(k);
                }
            }
            reports_seen = c.reports.len();
        }
        if pict_every > 0 && f % pict_every == 0 && in_stage {
            let (px, w, h) = g.frame();
            for k in picture_checks(px, w, h, usize::from(g.wide_margin())) {
                let _ = writeln!(findings, "picture f={f} {k} ({})", status(&g));
                kinds_now.push(k);
            }
        }
        if twin_now {
            kinds_now.push("twin");
        }
        if centre_now {
            kinds_now.push("centre");
        }
        for k in kinds_now {
            let i = KINDS.iter().position(|&x| x == k).unwrap();
            seg.counts[i] += 1;
            if seg.first[i].is_none() {
                seg.first[i] = Some(f);
                if k != "late" {
                    shot(&g, format!("f{f:06}_find_{k}.png"));
                }
            }
        }
    }
    // Table.
    let mut t = String::new();
    let _ = writeln!(t, "| frames | segment | deaths | state / centre differences (twin) | margin stale/slot/page/palette (late) | picture flat L/R, seam L/R, tilecol (checked every {pict_every}) |");
    let _ = writeln!(t, "|---|---|---|---|---|---|");
    for s in &segs {
        let c = |k: &str| s.counts[KINDS.iter().position(|&x| x == k).unwrap()];
        let _ = writeln!(
            t,
            "| {}-{} | {} | {} | {} / {} | {}/{}/{}/{} ({}) | {}/{}, {}/{}, {} |",
            s.from, s.to, s.label(), s.deaths, c("twin"), c("centre"), c("stale"), c("slot"), c("page"), c("palette"), c("late"),
            c("flat-left"), c("flat-right"), c("seam-left"), c("seam-right"), c("tilecol")
        );
    }
    let w = |name: &str, body: &str| {
        let p = out.join(name);
        std::fs::write(&p, body).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    };
    w("events.txt", &events);
    w("findings.txt", &findings);
    w("segments.md", &t);
    print!("{t}");
    println!("end: frame {frames} {}", status(&g));
    if twin.is_some() {
        println!("twin (widescreen off) state differences: {twin_bad} frames; picture centre differences: {centre_bad} frames");
    }
    println!("wrote {}/{{events.txt,findings.txt,segments.md}} and PNGs", out.display());
    i32::from(twin_bad > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 256 + 2*16 wide frame of vertical stripes (a "busy" centre).
    fn frame(m: usize) -> (Vec<u32>, usize, usize) {
        let (w, h) = (256 + 2 * m, 8);
        let px = (0..w * h).map(|i| if (i % w) / 3 % 2 == 0 { 0x406080 } else { 0x405070 }).collect();
        (px, w, h)
    }

    #[test]
    fn picture_checks_find_flat_margins_and_seams() {
        let (mut px, w, h) = frame(16);
        assert_eq!(picture_checks(&px, w, h, 16), Vec::<&str>::new());
        for y in 0..h {
            for x in 0..16 {
                px[y * w + x] = 0;
            }
        }
        let k = picture_checks(&px, w, h, 16);
        assert!(k.contains(&"flat-left") && k.contains(&"seam-left") && !k.contains(&"flat-right"), "{k:?}");
        // a garbage tile column in the right margin
        let (mut px, w, h) = frame(16);
        for y in 0..h {
            for x in 256 + 16 + 8..256 + 32 {
                px[y * w + x] = 0xFFFFFF;
            }
        }
        assert!(picture_checks(&px, w, h, 16).contains(&"tilecol"));
        // no margin: nothing to check
        assert!(picture_checks(&px, w, h, 0).is_empty());
    }
}
