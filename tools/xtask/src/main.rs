//! `cargo xtask <verify|dump|ledger|rominfo>`. Plain `std::env::args`.
//! All fs lives here, never in the libs.

mod net;

use std::path::{Path, PathBuf};
use std::process::exit;

use mmx_core::{ram, Game, Pads};
use mmx_verify::{movie_bk2, CompareOpts, Lockstep};

const USAGE: &str = "usage: cargo xtask <command>
  verify [--movie F.bk2|F.lsmv] [--frames N] [--no-frame] [--coop-off] [--tap F,..] [--hold HEX] [--p2-hold HEX]
  trace --frames N [--tap F,..] [--hold HEX] [--find HEX,..]   bank histogram of frame N's opcode fetches
  dump --frames N --out DIR [--at F,F,...] [--movie F] [--tap F,F,...] [--hold HEX] [--p2-hold HEX] [--coop]
       [--rom-any PATH] [--x3 PATH] [--poke ADDR=VAL@A[-B],..] [--load STATE] [--save F=PATH,..] [--wram] [--refill] [--to F] [--untrap HEX,..] [--wav PATH] [--apu] [--mute HEX]  start from / write a save state (P2 re-anchors after a load)
       --tap holds Start on pad 1 for 4 frames at each listed frame; --hold/--p2-hold OR a constant pad mask in
       --at A-B/S and --p1tap HEX@A-B/S[:N] take ranges; --vram also writes .vram/.oam/.cgram
  ledger
  rominfo
  zerosheet [--x3 PATH [--wave|--slash|--sfx]] --out DIR   X1 (or X3) Zero's frames (mmx_core::zero_data; --wave: the saber wave, --slash: the slash arc) -> DIR/NN_name.png + DIR/sheet.png; --sfx: his X3 sounds -> DIR/swing.wav
  netsim [--mode rollback|lockstep] [--rtt MS] [--jitter MS] [--loss PCT] [--delay N] [--prediction N] [--frames N]
       [--load STATE] [--x3 PATH] [--zero] [--timing ntsc|pal] [--seed N] [--report F.json] [--record PREFIX] [script]
       two session games over a simulated link (virtual clock); host plays pad 1 of the script, guest pad 2;
       exit 1 on desync or a mismatch with the plain run. --record writes PREFIX-host.log / PREFIX-guest.log
  netreplay LOG [LOG2] [--load STATE] [--x3 PATH] [--rom PATH]   a session log (mmxrs --net-record) against a
       plain replay of its confirmed inputs; with two logs (one per peer) also compare them
