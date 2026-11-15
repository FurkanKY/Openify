//! Çalma sırasında yerel çözümleme (`BeatTap`): çaların zaten yazdığı çözülmüş
//! sesten, ek ağ / çözme olmadan; AutoMix uç ölçümleri ve tam şarkı özellikleri
//! (BPM, ton, enerji) birlikte
//!
//! - baştaki sessizlik (`lead_ms`) ve sondaki sessizlik / sönme noktası
//!   (`tail_ms`: müziğin bittiği yerden şarkının sonuna),
//! - başın ve sonun ilk / son 20 sn'sinin saniye başına ses düzeyi zarfı
//!   (`head_env`, `tail_env`; dBFS, i8),
//! - başta ve sonda ayrı ayrı tempo (BPM), güven (0..1) ve vuruş evresi
//!   (`Edge`)
//!
//! hesaplanır. Sonuç `Analysis`; `library.db`'de şarkı başına saklanır
//! (`engine/automix.rs`) ve karar (`automix.rs`) bunu kullanır.
//!
//! # Çalıştığı yer
//!
//! Çalar (çözücü) iş parçacığında, `tap::LevelSink::write` içinde; SES iş
//! parçacığında (ses kartının geri çağrısı) HİÇ çalışmaz, ona hiçbir şey
//! eklemez. Paket başına iş: her örnek için iki tek kutuplu süzgeç ve dört
//! toplama (üç bant + toplam güç + tepe); her 512 karede (11,6 ms) bir
//! "hop": güç, bant başına log-enerji farkından (spektral akı) vuruş
//! başlangıç gücü, iki sabit dizi hücresi. Ayırma yok (diziler kurulurken
//! ayrılır, sonra yeniden kullanılır), kilit yok. Ağır iş (özilinti, evre)
//! şarkı başına iki kez: baş 30 sn dolunca ve şarkı bitince.
//!
//! # Tempo
//!
//! Vuruş başlangıç zarfı (yerel ortalaması çıkarılmış, yarım dalga
//! doğrultulmuş) üzerinde özilinti; 59–199 BPM'lik adaylarda `r(L) + ½ r(2L) +
//! ¼ r(4L)` (katlar da tepe: vuruş dizisini ölçü dizisinden ayırır) × ılımlı bir
//! 118 BPM önceliği; sonra "yarıya inme" kuralı (L/2 neredeyse aynı güçteyse
//! hızlı olanı seç: 174 BPM'i 87 diye okumasın); en iyi gecikme kesirli
//! aradeğerlemeyle `r(λ) + r(2λ) + r(4λ)` toplamıyla inceltilir (katlar hata
//! payını böler: ±1 BPM çıtası); evre `λ` periyotlu tarağın en güçlü hizası.
//! Güven = o gecikmedeki normalleştirilmiş özilinti (sürekli müzikte 0,2–0,6,
//! saf vuruş dizisinde ~1, gürültüde ~0).
//!
//! Oktav belirsizliği (tempo ×2 / ÷2) doğası gereği sezgiseldir; karar
//! katmanı zaten ×2 / ÷2'yi eşdeğer sayar (`automix.rs`).

use super::RATE;

/// Hop: 512 kare (11,6 ms).
pub const HOP: usize = 512;
/// Saniyedeki hop.
pub const HOP_RATE: f64 = RATE as f64 / HOP as f64;
/// Zarf uzunluğu (sn).
pub const ENV_SECS: usize = 20;
/// Baştan ve sondan saklanan hop (40 sn).
const WIN_HOPS: usize = 3456;
/// Tempo penceresinin en çok / en az uzunluğu: 30 sn / 8 sn.
const MAX_HOPS: usize = 2584;
const MIN_HOPS: usize = 690;
/// "Sessiz" hop: tepe genlik bunun altında (−60 dBFS). Yazar (`automix.rs`) aynı
/// eşiği kare düzeyinde kullanır.
pub const SILENCE_PEAK: f32 = 0.001;
/// Bilinmeyen zarf değeri.
pub const ENV_UNKNOWN: i8 = i8::MIN;

