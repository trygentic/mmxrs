//! `cargo xtask <verify|dump|ledger|rominfo>`. Plain `std::env::args`.
//! All fs lives here, never in the libs.

use std::path::{Path, PathBuf};
use std::process::exit;

use mmx_core::{ram, Game, Pads};
use mmx_verify::{movie_bk2, CompareOpts, Lockstep};

const USAGE: &str = "usage: cargo xtask <command>
  verify [--movie F.bk2|F.lsmv] [--frames N] [--no-frame] [--coop-off] [--tap F,..] [--hold HEX] [--p2-hold HEX]
  trace --frames N [--tap F,..] [--hold HEX] [--find HEX,..]   bank histogram of frame N's opcode fetches
  dump --frames N --out DIR [--at F,F,...] [--movie F] [--tap F,F,...] [--hold HEX] [--p2-hold HEX] [--coop]
       [--load STATE] [--save F=PATH,..]  start from / write a save state (P2 re-anchors after a load)
       --tap holds Start on pad 1 for 4 frames at each listed frame; --hold/--p2-hold OR a constant pad mask in
  ledger
  rominfo
env: MMX_ROM=path/to/Mega Man X (USA).sfc";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("verify") => verify(&args[1..]),
        Some("dump") => dump(&args[1..]),
        Some("trace") => trace(&args[1..]),
        Some("cooptrace") => cooptrace(&args[1..]),
        Some("ledger") => ledger(),
        Some("rominfo") => rominfo(),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    exit(code);
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn num(args: &[String], name: &str) -> Option<usize> {
    flag(args, name).map(|v| v.parse().unwrap_or_else(|_| die(&format!("{name}: not a number: {v}"))))
}

fn die(msg: &str) -> ! {
    eprintln!("xtask: {msg}");
    exit(2)
}

fn rom() -> Vec<u8> {
    let Some(path) = std::env::var_os("MMX_ROM") else {
        die("MMX_ROM is not set. Point it at your own Mega Man X (USA) ROM dump \
             (.sfc/.smc, v1.0 preferred); the ROM is never committed. \
             e.g. MMX_ROM=~/roms/mmx.sfc cargo xtask verify --frames 600")
    };
    std::fs::read(&path).unwrap_or_else(|e| die(&format!("MMX_ROM {}: {e}", path.to_string_lossy())))
}

/// `--load PATH`: start from a save state (frame numbering restarts at 1).
fn game_scripted(args: &[String]) -> Game {
    let mut g = game(rom());
    if let Some(p) = flag(args, "--load") {
        let s = std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")));
        g.load_state(&s).unwrap_or_else(|e| die(&format!("{p}: {e}")));
    }
    g
}

