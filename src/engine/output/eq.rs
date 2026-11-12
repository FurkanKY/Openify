//! 10 bantlı ekolayzer: çıkışın ses iş
//! parçacığında, halkadan okunan ham karelere, zarf ve ses düzeyi ÖNCE
//! uygulanır (`source.rs` → `shape`). Ses düzeyi ve zarf ekolayzırın
//! durumunu bozmaz: iniş / çıkış, duraklatma, ses düzeyi yalnız sonra
//! çarpılır.
//!
//! # Yapı
//!
//! - `EqCtl` (motor ↔ ses iş parçacığı): katsayılar atomik hücrelerde, bir
//!   seqlock ile. Yazar tek (motor iş parçacığı; birden çok yazar çıksa bile
//!   yazma sırası bir döndürme kilidiyle seri hâle gelir), okur beklemez:
//!   yırtık okumayı bırakır, bir sonraki blokta yeniden dener. Katsayıları
//!   (cos / sin / pow) motor hesaplar; ses iş parçacığı yalnız kopyalar.
//! - `Eq` (ses iş parçacığına ait): hedef ve geçerli katsayılar, durum.
//!   10 tepe (peaking) biquad, doğrudan biçim II devrik, f32; merkezler ISO
//!   benzeri 31,5 Hz … 16 kHz, Q ≈ 1,41.
//!
//! # Davranış
//!
//! - Kapalıyken dal HİÇ çalışmaz: `Eq::running` yanlışsa `shape` ekolayzıra
//!   dokunmaz; çıkış bit bit eskisiyle aynı.
//! - Katsayı değişince (ayar, açma, kapama) geçerli katsayılar hedefe 8
//!   blokta doğrusal yaklaşır; ikinci derece kutupların (a1, a2) dışbükey
//!   kümesi kararlıdır, yani ara değerler de kararlıdır. Açarken eski
//!   birim katsayılardan, kapatırken birime doğru süzülür; birime varınca dal
//!   kapanır.
//! - Sıfır kazançlı bant birim katsayılıdır ve hiç hesaplanmaz; bütün bantlar
//!   sıfırsa ve ön kazanç 1,0 ise açık ekolayzır da çıkışı değiştirmez
//!   (bit bit).
//! - Durum `flush`ta sıfırlanır (kuyruk atıldı, akış kesildi), denormal ve
//!   sonlu olmayan durumlar her blokta sıfıra çekilir.
//! - Otomatik ön kazanç: `önkazanç_dB − max(0, bileşik yanıtın tepesi)`
//!   (`peak_response_db`: 10 bandın toplam yanıtının 20 Hz – 20 kHz'teki
//!   en büyük değeri). Tek bantta tepe bant kazancının kendisidir; yan yana
//!   artı bantların üst üste binen etekleri de hesaba girer, böylece
//!   çıkış, bu ızgarada, girişi aşmaz (11 Kasım'a dek yalnız en büyük tek
//!   bant çıkarılıyordu ve komşu bantlar kırpmaya yol açabiliyordu).

use std::{
    f64::consts::PI,
    sync::atomic::{AtomicU32, Ordering::*, fence},
};

use super::RATE;

pub const BANDS: usize = 10;
/// Bant merkezleri (Hz).
pub const CENTERS_HZ: [f32; BANDS] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Her bantın kalite çarpanı.
const Q: f64 = 1.41;
/// Katsayıların hedefe vardığı blok sayısı.
pub const GLIDE_BLOCKS: u32 = 8;
/// Bant kazancı ve ön kazanç sınırı (dB, ±).
pub const MAX_DB: f32 = 24.0;
/// Bundan küçük kazanç sıfır sayılır (birim katsayı).
const MIN_DB: f32 = 0.005;
/// Durum bu eşiğin altındaysa sıfırlanır (denormal kesme); eşik, bir
/// bloğun (64 kare) içinde en hızlı sönümlenmede bile denormale düşmeyecek
/// kadar yüksek, işitme için sonsuz küçük (-300 dB).
const FLUSH_BELOW: f32 = 1.0e-15;
/// Durum bunu aşarsa (patlama, sonlu olmayan) sıfırlanır.
const FLUSH_ABOVE: f32 = 1.0e6;

