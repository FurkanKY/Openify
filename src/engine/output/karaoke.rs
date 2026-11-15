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
//! For each positive-frequency bin, a center estimate combines channel level
//! balance and phase agreement. A short frequency smoother and a
//! fast-attack/slow-release mask smoother prevent narrow-bin chirps. The soft
//! mask `(1 - amount * center * band)^3` (center rescaled above a floor of
//! 0.3) is applied to the mid component `M = (L+R)/2` only; the side component
//! `S = (L-R)/2` stays untouched and the output is rebuilt as `L = M' + S`,
//! `R = M' - S`. Panned and out-of-phase material therefore keeps its level
//! even at the frequency of a centered voice. Causal positive spectral flux
//! detects attacks and keeps the mask open for the next analysis frame, so a
//! centered drum hit is not dipped. A soft 150-350 Hz and 7.5-9 kHz transition
//! protects bass and high-frequency detail.
//!
//! The mask also removes part of the accompaniment, so the wet signal is
//! lifted by a slowly moving makeup gain (at most +4 dB, never below unity):
//! half of the loudness the effect removed, measured on one-second energies of
//! the input and the masked output, is given back. Peaks the gain pushes above
//! 0.9 go through a tanh soft limiter that stays below 1.0.
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
// Tuning, measured on 144 seven-second MUSDB18 clips (see `musdb_measure.rs`).
// Variant B (this one): vocals about -6.3 dB, bass/drums about +0.5 dB and other
// instruments about -1.9 dB at full amount, all after the makeup gain.
// Variant A (deeper on vocals, same loudness goal): MASK_CURVE 2.2,
// CENTER_FLOOR 0.0, MAKEUP_SHARE 0.7, band 120 Hz + 100 Hz / 9500 Hz - 1700 Hz;
// vocals about -6.8 dB, bass/drums about +0.9 dB, other about -2.4 dB.

