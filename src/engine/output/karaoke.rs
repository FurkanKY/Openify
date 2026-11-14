//! Beat-aware vocal reduction in the frequency domain.
//!
//! # Method and latency
//!
//! A 1024-sample periodic Hann window at 44.1 kHz (23.22 ms) is analyzed and
//! overlap-added every 256 samples (75% overlap). The fixed 1024-frame delay
//! lets every overlapping frame contribute before its output is heard. The
//! active output has one fixed latency; the 46 ms engage/disengage glide
//! crossfades between direct bypass and the delayed result. At 44.1 kHz the
//! added latency is exactly 1024 / 44100 = 23.22 ms, below the 60 ms
//! lyric-sync allowance.
//!
//! For each positive-frequency bin, the mask combines channel level balance
//! and phase agreement. It therefore attenuates components with similar level
//! and near-zero phase difference, while leaving panned and out-of-phase parts
//! largely intact. A short frequency smoother and a fast-attack/slow-release
//! mask smoother prevent narrow-bin chirps. Causal positive spectral flux
//! detects attacks and temporarily relaxes the mask; a centered drum hit can
//! therefore pass even though it occupies the vocal band. A soft 120–220 Hz
//! and 7.8–9.5 kHz transition protects bass and high-frequency detail.
//!
//! # Real-time constraints
//!
//! The project had no FFT dependency, and the registry was unreachable in this
//! build environment. The small radix-2 FFT therefore uses precomputed bit
//! reversal and twiddle tables instead of introducing an unresolvable crate.
//! The tables and all working memory are built in `new`; `process` allocates
//! nothing and takes no locks. The output callback calls this only while the
//! existing `running` gate is active. Exact mono is passed through bit for bit.
//! `reset` clears the delay, overlap-add, spectral history, and transient state
//! after a flush. Non-finite samples and denormal state are suppressed.

use std::{
    f64::consts::PI,
    sync::atomic::{AtomicU32, Ordering::Relaxed},
};

use super::RATE;

/// FFT window and fixed dry/wet alignment delay (samples).
pub const WINDOW: usize = 1024;
/// Hop size: 75% overlap.
pub const HOP: usize = WINDOW / 4;
/// Number of callback blocks for engage, disengage, and amount ramps.
pub const GLIDE_BLOCKS: u32 = 32;

const BINS: usize = WINDOW / 2 + 1;
const OLA_CAP: usize = WINDOW * 4;
const MONO_BELOW: f64 = 1.0e-4;
const STEREO_ABOVE: f64 = 4.0e-4;
const EXACT_MONO: f64 = 1.0e-9;
const EXACT_RUN: u32 = 4;
const LEAK: f64 = 0.990_38;
const SILENCE: f64 = 1.0e-8;
const FLUSH_BELOW: f32 = f32::MIN_POSITIVE;
const FLUSH_ABOVE: f32 = 1.0e6;
const TRANSIENT_HOLD: u8 = 8;
const TRANSIENT_SAMPLES: u16 = 2048;

/// Shared karaoke setting: one lock-free atomic float, 0 means disabled.
pub struct KaraokeCtl {
    depth: AtomicU32,
}