/// Biquad katsayıları: b0, b1, b2, a1, a2 (a0'a bölünmüş).
pub type Coef = [f32; 5];
const IDENT: Coef = [1.0, 0.0, 0.0, 0.0, 0.0];

/// Bir ekolayzır ayarı: bant katsayıları ve ön kazanç (doğrusal çarpan).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    pub coef: [Coef; BANDS],
    pub pre: f32,
}

impl Params {
    pub const IDENT: Params = Params { coef: [IDENT; BANDS], pre: 1.0 };

    /// Kazançlardan (dB) katsayıları ve otomatik ön kazancı hesaplar (motor
    /// iş parçacığında: sin / cos / pow burada). Otomatik pay, bileşik
    /// yanıtın GERÇEK tepesidir (`peak_response_db`): yan yana artı bantlar
    /// birbirinin eteğini toplar, en büyük tek banttan fazlasını verir.
    pub fn from_gains(gains_db: &[f32; BANDS], preamp_db: f32) -> Params {
        let mut p = Params::IDENT;
        let mut g = [0.0f32; BANDS];
        for i in 0..BANDS {
            g[i] = sane_db(gains_db[i]);
            if g[i].abs() >= MIN_DB {
                p.coef[i] = peaking(CENTERS_HZ[i] as f64, g[i] as f64);
            } else {
                g[i] = 0.0;
            }
        }
        let boost = peak_response_db(&g).max(0.0) as f32;
        let pre_db = sane_db(preamp_db) - boost;
        p.pre = if pre_db == 0.0 { 1.0 } else { 10f64.powf(pre_db as f64 / 20.0) as f32 };
        p
    }

    fn is_identity(&self) -> bool {
        *self == Params::IDENT
    }

    /// Hangi bantlar birim değil (bit i = bant i).
    fn active_mask(&self) -> u16 {
        (0..BANDS).fold(0u16, |m, i| if self.coef[i] != IDENT { m | 1 << i } else { m })
    }
}

fn sane_db(db: f32) -> f32 {
    if db.is_finite() { db.clamp(-MAX_DB, MAX_DB) } else { 0.0 }
}

/// RBJ tepe (peaking) süzgeci; merkezde kazanç tam `gain_db`.
fn peaking(f0: f64, gain_db: f64) -> Coef {
    let c = peaking_f64(f0, gain_db);
    [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32]
}

/// `peaking`in çift duyarlıklı hâli (b0, b1, b2, a1, a2; a0'a bölünmüş).
fn peaking_f64(f0: f64, gain_db: f64) -> [f64; 5] {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * f0 / RATE as f64;
    let alpha = w0.sin() / (2.0 * Q);
    let c = w0.cos();
    let a0 = 1.0 + alpha / a;
    [
        (1.0 + alpha * a) / a0,
        -2.0 * c / a0,
        (1.0 - alpha * a) / a0,
        -2.0 * c / a0,
        (1.0 - alpha / a) / a0,
    ]
}

/// Tek bir bandın `freq` Hz'teki genlik yanıtı (dB): `gain_db` kazançlı tepe
/// süzgeci, `from_gains` ile aynı katsayılardan.
pub fn band_response_db(band: usize, gain_db: f64, freq: f64) -> f64 {
    if gain_db.abs() < MIN_DB as f64 {
        return 0.0;
    }
    let [b0, b1, b2, a1, a2] = peaking_f64(CENTERS_HZ[band] as f64, gain_db);
    let w = 2.0 * PI * freq / RATE as f64;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let num = (b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2);
    let den = (1.0 + a1 * c1 + a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2);
    10.0 * (num / den).log10()
}