/// Tek kutuplu alçak geçiren katsayıları: 150 Hz ve 2 kHz.
const A_LOW: f32 = 0.0210;
const A_MID: f32 = 0.2476;
/// Bant ağırlıkları (bas, orta, tiz) vuruş başlangıç gücünde.
const BAND_W: [f32; 3] = [1.0, 0.7, 0.4];

pub const HAS_HEAD: u8 = 1;
pub const HAS_TAIL: u8 = 2;

/// Şarkının bir ucundaki (baş / son) ritim bilgisi.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    /// Tempo; 0 = bilinmiyor.
    pub bpm: f32,
    /// 0..1.
    pub conf: f32,
    /// Baş: müziğin başlangıcından ilk vuruşa ms. Son: son vuruştan müziğin
    /// bittiği yere ms. Her ikisi de bir vuruş periyodundan kısa.
    pub beat_ms: u32,
}

impl Edge {
    pub const NONE: Self = Self {
        bpm: 0.0,
        conf: 0.0,
        beat_ms: 0,
    };
}

/// Bir şarkının çözümlemesi. `flags`: hangi uçlar geçerli (`HAS_HEAD`: şarkı
/// baştan kesintisiz çalındı, `HAS_TAIL`: şarkı kendiliğinden bitti ve son
/// saniyeler kesintisiz çalındı).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Analysis {
    pub flags: u8,
    /// Çözülmüş toplam uzunluk (yalnız `HAS_TAIL` iken anlamlı).
    pub total_ms: u32,
    pub lead_ms: u32,
    pub tail_ms: u32,
    pub head: Edge,
    pub tail: Edge,
    /// Müziğin başlangıcından itibaren saniye başına dBFS.
    pub head_env: [i8; ENV_SECS],
    /// Müziğin bittiği yerden geriye saniye başına dBFS (0: son saniye).
    pub tail_env: [i8; ENV_SECS],
    /// Whole-track BPM, key, and energy. `None` means the track did not contain
    /// enough usable audio for a defensible estimate.
    pub features: Option<TrackFeatures>,
}

/// Whole-track musical features derived only from samples that were played.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrackFeatures {
    pub bpm: f32,
    pub bpm_conf: f32,
    /// Pitch class: C=0 through B=11.
    pub key: u8,
    /// `true` for minor, `false` for major.
    pub minor: bool,
    pub key_conf: f32,
    pub energy: u8,
}

impl Analysis {
    pub const EMPTY: Self = Self {
        flags: 0,
        total_ms: 0,
        lead_ms: 0,
        tail_ms: 0,
        head: Edge::NONE,
        tail: Edge::NONE,
        head_env: [ENV_UNKNOWN; ENV_SECS],
        tail_env: [ENV_UNKNOWN; ENV_SECS],
        features: None,
    };

    pub fn has_head(&self) -> bool {
        self.flags & HAS_HEAD != 0
    }

    pub fn has_tail(&self) -> bool {
        self.flags & HAS_TAIL != 0
    }

    /// Yeni çözümlemeyi eskinin üstüne işler: yeni çözümlemenin geçerli
    /// uçları eskininkilerin yerine geçer, geçersiz ucu eskisi korunur.
    pub fn merged(mut self, newer: &Analysis) -> Analysis {
        if newer.has_head() {
            self.flags |= HAS_HEAD;
            self.lead_ms = newer.lead_ms;
            self.head = newer.head;
            self.head_env = newer.head_env;
        }
        if newer.has_tail() {
            self.flags |= HAS_TAIL;
            self.total_ms = newer.total_ms;
            self.tail_ms = newer.tail_ms;
            self.tail = newer.tail;
            self.tail_env = newer.tail_env;
        }
        if newer.features.is_some() {
            self.features = newer.features;
        }
        self
    }
}

#[derive(Clone, Copy, Default)]
pub struct Hop {
    power: f32,
    onset: f32,
}

