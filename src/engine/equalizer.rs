//! Ekolayzer: kullanıcı ayarı, hazır ayarlar ve bir kulaklık profilinin
//! (parametrik EQ) 10 sabit banta yaklaştırılması.
//!
//! Çıkış hattındaki süzgeç (`output/eq.rs`) 10 sabit tepe (peaking) bandı
//! çalar: merkezler 31,5 Hz … 16 kHz, Q ≈ 1,41, kazanç ±24 dB'ye dek. Arayüz
//! ±12 dB, 0,5 dB adım sunar. Parametrik bir profil (OPRA: tepe, raf, alçak
//! / yüksek geçiren süzgeçler; bkz. `opra.rs`) bu bantlara en küçük kareler
//! ile uydurulur: hedef, profilin bileşik yanıtı (dB, log frekans ızgarası);
//! model, 10 bandın GERÇEK yanıtı (`output::eq::band_response_db`, motorun
//! kullandığı katsayılarla) artı bir sabit kayma (düzey önemsiz: otomatik
//! ön kazanç düzeyi zaten ayarlar). Gauss–Newton, kazançlar ±12 dB'e
//! kırpılarak; yaklaşım hatası (RMS ve en büyük sapma, dB) sonuçla gelir ve
//! testlerde ölçülür.

pub use super::output::eq::BANDS;
use super::output::eq::band_response_db;

/// Arayüzün sunduğu bant kazancı sınırı (dB, ±).
pub const RANGE_DB: f32 = 12.0;
/// Arayüz adımı (dB).
pub const STEP_DB: f32 = 0.5;
/// Ön kazanç sınırı (dB, ±).
pub const PREAMP_RANGE_DB: f32 = 12.0;

/// Ekolayzer ayarı: motora `Command::SetEq` olarak gider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSetting {
    pub enabled: bool,
    pub gains_db: [f32; BANDS],
    /// Elle ön kazanç (dB). Motor bundan, bileşik yanıtın artı tepesini
    /// (otomatik ön kazanç) çıkarır.
    pub preamp_db: f32,
}

impl Default for EqSetting {
    fn default() -> Self {
        Self {
            enabled: false,
            gains_db: [0.0; BANDS],
            preamp_db: 0.0,
        }
    }
}

impl EqSetting {
    /// Sınırlara çeker; sonlu olmayanlar 0 olur.
    pub fn sanitized(mut self) -> Self {
        for g in &mut self.gains_db {
            *g = clamp_db(*g, RANGE_DB);
        }
        self.preamp_db = clamp_db(self.preamp_db, PREAMP_RANGE_DB);
        self
    }
}

fn clamp_db(v: f32, range: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-range, range)
    } else {
        0.0
    }
}

/// Bir ayarı 0,5 dB'lik adıma yuvarlar ve sınırlar (sürgü, klavye).
pub fn snap_db(v: f32) -> f32 {
    (clamp_db(v, RANGE_DB) / STEP_DB).round() * STEP_DB
}

/// Hazır ayarlar: (anahtar, 10 bantın kazancı). Az ve sade: Düz, Bas artır,
/// Tiz artır, Vokal, Akustik. Hepsi ±12 dB içinde, 0,5 dB adımında.
pub const PRESETS: [(&str, [f32; BANDS]); 5] = [
    ("flat", [0.0; BANDS]),
    ("bass", [6.0, 5.5, 4.5, 2.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("treble", [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 1.5, 3.0, 4.5, 5.5]),
    (
        "vocal",
        [-2.0, -2.0, -1.5, -0.5, 1.0, 3.0, 3.5, 2.5, 0.5, -0.5],
    ),
    (
        "acoustic",
        [3.5, 3.0, 2.0, 0.5, 1.0, 1.0, 2.0, 2.5, 3.0, 2.0],
    ),
];

/// Bu kazançlar hangi hazır ayara eşit (yoksa `None`: elle ya da profil)?
pub fn preset_of(gains_db: &[f32; BANDS]) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(_, g)| g.iter().zip(gains_db).all(|(a, b)| (a - b).abs() < 0.01))
        .map(|(k, _)| *k)
}

// ── Parametrik profil ──────────────────────────────────────────────────

/// Parametrik EQ süzgeç türü (OPRA `eq_info.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeqKind {
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    BandPass,
    BandStop,
}

impl PeqKind {
    /// OPRA'daki ad.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "peak_dip" => PeqKind::Peak,
            "low_shelf" => PeqKind::LowShelf,
            "high_shelf" => PeqKind::HighShelf,
            "low_pass" => PeqKind::LowPass,
            "high_pass" => PeqKind::HighPass,
            "band_pass" => PeqKind::BandPass,
            "band_stop" => PeqKind::BandStop,
            _ => return None,
        })
    }
}