/// On bandın bileşik genlik yanıtı (dB) `freq` Hz'te (ön kazanç yok).
pub fn response_db(gains_db: &[f32; BANDS], freq: f64) -> f64 {
    (0..BANDS).map(|b| band_response_db(b, sane_db(gains_db[b]) as f64, freq)).sum()
}

/// Bileşik yanıtın 20 Hz – 20 kHz arasındaki en büyük değeri (dB): 1/12
/// oktav ızgarası + bant merkezleri, en iyi noktanın iki yanında altın
/// oran araması (çok bantlı toplamın tepesi ızgara noktasına denk gelmez;
/// kaba ızgara büyük artılarda 0,3 dB'e dek aşağıda kalırdı).
pub fn peak_response_db(gains_db: &[f32; BANDS]) -> f64 {
    if gains_db.iter().all(|g| g.abs() < MIN_DB) {
        return 0.0;
    }
    let step = 2f64.powf(1.0 / 12.0);
    let (mut best_f, mut best) = (20.0f64, f64::NEG_INFINITY);
    let mut f = 20.0f64;
    while f <= 20_000.0 {
        let r = response_db(gains_db, f);
        if r > best {
            (best_f, best) = (f, r);
        }
        f *= step;
    }
    for c in CENTERS_HZ {
        let r = response_db(gains_db, c as f64);
        if r > best {
            (best_f, best) = (c as f64, r);
        }
    }
    // Altın oran: [best_f / step, best_f * step] içinde (log ekseninde).
    const PHI: f64 = 0.618_033_988_749_895;
    let (mut lo, mut hi) = ((best_f / step).max(10.0).ln(), (best_f * step).min(24_000.0).ln());
    let at = |x: f64| response_db(gains_db, x.exp());
    let (mut x1, mut x2) = (hi - PHI * (hi - lo), lo + PHI * (hi - lo));
    let (mut f1, mut f2) = (at(x1), at(x2));
    for _ in 0..32 {
        if f1 < f2 {
            lo = x1;
            (x1, f1) = (x2, f2);
            x2 = lo + PHI * (hi - lo);
            f2 = at(x2);
        } else {
            hi = x2;
            (x2, f2) = (x1, f1);
            x1 = hi - PHI * (hi - lo);
            f1 = at(x1);
        }
    }
    best.max(f1).max(f2)
}

// ---------------------------------------------------------------------------
// Motor ↔ ses iş parçacığı: seqlock
// ---------------------------------------------------------------------------

/// Paylaşılan ekolayzır ayarı (`OutputCtl::eq`). Yazma motor iş parçacığında,
/// okuma ses iş parçacığında; ikisi de kilitsiz.
pub struct EqCtl {
    /// Çift: kararlı; tek: yazma sürüyor. Her yayında 2 artar.
    seq: AtomicU32,
    enabled: AtomicU32,
    pre: AtomicU32,
    coef: [AtomicU32; BANDS * 5],
}

impl EqCtl {
    pub fn new() -> Self {
        let id = Params::IDENT;
        Self {
            seq: AtomicU32::new(0),
            enabled: AtomicU32::new(0),
            pre: AtomicU32::new(id.pre.to_bits()),
            coef: std::array::from_fn(|i| AtomicU32::new(id.coef[i / 5][i % 5].to_bits())),
        }
    }

    /// Ayarı hesaplayıp yayınlar (motor iş parçacığı).
    pub fn set(&self, enabled: bool, gains_db: &[f32; BANDS], preamp_db: f32) {
        self.publish(enabled, &Params::from_gains(gains_db, preamp_db));
    }