/// Çözümleyici. Bir şarkı: `begin` → `feed`ler → `finish`.
pub struct BeatTap {
    lp_low: f32,
    lp_mid: f32,
    /// Hop içi birikim: bas, orta, tiz, toplam güç.
    acc: [f32; 4],
    peak: f32,
    n: usize,
    prev_log: [f32; 3],
    have_prev: bool,
    /// Şarkının ilk `WIN_HOPS` hop'u (baştan kesintisizse).
    head: Vec<Hop>,
    /// Son `WIN_HOPS` hop (halka).
    tail: Box<[Hop]>,
    tail_pos: usize,
    /// Şarkı başından beri hop sayısı.
    hops: u64,
    /// Son kesintiden (`discontinuity`) beri hop sayısı.
    seg_hops: u64,
    head_valid: bool,
    /// Müziğin başladığı hop (ilk sessiz olmayan); `None` henüz yok.
    lead: Option<u64>,
    /// Sondaki ardışık sessiz hop.
    trail: u64,
    head_edge: Option<Edge>,
    uri: Option<String>,
    active: bool,
    whole_onsets: Vec<f32>,
    total_power: f64,
    total_frames: u64,
    spectral_score: f64,
    spectral_total: f64,
    chroma_q1: [f64; 36],
    chroma_q2: [f64; 36],
    chroma_coeff: [f64; 36],
    chroma_samples: usize,
    chroma_sum: [f64; 12],
    chroma_decimator: u8,
    chroma_acc: f64,
}

impl Default for BeatTap {
    fn default() -> Self {
        Self::new()
    }
}

impl BeatTap {
    pub fn new() -> Self {
        Self {
            lp_low: 0.0,
            lp_mid: 0.0,
            acc: [0.0; 4],
            peak: 0.0,
            n: 0,
            prev_log: [0.0; 3],
            have_prev: false,
            head: Vec::with_capacity(WIN_HOPS),
            tail: vec![Hop::default(); WIN_HOPS].into_boxed_slice(),
            tail_pos: 0,
            hops: 0,
            seg_hops: 0,
            head_valid: false,
            lead: None,
            trail: 0,
            head_edge: None,
            uri: None,
            active: false,
            whole_onsets: Vec::with_capacity(32_768),
            total_power: 0.0,
            total_frames: 0,
            spectral_score: 0.0,
            spectral_total: 0.0,
            chroma_q1: [0.0; 36],
            chroma_q2: [0.0; 36],
            chroma_coeff: chroma_coefficients(),
            chroma_samples: 0,
            chroma_sum: [0.0; 12],
            chroma_decimator: 0,
            chroma_acc: 0.0,
        }
    }

    /// Yeni şarkı. `from_start`: şarkı 0. msten başladı (baş ucu geçerli olabilir).
    pub fn begin(&mut self, uri: Option<String>, from_start: bool) {
        self.reset_filters();
        self.head.clear();
        self.tail_pos = 0;
        self.hops = 0;
        self.seg_hops = 0;
        self.head_valid = from_start;
        self.lead = None;
        self.trail = 0;
        self.head_edge = None;
        self.whole_onsets.clear();
        self.total_power = 0.0;
        self.total_frames = 0;
        self.spectral_score = 0.0;
        self.spectral_total = 0.0;
        self.chroma_q1.fill(0.0);
        self.chroma_q2.fill(0.0);
        self.chroma_samples = 0;
        self.chroma_sum = [0.0; 12];
        self.chroma_decimator = 0;
        self.chroma_acc = 0.0;
        self.uri = uri;
        self.active = true;
    }

    /// Akış kesildi (sarma, `flush`): baş ucu artık geçersiz, son uç ve
    /// sessizlik sayaçları yeniden başlar.
    pub fn discontinuity(&mut self) {
        self.reset_filters();
        self.head_valid = false;
        self.seg_hops = 0;
        self.tail_pos = 0;
        self.trail = 0;
    }