/// Bir parametrik süzgeç: frekans (Hz), kazanç (dB), Q; alçak / yüksek
/// geçirenlerde Q yerine eğim (dB / oktav).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeqBand {
    pub kind: PeqKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    pub slope: f32,
}

/// Süzgeçlerin hesap örnekleme hızı (motorun çıkış hızı).
const RATE: f64 = super::output::RATE as f64;

impl PeqBand {
    /// `f` Hz'teki genlik yanıtı (dB). İkinci dereceden türler RBJ
    /// "audio EQ cookbook" katsayılarıyla; alçak / yüksek geçiren Butterworth.
    pub fn response_db(&self, f: f64) -> f64 {
        let f0 = (self.freq as f64).clamp(1.0, RATE / 2.0 - 1.0);
        let q = (self.q as f64).max(0.1);
        match self.kind {
            PeqKind::LowPass | PeqKind::HighPass => {
                let n = (self.slope as f64 / 6.0).clamp(1.0, 6.0);
                let ratio = if self.kind == PeqKind::LowPass {
                    f / f0
                } else {
                    f0 / f
                };
                -10.0 * (1.0 + ratio.powf(2.0 * n)).log10()
            }
            _ => biquad_db(self.kind, f0, self.gain_db as f64, q, f),
        }
    }
}

fn biquad_db(kind: PeqKind, f0: f64, gain_db: f64, q: f64, f: f64) -> f64 {
    use std::f64::consts::PI;
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * f0 / RATE;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * q);
    let sq = 2.0 * a.sqrt() * alpha;
    let (b, d) = match kind {
        PeqKind::Peak => (
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        ),
        PeqKind::LowShelf => (
            [
                a * ((a + 1.0) - (a - 1.0) * cos + sq),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - sq),
            ],
            [
                (a + 1.0) + (a - 1.0) * cos + sq,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - sq,
            ],
        ),
        PeqKind::HighShelf => (
            [
                a * ((a + 1.0) + (a - 1.0) * cos + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - sq),
            ],
            [
                (a + 1.0) - (a - 1.0) * cos + sq,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - sq,
            ],
        ),
        PeqKind::BandPass => ([alpha, 0.0, -alpha], [1.0 + alpha, -2.0 * cos, 1.0 - alpha]),
        PeqKind::BandStop => (
            [1.0, -2.0 * cos, 1.0],
            [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        ),
        PeqKind::LowPass | PeqKind::HighPass => return 0.0,
    };
    let w = 2.0 * PI * f / RATE;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let num = (b[0] + b[1] * c1 + b[2] * c2).powi(2) + (b[1] * s1 + b[2] * s2).powi(2);
    let den = (d[0] + d[1] * c1 + d[2] * c2).powi(2) + (d[1] * s1 + d[2] * s2).powi(2);
    10.0 * (num / den).max(1e-30).log10()
}

/// Profilin bileşik yanıtı (dB) `f` Hz'te.
pub fn profile_response_db(bands: &[PeqBand], f: f64) -> f64 {
    bands.iter().map(|b| b.response_db(f)).sum()
}

// ── En küçük kareler ───────────────────────────────────────────────────

/// Uydurma ızgarası: 20 Hz – 20 kHz, 1/24 oktav.
const GRID_N: usize = 241;
/// Hatanın raporlandığı çekirdek aralık (Hz): bantların kapsadığı yer.
const CORE_HZ: (f64, f64) = (30.0, 16_000.0);
/// Kazançlara küçük bir ceza: komşu bantların birbirini götüren ±12 dB
/// çiftlerine kaymasını önler (birimi dB² / bant).
const RIDGE: f64 = 0.4;
const ITERATIONS: usize = 24;

/// Uydurma sonucu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// 10 bandın kazançları (dB, 0,1 dB'e yuvarlı, ±12 içinde).
    pub gains_db: [f32; BANDS],
    /// Çekirdek aralıkta (30 Hz – 16 kHz) hedefle fark: karekök ortalama (dB).
    pub rms_db: f32,
    /// Aynı aralıkta en büyük sapma (dB).
    pub max_db: f32,
    /// Hedefin kendisinin aynı aralıkta (sabit kaymadan arındırılmış) RMS'i:
    /// "hiçbir şey yapmasaydık" hata payı; uydurma bunun ne kadarını kapattı.
    pub target_rms_db: f32,
}

