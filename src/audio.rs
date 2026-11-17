//! Системный звук (WASAPI loopback) -> 128 значений спектра (64 левый + 64 правый канал),
//! тот же формат, что отдаёт Wallpaper Engine в wallpaperRegisterAudioListener.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const N: usize = 1024;
const BANDS: usize = 64;

fn spectrum(fft: &dyn Fft<f32>, win: &[f32], s: &[f32], prev: &mut [f32], out: &mut Vec<f32>) {
    let mut c: Vec<Complex<f32>> = s.iter().zip(win).map(|(x, w)| Complex::new(x * w, 0.0)).collect();
    fft.process(&mut c);
    for b in 0..BANDS {
        // логарифмические полосы по первой половине спектра
        let lo = (512f32.powf(b as f32 / BANDS as f32) as usize).max(1);
        let hi = (512f32.powf((b + 1) as f32 / BANDS as f32) as usize).max(lo + 1).min(512);
        let m = c[lo..hi].iter().map(|z| z.norm()).sum::<f32>() / (hi - lo) as f32;
        let v = (m / (N as f32 * 0.05)).sqrt().min(1.0);
        prev[b] = v.max(prev[b] * 0.8); // плавное затухание
        out.push(prev[b]);
    }
}

/// Возвращает Stream: его нужно держать живым. None — если захват недоступен.
pub fn start<F: Fn(Vec<f32>) + Send + 'static>(enabled: Arc<AtomicBool>, send: F) -> Option<Stream> {
    // на Windows input-поток, открытый на устройстве вывода, работает как loopback
    let device = cpal::default_host().default_output_device()?;
    let cfg = device.default_output_config().ok()?;
    if cfg.sample_format() != SampleFormat::F32 {
        return None;
    }
    let ch = cfg.channels() as usize;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N);
    let win: Vec<f32> = (0..N)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N as f32 - 1.0)).cos())
        .collect();
    let (mut l, mut r) = (Vec::<f32>::with_capacity(N * 2), Vec::<f32>::with_capacity(N * 2));
    let (mut pl, mut pr) = (vec![0f32; BANDS], vec![0f32; BANDS]);
    let mut last = Instant::now();

    let stream = device
        .build_input_stream(
            &cfg.into(),
            move |data: &[f32], _| {
                for f in data.chunks(ch) {
                    l.push(f[0]);
                    r.push(*f.get(1).unwrap_or(&f[0]));
                }
                if l.len() < N {
                    return;
                }
                if enabled.load(Ordering::Relaxed) && last.elapsed() >= Duration::from_millis(33) {
                    last = Instant::now();
                    let mut out = Vec::with_capacity(BANDS * 2);
                    spectrum(&*fft, &win, &l[l.len() - N..], &mut pl, &mut out);
                    spectrum(&*fft, &win, &r[r.len() - N..], &mut pr, &mut out);
                    send(out);
                }
                l.clear();
                r.clear();
            },
            |_err| {},
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
