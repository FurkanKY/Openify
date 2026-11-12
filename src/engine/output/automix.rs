//! AutoMix kararı ve sessizlik atlama (saf; sesi yazan taraf `writer.rs`).
//!
//! # Karar (`decide`)
//!
//! Giden şarkının SON ucu (`EdgeView`: tempo, güven, son vuruştan müziğin
//! bittiği yere ms, sönme uzunluğu) ve gelen şarkının BAŞ ucu (varsa;
//! önbellekten) verilir; çıkan `Plan`:
//!
//! - Bindirme uzunluğu: `max(sönme, giriş)` saniye, 2–12 sn'ye sıkıştırılmış.
//!   Sönme: giden şarkının son saniyelerinde ses düzeyi gövdenin 6 dB altına
//!   inen süre (yavaş sönüş / sakin bitiş bindirmenin tamamını kaplasın).
//!   Giriş: gelenin gövdesine (6 dB altı) erişene dek süre (yavaş giriş
//!   bindirilsin). İkisi de sert bitiş / başlangıçsa 2 sn.
//! - Tempolar güvenli (≥ 0,30) ve ±%6 içinde (ya da ×2 / ÷2) ise bindirme
//!   başlangıcı giden şarkının vuruş ızgarasına oturtulur: gelenin ilk vuruşu
//!   giden şarkının bir vuruşuna denk gelir; vuruş sayısı mümkünse 4'ün katı
//!   ("ölçü başı gibi"; gerçek ölçü başı bilinmez). Aksi hâlde eşit güçle,
//!   istenen uzunlukta, hizalamasız.
//!
//! # Sessizlik atlama
//!
//! - `TailTrim`: giden şarkının sonunda (yalnız AutoMix isteği varken) 23 ms'den
//!   uzun, −60 dBFS altı sessizlik halkaya YAZILMAZ; müzik geri gelirse (şarkı
//!   içi boşluk) boşluk en çok 1,5 sn'ye sıkıştırılıp sıfırla doldurulur. Halkada
//!   şarkının sonu, müziğin bittiği yerdir: bindirme o noktada biter.
//! - `LeadSkip`: gelen şarkının başındaki sessizlik (en çok 6 sn) atılır; yeni
//!   şarkı müziğin başladığı kareden başlar.

use std::borrow::Cow;

use super::{
    RATE,
    beat::{Analysis, ENV_SECS, ENV_UNKNOWN, SILENCE_PEAK},
};

/// Bindirme planının sınırları (sn).
pub const PLAN_MIN_S: f64 = 2.0;
pub const PLAN_MAX_S: f64 = 12.0;
/// Çözümleme yokken sabit bindirme (ms).
pub const FALLBACK_MS: u32 = 5_000;
/// Hizalama için en düşük tempo güveni.
pub const CONF_MIN: f32 = 0.30;
/// Tempo uyumu: ±%6.
pub const TEMPO_TOL: f64 = 0.06;
/// "Ölçü" = 4 vuruş.
const BEATS_PER_BAR: usize = 4;
/// Ölçü olmayan vuruş sayısının maliyeti (vuruş cinsinden).
const OFF_BAR_COST: f64 = 0.75;
/// Zarfta "gövde"nin bu kadar dB altı = sönme / giriş sınırı.
const RAMP_DB: i32 = 6;

/// Bir ucun karar için özeti. `ramp_s`: giden için sönme, gelen için giriş
/// uzunluğu (sn, 0..=12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeView {
    pub bpm: f32,
    pub conf: f32,
    pub beat_ms: u32,
    pub ramp_s: u8,
}

/// Kararın sonucu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Bindirme (kare).
    pub overlap: u32,
    /// Başlangıç vuruş ızgarasına oturtuldu.
    pub aligned: bool,
}

/// Zarftan sönme / giriş uzunluğu (sn): sınırdan (0. saniye) başlayıp ses
/// düzeyi gövdenin (zarfın tepesi) `RAMP_DB` altına inmeyen ilk saniyeye dek.
pub fn ramp_len(env: &[i8; ENV_SECS]) -> u8 {
    let Some(body) = env.iter().copied().filter(|v| *v != ENV_UNKNOWN).max() else {
        return 0;
    };
    let k = env
        .iter()
        .position(|v| *v != ENV_UNKNOWN && *v as i32 >= body as i32 - RAMP_DB)
        .unwrap_or(0);
    k.min(PLAN_MAX_S as usize) as u8
}