fn grid() -> Vec<f64> {
    (0..GRID_N)
        .map(|i| 20.0 * (1000f64).powf(i as f64 / (GRID_N - 1) as f64))
        .collect()
}

/// `bands` profilini 10 sabit banta uydurur.
pub fn fit(bands: &[PeqBand]) -> Fit {
    let freqs = grid();
    let target: Vec<f64> = freqs
        .iter()
        .map(|&f| profile_response_db(bands, f))
        .collect();
    fit_target(&freqs, &target)
}

/// Verilen hedef eğriye (dB, `freqs` üzerinde) uydurur.
pub fn fit_target(freqs: &[f64], target: &[f64]) -> Fit {
    let n = freqs.len();
    let model = |g: &[f64; BANDS]| -> Vec<f64> {
        freqs
            .iter()
            .map(|&f| (0..BANDS).map(|b| band_response_db(b, g[b], f)).sum())
            .collect()
    };
    // Sabit kayma en küçük kareler çözümüyle ortalamaya oturur.
    let offset = |m: &[f64]| (0..n).map(|j| target[j] - m[j]).sum::<f64>() / n as f64;
    let cost = |g: &[f64; BANDS]| -> f64 {
        let m = model(g);
        let c = offset(&m);
        (0..n).map(|j| (target[j] - m[j] - c).powi(2)).sum::<f64>()
            + RIDGE * g.iter().map(|x| x * x).sum::<f64>()
    };
    let mut g = [0.0f64; BANDS];
    let mut best = cost(&g);
    for _ in 0..ITERATIONS {
        let m = model(&g);
        let c = offset(&m);
        // Sayısal Jacobian: her bant yalnız kendi kazancına bağlı.
        const D: f64 = 0.05;
        let mut jac = vec![[0.0f64; BANDS + 1]; n];
        for (j, &f) in freqs.iter().enumerate() {
            for b in 0..BANDS {
                jac[j][b] = (band_response_db(b, g[b] + D, f) - band_response_db(b, g[b] - D, f))
                    / (2.0 * D);
            }
            jac[j][BANDS] = 1.0;
        }
        // Normal denklemler: (JᵀJ + ridge) Δ = Jᵀ r − ridge·g.
        let mut a = [[0.0f64; BANDS + 1]; BANDS + 1];
        let mut rhs = [0.0f64; BANDS + 1];
        for j in 0..n {
            let r = target[j] - m[j] - c;
            for p in 0..=BANDS {
                rhs[p] += jac[j][p] * r;
                for q in 0..=BANDS {
                    a[p][q] += jac[j][p] * jac[j][q];
                }
            }
        }
        for b in 0..BANDS {
            a[b][b] += RIDGE + 1e-6;
            rhs[b] -= RIDGE * g[b];
        }
        a[BANDS][BANDS] += 1e-6;
        let Some(step) = solve(a, rhs) else { break };
        // Geri izleme: maliyet artarsa adım yarılanır.
        let mut scale = 1.0;
        let mut moved = false;
        for _ in 0..6 {
            let mut trial = g;
            for b in 0..BANDS {
                trial[b] = (g[b] + scale * step[b]).clamp(-(RANGE_DB as f64), RANGE_DB as f64);
            }
            let cst = cost(&trial);
            if cst < best - 1e-9 {
                g = trial;
                best = cst;
                moved = true;
                break;
            }
            scale *= 0.5;
        }
        if !moved {
            break;
        }
    }
    // Arayüzde 0,1 dB duyarlık yeter; hata yuvarlanmış kazançlarla ölçülür.
    let mut gains = [0.0f32; BANDS];
    for b in 0..BANDS {
        gains[b] = ((g[b] * 10.0).round() / 10.0) as f32;
    }
    let rounded: [f64; BANDS] = std::array::from_fn(|b| gains[b] as f64);
    let m = model(&rounded);
    let c = offset(&m);
    let (mut sq, mut max, mut tsq, mut count) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    let tmean = target.iter().sum::<f64>() / n as f64;
    for j in 0..n {
        if freqs[j] < CORE_HZ.0 || freqs[j] > CORE_HZ.1 {
            continue;
        }
        let e = target[j] - m[j] - c;
        sq += e * e;
        max = max.max(e.abs());
        tsq += (target[j] - tmean).powi(2);
        count += 1;
    }
    let count = count.max(1) as f64;
    Fit {
        gains_db: gains,
        rms_db: (sq / count).sqrt() as f32,
        max_db: max as f32,
        target_rms_db: (tsq / count).sqrt() as f32,
    }
}

