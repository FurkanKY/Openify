//! AutoMix çözümlemesi (`BeatTap`): çaların zaten yazdığı çözülmüş sesten, ek
//! ağ / çözme olmadan, şarkı başına
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub const NONE: Self = Self { bpm: 0.0, conf: 0.0, beat_ms: 0 };
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
    pub fn abort(&mut self) {
        self.active = false;
        self.uri = None;
    }

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
        for fr in samples.chunks_exact(2) {
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
        let logs = [(self.acc[0] * inv).sqrt(), (self.acc[1] * inv).sqrt(), (self.acc[2] * inv).sqrt()];
        let onset = if self.have_prev {
            (0..3).map(|b| BAND_W[b] * (logs[b] - self.prev_log[b]).max(0.0)).sum()
        } else {
            0.0
        };
        let hop = Hop { power: self.acc[3] * inv, onset };
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
                let ordered: Vec<Hop> = (0..content).map(|i| self.tail[(start + i) % WIN_HOPS]).collect();
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
                    Edge { bpm: t.bpm as f32, conf: t.conf as f32, beat_ms: hops_to_ms(delta) }
                });
                a.flags |= HAS_TAIL;
                a.total_ms = hops_to_ms(self.hops as f64);
                a.tail_ms = hops_to_ms(trail as f64);
                a.tail = edge;
                a.tail_env = env;
            }
        }
        (a.flags != 0).then_some((uri, a))
    }
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
        let mean = hops[range.clone()].iter().map(|h| h.power as f64).sum::<f64>() / range.len().max(1) as f64;
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
            let sum: f64 = o[..m].iter().zip(&o[lag..]).map(|(a, b)| *a as f64 * *b as f64).sum();
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
        let cand = (half.saturating_sub(1).max(1)..=half + 1).max_by(|a, b| rn[*a].total_cmp(&rn[*b]))?;
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
    Some(Tempo { bpm: bpm_of(lag), conf, lag, phase })
}