impl Analysis {
    /// Giden şarkı olarak: son uç (yalnız `HAS_TAIL`).
    pub fn tail_view(&self) -> Option<EdgeView> {
        self.has_tail().then(|| EdgeView {
            bpm: self.tail.bpm,
            conf: self.tail.conf,
            beat_ms: self.tail.beat_ms,
            ramp_s: ramp_len(&self.tail_env),
        })
    }

    /// Bu şarkı GİDEN, `inc` gelen iken bindirme planı (yalnız `HAS_TAIL`). Halkada
    /// müzik sonundan sonra kalan sessizlik (`TailTrim`) hesaba katılır.
    pub fn plan(&self, inc: Option<&EdgeView>) -> Option<Plan> {
        let view = self.tail_view()?;
        let extra = (self.tail_ms as u64 * RATE as u64 / 1000).min(TRIM_KEEP) as u32;
        Some(decide(&view, inc, extra))
    }

    /// Gelen şarkı olarak: baş uç (yalnız `HAS_HEAD`).
    pub fn head_view(&self) -> Option<EdgeView> {
        self.has_head().then(|| EdgeView {
            bpm: self.head.bpm,
            conf: self.head.conf,
            beat_ms: self.head.beat_ms,
            ramp_s: ramp_len(&self.head_env),
        })
    }
}

impl EdgeView {
    /// Tek atomike: tempo ×16 (12 bit), güven ×63 (6), `beat_ms` (12), `ramp_s`
    /// (4), bir "var" biti. `OutputCtl::arm_automix` bununla gelenin baş ucunu
    /// yazara iletir.
    pub fn pack(&self) -> u64 {
        let bpm = ((self.bpm * 16.0).round() as u64).min(0xFFF);
        let conf = ((self.conf.clamp(0.0, 1.0) * 63.0).round() as u64).min(63);
        let beat = (self.beat_ms as u64).min(0xFFF);
        let ramp = (self.ramp_s as u64).min(15);
        1 | (bpm << 1) | (conf << 13) | (beat << 19) | (ramp << 31)
    }

    pub fn unpack(bits: u64) -> Option<Self> {
        (bits & 1 == 1).then(|| Self {
            bpm: ((bits >> 1) & 0xFFF) as f32 / 16.0,
            conf: ((bits >> 13) & 63) as f32 / 63.0,
            beat_ms: ((bits >> 19) & 0xFFF) as u32,
            ramp_s: ((bits >> 31) & 15) as u8,
        })
    }
}

/// İki tempo uyumlu mu: oran 1, 2 ya da ½'ye ±%6 yakın.
pub fn tempo_compatible(a: f32, b: f32) -> bool {
    if a <= 0.0 || b <= 0.0 {
        return false;
    }
    let ratio = b as f64 / a as f64;
    [1.0, 2.0, 0.5]
        .iter()
        .any(|m| (ratio / m - 1.0).abs() <= TEMPO_TOL)
}

fn secs_to_frames(s: f64) -> f64 {
    s * RATE as f64
}

fn ms_to_frames_f(ms: u32) -> f64 {
    ms as f64 * RATE as f64 / 1000.0
}

/// Bindirme planı. `out`: giden şarkının son ucu; `inc`: gelenin baş ucu (biliniyorsa);
/// `ring_extra`: halkada müziğin bittiği yerden sonra kalan sessizlik (kare; `TailTrim`
/// 1 024'e kadar bırakır): vuruş ızgarası müzik sonuna göre, halka ucu ondan bu kadar sonra.
pub fn decide(out: &EdgeView, inc: Option<&EdgeView>, ring_extra: u32) -> Plan {
    let ramp = out.ramp_s.max(inc.map_or(0, |v| v.ramp_s)) as f64;
    let want = secs_to_frames(ramp.clamp(PLAN_MIN_S, PLAN_MAX_S));
    if let Some(inc) = inc
        && out.conf >= CONF_MIN
        && inc.conf >= CONF_MIN
        && tempo_compatible(out.bpm, inc.bpm)
    {
        let period = 60.0 / out.bpm as f64 * RATE as f64;
        // overlap(n) = (son vuruştan halka ucuna) + n × periyot + (gelenin ilk vuruşu)
        let base = ms_to_frames_f(out.beat_ms) + ring_extra as f64 + ms_to_frames_f(inc.beat_ms);
        let (lo, hi) = (secs_to_frames(PLAN_MIN_S), secs_to_frames(PLAN_MAX_S));
        let max_n = ((hi - base) / period).floor().max(0.0) as usize;
        let best = (0..=max_n)
            .map(|n| (n, base + n as f64 * period))
            .filter(|(_, ov)| *ov >= lo && *ov <= hi)
            .map(|(n, ov)| {
                let cost = (ov - want).abs() / period
                    + if n % BEATS_PER_BAR == 0 {
                        0.0
                    } else {
                        OFF_BAR_COST
                    };
                (cost, ov)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, ov)) = best {
            return Plan {
                overlap: ov.round() as u32,
                aligned: true,
            };
        }
    }
    Plan {
        overlap: want.round() as u32,
        aligned: false,
    }
}