/// Gauss yok etme (kısmi eksen seçimi); tekilse `None`.
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for col in 0..N {
        let piv = (col..N).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in col + 1..N {
            let k = a[row][col] / a[col][col];
            for c in col..N {
                a[row][c] -= k * a[col][c];
            }
            b[row] -= k * b[col];
        }
    }
    let mut x = [0.0; N];
    for row in (0..N).rev() {
        let s: f64 = (row + 1..N).map(|c| a[row][c] * x[c]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::output::eq::{Params, response_db};

    fn peak(freq: f32, gain_db: f32, q: f32) -> PeqBand {
        PeqBand {
            kind: PeqKind::Peak,
            freq,
            gain_db,
            q,
            slope: 12.0,
        }
    }

    fn shelf(kind: PeqKind, freq: f32, gain_db: f32) -> PeqBand {
        PeqBand {
            kind,
            freq,
            gain_db,
            q: 0.7,
            slope: 12.0,
        }
    }

    /// Tepe süzgecinin merkezdeki yanıtı tam kazancıdır; raflar uçta kazancına varır.
    #[test]
    fn target_filters_have_the_documented_gain() {
        assert!((peak(1000.0, 6.0, 1.0).response_db(1000.0) - 6.0).abs() < 1e-6);
        assert!((shelf(PeqKind::LowShelf, 200.0, 5.0).response_db(20.0) - 5.0).abs() < 0.1);
        assert!(
            shelf(PeqKind::LowShelf, 200.0, 5.0)
                .response_db(10_000.0)
                .abs()
                < 0.1
        );
        assert!((shelf(PeqKind::HighShelf, 8000.0, -4.0).response_db(20_000.0) + 4.0).abs() < 0.5);
        let hp = PeqBand {
            kind: PeqKind::HighPass,
            freq: 100.0,
            gain_db: 0.0,
            q: 0.7,
            slope: 12.0,
        };
        assert!(
            (hp.response_db(100.0) + 3.0103).abs() < 0.01,
            "Butterworth kesim noktası -3 dB"
        );
        assert!(hp.response_db(10_000.0).abs() < 1e-3);
    }

    /// Hedef zaten 10 bandın yanıtıysa uydurma onu bulur (hata ~0).
    #[test]
    fn fit_recovers_a_target_made_of_the_ten_bands() {
        let truth = [3.0f32, -2.0, 1.5, 0.0, -4.0, 2.5, 5.0, -1.0, 3.5, -2.5];
        let freqs = grid();
        let target: Vec<f64> = freqs.iter().map(|&f| response_db(&truth, f)).collect();
        let fit = fit_target(&freqs, &target);
        assert!(fit.rms_db < 0.15, "rms {}", fit.rms_db);
        assert!(fit.max_db < 0.4, "max {}", fit.max_db);
    }

    /// Bir yaygın AutoEQ biçimi: alçak raf + birkaç tepe + yüksek raf. Sentetik.
    fn synthetic_profile() -> Vec<PeqBand> {
        vec![
            shelf(PeqKind::LowShelf, 105.0, 3.5),
            peak(170.0, -1.9, 1.47),
            peak(1400.0, -1.3, 2.18),
            peak(3000.0, 4.0, 1.2),
            peak(7619.0, -3.3, 4.48),
            shelf(PeqKind::HighShelf, 10_000.0, -2.2),
        ]
    }

    /// 10 banda yaklaştırma hatası: yumuşak (düşük Q'lu) bir profilde RMS 1 dB'in
    /// altında; uydurma hedefin kendi sapmasının büyük kısmını kapatır.
    #[test]
    fn fit_error_on_a_typical_profile_is_small() {
        let f = fit(&synthetic_profile());
        println!(
            "sentetik profil: rms {:.3} dB, en büyük {:.3} dB, hedef rms {:.3} dB, kazançlar {:?}",
            f.rms_db, f.max_db, f.target_rms_db, f.gains_db
        );
        assert!(f.rms_db < 1.0, "rms {}", f.rms_db);
        assert!(f.max_db < 3.0, "en büyük {}", f.max_db);
        assert!(
            f.rms_db < 0.5 * f.target_rms_db,
            "uydurma hedefin sapmasını yarıya indirmedi"
        );
        assert!(
            f.gains_db.iter().all(|g| g.abs() <= RANGE_DB),
            "{:?}",
            f.gains_db
        );
    }

    /// Düz profil: bütün kazançlar sıfır.
    #[test]
    fn flat_profile_fits_to_zero() {
        let f = fit(&[]);
        assert!(f.gains_db.iter().all(|g| *g == 0.0), "{:?}", f.gains_db);
        let f = fit(&[peak(1000.0, 0.0, 1.0)]);
        assert!(
            f.gains_db.iter().all(|g| g.abs() < 0.05),
            "{:?}",
            f.gains_db
        );
    }

    /// Aşırı bir profil bile ±12 dB sınırında kalır.
    #[test]
    fn fit_respects_the_range() {
        let extreme = [
            shelf(PeqKind::LowShelf, 300.0, 24.0),
            shelf(PeqKind::HighShelf, 6000.0, -24.0),
        ];
        let f = fit(&extreme);
        assert!(
            f.gains_db.iter().all(|g| g.abs() <= RANGE_DB + 1e-6),
            "{:?}",
            f.gains_db
        );
    }

    /// Hazır ayarlar ±12 dB içinde, 0,5 dB adımında; adlar benzersiz; hepsi bir
    /// hazır ayar olarak tanınır.
    #[test]
    fn presets_stay_within_range_and_step() {
        let mut keys = std::collections::HashSet::new();
        for (key, gains) in PRESETS {
            assert!(keys.insert(key), "{key} iki kez");
            for g in gains {
                assert!(g.abs() <= RANGE_DB, "{key}: {g}");
                assert!(
                    ((g / STEP_DB).round() * STEP_DB - g).abs() < 1e-6,
                    "{key}: {g} adıma oturmuyor"
                );
            }
            assert_eq!(preset_of(&gains), Some(key));
        }
        assert_eq!(PRESETS[0].0, "flat");
        assert_eq!(preset_of(&[1.0; BANDS]), None);
    }

    /// Otomatik ön kazanç: hazır ayarlar ve komşu bantları birlikte artıran
    /// ayarlar için bileşik yanıt (ön kazançla) 0 dB'i aşmaz.
    #[test]
    fn automatic_preamp_prevents_clipping() {
        let mut sets: Vec<[f32; BANDS]> = PRESETS.iter().map(|(_, g)| *g).collect();
        sets.push([12.0; BANDS]);
        sets.push([6.0, 6.0, 6.0, 6.0, 6.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        sets.push([0.0, 0.0, 0.0, 9.0, 12.0, 9.0, 0.0, 0.0, 0.0, 0.0]);
        sets.push([-6.0, 3.0, -6.0, 3.0, -6.0, 3.0, -6.0, 3.0, -6.0, 3.0]);
        for g in sets {
            let p = Params::from_gains(&g, 0.0);
            let pre_db = 20.0 * (p.pre as f64).log10();
            let mut f = 20.0f64;
            let mut worst = f64::NEG_INFINITY;
            while f <= 20_000.0 {
                worst = worst.max(response_db(&g, f) + pre_db);
                f *= 2f64.powf(1.0 / 48.0);
            }
            assert!(worst <= 0.02, "{g:?}: en yüksek yanıt {worst:.3} dB");
            // Düz ayarda ön kazanç hiç değişmez.
            if g.iter().all(|x| *x == 0.0) {
                assert_eq!(p.pre, 1.0);
            }
        }
        // Elle ön kazanç eksilmeden eklenir: +3 dB isteyen +3 dB fazlasını alır.
        let g = PRESETS[1].1;
        let (a, b) = (
            Params::from_gains(&g, 0.0).pre,
            Params::from_gains(&g, 3.0).pre,
        );
        assert!((20.0 * ((b / a) as f64).log10() - 3.0).abs() < 0.01);
    }

    /// Artı bantlar yan yana ise tek bant tabanlı eski pay yetmezdi: yeni pay
    /// onu aşar (kırpmayı gerçekten önlediğinin kanıtı).
    #[test]
    fn neighbouring_boosts_need_more_than_the_largest_band() {
        let g = [0.0, 0.0, 0.0, 9.0, 12.0, 9.0, 0.0, 0.0, 0.0, 0.0];
        assert!(crate::engine::output::eq::peak_response_db(&g) > 12.5);
    }

    #[test]
    fn setting_sanitizes() {
        let s = EqSetting {
            enabled: true,
            gains_db: [99.0, f32::NAN, -99.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            preamp_db: 50.0,
        }
        .sanitized();
        assert_eq!(s.gains_db[0], RANGE_DB);
        assert_eq!(s.gains_db[1], 0.0);
        assert_eq!(s.gains_db[2], -RANGE_DB);
        assert_eq!(s.preamp_db, PREAMP_RANGE_DB);
        assert_eq!(snap_db(0.74), 0.5);
        assert_eq!(snap_db(0.76), 1.0);
        assert_eq!(snap_db(40.0), RANGE_DB);
    }
}