    fn reset_filters(&mut self) {
        self.lp_low = 0.0;
        self.lp_mid = 0.0;
        self.acc = [0.0; 4];
        self.peak = 0.0;
        self.n = 0;
        self.have_prev = false;
    }

    /// Çözümlemeyi bırak (AutoMix kapandı): sonraki `begin`e dek hiçbir şey birikmez.
    /// Kesintisiz beslenen toplam süre (sn); testler / bench.
    #[allow(dead_code)]
    pub fn seconds(&self) -> f64 {
        self.hops as f64 / HOP_RATE
    }

    /// Çözülmüş, karıştırılmış bir paket (taşınmış sıra: L, R, L, R…).
    pub fn feed(&mut self, samples: &[f64]) {
        if !self.active {
            return;
        }
        for fr in samples.as_chunks::<2>().0 {
            let (l, r) = (fr[0] as f32, fr[1] as f32);
            let m = 0.5 * (l + r);
            self.lp_low += A_LOW * (m - self.lp_low);
            self.lp_mid += A_MID * (m - self.lp_mid);
            let low = self.lp_low;
            let mid = self.lp_mid - self.lp_low;
            let high = m - self.lp_mid;
            self.acc[0] += low * low;
            self.acc[1] += mid * mid;
            self.acc[2] += high * high;
            self.acc[3] += m * m;
            self.total_power += (f64::from(l * l) + f64::from(r * r)) * 0.5;
            self.total_frames += 1;
            self.chroma_decimator += 1;
            self.chroma_acc += f64::from(m);
            if self.chroma_decimator == 4 {
                self.chroma_decimator = 0;
                self.feed_chroma(self.chroma_acc * 0.25);
                self.chroma_acc = 0.0;
            }
            let p = l.abs().max(r.abs());
            if p > self.peak {
                self.peak = p;
            }
            self.n += 1;
            if self.n == HOP {
                self.end_hop();
            }
        }
    }

    fn end_hop(&mut self) {
        let inv = 1.0 / HOP as f32;
        // Genlik alanında (karekök) akı: log sıkıştırma sessizlikten çıkan her olayı eşit
        // büyük gösterip vurgu farkını (tekme ≫ şapka) siliyordu.
        let logs = [
            (self.acc[0] * inv).sqrt(),
            (self.acc[1] * inv).sqrt(),
            (self.acc[2] * inv).sqrt(),
        ];
        let onset = if self.have_prev {
            (0..3)
                .map(|b| BAND_W[b] * (logs[b] - self.prev_log[b]).max(0.0))
                .sum()
        } else {
            0.0
        };
        let hop = Hop {
            power: self.acc[3] * inv,
            onset,
        };
        self.whole_onsets.push(hop.onset);
        let low = f64::from(self.acc[0]);
        let mid = f64::from(self.acc[1]);
        let high = f64::from(self.acc[2]);
        self.spectral_score += 0.2 * low + 0.5 * mid + 0.9 * high;
        self.spectral_total += low + mid + high;
        self.prev_log = logs;
        self.have_prev = true;
        let silent = self.peak < SILENCE_PEAK;
        // Süzgeç durumları sessizlikte denormal'e inmesin (yavaşlatır).
        if self.lp_low.abs() < 1e-15 {
            self.lp_low = 0.0;
        }
        if self.lp_mid.abs() < 1e-15 {
            self.lp_mid = 0.0;
        }
        self.acc = [0.0; 4];
        self.peak = 0.0;
        self.n = 0;

        if self.head_valid && self.head.len() < WIN_HOPS {
            self.head.push(hop);
        }
        self.tail[self.tail_pos] = hop;
        self.tail_pos = (self.tail_pos + 1) % WIN_HOPS;
        if silent {
            self.trail += 1;
        } else {
            self.trail = 0;
            if self.lead.is_none() && self.head_valid {
                self.lead = Some(self.hops);
            }
        }
        self.hops += 1;
        self.seg_hops += 1;

        // Baş penceresi doldu: ağır işi şarkının ortasına yay.
        if self.head_edge.is_none()
            && self.head_valid
            && let Some(lead) = self.lead
            && self.head.len() >= (lead as usize + MAX_HOPS).min(WIN_HOPS)
        {
            self.head_edge = Some(self.compute_head(lead as usize).0);
        }
    }

