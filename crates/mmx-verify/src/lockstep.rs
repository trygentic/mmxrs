//! Lockstep: step oracle and DUT with the same pads, diff every frame, stop
//! at the first difference. Compare order (cheapest / most telling first):
//! WRAM `$7E:0000-$1FFF` (direct page, stack, objects), rest of WRAM, OAM,
//! CGRAM, framebuffer.

use std::fmt;

use mmx_core::{Game, Pads};

/// Anything lockstep can drive. `Game` implements it; tests use fakes.
pub trait Port {
    fn step(&mut self, pads: Pads);
    fn wram(&self) -> &[u8; 0x20000];
    fn oam(&self) -> [u8; 544];
    fn cgram(&self) -> [u16; 256];
    fn frame(&self) -> (&[u32], usize, usize);
    /// Name of the last trap that fired, for the divergence report.
    fn last_trap(&self) -> Option<String> {
        None
    }
}

impl Port for Game {
    fn step(&mut self, pads: Pads) {
        Game::step(self, pads)
    }
    fn wram(&self) -> &[u8; 0x20000] {
        Game::wram(self)
    }
    fn oam(&self) -> [u8; 544] {
        Game::oam(self)
    }
    fn cgram(&self) -> [u16; 256] {
        Game::cgram(self)
    }
    fn frame(&self) -> (&[u32], usize, usize) {
        Game::frame(self)
    }
    fn last_trap(&self) -> Option<String> {
        self.traplog
            .last()
            .map(|r| format!("{} ${:02X}:{:04X} @frame {}", r.name, r.addr >> 16, r.addr & 0xFFFF, r.frame))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Wram,
    Oam,
    Cgram,
    Frame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// 0-based index of the input frame whose step produced the difference.
    pub frame: usize,
    pub region: Region,
    /// WRAM: 24-bit `$7E:xxxx`/`$7F:xxxx`. OAM/CGRAM/Frame: index (byte, word,
    /// pixel). Frame size mismatch: `u32::MAX`, values are `w << 16 | h`.
    pub addr: u32,
    pub expected: u32,
    pub actual: u32,
    pub last_trap: Option<String>,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (e, a) = (self.expected, self.actual);
        write!(f, "DIVERGENCE frame={} ", self.frame)?;
        match self.region {
            Region::Wram => write!(
                f,
                "region=WRAM addr=${:02X}:{:04X} expected=${e:02X} actual=${a:02X}",
                self.addr >> 16,
                self.addr & 0xFFFF
            )?,
            Region::Oam => write!(f, "region=OAM addr={:#05X} expected=${e:02X} actual=${a:02X}", self.addr)?,
            Region::Cgram => write!(f, "region=CGRAM addr={:#04X} expected=${e:04X} actual=${a:04X}", self.addr)?,
            Region::Frame if self.addr == u32::MAX => write!(
                f,
                "region=FRAME size expected={}x{} actual={}x{}",
                e >> 16,
                e & 0xFFFF,
                a >> 16,
                a & 0xFFFF
            )?,
            Region::Frame => write!(
                f,
                "region=FRAME pixel={} expected=#{e:06X} actual=#{a:06X}",
                self.addr
            )?,
        }
        write!(f, " last_trap={}", self.last_trap.as_deref().unwrap_or("-"))
    }
}

#[derive(Debug, Clone)]
pub struct CompareOpts {
    pub compare_frame: bool,
    /// Half-open 24-bit WRAM ranges `[start, end)` to ignore (known-noisy
    /// bytes). None known yet.
    pub skip_ranges: Vec<(u32, u32)>,
}

impl Default for CompareOpts {
    fn default() -> Self {
        Self { compare_frame: true, skip_ranges: Vec::new() }
    }
}

pub struct Lockstep;

impl Lockstep {
    /// Step both ports `frames` times; `inputs` shorter than `frames` pads
    /// with zero input. Returns the first divergence, if any.
    pub fn run(
        oracle: &mut impl Port,
        dut: &mut impl Port,
        inputs: &[Pads],
        frames: usize,
        opts: CompareOpts,
    ) -> Option<Divergence> {
        for frame in 0..frames {
            let pads = inputs.get(frame).copied().unwrap_or_default();
            oracle.step(pads);
            dut.step(pads);
            if let Some((region, addr, expected, actual)) = compare(oracle, dut, &opts) {
                return Some(Divergence { frame, region, addr, expected, actual, last_trap: dut.last_trap() });
            }
        }
        None
    }
}

