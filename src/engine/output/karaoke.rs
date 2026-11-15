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
    sync::OnceLock,
    sync::atomic::{
        AtomicU8, AtomicU32, AtomicU64,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
    },
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
pub(super) const REPET_BANDS: usize = 40;
const REPET_STRIDE: usize = 4;
const REPET_HOP: usize = HOP * REPET_STRIDE;
#[cfg(test)]
const REPET_MAX_INFLIGHT_CHUNKS: usize = 4; // worker + bounded channel + producer copy
pub(in crate::engine) const REPET_MAX_CHUNK_FRAMES: usize = RATE as usize / 4;
pub(in crate::engine) const REPET_WORKER_STACK_BYTES: usize = 256 * 1024;
const REPET_HISTORY_FRAMES: usize = (RATE as usize * 60).div_ceil(REPET_HOP);
#[cfg(test)]
const REPET_MAX_HISTORY_FRAMES: usize = REPET_HISTORY_FRAMES;
const REPET_MIN_DISTANCE_FRAMES: usize = (RATE as usize).div_ceil(REPET_HOP);
const REPET_NEIGHBORS: usize = 3;
const REPET_MAX_DISTANCE: u16 = 28;
const REPET_DB_RANGE: f32 = 70.0;
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
    repet: OnceLock<RepetMaskRing>,
    repet_generation: AtomicU64,
}

struct RepetMaskRing {
    bands: Box<[[AtomicU8; REPET_BANDS]]>,
    frames: Box<[AtomicU64]>,
}

impl RepetMaskRing {
    fn new() -> Self {
        Self {
            bands: (0..REPET_HISTORY_FRAMES)
                .map(|_| std::array::from_fn(|_| AtomicU8::new(u8::MAX)))
                .collect(),
            frames: (0..REPET_HISTORY_FRAMES)
                .map(|_| AtomicU64::new(u64::MAX))
                .collect(),
        }
    }
}

impl KaraokeCtl {
    pub fn new() -> Self {
        Self {
            depth: AtomicU32::new(0.0f32.to_bits()),
            repet: OnceLock::new(),
            repet_generation: AtomicU64::new(0),
        }
    }

    /// Publish an amount in 0..=1. Non-finite values disable the effect.
    pub fn set(&self, enabled: bool, amount: f32) {
        let depth = depth_of(enabled, amount);
        self.depth.store(depth.to_bits(), Relaxed);
        if depth == 0.0 {
            self.reset_repet();
        }
    }

    #[inline]
    pub(super) fn get(&self) -> f32 {
        f32::from_bits(self.depth.load(Relaxed))
    }

    #[inline]
    fn repet(&self, frame: u64, bin: usize) -> f32 {
        let Some(ring) = self.repet.get() else {
            return 1.0;
        };
        let row = (frame as usize / REPET_HOP) % REPET_HISTORY_FRAMES;
        for age in 0..=2 {
            let at = (row + REPET_HISTORY_FRAMES - age) % REPET_HISTORY_FRAMES;
            let stamp = ring.frames[at].load(Acquire);
            if stamp <= frame && frame - stamp <= (REPET_HOP * 2) as u64 {
                return f32::from(ring.bands[at][repet_band(bin)].load(Relaxed)) / 255.0;
            }
        }
        1.0
    }

    pub(super) fn generation(&self) -> u64 {
        self.repet_generation.load(Relaxed)
    }

    pub(super) fn prepare_repet_mask(&self) {
        let _ = self.repet.get_or_init(RepetMaskRing::new);
    }

    pub(super) fn reset_repet(&self) -> u64 {
        let generation = self.repet_generation.fetch_add(1, Relaxed).wrapping_add(1);
        if let Some(ring) = self.repet.get() {
            for frame in &ring.frames {
                frame.store(u64::MAX, Release);
            }
        }
        generation
    }

