//! cpal output: a shared stereo ring the callback drains. The game renders at
//! the device rate (`Game::set_audio_rate`), so there is no resampler here.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub type Ring = Arc<Mutex<VecDeque<(f32, f32)>>>;

pub struct Audio {
    pub ring: Ring,
    pub rate: u32,
    /// Samples pushed / dropped at the cap (`MMX_AUDIO_STATS=1` prints them).
    pub pushed: std::cell::Cell<u64>,
    pub dropped: std::cell::Cell<u64>,
    _stream: cpal::Stream,
}

impl Audio {
    /// Default output device, default config. `None` if there isn't one.
    pub fn open() -> Option<Audio> {
        let dev = cpal::default_host().default_output_device()?;
        let cfg = dev.default_output_config().ok()?;
        let ring: Ring = Arc::default();
        let fmt = cfg.sample_format();
        let cfg: cpal::StreamConfig = cfg.into();
        let stream = match fmt {
            SampleFormat::F32 => build::<f32>(&dev, &cfg, ring.clone()),
            SampleFormat::I16 => build::<i16>(&dev, &cfg, ring.clone()),
            SampleFormat::U16 => build::<u16>(&dev, &cfg, ring.clone()),
            other => {
                eprintln!("audio: unsupported sample format {other:?}");
                return None;
            }
        }
        .map_err(|e| eprintln!("audio: {e}"))
        .ok()?;
        stream.play().map_err(|e| eprintln!("audio: {e}")).ok()?;
        Some(Audio {
            ring,
            rate: cfg.sample_rate.0,
            pushed: Default::default(),
            dropped: Default::default(),
            _stream: stream,
        })
    }

    pub fn depth(&self) -> usize {
        self.ring.lock().unwrap().len()
    }

    pub fn push(&self, s: &[(f32, f32)]) {
        let mut r = self.ring.lock().unwrap();
        r.extend(s);
        self.pushed.set(self.pushed.get() + s.len() as u64);
        // ponytail: hard cap at 200 ms instead of a drift controller.
        let cap = self.rate as usize / 5;
        if r.len() > cap {
            let n = r.len() - cap;
            r.drain(..n);
            self.dropped.set(self.dropped.get() + n as u64);
        }
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    cfg: &cpal::StreamConfig,
    ring: Ring,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let ch = usize::from(cfg.channels).max(1);
    dev.build_output_stream(
        cfg,
        move |out: &mut [T], _| {
            let mut r = ring.lock().unwrap();
            for frame in out.chunks_mut(ch) {
                // underrun: silence (ponytail: no last-sample hold)
                let (l, rr) = r.pop_front().unwrap_or((0.0, 0.0));
                if ch == 1 {
                    frame[0] = T::from_sample((l + rr) * 0.5);
                } else {
                    frame[0] = T::from_sample(l);
                    frame[1] = T::from_sample(rr);
                    for x in &mut frame[2..] {
                        *x = T::from_sample(0.0);
                    }
                }
            }
        },
        |e| eprintln!("audio stream: {e}"),
        None,
    )
}