type Hit = (Region, u32, u32, u32);

fn first_diff<T: Copy + Into<u32> + PartialEq>(
    exp: &[T],
    act: &[T],
    base: usize,
    skip: impl Fn(usize) -> bool,
) -> Option<(usize, u32, u32)> {
    if exp == act {
        return None; // memcmp fast path
    }
    exp.iter()
        .zip(act)
        .enumerate()
        .map(|(i, (&e, &a))| (base + i, e, a))
        .find(|&(i, e, a)| e != a && !skip(i))
        .map(|(i, e, a)| (i, e.into(), a.into()))
}

fn compare(o: &impl Port, d: &impl Port, opts: &CompareOpts) -> Option<Hit> {
    let (ow, dw) = (o.wram(), d.wram());
    let skip = |i: usize| {
        let addr = 0x7E_0000 + i as u32;
        opts.skip_ranges.iter().any(|&(s, e)| (s..e).contains(&addr))
    };
    let wram = |r: std::ops::Range<usize>| {
        first_diff(&ow[r.clone()], &dw[r.clone()], r.start, skip)
            .map(|(i, e, a)| (Region::Wram, 0x7E_0000 + i as u32, e, a))
    };
    if let Some(h) = wram(0..0x2000).or_else(|| wram(0x2000..0x20000)) {
        return Some(h);
    }
    if let Some((i, e, a)) = first_diff(&o.oam(), &d.oam(), 0, |_| false) {
        return Some((Region::Oam, i as u32, e, a));
    }
    if let Some((i, e, a)) = first_diff(&o.cgram(), &d.cgram(), 0, |_| false) {
        return Some((Region::Cgram, i as u32, e, a));
    }
    if opts.compare_frame {
        let ((of, w0, h0), (df, w1, h1)) = (o.frame(), d.frame());
        if (w0, h0) != (w1, h1) {
            let pack = |w: usize, h: usize| (w as u32) << 16 | h as u32;
            return Some((Region::Frame, u32::MAX, pack(w0, h0), pack(w1, h1)));
        }
        // ponytail: slice == (memcmp) is the fast path instead of FNV; hashing
        // both frames would only be slower than comparing them.
        if let Some((i, e, a)) = first_diff(of, df, 0, |_| false) {
            return Some((Region::Frame, i as u32, e, a));
        }
    }
    None
}