    /// Downsampled Goertzel chroma at 11.025 kHz. A 1-second spectrum is
    /// accumulated once per second; the sample loop costs a fixed 36 bins.
    fn feed_chroma(&mut self, sample: f64) {
        const WINDOW: usize = 11_025;
        for i in 0..36 {
            let q0 = sample + self.chroma_coeff[i] * self.chroma_q1[i] - self.chroma_q2[i];
            self.chroma_q2[i] = self.chroma_q1[i];
            self.chroma_q1[i] = q0;
        }
        self.chroma_samples += 1;
        if self.chroma_samples == WINDOW {
            for i in 0..36 {
                let power = (self.chroma_q1[i] * self.chroma_q1[i]
                    + self.chroma_q2[i] * self.chroma_q2[i]
                    - self.chroma_coeff[i] * self.chroma_q1[i] * self.chroma_q2[i])
                    .max(0.0);
                self.chroma_sum[i % 12] += power;
            }
            self.chroma_q1.fill(0.0);
            self.chroma_q2.fill(0.0);
            self.chroma_samples = 0;
        }
    }

    /// Baş ucu: (kenar, zarf).
    fn compute_head(&self, lead: usize) -> (Edge, [i8; ENV_SECS]) {
        let from = lead.min(self.head.len());
        let hops = &self.head[from..];
        let env = env_db(hops, false);
        let window = &hops[..hops.len().min(MAX_HOPS)];
        let edge = tempo_of(window).map_or(Edge::NONE, |t| Edge {
            bpm: t.bpm as f32,
            conf: t.conf as f32,
            beat_ms: hops_to_ms(t.phase),
        });
        (edge, env)
    }

    /// Şarkı bitti ya da bırakıldı. `ended`: kendiliğinden bitti (son uç
    /// geçerli olabilir). Hiçbir uç geçerli değilse `None`.
    pub fn finish(&mut self, ended: bool) -> Option<(String, Analysis)> {
        if !self.active {
            return None;
        }
        self.active = false;
        let uri = self.uri.take()?;
        let mut a = Analysis::EMPTY;

        a.features = (ended && self.head_valid)
            .then(|| self.whole_track_features())
            .flatten();

        // Baş.
        if self.head_valid
            && let Some(lead) = self.lead
        {
            let lead = lead as usize;
            if self.head.len() >= lead + MIN_HOPS {
                let (edge, env) = match self.head_edge {
                    Some(edge) => (edge, self.compute_head(lead).1),
                    None => self.compute_head(lead),
                };
                a.flags |= HAS_HEAD;
                a.lead_ms = hops_to_ms(lead as f64);
                a.head = edge;
                a.head_env = env;
            }
        }

        // Son.
        if ended {
            let len = (self.seg_hops as usize).min(WIN_HOPS);
            let trail = (self.trail as usize).min(len);
            let content = len - trail;
            if content >= MIN_HOPS && trail < len {
                // Halkayı sırayla (eski → yeni) aç, müziğin bittiği yere kadar.
                let start = (self.tail_pos + WIN_HOPS - len) % WIN_HOPS;
                let ordered: Vec<Hop> = (0..content)
                    .map(|i| self.tail[(start + i) % WIN_HOPS])
                    .collect();
                let window = &ordered[ordered.len().saturating_sub(MAX_HOPS)..];
                let env = env_db(&ordered, true);
                let edge = tempo_of(window).map_or(Edge::NONE, |t| {
                    // Evre pencerenin başından; son vuruştan pencere sonuna uzaklık.
                    let n = window.len() as f64;
                    let mut last = t.phase;
                    while last + t.lag < n {
                        last += t.lag;
                    }
                    let delta = (n - last).max(0.0) % t.lag;
                    Edge {
                        bpm: t.bpm as f32,
                        conf: t.conf as f32,
                        beat_ms: hops_to_ms(delta),
                    }
                });
                a.flags |= HAS_TAIL;
                a.total_ms = hops_to_ms(self.hops as f64);
                a.tail_ms = hops_to_ms(trail as f64);
                a.tail = edge;
                a.tail_env = env;
            }
        }
        (a.flags != 0 || a.features.is_some()).then_some((uri, a))
    }