// ── Sessizlik atlama ────────────────────────────────────────────────────

/// Müziğin bittiği yerden sonra halkada kalan sessizlik (kare): yazar bir
/// çalışma aralığını bu kadar bırakır.
pub const TRIM_KEEP: u64 = 1_024;
/// Şarkı içi sessizlik boşluğunun yazıldığı en uzun süre (kare, 1,5 sn).
pub const GAP_MAX: u64 = RATE as u64 * 3 / 2;
/// Baştaki sessizliğin en çok atılan uzunluğu (kare, 6 sn).
pub const LEAD_CAP: u64 = RATE as u64 * 6;

fn silent(frame: &[f64]) -> bool {
    frame[0].abs() < SILENCE_PEAK as f64 && frame[1].abs() < SILENCE_PEAK as f64
}

/// Sondaki sessizliği halkaya yazmama (bkz. başlık).
#[derive(Debug, Default)]
pub struct TailTrim {
    /// Süren sessiz aralığın uzunluğu (kare; atılanlar dahil).
    run: u64,
}

impl TailTrim {
    pub fn reset(&mut self) {
        self.run = 0;
    }

    /// Paketi süzer. Değişiklik gerekmiyorsa (çoğu paket) aynısı döner, ayırma yok.
    pub fn apply<'a>(&mut self, samples: &'a [f64]) -> Cow<'a, [f64]> {
        // Geçiş 1: hiçbir aralık `TRIM_KEEP`i aşmıyorsa paket aynen geçer.
        let mut run = self.run;
        let mut change = false;
        for fr in samples.chunks_exact(2) {
            if silent(fr) {
                run += 1;
                if run > TRIM_KEEP {
                    change = true;
                    break;
                }
            } else {
                if run > TRIM_KEEP {
                    // Uzun bir aralık bitiyor: boşluk sıfırla doldurulacak.
                    change = true;
                    break;
                }
                run = 0;
            }
        }
        if !change {
            self.run = run;
            return Cow::Borrowed(samples);
        }
        let mut out = Vec::with_capacity(samples.len());
        let mut run = self.run;
        for fr in samples.chunks_exact(2) {
            if silent(fr) {
                run += 1;
                if run <= TRIM_KEEP {
                    out.extend_from_slice(fr);
                }
            } else {
                if run > TRIM_KEEP {
                    let zeros = run.min(GAP_MAX).saturating_sub(TRIM_KEEP);
                    out.resize(out.len() + zeros as usize * 2, 0.0);
                }
                run = 0;
                out.extend_from_slice(fr);
            }
        }
        self.run = run;
        Cow::Owned(out)
    }

    /// Süren sessiz aralığın uzunluğu (testler).
    #[cfg(test)]
    pub fn run(&self) -> u64 {
        self.run
    }
}

/// Gelen şarkının başındaki sessizliği atma.
#[derive(Debug, Default)]
pub struct LeadSkip {
    /// Şimdiye dek atılan kare.
    pub dropped: u64,
    done: bool,
}

