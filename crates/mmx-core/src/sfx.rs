//! Zero's X3 sounds (docs/zero.md "Sounds"): mixed by `Game::step` into the
//! frame's audio after the emulator's own; X1's sound driver is untouched.
//! State is `CoopState::sfx` (rollback carries it): a replayed frame mixes
//! the same, and the netplay path drops replayed audio anyway.

use crate::zero_data::Sample;

/// `CoopState::sfx` slots.
pub const P2: usize = 0;
pub const P1: usize = 1;

/// Mix frame `age - 1` of `s` into `out` (this frame's output samples, any
/// rate) for every playing slot and advance it; a slot past the end goes
/// idle (0). `fps` paces the sound in wall time (PAL plays it as long).
pub fn mix(ages: &mut [u16; 2], s: &Sample, fps: f64, out: &mut [(f32, f32)]) {
    let per_frame = f64::from(s.rate) / fps;
    for a in ages.iter_mut().filter(|a| **a > 0) {
        let t0 = f64::from(*a - 1) * per_frame;
        if t0 as usize >= s.pcm.len() {
            *a = 0;
            continue;
        }
        let n = out.len() as f64;
        for (j, o) in out.iter_mut().enumerate() {
            let t = t0 + per_frame * j as f64 / n;
            let (i, f) = (t as usize, t.fract() as f32);
            let at = |k: usize| f32::from(s.pcm.get(k).copied().unwrap_or(0)) / 32768.0;
            let x = at(i) + (at(i + 1) - at(i)) * f;
            *o = ((o.0 + x).clamp(-1.0, 1.0), (o.1 + x).clamp(-1.0, 1.0));
        }
        *a += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfx_mix_paces_and_ends() {
        let s = Sample { pcm: vec![16384; 1000], rate: 32_000 };
        let mut ages = [1, 0];
        let mut out = vec![(0.25, -0.25); 800];
        mix(&mut ages, &s, 60.0, &mut out);
        assert_eq!(ages, [2, 0]);
        assert_eq!(out[0], (0.75, 0.25));
        // 533.3 samples a frame: frame 1 still has some, frame 2 starts past the end.
        mix(&mut ages, &s, 60.0, &mut out);
        assert_eq!(ages, [3, 0]);
        let mut quiet = vec![(0.0, 0.0); 800];
        mix(&mut ages, &s, 60.0, &mut quiet);
        assert_eq!(ages, [0, 0]);
        assert!(quiet.iter().all(|&o| o == (0.0, 0.0)));
    }

    /// `hw-spiky`, P1 as Zero swings (X at frame 28): the whoosh is in the
    /// game's own output from the arc's first frame, found by cross-
    /// correlating the output with the sample; a run without the sounds
    /// differs from it by exactly the whoosh. Needs both ROMs and the state.
    #[test]
    fn sfx_whoosh_in_game_output() {
        let (Some(x1), Some(x3)) = (std::env::var_os("MMX_ROM"), std::env::var_os("MMX_X3_ROM")) else {
            eprintln!("MMX_ROM / MMX_X3_ROM not set; skipping");
            return;
        };
        let st = concat!(env!("CARGO_MANIFEST_DIR"), "/../../states/hw-spiky.state");
        let Ok(state) = std::fs::read(st) else {
            eprintln!("{st} missing; skipping");
            return;
        };
        let run = |sounds: bool| {
            let mut g = crate::Game::from_rom(std::fs::read(&x1).unwrap()).unwrap();
            g.set_x3_rom(&std::fs::read(&x3).unwrap()).unwrap();
            if !sounds {
                let mut z = g.zero_set().unwrap().clone();
                z.sounds = None;
                g.set_zero_set(Some(z));
            }
            g.load_state(&state).unwrap();
            crate::saber::tests::give_arm(&mut g);
            g.set_audio_rate(32_000); // after the load: the state carries the rate
            g.set_p1_zero(true).unwrap();
            let (mut pcm, mut starts, mut arc) = (Vec::new(), Vec::new(), None);
            for f in 1..=90 {
                let p1 = match f {
                    ..=26 => crate::BTN_RIGHT,
                    28 | 29 => crate::BTN_X,
                    _ => 0,
                };
                g.step(crate::Pads { p1, p2: 0 });
                starts.push(pcm.len());
                pcm.extend(g.take_audio().iter().map(|&(l, r)| (l + r) / 2.0));
                if arc.is_none() && g.coop.p1.frame == 88 {
                    arc = Some(f - 1);
                }
            }
            (pcm, starts, arc.expect("arc pose"), g.zero_set().and_then(|z| z.sounds.clone()), g.fps())
        };
        let (with, starts, arc, snd, fps) = run(true);
        let (without, _, _, _, _) = run(false);
        let snd = snd.unwrap();
        let snd_ref = &snd.swing;
        let w: Vec<f32> = snd.swing.pcm.iter().map(|&x| f32::from(x) / 32768.0).collect();
        let w = &w[..8000]; // the first quarter second: the sweep up
        let corr = |x: &[f32]| {
            let (d, n) = (x.iter().zip(w).map(|(a, b)| a * b).sum::<f32>(), x.iter().map(|a| a * a).sum::<f32>());
            d / (n.sqrt() * w.iter().map(|a| a * a).sum::<f32>().sqrt() + 1e-9)
        };
        let (lag, peak) = (0..with.len() - w.len()).map(|l| (l, corr(&with[l..l + w.len()]))).fold((0, 0.0), |b, c| if c.1 > b.1 { c } else { b });
        let frame = starts.iter().rposition(|&s| s <= lag).unwrap();
        assert!(peak > 0.5, "peak {peak} at frame {frame}"); // 0.69: X1's own audio is the rest
        assert_eq!(frame, arc, "whoosh found at sample {lag}");
        let diff: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
        assert!(diff[..starts[arc]].iter().all(|&d| d == 0.0), "nothing before the arc");
        // The whoosh as the mixer lays it onto those frames (639.9 source samples
        // per PAL frame stretched onto each frame's 639-641 output samples): the
        // difference must be exactly that. (Was: corr with the raw sample > 0.95,
        // which depends on where the resampler's 639/641-sample frames fall.)
        let mut expect = vec![(0.0f32, 0.0f32); with.len() - starts[arc]];
        let (mut ages, mut at) = ([1u16, 0], 0);
        for f in arc..starts.len() {
            let end = starts.get(f + 1).copied().unwrap_or(with.len()) - starts[arc];
            mix(&mut ages, snd_ref, fps, &mut expect[at..end]);
            at = end;
        }
        let expect: Vec<f32> = expect.iter().map(|&(l, r)| (l + r) / 2.0).collect();
        let d = &diff[starts[arc]..];
        let err = d.iter().zip(&expect).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-4, "difference is not the mixed whoosh: max error {err}");
        // And it is the sample itself, to within the per-frame stretch.
        assert!(corr(&diff[starts[arc]..starts[arc] + w.len()]) > 0.9);
    }
}
