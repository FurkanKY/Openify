//! Karaoke (vokal azaltma): çıkışın ses iş parçacığında, ham karelere
//! ekolayzırdan SONRA, zarf ve ses düzeyinden ÖNCE uygulanır (`source.rs` →
//! `shape`). Model yok, yapay ayırma yok: orta / yan (mid / side) ayrımı.
//!
//! # Yöntem
//!
//! `M = (L + R) / 2`, `S = (L − R) / 2`. Vokal çoğu kayıtta ortadadır (`M`),
//! yan kanal (`S`) stereo yayılımdır. Orta kanalın yalnız konuşma bandı
//! (150 Hz – 8 kHz) bastırılır; 150 Hz altı (bas, kick) ve 8 kHz üstü
//! (zil, hava) ortada da korunur:
//!
//! ```text
//! mid  = LR4_LP(8 kHz) · LR4_HP(150 Hz) · M          (bastırılan bant)
//! M'   = AP(M) − amount · mid
//! S'   = AP(S)
//! L'   = M' + S'     R' = M' − S'
//! ```
//!
//! `AP`, iki Linkwitz–Riley geçişinin (150 Hz, 8 kHz) toplamının tüm
//! geçiren (all-pass) süzgecidir: `AP(M) = alçak' + mid + yüksek` kesin
//! eşitlik (LR4 alçak + yüksek = 2. derece all-pass). Bu yüzden `amount = 1`
//! iken çıkış, orta kanalın yalnız alçak ve yüksek kısmı + tam yan kanaldır;
//! geçiş bölgelerinde çıkarma "faz kaymış bant geçiren çıkarma"nın kötü
//! sönümüne düşmez. Yan kanalın da aynı `AP`den geçmesi, `amount = 0`
//! iken iki kanalın aynı faz kaymasını görmesi, yani genlik ve stereo
//! görüntünün korunması içindir. Maliyet kare başına 8 biquad (4 bant, 2 + 2
//! faz); ekolayzırın 20'si yanında küçük.
//!
//! # Davranış
//!
//! - Kapalıyken (ya da miktar 0) dal HİÇ çalışmaz: `Karaoke::running` yanlışsa
//!   `shape` dokunmaz, çıkış bit bit eskisiyle aynı.
//! - Geçiş: açma / kapama ve miktar değişimi, kare kare doğrusal rampayla
//!   `GLIDE_BLOCKS` blokta (≈ 46 ms) yapılır; "ıslak / kuru" karışımı
//!   (`engage`) ve miktar (`amount`) ayrı rampalardır. Kapanınca süzgeç
//!   durumu sıfırlanır, dal durur.
//! - Mono kaynakta etkisiz: `S` ≈ 0 ise orta kanal bastırılırsa şarkı
//!   susar. Orta ve yan enerji, ~150 ms'lik sızıntılı toplayıcılarla
//!   ölçülür; yan / orta < −40 dB ise kaynak mono sayılır (−34 dB'ye çıkınca
//!   stereo; histerezis) ve karışım 0'a süzülür. Bir de hızlı yol: yan enerji
//!   orta enerjinin 1e-9'u bile değilse (örnek örnek aynı L = R, yani sayısal
//!   mono) 4 blok (~6 ms) sonra mono sayılır; boşluksuz geçişte stereo bir
//!   şarkıdan sonra gelen mono şarkıda ses ~1 sn susmasın. Arayüzde hiçbir
//!   şey göstermez: yalnız karaoke olmaz. Sessizlikte (ikisi de ~0) son karar
//!   korunur. Akış kesilince (`flush`) enerjiler ve durum sıfırlanır; yeni
//!   akışın ilk bloğunda mono çıkarsa karışım basamakla 0'a iner (akış zaten
//!   sessizlikten, girişle başlıyor).
//! - Denormal ve sonlu olmayan durumlar her blokta sıfıra çekilir.
//! - Ayırma ve kilit yok: bütün diziler gömülü; motordan ses iş parçacığına
//!   tek atomik (`KaraokeCtl`).

use std::{
    f64::consts::PI,
    sync::atomic::{AtomicU32, Ordering::Relaxed},
};

use super::RATE;