/// `--save F=PATH,...`: write a save state after frame F.
fn saves(args: &[String]) -> Vec<(usize, String)> {
    flag(args, "--save")
        .map(|s| {
            s.split(',')
                .map(|t| {
                    let (f, p) = t.split_once('=').unwrap_or_else(|| die("--save: want FRAME=PATH"));
                    (f.trim().parse().unwrap_or_else(|_| die("--save")), p.trim().to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn game(rom: Vec<u8>) -> Game {
    Game::from_rom(rom).unwrap_or_else(|e| die(&format!("ROM rejected: {e}")))
}

fn movie(args: &[String]) -> Vec<Pads> {
    let Some(path) = flag(args, "--movie") else { return Vec::new() };
    let bytes = std::fs::read(path).unwrap_or_else(|e| die(&format!("{path}: {e}")));
    let m = if path.ends_with(".lsmv") { movie_bk2::parse_lsmv(&bytes) } else { movie_bk2::parse_bk2(&bytes) };
    let m = m.unwrap_or_else(|e| die(&format!("{path}: {e}")));
    eprintln!("movie: {path}: {} frames, platform {:?}, rerecords {}", m.pads.len(), m.platform, m.rerecords);
    m.pads
}

/// `--tap F,..` (Start on pad 1 for 4 frames at each), `--hold HEX`, `--p2-hold HEX`,
/// OR'd over the movie track: the same input script for verify, dump and trace.
fn scripted(args: &[String]) -> impl Fn(usize, Pads) -> Pads {
    let taps: Vec<usize> = flag(args, "--tap")
        .map(|s| s.split(',').map(|f| f.trim().parse().unwrap_or_else(|_| die(&format!("--tap: bad frame {f}")))).collect())
        .unwrap_or_default();
    let hex = |k: &str| flag(args, k).map(|h| u16::from_str_radix(h.trim_start_matches("0x"), 16).unwrap_or_else(|_| die(&format!("{k}: bad hex {h}")))).unwrap_or(0);
    let (hold1, hold2) = (hex("--hold"), hex("--p2-hold"));
    let from = num(args, "--from").unwrap_or(0);
    // `--p1tap HEX@F,HEX@F` / `--p2tap`: hold HEX for 4 frames from F.
    let taps_for = |k: &str| -> Vec<(u16, usize)> {
        flag(args, k)
            .map(|s| {
                s.split(',')
                    .map(|t| {
                        let (m, f) = t.split_once('@').unwrap_or_else(|| die(&format!("{k}: want HEX@FRAME, got {t}")));
                        (u16::from_str_radix(m.trim().trim_start_matches("0x"), 16).unwrap_or_else(|_| die(k)), f.trim().parse().unwrap_or_else(|_| die(k)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (t1, t2) = (taps_for("--p1tap"), taps_for("--p2tap"));
    move |f, mut pads| {
        if f >= from {
            pads.p1 |= hold1;
            pads.p2 |= hold2;
        }
        for &(m, t) in &t1 {
            if (t..t + 4).contains(&f) {
                pads.p1 |= m;
            }
        }
        for &(m, t) in &t2 {
            if (t..t + 4).contains(&f) {
                pads.p2 |= m;
            }
        }
        if taps.iter().any(|&t| (t..t + 4).contains(&f)) {
            pads.p1 |= mmx_core::BTN_START;
        }
        pads
    }
}

fn verify(args: &[String]) -> i32 {
    let script = scripted(args);
    let inputs: Vec<Pads> = {
        let m = movie(args);
        let n = num(args, "--frames").unwrap_or(m.len()).max(m.len());
        (1..=n).map(|f| script(f, m.get(f - 1).copied().unwrap_or_default())).collect()
    };
    let frames = num(args, "--frames")
        .or((!inputs.is_empty()).then_some(inputs.len()))
        .unwrap_or_else(|| die("verify needs --frames N or --movie F"));
    let rom = rom();
    let mut oracle = game(rom.clone());
    oracle.traps.set_enabled(false);
    // TODO: register the default trap set on the DUT once mmx-core has one
    // (mmx_core::register_default_traps(&mut dut)). Until then DUT == oracle.
    let mut dut = game(rom);
    if args.iter().any(|a| a == "--coop-off") {
        // Parity rule: the co-op trap group registered but disabled must be
        // byte-identical to the untrapped oracle.
        mmx_core::coop::register_coop_traps(&mut dut);
        dut.coop.enabled = false;
    }
    let opts = CompareOpts { compare_frame: !args.iter().any(|a| a == "--no-frame"), ..Default::default() };
    match Lockstep::run(&mut oracle, &mut dut, &inputs, frames, opts) {
        Some(d) => {
            println!("{d}");
            1
        }
        None => {
            println!("OK {frames} frames identical ({} traps registered on DUT)", dut.traps.len());
            0
        }
    }
}

fn dump(args: &[String]) -> i32 {
    let frames = num(args, "--frames").unwrap_or_else(|| die("dump needs --frames N"));
    let out = PathBuf::from(flag(args, "--out").unwrap_or_else(|| die("dump needs --out DIR")));
    let at: Vec<usize> = match flag(args, "--at") {
        Some(s) => s.split(',').map(|f| f.trim().parse().unwrap_or_else(|_| die(&format!("--at: bad frame {f}")))).collect(),
        None => vec![frames],
    };
    let inputs = movie(args);
    let script = scripted(args);
    let saves = saves(args);
    let mut g = game_scripted(args);
    if args.iter().any(|a| a == "--coop") {
        g.coop.enabled = true;
        mmx_core::coop::register_coop_traps(&mut g);
    }
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    for f in 1..=frames {
        g.step(script(f, inputs.get(f - 1).copied().unwrap_or_default()));
        for (_, p) in saves.iter().filter(|(sf, _)| *sf == f) {
            std::fs::write(p, g.save_state()).unwrap_or_else(|e| die(&format!("{p}: {e}")));
            println!("saved state after frame {f}: {p}");
        }
        if at.contains(&f) {
            let path = out.join(format!("frame_{f:06}.png"));
            let (px, w, h) = g.frame();
            write_png(&path, px, w, h).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
            let w = g.wram();
            let b = |a: u32| w[mmx_core::ram::wram_index(a)];
            let w16 = |a: u32| u16::from_le_bytes([b(a), b(a + 1)]);
            println!(
                "{} mode {:02X}/{:02X}/{:02X} level {:02X} X=({},{}) act {:02X} vy {} hp {} lives {} cam {} pad1 {:04X} xin {:02X}{:02X} coop_hidden {} last_trap {:?}",
                path.display(), b(ram::MODE_D1), b(ram::MODE_D2), b(ram::STAGE_PHASE), b(ram::LEVEL_ID),
                w16(ram::X_POS_X), w16(ram::X_POS_Y), b(ram::X_ACTION), w16(ram::X_VEL_Y) as i16, b(ram::X_HP) & 0x7F, b(ram::LIVES), w16(ram::CAMERA_X), w16(ram::PAD1_HELD), b(ram::X_INPUT_HELD), b(ram::X_INPUT_HELD + 1), g.coop.p2_hidden,
                g.traplog.last().map(|t| (t.name, t.frame))
            );
            if g.coop.enabled {
                let p2 = &g.coop.p2;
                let p16 = |o: usize| u16::from_le_bytes([p2[o], p2[o + 1]]);
                println!(
                    "    P2 pos=({},{}) act {:02X} vy {} hp {} in {:02X}{:02X} f2B {:02X} f2C {:02X} (P1 f2B {:02X} f2C {:02X}) vis {:02X} oam base {} count {} hook D={:04X} DB={:02X} P={:02X}",
                    p16(0x05), p16(0x08), p2[0x02], p16(0x1C) as i16, p2[0x27] & 0x7F, p2[0x36], p2[0x37], p2[0x2B], p2[0x2C], b(ram::X_BASE + 0x2B), b(ram::X_BASE + 0x2C), p2[0x0E], g.coop.p2_oam_base, g.coop.p2_oam_count,
                    g.coop.dbg_regs.0, g.coop.dbg_regs.1, g.coop.dbg_regs.2
                );
            }
        }
    }
    0
}

/// `trace --frames N [--tap ..] [--hold HEX] [--find HEX,HEX]`: run N-1 frames,
/// trace frame N, print the bank histogram and hit counts for `--find` PCs.
fn trace(args: &[String]) -> i32 {
    let frames = num(args, "--frames").unwrap_or_else(|| die("trace needs --frames N"));
    let script = scripted(args);
    let mut g = game_scripted(args);
    let find: Vec<u32> = flag(args, "--find")
        .map(|s| s.split(',').map(|f| u32::from_str_radix(f.trim().trim_start_matches("0x"), 16).unwrap_or_else(|_| die("--find"))).collect())
        .unwrap_or_default();
    for f in 1..frames {
        g.step(script(f, Pads::default()));
    }
    let pcs = g.trace_frame(script(frames, Pads::default()));
    let mut banks = std::collections::BTreeMap::new();
    for &pc in &pcs {
        *banks.entry(pc >> 16).or_insert(0usize) += 1;
    }
    println!("frame {frames}: {} opcode fetches", pcs.len());
    for (b, n) in banks {
        println!("  bank ${b:02X}: {n}");
    }
    for a in find {
        println!("  ${a:06X}: {} hits", pcs.iter().filter(|&&p| p == a).count());
    }
    if args.iter().any(|a| a == "--calls") {
        // Call targets: a PC whose previous opcode was JSR/JSL/JSR (a,x)/JSR (a),
        // in first-seen order with counts and the first caller.
        let mut seen: Vec<(u32, u32, usize)> = Vec::new();
        for w in pcs.windows(2) {
            let op = g.rom_read(w[0]);
            if matches!(op, 0x20 | 0x22 | 0xFC) {
                match seen.iter_mut().find(|(t, _, _)| *t == w[1]) {
                    Some(e) => e.2 += 1,
                    None => seen.push((w[1], w[0], 1)),
                }
            }
        }
        println!("calls ({}):", seen.len());
        for (t, from, n) in seen {
            println!("  ${t:06X} x{n:<5} first from ${from:06X}");
        }
    }
    0
}

/// `cooptrace --frame F [script]`: both players run frame F in co-op; print the
/// first PC where P2's isolated player update diverges from P1's update.
fn cooptrace(args: &[String]) -> i32 {
    let frame = num(args, "--frame").unwrap_or_else(|| die("cooptrace needs --frame F"));
    let script = scripted(args);
    let mut g = game_scripted(args);
    g.coop.enabled = true;
    mmx_core::coop::register_coop_traps(&mut g);
    for f in 1..frame {
        g.step(script(f, Pads::default()));
    }
    g.isolated_trace = Some(Vec::new());
    let pcs = g.trace_frame(script(frame, Pads::default()));
    let p2 = g.isolated_trace.take().unwrap();
    let start = pcs.iter().position(|&p| p == mmx_core::coop::PLAYER_UPDATE).unwrap_or_else(|| die("P1 update not found in frame"));
    let end = start + pcs[start..].iter().position(|&p| p == mmx_core::coop::HOOK_UPDATE).unwrap_or_else(|| die("P1 update return not found"));
    let p1 = &pcs[start..end];
    // P2's trace holds the remap call then the update; skip to the update entry.
    let p2s = p2.iter().position(|&p| p == mmx_core::coop::PLAYER_UPDATE).unwrap_or(0);
    let p2 = &p2[p2s..];
    println!("frame {frame}: P1 update {} instrs, P2 update {} instrs", p1.len(), p2.len());
    // Walk both traces; at each split, report the last common PC (the branch)
    // and resynchronise on the nearest later common PC.
    let op = |pc: u32| (0..4).map(|d| format!("{:02X}", g.rom_read(pc + d))).collect::<Vec<_>>().join(" ");
    let (mut i, mut j, mut n) = (0usize, 0usize, 0usize);
    while i < p1.len() && j < p2.len() && n < 25 {
        if p1[i] == p2[j] {
            i += 1;
            j += 1;
            continue;
        }
        let last = if i > 0 { p1[i - 1] } else { 0 };
        n += 1;
        println!("split {n:2} after ${last:06X} [{}]: P1 -> ${:06X}, P2 -> ${:06X}", op(last), p1[i], p2[j]);
        // resync: smallest (a+b) with p1[i+a] == p2[j+b], window 400
        let mut best: Option<(usize, usize)> = None;
        for a in 0..400.min(p1.len() - i) {
            for b in 0..400.min(p2.len() - j) {
                if best.is_some_and(|(x, y)| a + b >= x + y) {
                    break;
                }
                if p1[i + a] == p2[j + b] {
                    best = Some((a, b));
                    break;
                }
            }
        }
        match best {
            Some((a, b)) => {
                println!("          P1 ran {a} instrs, P2 ran {b} instrs before rejoining at ${:06X}", p1[i + a]);
                i += a;
                j += b;
            }
            None => {
                println!("          no rejoin within 400 instructions");
                break;
            }
        }
    }
    if n == 0 {
        println!("identical");
    }
    0
}

fn write_png(path: &Path, px: &[u32], w: usize, h: usize) -> Result<(), Box<dyn std::error::Error>> {
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    enc.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

/// `(ported, verified, total)` from ports.toml text. `verified` counts as
/// ported too.
// ponytail: line scan instead of a toml parser; the file is hand-kept and flat.
fn ledger_counts(text: &str) -> (usize, usize, usize) {
    let total = text.lines().filter(|l| l.trim() == "[[routine]]").count();
    let status = |s: &str| text.lines().filter(|l| l.contains(&format!("status = \"{s}\""))).count();
    let verified = status("verified");
    (status("ported") + verified, verified, total)
}

fn ledger() -> i32 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ports.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
    let (ported, verified, total) = ledger_counts(&text);
    println!("routines: {ported}/{total} ported, {verified}/{total} verified");
    0
}

fn rominfo() -> i32 {
    let g = game(rom());
    let i = g.rom_info();
    let hex: String = i.sha256.iter().map(|b| format!("{b:02x}")).collect();
    let ver = match i.version {
        0 => "v1.0 (matches disassembly)",
        1 => "v1.1 (addresses unchecked)",
        _ => "unknown",
    };
    println!("title   {}\nversion {} {ver}\nsha256  {hex}\ncrc32   {:08x}\ncopier header stripped: {}", i.title, i.version, i.crc32, i.had_copier_header);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_counts_statuses() {
        let t = "[meta]\n[[routine]]\nstatus = \"ported\"\n[[routine]]\nstatus = \"verified\"\n[[routine]]\nstatus = \"unported\"\n";
        assert_eq!(ledger_counts(t), (2, 1, 3));
    }

    #[test]
    fn real_ports_toml_parses() {
        let t = include_str!("../../../ports.toml");
        let (_, _, total) = ledger_counts(t);
        assert!(total >= 20, "{total}");
    }

    #[test]
    fn flags() {
        let a: Vec<String> = ["--frames", "60", "--no-frame"].map(String::from).to_vec();
        assert_eq!(num(&a, "--frames"), Some(60));
        assert_eq!(flag(&a, "--out"), None);
    }
}