env: MMX_ROM=path/to/Mega Man X (USA).sfc, MMX_X3_ROM=path/to/Mega Man X3 (USA).sfc (P2 as X3 Zero; = --x3)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("verify") => verify(&args[1..]),
        Some("dump") => dump(&args[1..]),
        Some("trace") => trace(&args[1..]),
        Some("cooptrace") => cooptrace(&args[1..]),
        Some("ledger") => ledger(),
        Some("rominfo") => rominfo(),
        Some("zerosheet") => zerosheet(&args[1..]),
        Some("netsim") => netsim(&args[1..]),
        Some("netreplay") => netreplay(&args[1..]),
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
/// `--rom-any PATH`: boot that ROM with `Game::from_rom_any` (X3 research) instead of MMX_ROM.
fn game_scripted(args: &[String]) -> Game {
    let mut g = match flag(args, "--rom-any") {
        Some(p) => Game::from_rom_any(std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}"))))
            .unwrap_or_else(|e| die(&format!("ROM rejected: {e}"))),
        None => game(rom()),
    };
    if let Some(p) = flag(args, "--load") {
        let s = std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")));
        g.load_state(&s).unwrap_or_else(|e| die(&format!("{p}: {e}")));
    }
    // `--x3 PATH` (or MMX_X3_ROM): draw P2 as X3 Zero (docs/zero.md "Playing as Zero").
    if let Some(p) = flag(args, "--x3").map(PathBuf::from).or_else(|| std::env::var_os("MMX_X3_ROM").map(PathBuf::from)) {
        let rom = std::fs::read(&p).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
        g.set_x3_rom(&rom).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
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
    let timing = std::env::var("MMX_TIMING").ok().and_then(|v| mmx_core::Timing::parse(&v)).unwrap_or_default();
    Game::from_rom_timing(rom, timing).unwrap_or_else(|e| die(&format!("ROM rejected: {e}")))
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
    let to = num(args, "--to").unwrap_or(usize::MAX);
    // `--p1tap HEX@F,HEX@F:N` / `--p2tap`: hold HEX for 4 (or N) frames from F.
    let taps_for = |k: &str| -> Vec<(u16, usize, usize)> {
        flag(args, k)
            .map(|s| {
                s.split(',')
                    .flat_map(|t| {
                        let (m, f) = t.split_once('@').unwrap_or_else(|| die(&format!("{k}: want HEX@FRAME, got {t}")));
                        let (f, n) = f.split_once(':').unwrap_or((f, "4"));
                        let m = u16::from_str_radix(m.trim().trim_start_matches("0x"), 16).unwrap_or_else(|_| die(k));
                        let n: usize = n.trim().parse().unwrap_or_else(|_| die(k));
                        let p = |v: &str| -> usize { v.trim().parse().unwrap_or_else(|_| die(k)) };
                        // `HEX@A-B/S`: every S frames from A to B.
                        let frames: Vec<usize> = match f.split_once('-') {
                            Some((a, bs)) => {
                                let (b, st) = bs.split_once('/').unwrap_or((bs, "60"));
                                (p(a)..=p(b)).step_by(p(st).max(1)).collect()
                            }
                            None => vec![p(f)],
                        };
                        frames.into_iter().map(move |f| (m, f, n))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (t1, t2) = (taps_for("--p1tap"), taps_for("--p2tap"));
    move |f, mut pads| {
        if f >= from && f < to {
            pads.p1 |= hold1;
            pads.p2 |= hold2;
        }
        for &(m, t, n) in &t1 {
            if (t..t + n).contains(&f) {
                pads.p1 |= m;
            }
        }
        for &(m, t, n) in &t2 {
            if (t..t + n).contains(&f) {
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
    // `--at F,F,..` or `--at A-B/S`.
    let at: Vec<usize> = match flag(args, "--at") {
        Some(s) if s.contains('-') => {
            let (a, bs) = s.split_once('-').unwrap();
            let (b, st) = bs.split_once('/').unwrap_or((bs, "100"));
            let p = |v: &str| -> usize { v.parse().unwrap_or_else(|_| die("--at")) };
            (p(a)..=p(b)).step_by(p(st).max(1)).collect()
        }
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
        // `--untrap HEX,..`: A/B one co-op hook at a time.
        for a in flag(args, "--untrap").map(|s| s.split(',').collect::<Vec<_>>()).unwrap_or_default() {
            g.traps.set_untrapped(u32::from_str_radix(a.trim_start_matches('$'), 16).unwrap_or_else(|_| die("--untrap")), true);
        }
    }
    // `--zero`: P1 plays as X3 Zero (needs `--x3` / MMX_X3_ROM).
    if args.iter().any(|a| a == "--zero") {
        g.set_p1_zero(true).unwrap_or_else(|e| die(&format!("--zero: {e}")));
    }
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    let refill = args.iter().any(|a| a == "--refill");
    // `--poke ADDR=VAL@A[-B],..`: write WRAM byte ADDR (hex, e.g. 7E1FAE) before frames A..=B.
    let pokes: Vec<(usize, u8, usize, usize)> = flag(args, "--poke")
        .map(|s| {
            s.split(',')
                .map(|t| {
                    let bad = || -> ! { die(&format!("--poke: want ADDR=VAL@A[-B], got {t}")) };
                    let (av, fr) = t.split_once('@').unwrap_or_else(|| bad());
                    let (a, v) = av.split_once('=').unwrap_or_else(|| bad());
                    let (fa, fb) = fr.split_once('-').unwrap_or((fr, fr));
                    let h = |x: &str| u32::from_str_radix(x.trim(), 16).unwrap_or_else(|_| bad());
                    let d = |x: &str| x.trim().parse::<usize>().unwrap_or_else(|_| bad());
                    (ram::wram_index(h(a)), h(v) as u8, d(fa), d(fb))
                })
                .collect()
        })
        .unwrap_or_default();
    // `--wav PATH`: the run's audio (`take_audio`, 32 kHz) as 16-bit stereo WAV.
    // `--apu`: per frame, the DSP's sounding voices; SPC RAM at each `--at` frame (`.spc`).
    let apu = args.iter().any(|a| a == "--apu");
    // `--mute HEX`: research, bit n silences DSP voice n in the output.
    if let Some(m) = flag(args, "--mute") {
        Game::set_voice_mute(u8::from_str_radix(m, 16).unwrap_or_else(|_| die("--mute HEX")));
    }
    let mut wav = Vec::new();
    g.set_audio_rate(32_000);
    for f in 1..=frames {
        for &(a, v, fa, fb) in &pokes {
            if (fa..=fb).contains(&f) {
                g.wram_mut()[a] = v;
            }
        }
        if refill {
            // QA cheat: keep a living X at max HP (pits still kill).
            let w = g.wram_mut();
            let (hp, max) = (ram::wram_index(ram::X_HP), w[ram::wram_index(ram::MAX_HP)] & 0x7F);
            if (1..max).contains(&(w[hp] & 0x7F)) {
                w[hp] = max;
            }
        }
        g.step(script(f, inputs.get(f - 1).copied().unwrap_or_default()));
        wav.extend(g.take_audio());
        if apu {
            // Sounding voices: SRCN, PITCH, ENVX, ADSR/GAIN, VOL, and KON/KOFF/ENDX.
            let r = |a: u8| g.dsp_register(a);
            let v: Vec<String> = (0..8u8)
                .map(|k| k << 4)
                .filter(|&b| r(b | 8) != 0)
                .map(|b| format!("v{} src {:02X} p {:04X} env {:02X} adsr {:02X}{:02X} gain {:02X} vol {}/{}", b >> 4, r(b | 4), u16::from_le_bytes([r(b | 2), r(b | 3)]), r(b | 8), r(b | 5), r(b | 6), r(b | 7), r(b) as i8, r(b | 1) as i8))
                .collect();
            println!("apu {f}: kon {:02X} koff {:02X} endx {:02X} dir {:02X} | {}", r(0x4C), r(0x5C), r(0x7C), r(0x5D), v.join(" | "));
        }
        for (_, p) in saves.iter().filter(|(sf, _)| *sf == f) {
            std::fs::write(p, g.save_state()).unwrap_or_else(|e| die(&format!("{p}: {e}")));
            println!("saved state after frame {f}: {p}");
        }
        if at.contains(&f) {
            let path = out.join(format!("frame_{f:06}.png"));
            let (px, w, h) = g.frame();
            write_png(&path, px, w, h).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
            let w = g.wram();
            if args.iter().any(|a| a == "--wram") {
                let _ = std::fs::write(out.join(format!("frame_{f:06}.wram")), &w[..]);
            }
            if apu {
                let _ = std::fs::write(out.join(format!("frame_{f:06}.spc")), g.spc_ram());
            }
            if args.iter().any(|a| a == "--vram") {
                // VRAM (64 KiB, little-endian words), OAM (544 B), CGRAM (512 B LE).
                let v: Vec<u8> = g.vram().iter().flat_map(|w| w.to_le_bytes()).collect();
                let c: Vec<u8> = g.cgram().iter().flat_map(|w| w.to_le_bytes()).collect();
                let _ = std::fs::write(out.join(format!("frame_{f:06}.vram")), v);
                let _ = std::fs::write(out.join(format!("frame_{f:06}.oam")), g.oam());
                let _ = std::fs::write(out.join(format!("frame_{f:06}.cgram")), c);
            }
            let b = |a: u32| w[mmx_core::ram::wram_index(a)];
            let w16 = |a: u32| u16::from_le_bytes([b(a), b(a + 1)]);
            println!(
                "{} mode {:02X}/{:02X}/{:02X}/{:02X} level {:02X} X=({},{}) act {:02X} vy {} hp {} lives {} cam ({},{}) pause {:02X} pad1 {:04X} xin {:02X}{:02X} coop_hidden {} last_trap {:?}",
                path.display(), b(ram::MODE_D1), b(ram::MODE_D2), b(ram::STAGE_PHASE), b(ram::MODE_D4), b(ram::LEVEL_ID),
                w16(ram::X_POS_X), w16(ram::X_POS_Y), b(ram::X_ACTION), w16(ram::X_VEL_Y) as i16, b(ram::X_HP) & 0x7F, b(ram::LIVES), w16(ram::CAMERA_X), w16(ram::CAMERA_Y), b(ram::PAUSE), w16(ram::PAD1_HELD), b(ram::X_INPUT_HELD), b(ram::X_INPUT_HELD + 1), g.coop.p2_hidden,
                g.traplog.last().map(|t| (t.name, t.frame))
            );
            if g.p1_zero() {
                let e = |k: usize| w[ram::wram_index(ram::ENEMIES_BASE) + k * ram::ENEMY_STRIDE + ram::ENEMY_HP] & 0x7F;
                println!("    P1 zero zf {} saber t {} hit {} enemy hp {:?}", g.coop.p1.frame as i16, g.coop.p1.saber.t, g.coop.p1.saber.hit, (0..4).map(e).collect::<Vec<_>>());
            }
            if g.coop.enabled {
                let p2 = &g.coop.p2;
                let p16 = |o: usize| u16::from_le_bytes([p2[o], p2[o + 1]]);
                println!(
                    "    P2 pos=({},{}) act {:02X} vy {} hp {} in {:02X}{:02X} f2B {:02X} f2C {:02X} (P1 f2B {:02X} f2C {:02X}) vis {:02X} oam base {} count {} zf {} hook D={:04X} DB={:02X} P={:02X}",
                    p16(0x05), p16(0x08), p2[0x02], p16(0x1C) as i16, p2[0x27] & 0x7F, p2[0x36], p2[0x37], p2[0x2B], p2[0x2C], b(ram::X_BASE + 0x2B), b(ram::X_BASE + 0x2C), p2[0x0E], g.coop.p2_oam_base, g.coop.p2_oam_count,
                    g.coop.zero_frame as i16, g.coop.dbg_regs.0, g.coop.dbg_regs.1, g.coop.dbg_regs.2
                );
                if g.coop.down_frames > 0 {
                    println!("    P2 down {} spot {:?} touch {}", g.coop.down_frames, g.coop.revive.spot, g.coop.revive.touch);
                }
            }
        }
    }
    if let Some(p) = flag(args, "--wav") {
        write_wav(Path::new(p), &wav, 32_000).unwrap_or_else(|e| die(&format!("{p}: {e}")));
        println!("wrote {p} ({} samples)", wav.len());
    }
    0
}

/// 16-bit stereo PCM WAV.
fn write_wav(path: &Path, s: &[(f32, f32)], rate: u32) -> std::io::Result<()> {
    let n = s.len() as u32 * 4;
    let mut v = Vec::with_capacity(44 + n as usize);
    v.extend(b"RIFF");
    v.extend((36 + n).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend([1u16, 2].iter().flat_map(|x| x.to_le_bytes()));
    v.extend(rate.to_le_bytes());
    v.extend((rate * 4).to_le_bytes());
    v.extend([4u16, 16].iter().flat_map(|x| x.to_le_bytes()));
    v.extend(b"data");
    v.extend(n.to_le_bytes());
    for &(l, r) in s {
        for x in [l, r] {
            v.extend(((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(path, v)
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

/// `zerosheet [--x3 PATH] --out DIR`: every frame of `zero_data::extract_x1`
/// (64x64 cell, origin 32,44) or `extract_x3` (96x88, origin 48,44), one PNG
/// each, plus a contact sheet (8 per row, 2x, index in the corner).
fn zerosheet(args: &[String]) -> i32 {
    use mmx_core::zero_data;
    // 3x5 digits for the index.
    const DIG: [u16; 10] = [0x7B6F, 0x2C97, 0x73E7, 0x73CF, 0x5BC9, 0x79CF, 0x79EF, 0x7249, 0x7BEF, 0x7BCF];
    let out = PathBuf::from(flag(args, "--out").unwrap_or_else(|| die("zerosheet needs --out DIR")));
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    let (z, (cw, ch, ox, oy)) = match flag(args, "--x3") {
        Some(p) => {
            let mut b = std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")));
            if b.len() % 0x8000 == 512 {
                b.drain(..512);
            }
            (zero_data::extract_x3(&b).unwrap_or_else(|e| die(&e)), (96usize, 88usize, 48i32, 44i32))
        }
        None => (zero_data::extract_x1(game(rom()).rom_bytes()).unwrap_or_else(|e| die(&e)), (64, 64, 32, 44)),
    };
    // `--sfx` (with `--x3`): Zero's X3 sounds as DIR/NAME.wav instead.
    if args.iter().any(|a| a == "--sfx") {
        let s = &z.sounds.as_ref().unwrap_or_else(|| die("--sfx needs --x3")).swing;
        let pcm: Vec<(f32, f32)> = s.pcm.iter().map(|&x| (f32::from(x) / 32768.0, f32::from(x) / 32768.0)).collect();
        let p = out.join("swing.wav");
        write_wav(&p, &pcm, s.rate).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
        println!("wrote {} ({} samples at {} Hz)", p.display(), pcm.len(), s.rate);
        return 0;
    }
    // `--wave` (with `--x3`): the saber wave's frames instead, in its own palette.
    let fx = if args.iter().any(|a| a == "--wave") {
        Some(z.wave.clone())
    } else if args.iter().any(|a| a == "--slash") {
        z.slash.clone()
    } else {
        None
    };
    let z = if let Some(w) = fx {
        zero_data::ZeroSet { tiles: w.tiles, palette: w.palette, frames: w.frames, ..Default::default() }
    } else {
        z
    };
    let rgb = |c: u16| -> u32 {
        let ch = |v: u16| u32::from(v & 31) << 3 | u32::from(v & 31) >> 2;
        ch(c) << 16 | ch(c >> 5) << 8 | ch(c >> 10)
    };
    let cells: Vec<Vec<u32>> = z
        .frames
        .iter()
        .map(|f| {
            let mut px = vec![0x303048u32; cw * ch];
            // Last OAM entry first, so the first ends on top.
            for s in f.sprites.iter().rev() {
                let n = if s.big { 16 } else { 8 };
                for yy in 0..n {
                    for xx in 0..n {
                        let sx = if s.hflip { n - 1 - xx } else { xx };
                        let sy = if s.vflip { n - 1 - yy } else { yy };
                        let slot = usize::from(s.tile) + sx / 8 + 16 * (sy / 8);
                        let t = f.tiles_used.get(slot).copied().unwrap_or(zero_data::NO_TILE);
                        if t == zero_data::NO_TILE {
                            continue;
                        }
                        let b = &z.tiles[usize::from(t) * 32..][..32];
                        let (r, bit) = (sy % 8, 7 - sx % 8);
                        let v = (b[2 * r] >> bit & 1)
                            | (b[2 * r + 1] >> bit & 1) << 1
                            | (b[16 + 2 * r] >> bit & 1) << 2
                            | (b[17 + 2 * r] >> bit & 1) << 3;
                        let (x, y) = (ox + i32::from(s.x) + xx as i32, oy + i32::from(s.y) + yy as i32);
                        if v != 0 && (0..cw as i32).contains(&x) && (0..ch as i32).contains(&y) {
                            px[y as usize * cw + x as usize] = rgb(z.palette[usize::from(v)]);
                        }
                    }
                }
            }
            px
        })
        .collect();
    for (i, (f, px)) in z.frames.iter().zip(&cells).enumerate() {
        let path = out.join(format!("{i:02}_{}.png", f.name.replace(' ', "_")));
        write_png(&path, px, cw, ch).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
    }
    let cols = 8;
    let (w, h) = (cols * cw * 2, cells.len().div_ceil(cols) * ch * 2);
    let mut sheet = vec![0u32; w * h];
    for (i, px) in cells.iter().enumerate() {
        let (sx, sy) = ((i % cols) * cw * 2, (i / cols) * ch * 2);
        let mut cell = px.clone();
        for (k, ch) in i.to_string().bytes().enumerate() {
            let d = DIG[usize::from(ch - b'0')];
            for b in 0..15 {
                if d >> (14 - b) & 1 != 0 {
                    cell[(1 + b / 3) * cw + 1 + 4 * k + b % 3] = 0xFFFF00;
                }
            }
        }
        for y in 1..ch * 2 {
            for x in 1..cw * 2 {
                sheet[(sy + y) * w + sx + x] = cell[(y / 2) * cw + x / 2];
            }
        }
    }
    let path = out.join("sheet.png");
    write_png(&path, &sheet, w, h).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())));
    println!("{} frames, {} tiles -> {}", z.frames.len(), z.tiles.len() / 32, path.display());
    for (i, f) in z.frames.iter().enumerate() {
        println!("  {i:2} {:14} {:2} sprites", f.name, f.sprites.len());
    }
    0
}

/// `--x3 PATH` or `$MMX_X3_ROM`: the X3 Zero set.
fn x3_set(args: &[String]) -> Option<mmx_core::zero_data::ZeroSet> {
    let p = flag(args, "--x3").map(PathBuf::from).or_else(|| std::env::var_os("MMX_X3_ROM").filter(|v| !v.is_empty()).map(PathBuf::from))?;
    let b = std::fs::read(&p).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    let b = if b.len() % 0x8000 == 512 { &b[512..] } else { &b[..] };
    Some(mmx_core::zero_data::extract_x3(b).unwrap_or_else(|e| die(&format!("{}: {e}", p.display()))))
}

fn netsim(args: &[String]) -> i32 {
    use mmx_core::netgame::Mode;
    let mode = flag(args, "--mode").map_or(Mode::Rollback, |m| Mode::parse(m).unwrap_or_else(|| die("--mode: rollback or lockstep")));
    let mut o = net::SimOpts::new(mode);
    let n = |k: &str, d: u32| num(args, k).map_or(d, |v| v as u32);
    o.delay = n("--delay", u32::from(o.delay)) as u8;
    o.prediction = n("--prediction", u32::from(o.prediction)) as u8;
    o.rtt_ms = n("--rtt", o.rtt_ms);
    o.jitter_ms = n("--jitter", o.jitter_ms);
    o.frames = n("--frames", o.frames);
    o.seed = u64::from(n("--seed", 1));
    o.loss_pct = flag(args, "--loss").map_or(o.loss_pct, |v| v.parse().unwrap_or_else(|_| die("--loss: percent")));
    o.timing = flag(args, "--timing")
        .map(String::from)
        .or_else(|| std::env::var("MMX_TIMING").ok())
        .map_or(mmx_core::Timing::Auto, |t| mmx_core::Timing::parse(&t).unwrap_or_else(|| die("--timing: auto, ntsc or pal")));
    o.x3 = x3_set(args);
    o.zero = args.iter().any(|a| a == "--zero");
    o.load = flag(args, "--load").map(|p| (p.to_string(), std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}")))));
    let record = flag(args, "--record");
    o.record = record.is_some();
    let script = scripted(args);
    let r = net::netsim(&rom(), &o, &script).unwrap_or_else(|e| die(&e));
    print!("{}", net::sim_text(&r));
    if let Some(p) = flag(args, "--report") {
        std::fs::write(p, net::sim_json(&r)).unwrap_or_else(|e| die(&format!("{p}: {e}")));
    }
    if let Some(prefix) = record {
        for p in &r.peers {
            let path = format!("{prefix}-{}.log", p.role);
            std::fs::write(&path, p.log.as_deref().unwrap_or_default()).unwrap_or_else(|e| die(&format!("{path}: {e}")));
            println!("log: {path}");
        }
    }
    i32::from(!r.ok())
}

fn netreplay(args: &[String]) -> i32 {
    let mut logs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if matches!(args[i].as_str(), "--load" | "--x3" | "--rom") {
            i += 2;
            continue;
        }
        logs.push(net::parse_log(&args[i]).unwrap_or_else(|e| die(&e)));
        i += 1;
    }
    if logs.is_empty() || logs.len() > 2 {
        die("netreplay LOG [LOG2] [--load STATE] [--x3 PATH] [--rom PATH]");
    }
    let rom = match flag(args, "--rom") {
        Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}"))),
        None => rom(),
    };
    let x3 = x3_set(args);
    let load = flag(args, "--load").map(|p| std::fs::read(p).unwrap_or_else(|e| die(&format!("{p}: {e}"))));
    let mut bad = false;
    for l in &logs {
        print!("{}", net::summary(l));
        match net::replay(&rom, l, x3.as_ref(), load.as_deref()) {
            Ok(r) => {
                bad |= r.first_bad.is_some();
                println!("{}", net::verdict(l, &r));
            }
            Err(e) => {
                bad = true;
                println!("replay: {e}");
            }
        }
    }
    if let [a, b] = &logs[..] {
        let c = net::compare(a, b);
        bad |= c.contains("differ");
        print!("{c}");
    }
    i32::from(bad)
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
    println!(
        "title   {}\nregion  {} (${:02X}; MMX_TIMING=ntsc|pal overrides)\nversion {} {ver}\nsha256  {hex}\ncrc32   {:08x}\ncopier header stripped: {}",
        i.title, i.region_name(), i.region, i.version, i.crc32, i.had_copier_header
    );
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
