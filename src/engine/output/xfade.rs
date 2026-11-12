//! Geçiş (crossfade) türleri ve eğrileri: `Transition`, `TransitionSpec`,
//! `Curve`. Bindirmenin kendisi yazarda (`writer.rs`), politika (hangi
//! geçişte, ne kadar) `playback/crossfade.rs`'te. Bkz. `mod.rs` başlığı,
//! "Geçiş (crossfade)".

use std::sync::OnceLock;

use super::RATE;

/// Kullanıcının seçebileceği en uzun bindirme (ms): `Command::SetCrossfade`.
pub const MAX_OVERLAP_MS: u32 = 12_000;

/// Bindirme kare sayısı 20 bite sığar (en çok 1 048 575 kare = 23,8 sn).
const OVERLAP_BITS: u32 = 20;
const OVERLAP_MASK: u64 = (1 << OVERLAP_BITS) - 1;
/// Başlangıç karesi 44 bite sığar (400 günden uzun çalma).
const START_MAX: u64 = (1 << 44) - 1;
const _: () = assert!((MAX_OVERLAP_MS as u64 * RATE as u64 / 1000) < OVERLAP_MASK);

/// Eğri: giden şarkının kazancı `out(t)`, gelenin `in(t)`, `t` 0 → 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// `out = 1 − t`, `in = t`: ilişkili (aynı) sesler için; ilişkisiz müzikte
    /// ortada −6 dB'lik çukur.
    Linear,
    /// `out = cos(πt/2)`, `in = sin(πt/2)`: `out² + in² = 1` (ilişkisiz
    /// müzikte güç sabit). Doğal geçişin eğrisi.
    EqualPower,
    /// Eşit güç eğrisinin tablodan, doğrusal aradeğerlemeli hâli. Şarkı başına
    /// özel eğrilerin (AutoMix) yeri: tablo değişince eğri değişir.
    Lut,
}

impl Curve {
    fn bits(self) -> u64 {
        match self {
            Curve::Linear => 0,
            Curve::EqualPower => 1,
            Curve::Lut => 2,
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0 => Curve::Linear,
            2 => Curve::Lut,
            _ => Curve::EqualPower,
        }
    }

    /// `(giden, gelen)` kazanç; `t` 0..1 (bindirmede karenin ortası:
    /// `(k + 0,5) / n`).
    pub fn gains(self, t: f64) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => ((1.0 - t) as f32, t as f32),
            Curve::EqualPower => {
                let (s, c) = (t * std::f64::consts::FRAC_PI_2).sin_cos();
                (c as f32, s as f32)
            }
            Curve::Lut => (lut_in(1.0 - t), lut_in(t)),
        }
    }
}

/// Eşit güç eğrisinin gelen kolunun tablosu (`sin(πt/2)`).
const LUT_SIZE: usize = 257;

fn lut() -> &'static [f32; LUT_SIZE] {
    static LUT: OnceLock<[f32; LUT_SIZE]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut table = [0.0f32; LUT_SIZE];
        for (i, v) in table.iter_mut().enumerate() {
            *v = (i as f64 / (LUT_SIZE - 1) as f64 * std::f64::consts::FRAC_PI_2).sin() as f32;
        }
        table
    })
}

fn lut_in(t: f64) -> f32 {
    let table = lut();
    let x = t.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f64;
    let i = (x as usize).min(LUT_SIZE - 2);
    let frac = (x - i as f64) as f32;
    table[i] + (table[i + 1] - table[i]) * frac
}

/// Bir geçişte istenen: bindirme süresi (kare) ve eğri. Motor, kendiliğinden
/// geçişten önce çıkışa verir (`OutputCtl::arm_transition`); `overlap` 0 ise
/// geçiş boşluksuz (bindirmesiz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionSpec {
    /// Planlanan bindirme (kare). AutoMix'te yazar şarkı sonu çözümlemesiyle yeniden
    /// belirler; bu değer çözümleme yokken sabit bindirme ve şarkı sonu payının derinliği.
    pub overlap: u32,
    pub curve: Curve,
    /// AutoMix: sondaki / baştaki sessizlik atlanır, bindirme çözümlemeden seçilir
    /// (bkz. `automix.rs`).
    pub auto: bool,
}

impl TransitionSpec {
    /// Bindirme yok.
    pub const NONE: Self = Self {
        overlap: 0,
        curve: Curve::EqualPower,
        auto: false,
    };

    /// `ms` milisaniyelik bindirme (en çok `MAX_OVERLAP_MS`).
    pub fn from_ms(ms: u32, curve: Curve) -> Self {
        let ms = ms.min(MAX_OVERLAP_MS);
        Self {
            overlap: (ms as u64 * RATE as u64 / 1000) as u32,
            curve,
            auto: false,
        }
    }

    /// AutoMix isteği: `ms` planlanan bindirme (çözümleme yokken sabit, şarkı sonu
    /// payının derinliği), eşit güç.
    pub fn automix(ms: u32) -> Self {
        Self {
            auto: true,
            ..Self::from_ms(ms, Curve::EqualPower)
        }
    }

    pub fn is_none(&self) -> bool {
        self.overlap == 0
    }

    pub(super) fn pack(self) -> u64 {
        (self.overlap as u64 & OVERLAP_MASK)
            | ((self.curve.bits() & 3) << OVERLAP_BITS)
            | ((self.auto as u64) << (OVERLAP_BITS + 2))
    }

    pub(super) fn unpack(bits: u64) -> Self {
        Self {
            overlap: (bits & OVERLAP_MASK) as u32,
            curve: Curve::from_bits((bits >> OVERLAP_BITS) & 3),
            auto: (bits >> (OVERLAP_BITS + 2)) & 1 == 1,
        }
    }
}

/// Süren (ya da sürecek) geçiş: yeni şarkının 0. karesi halkada `start`'ta,
/// `overlap` kare boyunca eskinin sonuyla karışır (`OutputCtl::transition`).
/// Arayüzün şarkı değişimi `start` duyulunca yapılır.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub start: u64,
    pub overlap: u32,
}

impl Transition {
    /// Tek `AtomicU64`: üst 44 bit `start`, alt 20 bit `overlap`.
    pub(super) fn pack(self) -> u64 {
        (self.start.min(START_MAX) << OVERLAP_BITS) | (self.overlap as u64 & OVERLAP_MASK)
    }

    /// `overlap` 0 ise geçiş yok.
    pub(super) fn unpack(bits: u64) -> Option<Self> {
        let overlap = (bits & OVERLAP_MASK) as u32;
        (overlap > 0).then_some(Self {
            start: bits >> OVERLAP_BITS,
            overlap,
        })
    }
}