/// FNV-1a 64 over the concatenation of `parts` (desync fingerprints).
pub fn hash_state(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in parts.iter().flat_map(|p| p.iter()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WRAM + counters; `pokes` are `(frame, wram index, value)` applied
    /// after that step (1-based step count).
    struct FakePort {
        wram: Box<[u8; 0x20000]>,
        oam: [u8; 544],
        cgram: [u16; 256],
        fb: Vec<u32>,
        steps: usize,
        seen: Vec<Pads>,
        pokes: Vec<(usize, usize, u8)>,
        trap: Option<&'static str>,
    }

    impl FakePort {
        fn new() -> Self {
            Self {
                wram: Box::new([0; 0x20000]),
                oam: [0; 544],
                cgram: [0; 256],
                fb: vec![0; 256 * 224],
                steps: 0,
                seen: Vec::new(),
                pokes: Vec::new(),
                trap: None,
            }
        }
        fn poke(mut self, frame: usize, idx: usize, v: u8) -> Self {
            self.pokes.push((frame, idx, v));
            self
        }
    }

    impl Port for FakePort {
        fn step(&mut self, pads: Pads) {
            self.steps += 1;
            self.seen.push(pads);
            for &(f, i, v) in &self.pokes {
                if f == self.steps {
                    self.wram[i] = v;
                }
            }
        }
        fn wram(&self) -> &[u8; 0x20000] {
            &self.wram
        }
        fn oam(&self) -> [u8; 544] {
            self.oam
        }
        fn cgram(&self) -> [u16; 256] {
            self.cgram
        }
        fn frame(&self) -> (&[u32], usize, usize) {
            (&self.fb, 256, 224)
        }
        fn last_trap(&self) -> Option<String> {
            self.trap.map(str::to_string)
        }
    }

    #[test]
    fn identical_runs_never_diverge() {
        let (mut o, mut d) = (FakePort::new(), FakePort::new());
        assert_eq!(Lockstep::run(&mut o, &mut d, &[], 100, CompareOpts::default()), None);
        assert_eq!((o.steps, d.steps), (100, 100));
    }

    #[test]
    fn first_divergence_frame_and_address() {
        let mut o = FakePort::new();
        let mut d = FakePort::new().poke(5, 0x0BCF, 7).poke(9, 0x0010, 1);
        d.trap = Some("player_update");
        let div = Lockstep::run(&mut o, &mut d, &[], 20, CompareOpts::default()).unwrap();
        assert_eq!(div.frame, 4); // 5th step
        assert_eq!((div.region, div.addr, div.expected, div.actual), (Region::Wram, 0x7E0BCF, 0, 7));
        assert_eq!(d.steps, 5, "stops at the first divergence");
        let s = div.to_string();
        assert!(s.contains("frame=4") && s.contains("$7E:0BCF") && s.contains("player_update"), "{s}");
    }

    #[test]
    fn low_wram_before_high_wram_before_oam() {
        let mut o = FakePort::new();
        o.oam[3] = 1;
        let mut d = FakePort::new().poke(1, 0x1F000, 2).poke(1, 0x1FFF, 3);
        let div = Lockstep::run(&mut o, &mut d, &[], 1, CompareOpts::default()).unwrap();
        assert_eq!(div.addr, 0x7E1FFF);
        // without the low byte, the $7F bank address wins over OAM
        let mut d = FakePort::new().poke(1, 0x1F000, 2);
        let div = Lockstep::run(&mut FakePort::new(), &mut d, &[], 1, CompareOpts::default()).unwrap();
        assert_eq!(div.addr, 0x7FF000);
        // clean WRAM: OAM reported, then CGRAM, then frame
        let mut d = FakePort::new();
        d.cgram[1] = 0x7FFF;
        d.fb[10] = 0xFF;
        let div = Lockstep::run(&mut o, &mut d, &[], 1, CompareOpts::default()).unwrap();
        assert_eq!((div.region, div.addr), (Region::Oam, 3));
        let div = Lockstep::run(&mut FakePort::new(), &mut d, &[], 1, CompareOpts::default()).unwrap();
        assert_eq!((div.region, div.addr, div.actual), (Region::Cgram, 1, 0x7FFF));
        d.cgram[1] = 0;
        let div = Lockstep::run(&mut FakePort::new(), &mut d, &[], 1, CompareOpts::default()).unwrap();
        assert_eq!((div.region, div.addr), (Region::Frame, 10));
        let opts = CompareOpts { compare_frame: false, ..Default::default() };
        assert_eq!(Lockstep::run(&mut FakePort::new(), &mut d, &[], 1, opts), None);
    }

    #[test]
    fn skip_ranges_hide_noisy_bytes() {
        let mut d = FakePort::new().poke(2, 0x0BA6, 9).poke(3, 0x0BA7, 9).poke(4, 0x0BA8, 9);
        let opts = CompareOpts { skip_ranges: vec![(0x7E0BA6, 0x7E0BA8)], ..Default::default() };
        let div = Lockstep::run(&mut FakePort::new(), &mut d, &[], 10, opts).unwrap();
        assert_eq!((div.frame, div.addr), (3, 0x7E0BA8));
    }

    #[test]
    fn short_input_track_pads_with_zero() {
        let (mut o, mut d) = (FakePort::new(), FakePort::new());
        let inputs = [Pads { p1: mmx_core::BTN_START, p2: 0 }, Pads { p1: 0, p2: mmx_core::BTN_B }];
        assert_eq!(Lockstep::run(&mut o, &mut d, &inputs, 4, CompareOpts::default()), None);
        let mut want = inputs.to_vec();
        want.extend([Pads::default(); 2]);
        assert_eq!(o.seen, want);
        assert_eq!(d.seen, want);
    }

    #[test]
    fn fnv1a_known_vectors() {
        assert_eq!(hash_state(&[]), 0xcbf29ce484222325);
        assert_eq!(hash_state(&[b"a"]), 0xaf63dc4c8601ec8c);
        assert_eq!(hash_state(&[b"fo", b"obar"]), hash_state(&[b"foobar"]));
    }
}