    fn whole_track_features(&self) -> Option<TrackFeatures> {
        if self.total_frames < RATE as u64 * 8 {
            return None;
        }
        let level = (self.total_power / self.total_frames as f64).sqrt();
        let tempo = tempo(&self.whole_onsets, level)?;
        let chroma_total: f64 = self.chroma_sum.iter().sum();
        if chroma_total <= 1.0e-12 || self.spectral_total <= 1.0e-12 {
            return None;
        }
        let (key, minor, key_conf) = krumhansl_key(&self.chroma_sum)?;
        // Energy = 70% normalized RMS loudness (-60..0 dBFS) and 30% spectral
        // density (energy-weighted low/mid/high bands; 0.2/0.5/0.9).
        let dbfs = 20.0 * level.max(1.0e-10).log10();
        let loudness = ((dbfs + 60.0) / 60.0).clamp(0.0, 1.0);
        let density = (self.spectral_score / self.spectral_total).clamp(0.0, 1.0);
        let energy = ((70.0 * loudness + 30.0 * density).round() as i32).clamp(0, 100) as u8;
        Some(TrackFeatures {
            bpm: tempo.bpm as f32,
            bpm_conf: tempo.conf as f32,
            key,
            minor,
            key_conf,
            energy,
        })
    }
}

fn chroma_coefficients() -> [f64; 36] {
    let mut coeff = [0.0; 36];
    for (i, value) in coeff.iter_mut().enumerate() {
        let midi = 48 + i as i32;
        let hz = 440.0 * 2.0f64.powf((midi as f64 - 69.0) / 12.0);
        *value = 2.0 * (2.0 * std::f64::consts::PI * hz / (RATE as f64 / 4.0)).cos();
    }
    coeff
}

fn krumhansl_key(chroma: &[f64; 12]) -> Option<(u8, bool, f32)> {
    const MAJOR: [f64; 12] = [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
    ];
    const MINOR: [f64; 12] = [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
    ];
    let mean = chroma.iter().sum::<f64>() / 12.0;
    let centered = chroma.map(|v| v - mean);
    let norm = centered.iter().map(|v| v * v).sum::<f64>().sqrt();
    if norm <= 1.0e-9 {
        return None;
    }
    let score = |profile: &[f64; 12], root: usize| {
        let pm = profile.iter().sum::<f64>() / 12.0;
        let pnorm = profile
            .iter()
            .map(|v| (v - pm) * (v - pm))
            .sum::<f64>()
            .sqrt();
        (0..12)
            .map(|i| centered[i] * (profile[(i + 12 - root) % 12] - pm))
            .sum::<f64>()
            / (norm * pnorm)
    };
    let mut best = (-2.0, 0u8, false);
    let mut second = -2.0f64;
    for root in 0..12 {
        for minor in [false, true] {
            let value = score(if minor { &MINOR } else { &MAJOR }, root);
            if value > best.0 {
                second = best.0;
                best = (value, root as u8, minor);
            } else if value > second {
                second = value;
            }
        }
    }
    Some((
        best.1,
        best.2,
        ((best.0 - second) / best.0.abs().max(1.0e-9)).clamp(0.0, 1.0) as f32,
    ))
}

fn hops_to_ms(hops: f64) -> u32 {
    (hops * HOP as f64 * 1000.0 / RATE as f64).round().max(0.0) as u32
}

