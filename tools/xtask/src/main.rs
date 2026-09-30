//! `cargo xtask <verify|dump|ledger|rominfo>`. Plain `std::env::args`.
//! All fs lives here, never in the libs.

use std::path::{Path, PathBuf};
use std::process::exit;

use mmx_core::{Game, Pads};
use mmx_verify::{movie_bk2, CompareOpts, Lockstep};

const USAGE: &str = "usage: cargo xtask <command>
  verify [--movie F.bk2|F.lsmv] [--frames N] [--no-frame]
  dump --frames N --out DIR [--at F,F,...] [--movie F]
  ledger
  rominfo
env: MMX_ROM=path/to/Mega Man X (USA).sfc";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("verify") => verify(&args[1..]),
        Some("dump") => dump(&args[1..]),
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

fn verify(args: &[String]) -> i32 {
    let inputs = movie(args);
    let frames = num(args, "--frames")
        .or((!inputs.is_empty()).then_some(inputs.len()))
        .unwrap_or_else(|| die("verify needs --frames N or --movie F"));
    let rom = rom();
    let mut oracle = game(rom.clone());
    oracle.traps.set_enabled(false);
    // TODO: register the default trap set on the DUT once mmx-core has one
    // (mmx_core::register_default_traps(&mut dut)). Until then DUT == oracle.
    let mut dut = game(rom);
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
    let mut g = game(rom());
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    for f in 1..=frames {
        g.step(inputs.get(f - 1).copied().unwrap_or_default());
        if at.contains(&f) {
            let path = out.join(format!("frame_{f:06}.png"));
            let (px, w, h) = g.frame();
            write_png(&path, px, w, h).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
            println!("{}", path.display());
        }
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