impl LeadSkip {
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Paketin müzikli kısmı (ilk sessiz olmayan kareden itibaren).
    pub fn skip<'a>(&mut self, samples: &'a [f64]) -> &'a [f64] {
        if self.done {
            return samples;
        }
        let frames = (samples.len() / 2) as u64;
        match samples.chunks_exact(2).position(|fr| !silent(fr)) {
            Some(i) => {
                self.dropped += i as u64;
                self.done = true;
                &samples[i * 2..]
            }
            None => {
                self.dropped += frames;
                if self.dropped >= LEAD_CAP {
                    self.done = true;
                }
                &samples[samples.len()..]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(bpm: f32, conf: f32, beat_ms: u32, ramp_s: u8) -> EdgeView {
        EdgeView {
            bpm,
            conf,
            beat_ms,
            ramp_s,
        }
    }

    #[test]
    fn view_packs_into_one_atomic_and_round_trips() {
        let v = view(127.5, 0.62, 321, 9);
        let back = EdgeView::unpack(v.pack()).unwrap();
        assert!((back.bpm - 127.5).abs() < 0.07);
        assert!((back.conf - 0.62).abs() < 0.01);
        assert_eq!((back.beat_ms, back.ramp_s), (321, 9));
        assert_eq!(EdgeView::unpack(0), None);
        // Uç değerler kırpılır, taşmaz.
        let big = EdgeView::unpack(view(900.0, 3.0, 99_999, 200).pack()).unwrap();
        assert_eq!((big.beat_ms, big.ramp_s), (0xFFF, 15));
        assert!(big.bpm <= 256.0 && big.conf <= 1.0);
    }

    #[test]
    fn ramp_is_the_time_until_the_level_reaches_the_body() {
        // Düz: sert bitiş / başlangıç.
        assert_eq!(ramp_len(&[-14; ENV_SECS]), 0);
        // Son 8 sn'de her saniye 4 dB düşen sönme: fade[0] = -46 ... fade[7] = -18,
        // gövde -14, eşik -20: ilk uygun saniye 7.
        let mut fade = [-14i8; ENV_SECS];
        for (k, v) in fade.iter_mut().enumerate().take(8) {
            *v = -14 - 4 * (8 - k as i8);
        }
        assert_eq!(ramp_len(&fade), 7);
        // Bilinmeyen zarf: 0.
        assert_eq!(ramp_len(&[ENV_UNKNOWN; ENV_SECS]), 0);
        // En çok 12.
        let mut slow = [-40i8; ENV_SECS];
        slow[19] = -10;
        assert_eq!(ramp_len(&slow), 12);
    }

    #[test]
    fn hard_edges_get_the_two_second_floor_and_slow_edges_follow_the_ramp() {
        let hard = view(0.0, 0.0, 0, 0);
        let plan = decide(&hard, None, 0);
        assert_eq!(
            plan,
            Plan {
                overlap: 2 * RATE,
                aligned: false
            }
        );
        let fade = view(0.0, 0.0, 0, 7);
        assert_eq!(decide(&fade, Some(&hard), 0).overlap, 7 * RATE);
        // Gelenin yavaş girişi de uzatır; üst sınır 12 sn.
        let intro = view(0.0, 0.0, 0, 15);
        assert_eq!(decide(&hard, Some(&intro), 0).overlap, 12 * RATE);
    }

    #[test]
    fn tempos_within_six_percent_or_octaves_are_compatible() {
        assert!(tempo_compatible(120.0, 120.0));
        assert!(tempo_compatible(120.0, 127.0));
        assert!(!tempo_compatible(120.0, 128.5));
        assert!(tempo_compatible(87.0, 174.0));
        assert!(tempo_compatible(174.0, 88.0));
        assert!(!tempo_compatible(120.0, 150.0));
        assert!(!tempo_compatible(0.0, 120.0));
    }

    #[test]
    fn aligned_start_puts_the_incoming_first_beat_on_an_outgoing_beat() {
        // 120 BPM: periyot 22 050 kare. Son vuruş müzik bitişinden 300 ms önce,
        // gelenin ilk vuruşu müzik başlangıcından 100 ms sonra.
        let out = view(120.0, 0.8, 300, 6);
        let inc = view(120.0, 0.8, 100, 0);
        let plan = decide(&out, Some(&inc), 0);
        assert!(plan.aligned);
        let period = 22_050.0;
        let base = 0.4 * RATE as f64;
        let n = ((plan.overlap as f64 - base) / period).round();
        assert!(
            (plan.overlap as f64 - (base + n * period)).abs() < 1.5,
            "{plan:?}"
        );
        // İstenen 6 sn'ye en yakın, ölçü (n % 4 == 0) tercihli.
        assert!(
            (plan.overlap as f64 / RATE as f64 - 6.0).abs() < 1.2,
            "{plan:?}"
        );
        assert_eq!(n as usize % 4, 0, "ölçü tercihi");
    }

    #[test]
    fn low_confidence_or_incompatible_tempo_falls_back_to_equal_power_unaligned() {
        let out = view(120.0, 0.8, 300, 5);
        let shaky = view(120.0, 0.1, 100, 0);
        let plan = decide(&out, Some(&shaky), 0);
        assert_eq!(
            plan,
            Plan {
                overlap: 5 * RATE,
                aligned: false
            }
        );
        let far = view(150.0, 0.9, 100, 0);
        assert!(!decide(&out, Some(&far), 0).aligned);
        let unsure_out = view(120.0, 0.1, 300, 5);
        assert!(!decide(&unsure_out, Some(&view(120.0, 0.9, 100, 0)), 0).aligned);
        assert!(!decide(&out, None, 0).aligned);
    }

    #[test]
    fn plan_stays_within_two_to_twelve_seconds_for_every_phase() {
        for bpm in [70.0f32, 90.0, 120.0, 128.0, 174.0] {
            for beat in (0..1000).step_by(97) {
                for ramp in [0u8, 3, 8, 12] {
                    let out = view(bpm, 0.9, beat, ramp);
                    let inc = view(bpm, 0.9, 500 - beat.min(500), 0);
                    let plan = decide(&out, Some(&inc), 700);
                    let s = plan.overlap as f64 / RATE as f64;
                    assert!(
                        (2.0 - 1e-6..=12.0 + 1e-6).contains(&s),
                        "bpm {bpm} beat {beat}: {s}"
                    );
                }
            }
        }
    }

    fn stereo(frames: &[(f64, f64)]) -> Vec<f64> {
        frames.iter().flat_map(|(l, r)| [*l, *r]).collect()
    }

    #[test]
    fn tail_trim_drops_long_trailing_silence_and_keeps_short_gaps_exact() {
        let mut t = TailTrim::default();
        // Müzik, 100 ms sessizlik (4 410 kare), müzik: aralık sıkıştırılmaz (≤ 1,5 sn)
        // ama TRIM_KEEP'ten uzun olduğu için fazlası atılıp sıfırla geri doldurulur:
        // aynı uzunluk korunur.
        let mut pkt = vec![(0.5, 0.5); 100];
        pkt.extend(vec![(0.0, 0.0); 4_410]);
        pkt.extend(vec![(0.5, 0.5); 100]);
        let out = t.apply(&stereo(&pkt)).into_owned();
        assert_eq!(
            out.len(),
            2 * pkt.len(),
            "kısa boşluk (< 1,5 sn) aynı uzunlukta"
        );
        // Şarkı sonu: müzik + 3 sn sessizlik: yalnız TRIM_KEEP kalır.
        let mut t = TailTrim::default();
        let mut pkt = vec![(0.5, 0.5); 100];
        pkt.extend(vec![(0.0, 0.0); 3 * RATE as usize]);
        let input = stereo(&pkt);
        let out = t.apply(&input);
        assert_eq!(out.len() / 2, 100 + TRIM_KEEP as usize);
        assert_eq!(t.run(), 3 * RATE as u64, "atılanlar da sayılır");
        // Sonra gelen müzikle boşluk 1,5 sn'ye sıkıştırılır.
        let input = stereo(&[(0.5, 0.5); 10]);
        let out = t.apply(&input);
        assert_eq!(out.len() / 2, (GAP_MAX - TRIM_KEEP) as usize + 10);
        assert!(
            out[..(GAP_MAX - TRIM_KEEP) as usize * 2]
                .iter()
                .all(|x| *x == 0.0)
        );
    }

    #[test]
    fn tail_trim_passes_ordinary_packets_without_allocating() {
        let mut t = TailTrim::default();
        let music: Vec<f64> = (0..4096).map(|i| 0.3 * (i as f64 * 0.05).sin()).collect();
        assert!(matches!(t.apply(&music), Cow::Borrowed(_)));
        // Sıfır geçişlerindeki birkaç sessiz kare aralık sayılmaz.
        let mut crossing = vec![0.0f64; 40];
        crossing.extend(vec![0.4; 200]);
        assert!(matches!(t.apply(&crossing), Cow::Borrowed(_)));
        // Süren kısa aralık paketler arasında sayılır.
        let quiet = vec![0.0f64; 2 * 600];
        assert!(matches!(t.apply(&quiet), Cow::Borrowed(_)));
        assert!(
            matches!(t.apply(&quiet), Cow::Owned(_)),
            "ikinci paketle 1 200 kare > 1 024"
        );
    }

    #[test]
    fn lead_skip_starts_at_the_first_audible_frame_across_packets() {
        let mut s = LeadSkip::default();
        let quiet = vec![0.0f64; 2 * 1000];
        assert!(s.skip(&quiet).is_empty());
        assert!(!s.is_done());
        let mut p = vec![0.0f64; 2 * 300];
        p.extend([0.2, 0.2, 0.3, 0.3]);
        let rest = s.skip(&p);
        assert_eq!(rest, &[0.2, 0.2, 0.3, 0.3]);
        assert_eq!(s.dropped, 1_300);
        assert!(s.is_done());
        // Bittikten sonra olduğu gibi.
        assert_eq!(s.skip(&quiet).len(), quiet.len());
        // Sınır: 6 sn'den fazla atmaz.
        let mut s = LeadSkip::default();
        let long = vec![0.0f64; 2 * LEAD_CAP as usize];
        assert!(s.skip(&long).is_empty());
        assert!(s.is_done());
        assert_eq!(s.dropped, LEAD_CAP);
    }
}