    pub(super) fn publish_repet(&self, generation: u64, frame: u64, gains: &[f32; REPET_BANDS]) {
        let Some(ring) = self.repet.get() else {
            return;
        };
        if self.repet_generation.load(Relaxed) != generation {
            return;
        }
        let row = (frame as usize / REPET_HOP) % REPET_HISTORY_FRAMES;
        ring.frames[row].store(u64::MAX, Release);
        for (slot, gain) in ring.bands[row].iter().zip(gains) {
            slot.store((gain.clamp(0.0, 1.0) * 255.0).round() as u8, Relaxed);
        }
        if self.repet_generation.load(Relaxed) == generation {
            ring.frames[row].store(frame, Release);
        } else {
            let _ = ring.frames[row].compare_exchange(frame, u64::MAX, AcqRel, Relaxed);
        }
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

/// Causal k-nearest-neighbour repetition estimate using a 60-second,
/// eight-bit log-spectrum ring. Recent frames are excluded to avoid matching
/// a sound against its own local decay.
pub(in crate::engine) struct RepetAnalyzer {
    history: Vec<[u8; REPET_BANDS]>,
    write: usize,
    len: usize,
    input: [f32; WINDOW],
    input_write: usize,
    input_count: usize,
    hops: usize,
    window: [f32; WINDOW],
    spectrum: [Complex; WINDOW],
    fft: FftPlan,
}

impl RepetAnalyzer {
    pub(in crate::engine) fn new() -> Self {
        let mut window = [0.0; WINDOW];
        for (i, value) in window.iter_mut().enumerate() {
            *value = (0.5 - 0.5 * (2.0 * PI * i as f64 / WINDOW as f64).cos()) as f32;
        }
        Self {
            history: vec![[u8::MAX; REPET_BANDS]; REPET_HISTORY_FRAMES],
            write: 0,
            len: 0,
            input: [0.0; WINDOW],
            input_write: 0,
            input_count: 0,
            hops: 0,
            window,
            spectrum: [Complex::default(); WINDOW],
            fft: FftPlan::new(),
        }
    }

    /// Feed centered mono samples. Returns a new non-repeating ratio every
    /// fourth 256-sample hop, or `None` between analysis frames.
    pub(in crate::engine) fn feed(
        &mut self,
        samples: &[f32],
        start_frame: u64,
        mut publish: impl FnMut(u64, [f32; REPET_BANDS]),
    ) {
        for (offset, sample) in samples.iter().enumerate() {
            self.input[self.input_write] = finite_sample(*sample);
            self.input_write = (self.input_write + 1) % WINDOW;
            self.input_count += 1;
            if self.input_count.is_multiple_of(HOP) {
                self.hops += 1;
                if self.hops.is_multiple_of(REPET_STRIDE) {
                    self.spectral_frame();
                    let bands = self.band_spectrum();
                    let non_repeating = self.observe(&bands);
                    publish(start_frame + offset as u64, non_repeating);
                }
            }
        }
    }

    fn spectral_frame(&mut self) {
        for i in 0..WINDOW {
            let sample = if self.input_count >= WINDOW {
                self.input[(self.input_write + i) % WINDOW]
            } else {
                0.0
            };
            self.spectrum[i] = Complex::new(sample * self.window[i], 0.0);
        }
        self.fft.process(&mut self.spectrum, false);
    }

    fn band_spectrum(&self) -> [f32; REPET_BANDS] {
        let mut bands = [0.0f32; REPET_BANDS];
        for bin in 1..BINS {
            let hz = bin as f32 * RATE as f32 / WINDOW as f32;
            if (120.0..=9500.0).contains(&hz) {
                let band = repet_band(bin);
                bands[band] = bands[band].max(self.spectrum[bin].magnitude());
            }
        }
        bands
    }

    fn observe(&mut self, spectrum: &[f32; REPET_BANDS]) -> [f32; REPET_BANDS] {
        let peak = spectrum.iter().copied().fold(0.0f32, f32::max).max(1.0e-12);
        let mut current = [u8::MAX; REPET_BANDS];
        for (slot, magnitude) in current.iter_mut().zip(spectrum) {
            if magnitude.is_finite() && *magnitude > 0.0 {
                let db = 20.0 * (*magnitude / peak).log10();
                *slot = ((-db).clamp(0.0, REPET_DB_RANGE) * 255.0 / REPET_DB_RANGE) as u8;
            }
        }

        let mut best_dist = [u16::MAX; REPET_NEIGHBORS];
        let mut best_at = [0usize; REPET_NEIGHBORS];
        for age in REPET_MIN_DISTANCE_FRAMES..=self.len {
            let at = (self.write + REPET_HISTORY_FRAMES - age) % REPET_HISTORY_FRAMES;
            let prior = &self.history[at];
            let mut distance = 0u16;
            for band in 0..REPET_BANDS {
                distance += current[band].abs_diff(prior[band]) as u16;
            }
            distance /= REPET_BANDS as u16;
            if distance <= REPET_MAX_DISTANCE
                && let Some(slot) = best_dist.iter().position(|best| distance < *best)
            {
                for i in (slot + 1..REPET_NEIGHBORS).rev() {
                    best_dist[i] = best_dist[i - 1];
                    best_at[i] = best_at[i - 1];
                }
                best_dist[slot] = distance;
                best_at[slot] = at;
            }
        }

        let mut non_repeating = [1.0; REPET_BANDS];
        for band in 0..REPET_BANDS {
            let mut median = [u8::MAX; REPET_NEIGHBORS];
            let mut count = 0;
            for (distance, at) in best_dist.iter().zip(best_at) {
                if *distance != u16::MAX {
                    median[count] = self.history[at][band];
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            median[..count].sort_unstable();
            let background_db = -f32::from(median[count / 2]) * REPET_DB_RANGE / 255.0;
            let current_db = -f32::from(current[band]) * REPET_DB_RANGE / 255.0;
            let repeated = 10.0f32
                .powf((background_db - current_db) / 20.0)
                .clamp(0.0, 1.0);
            non_repeating[band] = 1.0 - repeated;
        }

        self.history[self.write] = current;
        self.write = (self.write + 1) % REPET_HISTORY_FRAMES;
        self.len = (self.len + 1).min(REPET_HISTORY_FRAMES);
        non_repeating
    }

    pub(in crate::engine) fn reset(&mut self) {
        self.history.fill([u8::MAX; REPET_BANDS]);
        self.write = 0;
        self.len = 0;
        self.input.fill(0.0);
        self.input_write = 0;
        self.input_count = 0;
        self.hops = 0;
    }
}

fn repet_band(bin: usize) -> usize {
    let hz = (bin as f32 * RATE as f32 / WINDOW as f32).max(120.0);
    let ratio = (hz / 120.0).ln() / (9500.0f32 / 120.0).ln();
    ((ratio * (REPET_BANDS - 1) as f32).round() as usize).min(REPET_BANDS - 1)
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
    #[cfg(test)]
    pub fn process(&mut self, raw: &mut [f32], n: usize) {
        self.process_inner(raw, n, None);
    }

    pub(super) fn process_with_repet(
        &mut self,
        raw: &mut [f32],
        n: usize,
        repet: &KaraokeCtl,
        frame_start: u64,
    ) {
        self.process_inner(raw, n, Some((repet, frame_start)));
    }

    fn process_inner(&mut self, raw: &mut [f32], n: usize, repet: Option<(&KaraokeCtl, u64)>) {
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
                self.analyze_frame(a, repet.map(|(ctl, frame)| (ctl, frame + f as u64)));
            }
        }

        self.engage = if warmed { w1 } else { w0 };
        self.amount = a1;
        self.running = self.depth > 0.0 || self.engage > 0.0;
        if !self.running {
            self.clear_dsp();
        }
    }

    fn analyze_frame(&mut self, amount: f32, repet: Option<(&KaraokeCtl, u64)>) {
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
            let non_repeating = repet.map_or(1.0, |(ctl, frame)| ctl.repet(frame, k));
            let target = suppression_target(amount, smooth_center, band, non_repeating);
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
fn suppression_target(amount: f32, center: f32, band: f32, non_repeating: f32) -> f32 {
    1.0 - amount * center * band * non_repeating
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
mod repet_tests {
    use super::*;

    #[test]
    fn online_repet_distinguishes_repeated_bands_from_a_changing_melody() {
        let mut repet = RepetAnalyzer::new();
        let mut last = [0.0; REPET_BANDS];

        for frame in 0..240 {
            let mut spectrum = [0.0; REPET_BANDS];
            spectrum[4] = 0.8; // Repeating kick / bass band.
            spectrum[10] = 0.45; // Repeating chord band.
            spectrum[if frame == 239 { 35 } else { 24 + frame % 11 }] = 0.7; // Last note has no prior occurrence in this band.
            last = repet.observe(&spectrum);
        }

        assert!(last[4] < 0.2, "repeated beat attenuation: {}", last[4]);
        assert!(last[10] < 0.2, "repeated chord attenuation: {}", last[10]);
        assert!(last[35] > 0.75, "novel melody attenuation: {}", last[35]);
    }

    #[test]
    fn online_repet_uses_stage_one_factor_until_a_distant_match_exists() {
        let mut repet = RepetAnalyzer::new();
        let mut spectrum = [0.0; REPET_BANDS];
        spectrum[12] = 1.0;

        for _ in 0..REPET_MIN_DISTANCE_FRAMES {
            assert!(repet.observe(&spectrum).iter().all(|gain| *gain == 1.0));
        }
    }

    #[test]
    fn online_repet_history_stays_within_the_memory_budget() {
        let analyzer_bytes = REPET_HISTORY_FRAMES
            * (REPET_BANDS + std::mem::size_of::<AtomicU64>())
            + std::mem::size_of::<RepetAnalyzer>()
            + REPET_MAX_INFLIGHT_CHUNKS * REPET_MAX_CHUNK_FRAMES * std::mem::size_of::<f32>()
            + std::mem::size_of::<KaraokeCtl>()
            + REPET_WORKER_STACK_BYTES;
        assert!(analyzer_bytes <= 10 * 1024 * 1024, "{analyzer_bytes} bytes");
        eprintln!("REPET bounded state estimate: {analyzer_bytes} bytes");
        const { assert!(REPET_HISTORY_FRAMES <= REPET_MAX_HISTORY_FRAMES) };
    }

    #[test]
    fn repetition_gate_only_attenuates_centered_non_repeating_material() {
        let amount = 0.95;
        let vocal_gain = suppression_target(amount, 1.0, 1.0, 1.0);
        let beat_gain = suppression_target(amount, 1.0, 1.0, 0.0);
        let panned_gain = suppression_target(amount, 0.0, 1.0, 1.0);
        let db = |gain: f32| 20.0 * gain.max(1.0e-6).log10();

        assert!(
            db(vocal_gain) <= -20.0,
            "vocal reduction {} dB",
            db(vocal_gain)
        );
        assert!(
            db(beat_gain).abs() <= 2.0,
            "beat change {} dB",
            db(beat_gain)
        );
        assert!(
            db(panned_gain).abs() <= 2.0,
            "panned change {} dB",
            db(panned_gain)
        );
    }

    #[test]
    fn decoded_loop_keeps_repeated_backing_and_reduces_a_new_centered_note() {
        let seconds = 8;
        let mut audio = Vec::with_capacity(RATE as usize * seconds);
        for sample in 0..RATE as usize * seconds {
            let time = sample as f32 / RATE as f32;
            let note = 1000.0 * 1.24f32.powf(time.floor());
            let backing = 0.22 * (2.0 * PI as f32 * 220.0 * time).sin()
                + 0.16 * (2.0 * PI as f32 * 330.0 * time).sin()
                + 0.12 * (2.0 * PI as f32 * 440.0 * time).sin();
            let beat_phase = sample % (RATE as usize / 2);
            let beat = if beat_phase < 2400 {
                0.3 * (2.0 * PI as f32 * 180.0 * time).sin()
            } else {
                0.0
            };
            let vocal = 0.2 * (2.0 * PI as f32 * note * time).sin();
            audio.push(backing + beat + vocal);
        }

        let mut repet = RepetAnalyzer::new();
        let mut latest = [1.0; REPET_BANDS];
        repet.feed(&audio, 0, |_, mask| latest = mask);
        let bass = latest[repet_band((220.0 * WINDOW as f32 / RATE as f32).round() as usize)];
        let voice_hz = 1000.0 * 1.24f32.powf((seconds - 1) as f32);
        let voice = latest[repet_band((voice_hz * WINDOW as f32 / RATE as f32).round() as usize)];
        let bass_db = 20.0 * suppression_target(0.95, 1.0, 1.0, bass).max(1.0e-6).log10();
        let voice_db = 20.0
            * suppression_target(0.95, 1.0, 1.0, voice)
                .max(1.0e-6)
                .log10();

        assert!(
            bass_db >= -2.0,
            "repeating bass changed by {bass_db:.1} dB (factor {bass})"
        );
        assert!(
            voice_db <= -20.0,
            "new melody reduced by {voice_db:.1} dB (factor {voice})"
        );
        eprintln!("REPET synthetic mix: bass {bass_db:.1} dB, melody {voice_db:.1} dB");
    }

    #[test]
    fn karaoke_mask_reset_rejects_old_track_results_and_returns_to_stage_one() {
        let ctl = KaraokeCtl::new();
        ctl.set(true, 1.0);
        ctl.prepare_repet_mask();
        let old_generation = ctl.generation();
        let mut repeated = [0.0; REPET_BANDS];
        ctl.publish_repet(old_generation, 1024, &repeated);
        assert_eq!(ctl.repet(1024, 100), 0.0);

        let generation = ctl.reset_repet();
        assert!(generation != old_generation);
        assert_eq!(ctl.repet(1024, 100), 1.0);
        ctl.publish_repet(old_generation, 1024, &repeated);
        assert_eq!(ctl.repet(1024, 100), 1.0);

        repeated.fill(0.0);
        ctl.set(false, 1.0);
        assert_eq!(ctl.get(), 0.0);
        assert_eq!(ctl.repet(1024, 100), 1.0);
    }

    #[test]
    fn repet_mask_is_selected_by_the_audio_frame_being_played() {
        let ctl = KaraokeCtl::new();
        ctl.set(true, 1.0);
        ctl.prepare_repet_mask();
        let mut mask = [0.0; REPET_BANDS];
        ctl.publish_repet(ctl.generation(), 2048, &mask);

        assert_eq!(ctl.repet(2048, 100), 0.0);
        assert_eq!(ctl.repet(2048 - 1, 100), 1.0);
        assert_eq!(ctl.repet((2048 + REPET_HOP * 2 + 1) as u64, 100), 1.0);

        mask.fill(1.0);
    }

    #[test]
    fn callback_applies_repet_mask_to_centered_material() {
        let measured_db = |non_repeating: f32| {
            let ctl = KaraokeCtl::new();
            ctl.set(true, 0.95);
            ctl.prepare_repet_mask();
            let mask = [non_repeating; REPET_BANDS];
            for frame in (0..RATE as usize * 2).step_by(REPET_HOP) {
                ctl.publish_repet(ctl.generation(), frame as u64, &mask);
            }
            let mut karaoke = Karaoke::new();
            let mut input_energy = 0.0f64;
            let mut output_energy = 0.0f64;
            let total = RATE as usize * 2;
            for start in (0..total).step_by(64) {
                let mut block = [0.0f32; 128];
                for frame in 0..64 {
                    let sample = start + frame;
                    let phase = 2.0 * PI * 440.0 * sample as f64 / RATE as f64;
                    block[frame * 2] = phase.sin() as f32;
                    block[frame * 2 + 1] = (phase + 0.05).sin() as f32;
                }
                let dry = block;
                karaoke.poll(&ctl);
                karaoke.process_with_repet(&mut block, 64, &ctl, start as u64);
                if start >= RATE as usize / 2 {
                    for (dry, wet) in dry.as_chunks::<2>().0.iter().zip(block.as_chunks::<2>().0) {
                        input_energy += f64::from(dry[0] * dry[0] + dry[1] * dry[1]);
                        output_energy += f64::from(wet[0] * wet[0] + wet[1] * wet[1]);
                    }
                }
            }
            10.0 * (output_energy / input_energy).log10()
        };

        let vocal_db = measured_db(1.0);
        let accompaniment_db = measured_db(0.0);
        assert!(
            vocal_db <= -20.0,
            "centered vocal changed by {vocal_db:.1} dB"
        );
        assert!(
            accompaniment_db >= -2.0,
            "repeating accompaniment changed by {accompaniment_db:.1} dB"
        );
        eprintln!(
            "REPET callback: centered vocal {vocal_db:.1} dB, repeating centered accompaniment {accompaniment_db:.1} dB"
        );
    }

    #[test]
    fn four_minute_repet_cpu_stays_below_three_percent_of_one_core() {
        use std::time::Instant;

        let mut analyzer = RepetAnalyzer::new();
        let signal: Vec<f32> = (0..REPET_HOP)
            .map(|sample| {
                let phase = sample as f32 / REPET_HOP as f32 * 2.0 * PI as f32;
                0.2 * phase.mul_add(10.0, 0.0).sin() + 0.1 * phase.mul_add(20.0, 0.0).sin()
            })
            .collect();
        let frames = RATE as usize * 240 / REPET_HOP;
        let start = Instant::now();
        for frame in 0..frames {
            analyzer.feed(&signal, (frame * REPET_HOP) as u64, |_, _| {});
        }
        let elapsed = start.elapsed().as_secs_f64();
        let core_fraction = elapsed / 240.0;
        eprintln!(
            "REPET: 4 min stream processed in {elapsed:.3}s ({:.2}% of one core)",
            core_fraction * 100.0
        );
        assert!(
            core_fraction <= 0.03,
            "REPET used {:.2}% of one core",
            core_fraction * 100.0
        );
    }
}