/// Saniye başına ortalama güç (dBFS). `from_end`: zarf bitişten geriye
/// (0 = son saniye); değilse baştan. Pencere dolu değilse `ENV_UNKNOWN`.
fn env_db(hops: &[Hop], from_end: bool) -> [i8; ENV_SECS] {
    let mut env = [ENV_UNKNOWN; ENV_SECS];
    let n = hops.len();
    for (k, slot) in env.iter_mut().enumerate() {
        let a = (k as f64 * HOP_RATE).round() as usize;
        let b = ((k + 1) as f64 * HOP_RATE).round() as usize;
        if b > n {
            break;
        }
        let range = if from_end { (n - b)..(n - a) } else { a..b };
        let mean = hops[range.clone()]
            .iter()
            .map(|h| h.power as f64)
            .sum::<f64>()
            / range.len().max(1) as f64;
        *slot = (10.0 * mean.max(1e-10).log10()).round().clamp(-100.0, 0.0) as i8;
    }
    env
}

// ── Tempo ───────────────────────────────────────────────────────────────

/// Tempo kestirimi.
#[derive(Clone, Copy, Debug)]
pub struct Tempo {
    pub bpm: f64,
    pub conf: f64,
    /// Vuruş periyodu (hop, kesirli).
    pub lag: f64,
    /// Pencere başından ilk vuruşa (hop, `[0, lag)`).
    pub phase: f64,
}

const LAG_MIN: usize = 26; // ≈ 198,8 BPM
const LAG_MAX: usize = 87; // ≈ 59,4 BPM
/// Vuruş başlangıç gücünün standart sapmasının pencere genliğine oranı için alt sınır;
/// vuruşlu içerikte ~0,3–2, sabit tonun hop dalgacığında ~0,003.
const MIN_ONSET_STD: f64 = 0.03;
/// Yerel ortalama yarı genişliği (hop): ±0,5 sn.
const LOCAL_HALF: usize = 43;

/// Vuruş başlangıç gücü dizisinden tempo ve evre. 8 sn'den kısa ya da vuruşsuz
/// (sessiz / sabit) ise `None`.
pub fn tempo_of(hops: &[Hop]) -> Option<Tempo> {
    let raw: Vec<f32> = hops.iter().map(|h| h.onset).collect();
    let level = hops.iter().map(|h| h.power.sqrt() as f64).sum::<f64>() / hops.len().max(1) as f64;
    tempo(&raw, level)
}