/// Bastırılan bandın alt kenarı: bunun altı ortada da korunur (Hz).
pub const LOW_HZ: f64 = 150.0;
/// Bastırılan bandın üst kenarı: bunun üstü ortada da korunur (Hz).
pub const HIGH_HZ: f64 = 8000.0;
/// Açma / kapama / miktar değişiminin tamamlandığı blok sayısı.
pub const GLIDE_BLOCKS: u32 = 32;
/// Yan / orta güç oranı bunun altına inerse kaynak mono sayılır (−40 dB).
const MONO_BELOW: f64 = 1.0e-4;
/// Mono sayılan kaynak, oran bunun üstüne çıkınca yeniden stereo (−34 dB).
const STEREO_ABOVE: f64 = 4.0e-4;
/// Hızlı yol: bir bloğun yan / orta güç oranı bunun altındaysa sayısal mono.
const EXACT_MONO: f64 = 1.0e-9;
/// Hızlı yol için üst üste kaç sayısal mono blok gerekir.
const EXACT_RUN: u32 = 4;
/// Enerji sızıntı çarpanı (blok başına): 64 / (0,15 sn · 44100) süre sabiti.
const LEAK: f64 = 0.990_38;
/// Orta + yan enerjisi bunun altındaysa (≈ sessizlik) mono kararı korunur.
const SILENCE: f64 = 1.0e-8;
/// Durum bu eşiğin altındaysa sıfırlanır (denormal kesme, bkz. `eq.rs`).
const FLUSH_BELOW: f32 = 1.0e-15;
/// Durum bunu aşarsa (patlama, sonlu olmayan) sıfırlanır.
const FLUSH_ABOVE: f32 = 1.0e6;

// ---------------------------------------------------------------------------
// Motor ↔ ses iş parçacığı
// ---------------------------------------------------------------------------

/// Paylaşılan karaoke ayarı (`OutputCtl::karaoke`): tek atomik, derinlik
/// (f32 bitleri). 0 = kapalı. Kilitsiz, yırtılma yok.
pub struct KaraokeCtl {
    depth: AtomicU32,
}

impl KaraokeCtl {
    pub fn new() -> Self {
        Self { depth: AtomicU32::new(0.0f32.to_bits()) }
    }

    /// Ayarı yayınlar (motor iş parçacığı). `amount` 0..1 sınırlanır; sonlu
    /// olmayan 0 olur.
    pub fn set(&self, enabled: bool, amount: f32) {
        self.depth.store(depth_of(enabled, amount).to_bits(), Relaxed);
    }

    #[inline]
    fn get(&self) -> f32 {
        f32::from_bits(self.depth.load(Relaxed))
    }
}

/// Etkin bastırma derinliği: kapalıysa 0, değilse 0..1.
fn depth_of(enabled: bool, amount: f32) -> f32 {
    if enabled && amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 }
}

// ---------------------------------------------------------------------------
// Ses iş parçacığı: süzgeç
// ---------------------------------------------------------------------------

/// Biquad katsayıları: b0, b1, b2, a1, a2 (a0'a bölünmüş).
type Coef = [f32; 5];

/// Butterworth (Q = 1/√2) 2. derece; `kind`: alçak / yüksek / all-pass (RBJ).
#[derive(Clone, Copy)]
enum Kind {
    Low,
    High,
    All,
}

fn biquad(kind: Kind, f0: f64) -> Coef {
    let w0 = 2.0 * PI * f0 / RATE as f64;
    let (s, c) = w0.sin_cos();
    let alpha = s / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = match kind {
        Kind::Low => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0),
        Kind::High => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0),
        Kind::All => (1.0 - alpha, -2.0 * c, 1.0 + alpha),
    };
    let (a1, a2) = (-2.0 * c, 1.0 - alpha);
    [(b0 / a0) as f32, (b1 / a0) as f32, (b2 / a0) as f32, (a1 / a0) as f32, (a2 / a0) as f32]
}

/// Bir biquad'ın durumu (doğrudan biçim II devrik).
#[derive(Clone, Copy, Default)]
struct Bq {
    z1: f32,
    z2: f32,
}

impl Bq {
    #[inline(always)]
    fn run(&mut self, c: &Coef, x: f32) -> f32 {
        let y = c[0] * x + self.z1;
        self.z1 = c[1] * x - c[3] * y + self.z2;
        self.z2 = c[2] * x - c[4] * y;
        y
    }
}