    /// Hazır ayarı yayınlar. Yazmalar seridir (birden çok yazar olursa
    /// sırayla; yazma birkaç düzine atomik depodur).
    pub fn publish(&self, enabled: bool, p: &Params) {
        let mut s = self.seq.load(Relaxed);
        loop {
            if s & 1 == 1 {
                std::hint::spin_loop();
                s = self.seq.load(Relaxed);
                continue;
            }
            match self.seq.compare_exchange_weak(s, s.wrapping_add(1), Relaxed, Relaxed) {
                Ok(_) => break,
                Err(now) => s = now,
            }
        }
        // Veri depoları tek sayılı değerden ÖNCE görünmesin.
        fence(Release);
        self.enabled.store(enabled as u32, Relaxed);
        self.pre.store(p.pre.to_bits(), Relaxed);
        for (cell, v) in self.coef.iter().zip(p.coef.iter().flatten()) {
            cell.store(v.to_bits(), Relaxed);
        }
        self.seq.store(s.wrapping_add(2), Release);
    }

    /// Yayın sayacı (ses iş parçacığı her blokta bir atomik okur).
    #[inline]
    fn seq(&self) -> u32 {
        self.seq.load(Relaxed)
    }

    /// Yırtıksız okuma; yazma sürüyorsa birkaç kez dener, olmazsa `None`
    /// (ses iş parçacığı bir sonraki blokta yeniden dener, beklemez).
    /// Dönen sayı, okunan sürümün `seq` değeri.
    pub fn read(&self) -> Option<(u32, bool, Params)> {
        for _ in 0..4 {
            let s1 = self.seq.load(Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let enabled = self.enabled.load(Relaxed) != 0;
            let mut p = Params::IDENT;
            p.pre = f32::from_bits(self.pre.load(Relaxed));
            for (cell, v) in self.coef.iter().zip(p.coef.iter_mut().flatten()) {
                *v = f32::from_bits(cell.load(Relaxed));
            }
            fence(Acquire);
            if self.seq.load(Relaxed) == s1 {
                return Some((s1, enabled, p));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Ses iş parçacığı: süzgeç
// ---------------------------------------------------------------------------

/// Ses iş parçacığına ait ekolayzır. Ayırma yok: bütün diziler gömülü.
pub struct Eq {
    /// Son alınan `EqCtl::seq`.
    seen: u32,
    target: Params,
    cur: Params,
    /// Blok başına eklenen fark (`pre` dahil).
    step: Params,
    /// Hedefe kalan blok.
    left: u32,
    /// Geçerli katsayılarda birim olmayan bantlar.
    active: u16,
    /// Dal çalışıyor mu (kapalı ve birimse hayır; `shape` buna bakar).
    running: bool,
    /// Durum: `z[bant][z1 | z2][kanal]`.
    z: [[[f32; 2]; 2]; BANDS],
}

impl Eq {
    pub fn new() -> Self {
        Self {
            seen: 0,
            target: Params::IDENT,
            cur: Params::IDENT,
            step: Params { coef: [[0.0; 5]; BANDS], pre: 0.0 },
            left: 0,
            active: 0,
            running: false,
            z: [[[0.0; 2]; 2]; BANDS],
        }
    }

    /// Dal bu blokta çalışmalı mı.
    #[inline]
    pub fn running(&self) -> bool {
        self.running
    }

    /// Yeni ayar yayınlandıysa alır (blok başına bir atomik okuma).
    #[inline]
    pub fn poll(&mut self, ctl: &EqCtl) {
        if ctl.seq() != self.seen {
            self.take(ctl);
        }
    }

    #[cold]
    fn take(&mut self, ctl: &EqCtl) {
        let Some((seq, enabled, p)) = ctl.read() else { return };
        self.seen = seq;
        self.target = if enabled { p } else { Params::IDENT };
        if self.target == self.cur {
            self.left = 0;
        } else {
            self.left = GLIDE_BLOCKS;
            let k = 1.0 / GLIDE_BLOCKS as f32;
            for b in 0..BANDS {
                for j in 0..5 {
                    self.step.coef[b][j] = (self.target.coef[b][j] - self.cur.coef[b][j]) * k;
                }
            }
            self.step.pre = (self.target.pre - self.cur.pre) * k;
        }
        self.running = !(self.cur.is_identity() && self.target.is_identity());
    }

    /// Durumu sıfırlar (kuyruk atıldı: akış kesildi).
    pub fn reset(&mut self) {
        self.z = [[[0.0; 2]; 2]; BANDS];
    }

    /// `raw` (L, R, L, R, …) ilk `n` karesini süzer. Her çağrı bir blok
    /// sayılır (katsayı süzülmesi).
    pub fn process(&mut self, raw: &mut [f32], n: usize) {
        let pre0 = self.cur.pre;
        if self.left > 0 {
            self.left -= 1;
            if self.left == 0 {
                self.cur = self.target;
            } else {
                for b in 0..BANDS {
                    for j in 0..5 {
                        self.cur.coef[b][j] += self.step.coef[b][j];
                    }
                }
                self.cur.pre += self.step.pre;
            }
            self.active = self.cur.active_mask();
        }
        // Ön kazanç blok içinde kareden kareye süzülür (blok sınırında basamak
        // olmasın); değişmiyorsa adım tam 0 ve çarpan olduğu gibi kalır.
        let dpre = (self.cur.pre - pre0) / n as f32;
        let mut pre = pre0;
        let coef = self.cur.coef;
        let mut idx = [0usize; BANDS];
        let mut na = 0;
        for b in 0..BANDS {
            if self.active >> b & 1 == 1 {
                idx[na] = b;
                na += 1;
            }
        }
        // Birim olan bandın durumu anlamsız: sıfır (yeniden açılınca bayat
        // durum çalmasın).
        for b in 0..BANDS {
            if self.active >> b & 1 == 0 {
                self.z[b] = [[0.0; 2]; 2];
            }
        }
        for f in 0..n {
            pre += dpre;
            let mut l = raw[f * 2] * pre;
            let mut r = raw[f * 2 + 1] * pre;
            for &b in &idx[..na] {
                let [b0, b1, b2, a1, a2] = coef[b];
                let z = &mut self.z[b];
                let yl = b0 * l + z[0][0];
                z[0][0] = b1 * l - a1 * yl + z[1][0];
                z[1][0] = b2 * l - a2 * yl;
                let yr = b0 * r + z[0][1];
                z[0][1] = b1 * r - a1 * yr + z[1][1];
                z[1][1] = b2 * r - a2 * yr;
                l = yl;
                r = yr;
            }
            raw[f * 2] = l;
            raw[f * 2 + 1] = r;
        }
        // Denormal ve patlama kesme.
        for &b in &idx[..na] {
            for v in self.z[b].iter_mut().flatten() {
                let a = v.abs();
                if !(a >= FLUSH_BELOW && a <= FLUSH_ABOVE) {
                    *v = 0.0;
                }
            }
        }
        if self.left == 0 && self.cur.is_identity() {
            self.running = false;
            self.reset();
        }
    }

    /// Testler: durum tamamen sıfır mı.
    #[cfg(test)]
    pub fn state_is_zero(&self) -> bool {
        self.z.iter().flatten().flatten().all(|v| *v == 0.0)
    }

    /// Testler: en küçük sıfırdan farklı durum büyüklüğü.
    #[cfg(test)]
    pub fn min_nonzero_state(&self) -> f32 {
        self.z.iter().flatten().flatten().map(|v| v.abs()).filter(|a| *a != 0.0).fold(f32::INFINITY, f32::min)
    }

    /// Testler: geçerli katsayılar hedefte mi.
    #[cfg(test)]
    pub fn settled(&self) -> bool {
        self.left == 0
    }
}

/// Testler: motor komutunun çıkışa ulaştığını görmek için (`OutputCtl::eq`
/// alanı özel; çocuk modülden okunur).
#[cfg(test)]
impl super::OutputCtl {
    /// Yayınlanan son ayar: (açık mı, ön kazanç çarpanı, katsayılar).
    pub(in crate::engine) fn eq_published(&self) -> Option<(bool, Params)> {
        self.eq.read().map(|(_, enabled, p)| (enabled, p))
    }
}