impl KaraokeCtl {
    pub fn new() -> Self {
        Self {
            depth: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    /// Publish an amount in 0..=1. Non-finite values disable the effect.
    pub fn set(&self, enabled: bool, amount: f32) {
        self.depth
            .store(depth_of(enabled, amount).to_bits(), Relaxed);
    }

    #[inline]
    fn get(&self) -> f32 {
        f32::from_bits(self.depth.load(Relaxed))
    }
}

fn depth_of(enabled: bool, amount: f32) -> f32 {
    if enabled && amount.is_finite() {
        amount.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    #[inline]
    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    #[inline]
    fn magnitude(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }

    #[inline]
    fn multiply(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    #[inline]
    fn scale(&mut self, factor: f32) {
        self.re *= factor;
        self.im *= factor;
    }
}

/// Radix-2 FFT permutation and twiddles, built before the callback starts.
struct FftPlan {
    bit_reverse: [usize; WINDOW],
    twiddle: [Complex; WINDOW / 2],
}

impl FftPlan {
    fn new() -> Self {
        let bits = WINDOW.trailing_zeros();
        let mut bit_reverse = [0; WINDOW];
        for (i, value) in bit_reverse.iter_mut().enumerate() {
            *value = i.reverse_bits() >> (usize::BITS - bits);
        }
        let mut twiddle = [Complex::default(); WINDOW / 2];
        for (i, value) in twiddle.iter_mut().enumerate() {
            let angle = -2.0 * PI * i as f64 / WINDOW as f64;
            *value = Complex {
                re: angle.cos() as f32,
                im: angle.sin() as f32,
            };
        }
        Self {
            bit_reverse,
            twiddle,
        }
    }

    fn process(&self, values: &mut [Complex; WINDOW], inverse: bool) {
        for i in 0..WINDOW {
            let j = self.bit_reverse[i];
            if j > i {
                values.swap(i, j);
            }
        }
        let mut width = 2;
        while width <= WINDOW {
            let half = width / 2;
            let stride = WINDOW / width;
            for base in (0..WINDOW).step_by(width) {
                for j in 0..half {
                    let mut twiddle = self.twiddle[j * stride];
                    if inverse {
                        twiddle.im = -twiddle.im;
                    }
                    let even = values[base + j];
                    let odd = values[base + j + half].multiply(twiddle);
                    values[base + j] = Complex {
                        re: even.re + odd.re,
                        im: even.im + odd.im,
                    };
                    values[base + j + half] = Complex {
                        re: even.re - odd.re,
                        im: even.im - odd.im,
                    };
                }
            }
            width *= 2;
        }
        if inverse {
            for value in values {
                value.re /= WINDOW as f32;
                value.im /= WINDOW as f32;
            }
        }
    }
}

/// All FFT state and sample storage are owned by the output source.
pub struct Karaoke {
    depth: f32,
    amount: f32,
    engage: f32,
    mono: bool,
    e_mid: f64,
    e_side: f64,
    exact_run: u32,
    fresh: bool,
    running: bool,
    sample_count: u64,
    input_l: [f32; WINDOW],
    input_r: [f32; WINDOW],
    overlap_l: [f32; OLA_CAP],
    overlap_r: [f32; OLA_CAP],
    window: [f32; WINDOW],
    norm: [f32; HOP],
    left: [Complex; WINDOW],
    right: [Complex; WINDOW],
    previous_magnitude: Vec<f32>,
    center: Vec<f32>,
    mask: Vec<f32>,
    transient_left: u8,
    transient_samples: u16,
    last_l: f32,
    last_r: f32,
    fft: FftPlan,
}

impl Karaoke {
    pub fn new() -> Self {
        let fft = FftPlan::new();
        let mut window = [0.0; WINDOW];
        for (i, w) in window.iter_mut().enumerate() {
            *w = (0.5 - 0.5 * (2.0 * PI * i as f64 / WINDOW as f64).cos()) as f32;
        }
        let mut norm = [0.0; HOP];
        for (phase, sum) in norm.iter_mut().enumerate() {
            for i in (phase..WINDOW).step_by(HOP) {
                *sum += window[i] * window[i];
            }
        }
        Self {
            depth: 0.0,
            amount: 0.0,
            engage: 0.0,
            mono: false,
            e_mid: 0.0,
            e_side: 0.0,
            exact_run: 0,
            fresh: true,
            running: false,
            sample_count: 0,
            input_l: [0.0; WINDOW],
            input_r: [0.0; WINDOW],
            overlap_l: [0.0; OLA_CAP],
            overlap_r: [0.0; OLA_CAP],
            window,
            norm,
            left: [Complex::default(); WINDOW],
            right: [Complex::default(); WINDOW],
            previous_magnitude: vec![0.0; BINS],
            center: vec![0.0; BINS],
            mask: vec![1.0; BINS],
            transient_left: 0,
            transient_samples: 0,
            last_l: 0.0,
            last_r: 0.0,
            fft,
        }
    }

    #[inline]
    pub fn running(&self) -> bool {
        self.running
    }

    #[inline]
    pub fn poll(&mut self, ctl: &KaraokeCtl) {
        let was_running = self.running;
        self.depth = ctl.get();
        self.running = self.depth > 0.0 || self.engage > 0.0;
        if was_running && !self.running {
            self.clear_dsp();
        }
    }

    /// Clear every delayed sample and detector history after a stream flush.
    pub fn reset(&mut self) {
        self.clear_dsp();
        self.e_mid = 0.0;
        self.e_side = 0.0;
        self.exact_run = 0;
        self.engage = 0.0;
        self.amount = 0.0;
        self.fresh = true;
    }

    fn clear_dsp(&mut self) {
        self.sample_count = 0;
        self.input_l.fill(0.0);
        self.input_r.fill(0.0);
        self.overlap_l.fill(0.0);
        self.overlap_r.fill(0.0);
        self.previous_magnitude.fill(0.0);
        self.center.fill(0.0);
        self.mask.fill(1.0);
        self.left.fill(Complex::default());
        self.right.fill(Complex::default());
        self.transient_left = 0;
        self.transient_samples = 0;
        self.last_l = 0.0;
        self.last_r = 0.0;
    }

    /// Process interleaved stereo samples. The caller invokes this only while
    /// the pre-existing output callback gate says the effect is running.
    pub fn process(&mut self, raw: &mut [f32], n: usize) {
        if n == 0 {
            return;
        }

        if self.depth > 0.0 {
            let (mut em, mut es) = (0.0f64, 0.0f64);
            let mut exact = true;
            for f in 0..n {
                let l = finite_sample(raw[f * 2]);
                let r = finite_sample(raw[f * 2 + 1]);
                let (m, s) = ((l + r) * 0.5, (l - r) * 0.5);
                em += f64::from(m * m);
                es += f64::from(s * s);
                exact &= l.to_bits() == r.to_bits();
            }
            self.e_mid = self.e_mid * LEAK + em;
            self.e_side = self.e_side * LEAK + es;
            self.exact_run = if em >= SILENCE && es <= em * EXACT_MONO {
                self.exact_run.saturating_add(1)
            } else {
                0
            };
            if (exact && em >= SILENCE) || self.exact_run >= EXACT_RUN {
                self.e_mid = em;
                self.e_side = es;
                self.mono = true;
            } else if self.e_mid + self.e_side >= SILENCE {
                let ratio = self.e_side / self.e_mid.max(f64::MIN_POSITIVE);
                self.mono = if self.mono {
                    ratio < STEREO_ABOVE
                } else {
                    ratio < MONO_BELOW
                };
            }
        }

        let engage_target = if self.depth > 0.0 && !self.mono {
            1.0
        } else {
            0.0
        };
        if self.fresh {
            self.fresh = false;
            if engage_target < self.engage {
                self.engage = engage_target;
            }
        }
        if self.engage == 0.0 {
            self.amount = self.depth;
        }
        if self.mono && self.engage == 0.0 {
            for frame in raw[..n * 2].as_chunks_mut::<2>().0 {
                frame[0] = finite_sample(frame[0]);
                frame[1] = finite_sample(frame[1]);
            }
            self.clear_dsp();
            self.running = self.depth > 0.0;
            return;
        }
        if self.engage == 0.0 && engage_target == 0.0 {
            self.clear_dsp();
            self.running = self.depth > 0.0;
            return;
        }

        let step = 1.0 / GLIDE_BLOCKS as f32;
        let w0 = self.engage;
        let w1 = w0 + (engage_target - w0).clamp(-step, step);
        let a0 = self.amount;
        let a1 = a0 + (self.depth - a0).clamp(-step, step);
        let (dw, da) = ((w1 - w0) / n as f32, (a1 - a0) / n as f32);
        let (mut w, mut a) = (w0, a0);
        let warmed = self.sample_count >= WINDOW as u64;

        for f in 0..n {
            if warmed {
                w += dw;
            }
            a += da;
            let count = self.sample_count;
            let input_ix = count as usize % WINDOW;
            let output_ix = count as usize % OLA_CAP;
            let l = finite_sample(raw[f * 2]);
            let r = finite_sample(raw[f * 2 + 1]);
            let previous_peak = self.last_l.abs().max(self.last_r.abs());
            let current_peak = l.abs().max(r.abs());
            if current_peak - previous_peak > 0.55 {
                self.transient_samples = TRANSIENT_SAMPLES;
            } else {
                self.transient_samples = self.transient_samples.saturating_sub(1);
            }
            self.last_l = l;
            self.last_r = r;
            self.input_l[input_ix] = l;
            self.input_r[input_ix] = r;

            let phase = count as usize % HOP;
            let wet_l = clean_state(self.overlap_l[output_ix] / self.norm[phase]);
            let wet_r = clean_state(self.overlap_r[output_ix] / self.norm[phase]);
            self.overlap_l[output_ix] = 0.0;
            self.overlap_r[output_ix] = 0.0;
            if count < WINDOW as u64 {
                raw[f * 2] = l;
                raw[f * 2 + 1] = r;
            } else {
                raw[f * 2] = clean_state(l + w * (wet_l - l));
                raw[f * 2 + 1] = clean_state(r + w * (wet_r - r));
            }

            self.sample_count = count + 1;
            if (self.sample_count as usize).is_multiple_of(HOP) {
                self.analyze_frame(a);
            }
        }

        self.engage = if warmed { w1 } else { w0 };
        self.amount = a1;
        self.running = self.depth > 0.0 || self.engage > 0.0;
        if !self.running {
            self.clear_dsp();
        }
    }

    fn analyze_frame(&mut self, amount: f32) {
        let start = self.sample_count as i64 - WINDOW as i64;
        for i in 0..WINDOW {
            let frame_ix = start + i as i64;
            let (l, r) = if frame_ix >= 0 {
                let ix = frame_ix as usize % WINDOW;
                (self.input_l[ix], self.input_r[ix])
            } else {
                (0.0, 0.0)
            };
            let win = self.window[i];
            self.left[i] = Complex::new(l * win, 0.0);
            self.right[i] = Complex::new(r * win, 0.0);
        }
        self.fft.process(&mut self.left, false);
        self.fft.process(&mut self.right, false);

        let (mut flux, mut total) = (0.0f32, 0.0f32);
        for k in 0..BINS {
            let l = self.left[k];
            let r = self.right[k];
            let lm = l.magnitude();
            let rm = r.magnitude();
            let mag = clean_state((lm + rm) * 0.5);
            total += mag;
            flux += (mag - self.previous_magnitude[k]).max(0.0);
            self.previous_magnitude[k] = mag;
            let level_match = 2.0 * lm.min(rm) / (lm + rm + 1.0e-20);
            let phase_match = ((l.re * r.re + l.im * r.im) / (lm * rm + 1.0e-20)).clamp(0.0, 1.0);
            self.center[k] = clean_state(level_match * phase_match);
        }
        if total > 1.0e-8 && flux / total > 0.16 {
            self.transient_left = TRANSIENT_HOLD;
        } else {
            self.transient_left = self.transient_left.saturating_sub(1);
        }
        if self.transient_samples > 0 {
            self.transient_left = TRANSIENT_HOLD;
        }
        for k in 0..BINS {
            let smooth_center = (center_at(&self.center, k as isize - 2)
                + 2.0 * center_at(&self.center, k as isize - 1)
                + 3.0 * self.center[k]
                + 2.0 * center_at(&self.center, k as isize + 1)
                + center_at(&self.center, k as isize + 2))
                / 9.0;
            let hz = k as f32 * RATE as f32 / WINDOW as f32;
            let band = band_weight(hz);
            let target = 1.0 - amount * smooth_center * band;
            // Fast attack removes sustained center material; a softer release
            // avoids a zipper-like change between adjacent analysis frames.
            if self.transient_left > 0 {
                self.mask[k] = 1.0;
            } else {
                let smoothing = if target < self.mask[k] { 0.78 } else { 0.28 };
                self.mask[k] = clean_state(self.mask[k] + (target - self.mask[k]) * smoothing);
            }
            self.left[k].scale(self.mask[k]);
            self.right[k].scale(self.mask[k]);
            if k > 0 && k < WINDOW / 2 {
                let mirror = WINDOW - k;
                self.left[mirror].scale(self.mask[k]);
                self.right[mirror].scale(self.mask[k]);
            }
        }

        self.fft.process(&mut self.left, true);
        self.fft.process(&mut self.right, true);
        for i in 0..WINDOW {
            let frame_ix = start + i as i64;
            let output_frame = frame_ix + WINDOW as i64;
            if output_frame < self.sample_count as i64 {
                continue;
            }
            let ix = output_frame as usize % OLA_CAP;
            // The inverse transform is normalized in `FftPlan::process`.
            let window = self.window[i];
            self.overlap_l[ix] = clean_state(self.overlap_l[ix] + self.left[i].re * window);
            self.overlap_r[ix] = clean_state(self.overlap_r[ix] + self.right[i].re * window);
        }

        for spectrum in [&mut self.left, &mut self.right] {
            for value in spectrum.iter_mut() {
                value.re = clean_state(value.re);
                value.im = clean_state(value.im);
            }
        }
    }

    #[cfg(test)]
    pub fn engage(&self) -> f32 {
        self.engage
    }

    #[cfg(test)]
    pub fn is_mono(&self) -> bool {
        self.mono
    }

    #[cfg(test)]
    pub fn state_is_zero(&mut self) -> bool {
        [
            self.sample_count == 0,
            self.input_l.iter().all(|v| *v == 0.0),
            self.input_r.iter().all(|v| *v == 0.0),
            self.overlap_l.iter().all(|v| *v == 0.0),
            self.overlap_r.iter().all(|v| *v == 0.0),
            self.previous_magnitude.iter().all(|v| *v == 0.0),
            self.transient_samples == 0,
        ]
        .into_iter()
        .all(|ok| ok)
    }

    #[cfg(test)]
    pub fn state_is_finite(&self) -> bool {
        let clean = |value: f32| value.is_finite() && (value == 0.0 || value.abs() >= FLUSH_BELOW);
        self.input_l.iter().chain(&self.input_r).all(|v| clean(*v))
            && self
                .overlap_l
                .iter()
                .chain(&self.overlap_r)
                .all(|v| clean(*v))
            && self
                .left
                .iter()
                .chain(&self.right)
                .all(|v| clean(v.re) && clean(v.im))
            && self.previous_magnitude.iter().all(|v| clean(*v))
            && self.center.iter().all(|v| clean(*v))
            && self.mask.iter().all(|v| clean(*v))
            && clean(self.last_l)
            && clean(self.last_r)
    }
}

fn center_at(values: &[f32], index: isize) -> f32 {
    if index < 0 || index as usize >= values.len() {
        0.0
    } else {
        values[index as usize]
    }
}

fn band_weight(hz: f32) -> f32 {
    let low = ((hz - 120.0) / 100.0).clamp(0.0, 1.0);
    let high = ((9500.0 - hz) / 1700.0).clamp(0.0, 1.0);
    low.min(high)
}

#[inline]
fn finite_sample(value: f32) -> f32 {
    if value.is_finite() && value.abs() <= FLUSH_ABOVE {
        clean_state(value)
    } else {
        0.0
    }
}

#[inline]
fn clean_state(value: f32) -> f32 {
    if !value.is_finite() || value.abs() < FLUSH_BELOW || value.abs() > FLUSH_ABOVE {
        0.0
    } else {
        value
    }
}

#[cfg(test)]
impl super::OutputCtl {
    pub(in crate::engine) fn karaoke_published(&self) -> (bool, f32) {
        let depth = self.karaoke.get();
        (depth > 0.0, depth)
    }
}