/// Bir süzgeç zinciri: iki LR4 bastırma bandı (HP 150 → LP 8 k, her biri iki
/// biquad), orta ve yan için all-pass çiftleri.
struct Chain {
    hp: [Bq; 2],
    lp: [Bq; 2],
    ap_m: [Bq; 2],
    ap_s: [Bq; 2],
}

impl Chain {
    const ZERO: Chain = Chain {
        hp: [Bq { z1: 0.0, z2: 0.0 }; 2],
        lp: [Bq { z1: 0.0, z2: 0.0 }; 2],
        ap_m: [Bq { z1: 0.0, z2: 0.0 }; 2],
        ap_s: [Bq { z1: 0.0, z2: 0.0 }; 2],
    };

    fn states(&mut self) -> impl Iterator<Item = &mut f32> {
        self.hp
            .iter_mut()
            .chain(self.lp.iter_mut())
            .chain(self.ap_m.iter_mut())
            .chain(self.ap_s.iter_mut())
            .flat_map(|b| [&mut b.z1, &mut b.z2])
    }

    fn clear(&mut self) {
        *self = Chain::ZERO;
    }
}

/// Ses iş parçacığına ait karaoke. Ayırma yok.
pub struct Karaoke {
    /// Etkin hedef derinlik (0 = kapalı).
    depth: f32,
    /// Geçerli bastırma miktarı (`depth`e süzülür).
    amount: f32,
    /// Islak / kuru karışım (0 kuru … 1 ıslak).
    engage: f32,
    /// Kaynak mono mu (bastırma bu durumda yok).
    mono: bool,
    /// Sızıntılı orta ve yan güç toplamları.
    e_mid: f64,
    e_side: f64,
    /// Üst üste sayısal mono (L = R) blok sayısı.
    exact_run: u32,
    /// Sıfırlandı, henüz blok işlenmedi: mono çıkarsa karışım basamakla iner.
    fresh: bool,
    /// Dal bu blokta çalışmalı mı.
    running: bool,
    /// Katsayılar: 150 Hz yüksek geçiren, 8 kHz alçak geçiren, 150 Hz ve 8 kHz
    /// all-pass.
    hp: Coef,
    lp: Coef,
    ap_lo: Coef,
    ap_hi: Coef,
    chain: Chain,
}