/// `level`: penceredeki ortalama genlik (RMS); oynaklık buna göre değerlendirilir.
pub fn tempo(onset: &[f32], level: f64) -> Option<Tempo> {
    let n = onset.len();
    if n < MIN_HOPS {
        return None;
    }
    // Yerel ortalamayı çıkar, yarım dalga doğrult.
    let mut prefix = vec![0.0f64; n + 1];
    for (i, x) in onset.iter().enumerate() {
        prefix[i + 1] = prefix[i] + *x as f64;
    }
    let rect: Vec<f32> = (0..n)
        .map(|t| {
            let a = t.saturating_sub(LOCAL_HALF);
            let b = (t + LOCAL_HALF + 1).min(n);
            let mean = (prefix[b] - prefix[a]) / (b - a) as f64;
            (onset[t] as f64 - mean).max(0.0) as f32
        })
        .collect();
    let mean = rect.iter().map(|x| *x as f64).sum::<f64>() / n as f64;
    let o: Vec<f32> = rect.iter().map(|x| (*x as f64 - mean) as f32).collect();
    let var = o.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / n as f64;
    // Vuruş başlangıç gücünün oynaklığı çok düşükse (sabit ton, ses çıkışı dalgacığı)
    // ritim yok: özilinti sahte bir tepe bulur.
    if var.sqrt() < MIN_ONSET_STD * level.max(1e-4) {
        return None;
    }

    // Tam sayı gecikmeli normalleştirilmiş özilinti.
    let max_lag = 4 * LAG_MAX;
    let rn: Vec<f64> = (0..=max_lag)
        .map(|lag| {
            if lag == 0 {
                return 1.0;
            }
            let m = n - lag;
            let sum: f64 = o[..m]
                .iter()
                .zip(&o[lag..])
                .map(|(a, b)| *a as f64 * *b as f64)
                .sum();
            sum / m as f64 / var
        })
        .collect();

    let bpm_of = |lag: f64| 60.0 * HOP_RATE / lag;
    let prior = |lag: f64| {
        let octaves = (bpm_of(lag) / 118.0).log2() / 0.9;
        (-0.5 * octaves * octaves).exp()
    };
    let score = |l: usize| (rn[l] + 0.5 * rn[2 * l] + 0.25 * rn[4 * l]) * prior(l as f64);
    let mut best = (LAG_MIN..=LAG_MAX).max_by(|a, b| score(*a).total_cmp(&score(*b)))?;
    if score(best) <= 0.0 {
        return None;
    }
    // Yarıya inme: yarı gecikme neredeyse aynı güçteyse hızlı olanı seç.
    loop {
        let half = (best as f64 / 2.0).round() as usize;
        if half < LAG_MIN {
            break;
        }
        let cand =
            (half.saturating_sub(1).max(1)..=half + 1).max_by(|a, b| rn[*a].total_cmp(&rn[*b]))?;
        if rn[cand] >= 0.9 * rn[best] {
            best = cand;
        } else {
            break;
        }
    }

    // Kesirli aradeğerlemeyle incelt: r(λ) + r(2λ) + r(4λ).
    let frac_r = |lag: f64| -> f64 {
        let i = lag.floor() as usize;
        let f = (lag - i as f64) as f32;
        if i + 2 >= n {
            return 0.0;
        }
        let m = n - i - 1;
        let sum: f64 = (0..m)
            .map(|t| o[t] as f64 * (o[t + i] * (1.0 - f) + o[t + i + 1] * f) as f64)
            .sum();
        sum / m as f64 / var
    };
    let mut lag = best as f64;
    let mut top = f64::MIN;
    let mut d = -1.2f64;
    while d <= 1.2 + 1e-9 {
        let l = best as f64 + d;
        if l >= LAG_MIN as f64 - 1.0 {
            let s = frac_r(l) + frac_r(2.0 * l) + frac_r(4.0 * l);
            if s > top {
                top = s;
                lag = l;
            }
        }
        d += 0.04;
    }
    let conf = frac_r(lag).clamp(0.0, 1.0);

    // Evre ve ince periyot: λ periyotlu tarağın (vuruş başına ortalama) en güçlü hizası,
    // doğrultulmuş dizide. Periyot tarağın birikimiyle ±0,12 hop içinde yeniden incelenir
    // (özilinti inceltmesinin ~%0,15'lik kalıntısı pencere başında onlarca ms evre kaydırırdı).
    let at = |x: f64| -> f64 {
        let i = x.floor() as usize;
        if i + 1 >= n {
            return 0.0;
        }
        let f = (x - i as f64) as f32;
        (rect[i] * (1.0 - f) + rect[i + 1] * f) as f64
    };
    let comb = |lag: f64, p: f64| -> f64 {
        let (mut sum, mut count) = (0.0, 0usize);
        let mut x = p;
        while x < (n - 1) as f64 {
            sum += at(x);
            count += 1;
            x += lag;
        }
        sum / count.max(1) as f64
    };
    let (mut phase, mut top) = (0.0, f64::MIN);
    let base_lag = lag;
    let mut d = -0.12f64;
    while d <= 0.12 + 1e-9 {
        let l = base_lag + d;
        let mut p = 0.0f64;
        while p < l {
            let c = comb(l, p);
            if c > top {
                top = c;
                phase = p;
                lag = l;
            }
            p += 0.25;
        }
        d += 0.02;
    }
    // Vuruş başlangıcı hop'un içinde, ortalama yarısında.
    let phase = (phase + 0.5) % lag;
    Some(Tempo {
        bpm: bpm_of(lag),
        conf,
        lag,
        phase,
    })
}