/// Exponent that deepens the mask where the center estimate is high while
/// leaving weakly centered bins nearly untouched.
const MASK_CURVE: f32 = 3.0;
/// Depth multiplier of the mask (1 = the mask reaches full depth).
const STRENGTH: f32 = 1.0;
/// Center estimates below this are left alone; above it they are rescaled to 0..1.
const CENTER_FLOOR: f32 = 0.3;
/// Share (0..1, in dB) of the measured loudness loss that the makeup gain gives back.
const MAKEUP_SHARE: f32 = 0.5;
/// The mask is fully active from `BAND_LOW_START + BAND_LOW_WIDTH` Hz up to
/// `BAND_HIGH_END - BAND_HIGH_WIDTH` Hz and fades out toward both ends.
const BAND_LOW_START: f32 = 150.0;
const BAND_LOW_WIDTH: f32 = 200.0;
const BAND_HIGH_END: f32 = 9000.0;
const BAND_HIGH_WIDTH: f32 = 1500.0;
/// Largest makeup gain (linear), +4 dB.
const MAKEUP_MAX: f32 = 1.584_893_2;
/// Per-sample coefficient of the 1 s input/output level trackers.
const LEVEL_K: f64 = 1.0 / RATE as f64;
/// Per-sample smoothing of the makeup gain (about 0.2 s).
const MAKEUP_SMOOTH: f32 = 1.0 / (0.2 * RATE as f32);
/// Samples above this are soft-limited once the makeup gain is above unity.
const LIMIT_KNEE: f32 = 0.9;
const LEAK: f64 = 0.990_38;
const SILENCE: f64 = 1.0e-8;
const FLUSH_BELOW: f32 = f32::MIN_POSITIVE;
const FLUSH_ABOVE: f32 = 1.0e6;
/// Positive spectral flux, relative to the total magnitude, that marks an attack.
const TRANSIENT_FLUX: f32 = 0.30;
/// Analysis frames the mask stays open after an attack.
const TRANSIENT_HOLD: u8 = 1;

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
        let depth = depth_of(enabled, amount);
        self.depth.store(depth.to_bits(), Relaxed);
    }

    #[inline]
    pub(super) fn get(&self) -> f32 {
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
    level_in: f64,
    level_wet: f64,
    makeup_target: f32,
    makeup: f32,
    /// Lets mask tests look at the mask alone.
    #[cfg(test)]
    makeup_off: bool,
    /// Recorded masks; `None` unless a measurement enables it, so ordinary
    /// tests exercise the allocation-free path.
    #[cfg(test)]
    mask_trace: Option<Vec<TraceFrame>>,
    transient_left: u8,
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
            level_in: 0.0,
            level_wet: 0.0,
            makeup_target: 1.0,
            makeup: 1.0,
            #[cfg(test)]
            makeup_off: false,
            #[cfg(test)]
            mask_trace: None,
            transient_left: 0,
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
        self.level_in = 0.0;
        self.level_wet = 0.0;
        self.makeup_target = 1.0;
        self.makeup = 1.0;
        #[cfg(test)]
        if let Some(trace) = &mut self.mask_trace {
            trace.clear();
        }
        self.left.fill(Complex::default());
        self.right.fill(Complex::default());
        self.transient_left = 0;
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
            self.input_l[input_ix] = l;
            self.input_r[input_ix] = r;

            let phase = count as usize % HOP;
            let wet_l = clean_state(self.overlap_l[output_ix] / self.norm[phase]);
            let wet_r = clean_state(self.overlap_r[output_ix] / self.norm[phase]);
            self.overlap_l[output_ix] = 0.0;
            self.overlap_r[output_ix] = 0.0;
            self.track_levels(l, r, wet_l, wet_r);
            let (wet_l, wet_r) = self.apply_makeup(wet_l, wet_r);
            if count < WINDOW as u64 {
                raw[f * 2] = l;
                raw[f * 2 + 1] = r;
            } else {
                raw[f * 2] = clean_state(l + w * (wet_l - l));
                raw[f * 2 + 1] = clean_state(r + w * (wet_r - r));
            }

            self.sample_count = count + 1;
            if (self.sample_count as usize).is_multiple_of(HOP) {
                self.update_makeup();
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

    /// Leaky one-second energy of the dry input and of the masked output.
    #[inline]
    fn track_levels(&mut self, l: f32, r: f32, wet_l: f32, wet_r: f32) {
        let dry = f64::from(l * l + r * r);
        let wet = f64::from(wet_l * wet_l + wet_r * wet_r);
        self.level_in += (dry - self.level_in) * LEVEL_K;
        self.level_wet += (wet - self.level_wet) * LEVEL_K;
        if self.level_in < 1.0e-30 {
            self.level_in = 0.0;
        }
        if self.level_wet < 1.0e-30 {
            self.level_wet = 0.0;
        }
    }

    /// Once per hop: the gain that gives back part of what the mask removed,
    /// never more than `MAKEUP_MAX` and never below unity.
    fn update_makeup(&mut self) {
        #[cfg(test)]
        if self.makeup_off {
            return;
        }
        self.makeup_target = if self.level_in > 1.0e-9 && self.level_wet > 1.0e-12 {
            let loss = (self.level_in / self.level_wet) as f32;
            loss.powf(MAKEUP_SHARE * 0.5).clamp(1.0, MAKEUP_MAX)
        } else {
            1.0
        };
    }

    /// Smooths the makeup gain, applies it to one wet frame and soft-limits
    /// peaks that the gain pushed above the knee.
    #[inline]
    fn apply_makeup(&mut self, wet_l: f32, wet_r: f32) -> (f32, f32) {
        self.makeup += (self.makeup_target - self.makeup) * MAKEUP_SMOOTH;
        let gain = self.makeup;
        if gain <= 1.0 + 1.0e-4 {
            return (wet_l, wet_r);
        }
        (soft_limit(wet_l * gain), soft_limit(wet_r * gain))
    }

    /// Scales only the mid part `(L+R)/2` of bin `k` by `self.mask[k]`; the side
    /// part `(L-R)/2` is kept, so L = M' + S and R = M' - S. The negative
    /// frequency partner is the conjugate, keeping the frame real.
    #[inline]
    fn mask_mid(&mut self, k: usize) {
        let gain = self.mask[k];
        let (l, r) = (self.left[k], self.right[k]);
        let mid = Complex::new((l.re + r.re) * 0.5, (l.im + r.im) * 0.5);
        let side = Complex::new((l.re - r.re) * 0.5, (l.im - r.im) * 0.5);
        let left = Complex::new(mid.re * gain + side.re, mid.im * gain + side.im);
        let right = Complex::new(mid.re * gain - side.re, mid.im * gain - side.im);
        self.left[k] = left;
        self.right[k] = right;
        if k > 0 && k < WINDOW / 2 {
            let mirror = WINDOW - k;
            self.left[mirror] = Complex::new(left.re, -left.im);
            self.right[mirror] = Complex::new(right.re, -right.im);
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
        if total > 1.0e-8 && flux / total > TRANSIENT_FLUX {
            self.transient_left = TRANSIENT_HOLD;
        } else {
            self.transient_left = self.transient_left.saturating_sub(1);
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
            let target = suppression_target(amount, smooth_center, band);
            // Fast attack removes sustained center material; a softer release
            // avoids a zipper-like change between adjacent analysis frames.
            if self.transient_left > 0 {
                self.mask[k] = 1.0;
            } else {
                let smoothing = if target < self.mask[k] { 0.78 } else { 0.28 };
                self.mask[k] = clean_state(self.mask[k] + (target - self.mask[k]) * smoothing);
            }
            self.mask_mid(k);
        }
        #[cfg(test)]
        if let Some(trace) = &mut self.mask_trace {
            trace.push(TraceFrame {
                mask: self.mask.clone(),
                makeup: self.makeup_target,
            });
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

    /// Turns the makeup gain off so a test sees the mask alone.
    #[cfg(test)]
    pub(super) fn disable_makeup(&mut self) {
        self.makeup_off = true;
    }

    /// Starts recording the mask of every analysis frame (measurements only).
    #[cfg(test)]
    pub(super) fn record_mask_trace(&mut self) {
        self.mask_trace = Some(Vec::new());
    }

    /// Hands over the recorded frames (measurements only).
    #[cfg(test)]
    pub(super) fn take_mask_trace(&mut self) -> Vec<TraceFrame> {
        self.mask_trace.take().unwrap_or_default()
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
    }
}

#[cfg(test)]
impl Karaoke {
    /// Replays the measured mix mask over a stem for offline MUSDB validation.
    pub(super) fn process_with_mask_trace(&mut self, raw: &mut [f32], trace: &[TraceFrame]) {
        if trace.is_empty() {
            return;
        }
        self.clear_dsp();
        self.sample_count = 0;
        self.engage = 0.0;
        let mut trace_index = 0;
        for block_start in (0..raw.len() / 2).step_by(64) {
            let n = (raw.len() / 2 - block_start).min(64);
            let warmed = self.sample_count >= WINDOW as u64;
            let w0 = self.engage;
            let w1 = w0 + (1.0 - w0).clamp(-1.0 / GLIDE_BLOCKS as f32, 1.0 / GLIDE_BLOCKS as f32);
            let dw = (w1 - w0) / n as f32;
            let mut w = w0;
            for f in 0..n {
                if warmed {
                    w += dw;
                }
                let count = self.sample_count;
                let input_ix = count as usize % WINDOW;
                let output_ix = count as usize % OLA_CAP;
                let l = finite_sample(raw[(block_start + f) * 2]);
                let r = finite_sample(raw[(block_start + f) * 2 + 1]);
                self.input_l[input_ix] = l;
                self.input_r[input_ix] = r;
                let phase = count as usize % HOP;
                let wet_l = clean_state(self.overlap_l[output_ix] / self.norm[phase]);
                let wet_r = clean_state(self.overlap_r[output_ix] / self.norm[phase]);
                self.overlap_l[output_ix] = 0.0;
                self.overlap_r[output_ix] = 0.0;
                let (wet_l, wet_r) = self.apply_makeup(wet_l, wet_r);
                if count < WINDOW as u64 {
                    raw[(block_start + f) * 2] = l;
                    raw[(block_start + f) * 2 + 1] = r;
                } else {
                    raw[(block_start + f) * 2] = clean_state(l + w * (wet_l - l));
                    raw[(block_start + f) * 2 + 1] = clean_state(r + w * (wet_r - r));
                }
                self.sample_count += 1;
                if (self.sample_count as usize).is_multiple_of(HOP) {
                    let frame = &trace[trace_index.min(trace.len() - 1)];
                    trace_index += 1;
                    self.makeup_target = frame.makeup;
                    self.apply_trace_frame(&frame.mask);
                }
            }
            if warmed {
                self.engage = w1;
            }
        }
    }

    fn apply_trace_frame(&mut self, mask: &[f32]) {
        let start = self.sample_count as i64 - WINDOW as i64;
        for i in 0..WINDOW {
            let ix = (start + i as i64) as usize % WINDOW;
            self.left[i] = Complex::new(self.input_l[ix] * self.window[i], 0.0);
            self.right[i] = Complex::new(self.input_r[ix] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.left, false);
        self.fft.process(&mut self.right, false);
        self.mask.copy_from_slice(mask);
        for k in 0..BINS {
            self.mask_mid(k);
        }
        self.fft.process(&mut self.left, true);
        self.fft.process(&mut self.right, true);
        for i in 0..WINDOW {
            let output_frame = start + i as i64 + WINDOW as i64;
            if output_frame >= self.sample_count as i64 {
                let ix = output_frame as usize % OLA_CAP;
                self.overlap_l[ix] =
                    clean_state(self.overlap_l[ix] + self.left[i].re * self.window[i]);
                self.overlap_r[ix] =
                    clean_state(self.overlap_r[ix] + self.right[i].re * self.window[i]);
            }
        }
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
    let low = ((hz - BAND_LOW_START) / BAND_LOW_WIDTH).clamp(0.0, 1.0);
    let high = ((BAND_HIGH_END - hz) / BAND_HIGH_WIDTH).clamp(0.0, 1.0);
    low.min(high)
}

#[inline]
fn suppression_target(amount: f32, center: f32, band: f32) -> f32 {
    let center = ((center - CENTER_FLOOR) / (1.0 - CENTER_FLOOR)).clamp(0.0, 1.0);
    (1.0 - STRENGTH * amount * center * band)
        .max(0.0)
        .powf(MASK_CURVE)
}

/// Smooth ceiling at 1.0: identity up to `LIMIT_KNEE`, then a tanh knee.
#[inline]
fn soft_limit(x: f32) -> f32 {
    let over = x.abs() - LIMIT_KNEE;
    if over <= 0.0 {
        x
    } else {
        let room = 1.0 - LIMIT_KNEE;
        (LIMIT_KNEE + room * (over / room).tanh()).copysign(x)
    }
}

/// One analysis frame as recorded for offline measurements.
#[cfg(test)]
pub(super) struct TraceFrame {
    pub(super) mask: Vec<f32>,
    pub(super) makeup: f32,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_limit_is_transparent_below_the_knee_and_stays_under_one() {
        for x in [0.0f32, 0.3, -0.7, LIMIT_KNEE, -LIMIT_KNEE] {
            assert_eq!(soft_limit(x), x);
        }
        let mut previous = soft_limit(LIMIT_KNEE);
        for i in 1..=200 {
            let y = soft_limit(LIMIT_KNEE + i as f32 * 0.05);
            assert!(y >= previous && y <= 1.0, "{y}");
            previous = y;
        }
        assert_eq!(soft_limit(-5.0), -soft_limit(5.0));
    }

    #[test]
    fn mask_target_reaches_zero_only_for_a_fully_centered_bin_at_full_amount() {
        let band = band_weight(1000.0);
        assert_eq!(band, 1.0);
        assert_eq!(suppression_target(1.0, 1.0, band), 0.0);
        // Weakly centered bins are left alone, and amount 0 does nothing.
        assert_eq!(suppression_target(1.0, CENTER_FLOOR, band), 1.0);
        assert_eq!(suppression_target(0.0, 1.0, band), 1.0);
        // The mask never leaves 0..=1, even if the amount ramp overshoots.
        assert!((0.0..=1.0).contains(&suppression_target(1.0001, 1.0, band)));
        // Bass and air stay outside the band.
        assert_eq!(band_weight(60.0), 0.0);
        assert_eq!(band_weight(12_000.0), 0.0);
    }

    #[test]
    fn makeup_gain_is_bounded_and_never_attenuates() {
        let mut k = Karaoke::new();
        k.level_in = 1.0;
        k.level_wet = 1.0e-6; // the mask removed almost everything
        k.update_makeup();
        assert_eq!(k.makeup_target, MAKEUP_MAX);
        k.level_wet = 4.0; // output louder than input
        k.update_makeup();
        assert_eq!(k.makeup_target, 1.0);
        k.level_in = 0.0; // silence
        k.update_makeup();
        assert_eq!(k.makeup_target, 1.0);
    }
}