impl Karaoke {
    pub fn new() -> Self {
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
            hp: biquad(Kind::High, LOW_HZ),
            lp: biquad(Kind::Low, HIGH_HZ),
            ap_lo: biquad(Kind::All, LOW_HZ),
            ap_hi: biquad(Kind::All, HIGH_HZ),
            chain: Chain::ZERO,
        }
    }

    /// Dal bu blokta çalışmalı mı.
    #[inline]
    pub fn running(&self) -> bool {
        self.running
    }

    /// Yeni ayar yayınlandıysa alır (blok başına bir atomik okuma).
    #[inline]
    pub fn poll(&mut self, ctl: &KaraokeCtl) {
        self.depth = ctl.get();
        self.running = self.depth > 0.0 || self.engage > 0.0;
    }

    /// Durumu sıfırlar (kuyruk atıldı: akış kesildi).
    pub fn reset(&mut self) {
        self.chain.clear();
        self.e_mid = 0.0;
        self.e_side = 0.0;
        self.exact_run = 0;
        self.fresh = true;
    }

    /// `raw` (L, R, L, R, …) ilk `n` karesini işler. Her çağrı bir blok sayılır
    /// (rampalar, enerji sızıntısı).
    pub fn process(&mut self, raw: &mut [f32], n: usize) {
        // Mono ölçümü: yalnız bastırma istenirken (kapanırken karar değişmez).
        if self.depth > 0.0 {
            let (mut em, mut es) = (0.0f64, 0.0f64);
            for f in 0..n {
                let (l, r) = (raw[f * 2] as f64, raw[f * 2 + 1] as f64);
                let (m, s) = ((l + r) * 0.5, (l - r) * 0.5);
                em += m * m;
                es += s * s;
            }
            self.e_mid = self.e_mid * LEAK + em;
            self.e_side = self.e_side * LEAK + es;
            self.exact_run = if em >= SILENCE && es <= em * EXACT_MONO { self.exact_run.saturating_add(1) } else { 0 };
            if self.exact_run >= EXACT_RUN {
                // Sayısal mono: eski (stereo) geçmişi at, karar hemen mono.
                (self.e_mid, self.e_side, self.mono) = (em, es, true);
            } else if self.e_mid + self.e_side >= SILENCE {
                let ratio = self.e_side / self.e_mid.max(f64::MIN_POSITIVE);
                if self.mono {
                    self.mono = ratio < STEREO_ABOVE;
                } else {
                    self.mono = ratio < MONO_BELOW;
                }
            }
        }

        let engage_target = if self.depth > 0.0 && !self.mono { 1.0f32 } else { 0.0 };
        if self.fresh {
            self.fresh = false;
            if engage_target < self.engage {
                self.engage = engage_target;
            }
        }
        // Görünmezken (karışım 0) miktar doğrudan hedefe: rampa ıslak girişte.
        if self.engage == 0.0 {
            self.amount = self.depth;
        }
        if self.engage == 0.0 && engage_target == 0.0 {
            // Etkisiz blok (mono, ya da kapanmış): süzgeç durumu bayat kalmasın.
            self.chain.clear();
            self.running = self.depth > 0.0;
            return;
        }

        // Rampalar: blok başına hedefe en çok toplam yolun `1 / GLIDE_BLOCKS`'ı
        // kadar, blok içinde kare kare doğrusal (basamak yok).
        let step = 1.0 / GLIDE_BLOCKS as f32;
        let w0 = self.engage;
        let w1 = w0 + (engage_target - w0).clamp(-step, step);
        let a0 = self.amount;
        let a1 = a0 + (self.depth - a0).clamp(-step, step);
        let (dw, da) = ((w1 - w0) / n as f32, (a1 - a0) / n as f32);
        let (mut w, mut a) = (w0, a0);

        let (hp, lp, ap_lo, ap_hi) = (self.hp, self.lp, self.ap_lo, self.ap_hi);
        let c = &mut self.chain;
        for f in 0..n {
            w += dw;
            a += da;
            let (l, r) = (raw[f * 2], raw[f * 2 + 1]);
            let (m, s) = ((l + r) * 0.5, (l - r) * 0.5);
            // Bastırılan bant: HP150 (LR4) → LP8k (LR4).
            let mut band = c.hp[0].run(&hp, m);
            band = c.hp[1].run(&hp, band);
            band = c.lp[0].run(&lp, band);
            band = c.lp[1].run(&lp, band);
            // Faz eşleme: iki kanal da aynı all-pass'tan geçer.
            let m_lo = c.ap_m[0].run(&ap_lo, m);
            let m_ap = c.ap_m[1].run(&ap_hi, m_lo);
            let s_lo = c.ap_s[0].run(&ap_lo, s);
            let s_ap = c.ap_s[1].run(&ap_hi, s_lo);
            let mp = m_ap - a * band;
            let (wl, wr) = (mp + s_ap, mp - s_ap);
            raw[f * 2] = l + w * (wl - l);
            raw[f * 2 + 1] = r + w * (wr - r);
        }
        self.engage = w1;
        self.amount = a1;

        // Denormal ve patlama kesme.
        for v in c.states() {
            let abs = v.abs();
            if !(abs >= FLUSH_BELOW && abs <= FLUSH_ABOVE) {
                *v = 0.0;
            }
        }
        self.running = self.depth > 0.0 || self.engage > 0.0;
        if !self.running {
            c.clear();
        }
    }

    /// Testler: karışım (0 kuru … 1 ıslak).
    #[cfg(test)]
    pub fn engage(&self) -> f32 {
        self.engage
    }

    /// Testler: kaynak mono sayılıyor mu.
    #[cfg(test)]
    pub fn is_mono(&self) -> bool {
        self.mono
    }

    /// Testler: süzgeç durumu tamamen sıfır mı.
    #[cfg(test)]
    pub fn state_is_zero(&mut self) -> bool {
        self.chain.states().all(|v| *v == 0.0)
    }
}

/// Testler: motor komutunun çıkışa ulaştığını görmek için (`OutputCtl::karaoke`
/// alanı özel; çocuk modülden okunur).
#[cfg(test)]
impl super::OutputCtl {
    /// Yayınlanan son ayar: (açık mı, derinlik 0..1).
    pub(in crate::engine) fn karaoke_published(&self) -> (bool, f32) {
        let depth = self.karaoke.get();
        (depth > 0.0, depth)
    }
}
