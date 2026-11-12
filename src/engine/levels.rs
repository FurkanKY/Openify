//! Sese tepki veren çubuklar: çözülmüş sesten bas / orta / tiz seviyesi.
//!
//! Bu dosya librespot'u bilmez, yalnız hesabı yapar (`tap.rs` onu ses
//! çıkışına bağlar):
//!
//!   çaların yazdığı örnekler ──▶ `LevelTap` ──▶ `AudioLevels` ──▶ arayüz
//!   (pre-volume, 44,1 kHz stereo)  (3 bant,     (kilitsiz halka,
//!                                   zamanlama)    `read(now)`)
//!
//! # Bantlar
//!
//! 4. derece Butterworth (24 dB/oktav) süzgeçlerle üç bant, kesişimler
//! 200 Hz ve 3 kHz:
//!   - bas   : alçak geçiren 200 Hz,
//!   - orta  : yüksek geçiren 200 Hz + alçak geçiren 3 kHz,
//!   - tiz   : yüksek geçiren 3 kHz.
//! Dik eğim şart: 60 Hz'lik saf bir sinüs orta banda -42 dB sızar (taban
//! çubuğun altında kalır); 2. derece süzgeç -21 dB sızıp orta çubuğu da
//! kaldırırdı. İki kanal ayrı süzülür, güçleri ortalanır (karşı fazlı
//! içerik tek kanala indirgemede silinirdi).
//!
//! # Pencere, ölçek, yumuşatma
//!
//! Her 1024 karede (23,2 ms; 44,1 kHz) bant başına ortalama kare güç
//! alınır, dBFS'e çevrilir ve bant başına bir pencereyle 0..1'e eşlenir
//! (`TOP_DB`, `RANGE_DB`; dB eksenli, yani algıya yakın ölçek), hafif bir
//! üs (`GAMMA`) tepeleri öne çıkarır. Ardından hızlı çıkış (atak 12 ms),
//! yavaş iniş (salım 160 ms) yumuşatması. Dijital sessizlik tam 0.
//! Pencere değerleri tipik bir -14 LUFS pop şarkısına göre ayarlandı
//! (bas ortalaması ~0,65, tekme tepesi ~0,95; tiz bandı doğal olarak 10 dB
//! daha alçak olduğu için kendi penceresi var); gerçek müzikle gözle
//! ayarlanacak, sabitler tek yerde.
//!
//! # Gecikme: arayüz DUYULANI göstermeli
//!
//! Sesi Openify'ın kendi çıkışı çalar (`output.rs`): yazılan kareler bir
//! halka kuyruğa girer, ses kartının geri çağrısı oradan çeker. Çıkış her an
//! hangi karenin ne zaman ses kartına verileceğini bilir (son geri çağrının
//! saati + aradaki kare / 44 100; `OutputCtl::pull_ns`). Yazılan parçanın
//! ilk karesi için o an `P` ise:
//!
//!   duyulma(pencere) = P + D + (pencere merkezinin parçadaki yeri) / 44 100
//!
//! Kuyruk boyu tahmin edilmez, ölçülür; her yazmada yeniden ölçüldüğü için
//! sürüklenme birikmez (eski rodio kuyruğu modeli ve onun gözlemle
//! düzeltilmesi artık yok).
//!
//! D (ses kartı geri çağrısından hoparlörden duyulmaya: cpal/WASAPI
//! paylaşımlı kipte kalan tampon + ses motoru + sürücü) koddan okunamaz:
//! cpal 0.16 WASAPI `Initialize`'a tampon süresi 0 verir
//! (`BufferSize::Default`), sistem varsayılan 10 ms dönemini seçer. Tahmin:
//! ~10 ms tampon + ~10 ms ses motoru + ~5-10 ms sürücü ≈ 30 ms
//! (`DEVICE_MS`). Bluetooth gibi aygıtlarda gerçek gecikme yüzlerce ms
//! olabilir; `OPENIFY_EQ_DEVICE_MS=<ms>` D'yi değiştirir (ölçüm / ayar için).
//!
//! # Duraklatma, sarma: kesme ve yeniden zamanlama
//!
//! Çıkış susturulunca (duraklat) ya da kuyruk atılınca (sar, şarkı değiştir)
//! halkada henüz duyulmamış sesin girdileri kalır; çubuklar onlarla oynamaya
//! devam etmesin diye `AudioLevels::cut(t)`: `t`'den sonraki girdiler
//! okunmaz. Duraklatmada kuyruk atılmaz; devam edince o kareler çalınır, bu
//! yüzden `LevelTap::resume` onların girdilerini yeni duyulma anlarıyla
//! yeniden yayar. Atmada (`on_flush`) çözümleyici sıfırlanır.
//!
//! # Yayın: kilitsiz
//!
//! Tek yazar (çalar iş parçacığı), çok okuyan. Halka 128 hücre, her hücre
//! TEK `AtomicU64` (40 bit zaman, 100 µs birim; 3 × 8 bit seviye), bu yüzden
//! yırtılma yok, kilit yok. `head` Release/Acquire ile yayılır. Kesme iki
//! atomik (`cut_t`, `cut_idx`): `cut_idx`'ten önce yazılmış ve zamanı
//! `cut_t`'den sonra olan girdiler geçersiz.

use std::{
    collections::VecDeque,
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Bant sayısı: bas, orta, tiz (bu sırayla).
pub const BANDS: usize = 3;

/// librespot'un sabit örnekleme hızı (`librespot::playback::SAMPLE_RATE`;
/// `tap.rs` eşitliği derleme zamanında doğrular).
pub const SAMPLE_RATE: f64 = 44_100.0;

/// Çözümleme penceresi (kare). 23,2 ms.
const WINDOW: usize = 1024;
const WINDOW_NS: f64 = WINDOW as f64 * 1e9 / SAMPLE_RATE;

const CROSS_LOW_HZ: f64 = 200.0;
const CROSS_HIGH_HZ: f64 = 3000.0;
/// 4. derece Butterworth'u oluşturan iki 2. derece bölümün Q'ları.
const BUTTER4_Q: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_4];

/// Çubuğun tam dolduğu bant gücü (dBFS, ortalama kare güç; tam ölçekli saf
/// sinüs -3 dB okur). Bas / orta / tiz.
const TOP_DB: [f64; BANDS] = [-8.0, -9.0, -19.0];
/// Pencerenin genişliği: `TOP_DB - RANGE_DB` altı çubuğun sıfırı.
const RANGE_DB: f64 = 30.0;
/// dB ekseninden sonra tepeleri öne çıkaran üs.
const GAMMA: f32 = 1.15;
const ATTACK_TAU_MS: f64 = 12.0;
const RELEASE_TAU_MS: f64 = 160.0;
/// Süzgeç durumları dijital sessizlikte "denormal" sayılara inmesin (x86'da
/// 100 kat yavaş); girişe eklenen, duyulmaz (-400 dB) bir sabit.
const ANTI_DENORMAL: f64 = 1e-20;

/// Aygıt tarafı gecikme tahmini: geri çağrıdan hoparlöre (bkz. başlık).
const DEVICE_MS: f64 = 30.0;

/// Halka hücre sayısı (2'nin kuvveti): 512 × 23 ms ≈ 11,9 sn. Gelecekteki
/// kuyruk şarkı sonu payıyla ~4 sn'ye (çıkış halkası en çok 5,9 sn) çıkar;
/// 128 hücre (3 sn) olsa duyulan anın girdileri ezilir, çubuklar şarkının
/// son 30 sn'sinde dinlenmeye düşerdi. 4 KiB.
const RING: usize = 512;
const MASK: usize = RING - 1;
/// Ardışık iki girdi arası bundan fazlaysa kesinti sayılır (enterpolasyon
/// yerine sönüm).
const GAP_NS: u64 = (2.0 * WINDOW_NS) as u64;
/// Sondan sonra bu kadar sürüde `read` `None` döner (çubuklar dinlenmede).
const SETTLE_NS: u64 = 1_200_000_000;
/// Yeniden zamanlanabilecek son pencereler: çıkış halkası en çok ~5,9 sn
/// (256 pencere) tutar.
const RECENT: usize = 288;
/// `cut_t`: kesme yok. `cut_idx`: kesme sürüyor (yazar henüz kapatmadı).
const NO_CUT: u64 = u64::MAX;

/// Aygıt tarafı gecikmesi D (ns): `OPENIFY_EQ_DEVICE_MS` ya da tahmin.
pub fn device_delay_ns() -> u64 {
    let ms = std::env::var("OPENIFY_EQ_DEVICE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| (0.0..=2000.0).contains(v))
        .unwrap_or(DEVICE_MS);
    (ms * 1e6) as u64
}

fn frames_ns(frames: f64) -> i64 {
    (frames * 1e9 / SAMPLE_RATE) as i64
}

// ---------------------------------------------------------------------------
// Halka: kilitsiz yayın
// ---------------------------------------------------------------------------

struct Shared {
    base: Instant,
    /// `base` anının Unix zamanı (ms); yalnız iz kaydı için.
    base_unix_ms: f64,
    ring: [AtomicU64; RING],
    /// Şimdiye dek yazılan girdi sayısı (sonuncusu `head - 1`).
    head: AtomicU64,
    /// Son yazılan parçanın yazma-duyulma gecikmesi (ns): kuyruk + aygıt.
    delay_ns: AtomicU64,
    /// Kesme: bu andan (ns) sonraki girdiler, `cut_idx`'ten önce
    /// yazılmışlarsa geçersiz. `cut_idx == NO_CUT`: kesme sürüyor.
    cut_t: AtomicU64,
    cut_idx: AtomicU64,
    /// Şu anki ses çıkışı (söz saati için; bkz. `audible_track_ms`).
    output: std::sync::Mutex<std::sync::Weak<super::output::OutputCtl>>,
}

impl Shared {
    fn pending_cut(&self) -> Option<u64> {
        (self.cut_idx.load(Ordering::Acquire) == NO_CUT).then(|| self.cut_t.load(Ordering::Acquire))
    }
}

/// Duyulan sesin bas / orta / tiz seviyeleri. Ucuz kopyalanır (`Arc`), her
/// iş parçacığından okunur, kilitsizdir.
///
/// ```ignore
/// // Arayüzde (çizim zamanında), `engine: EngineHandle`:
/// let levels = engine.levels();                    // bir kez al, sakla
/// // Her karede; ekrana gelecek an için bir kare ileriye bak:
/// let heights = levels.read(Instant::now() + Duration::from_millis(16));
/// // Some([bas, orta, tiz]): her biri 0..1 (0 = sessiz, 1 = tam dolu).
/// // None: ses yok (çalmıyor / durakladı ve çubuklar durulmuş): dinlenme
/// // yüksekliği çiz, kare üretmeyi bırakabilirsin.
/// ```
#[derive(Clone)]
pub struct AudioLevels(Arc<Shared>);

impl Default for AudioLevels {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioLevels {
    pub fn new() -> Self {
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1e3)
            .unwrap_or(0.0);
        Self(Arc::new(Shared {
            base: Instant::now(),
            base_unix_ms: unix,
            ring: std::array::from_fn(|_| AtomicU64::new(0)),
            head: AtomicU64::new(0),
            delay_ns: AtomicU64::new(0),
            cut_t: AtomicU64::new(NO_CUT),
            cut_idx: AtomicU64::new(0),
            output: std::sync::Mutex::new(std::sync::Weak::new()),
        }))
    }

    /// Ses çıkışı kurulunca (her bağlantıda) çağrılır.
    pub(super) fn attach_output(&self, out: std::sync::Weak<super::output::OutputCtl>) {
        if let Ok(mut slot) = self.0.output.lock() {
            *slot = out;
        }
    }

    /// Şu an DUYULAN yerin çalan şarkıdaki konumu (ms; şarkı sözü saati).
    /// Kuyruk, aygıt gecikmesi ve boşluksuz geçişte önceki şarkının
    /// kuyruktaki sonu hesaba katılmıştır: yeni şarkı henüz duyulmuyorsa
    /// eksi. Ses çıkışı yoksa ya da konum bilinmiyorsa (`flush` sürüyor,
    /// şarkı başlamadı) `None`: arayüz kendi saatine düşer. Kilit yalnız
    /// tutamağı almak için (yarışan tek yazar bağlantı kurulumu).
    pub fn audible_track_ms(&self) -> Option<i64> {
        let out = self.0.output.lock().ok()?.upgrade()?;
        out.audible_track_ms()
    }

    /// `now` anında HOPARLÖRDEN duyulan sesin seviyesi (gecikme zaten
    /// hesaba katılmıştır: halkada her girdi sesin duyulacağı anla
    /// saklanır). Girdiler arası doğrusal enterpolasyon. Sessizlikte
    /// (durakladı, bitti, ağ bekliyor) son değer 160 ms'lik salımla söner;
    /// tamamen durulunca `None`.
    pub fn read(&self, now: Instant) -> Option<[f32; BANDS]> {
        let t = now.saturating_duration_since(self.0.base).as_nanos() as u64;
        self.at(t)
    }

    /// Bu tutamağın saati: `AudioLevels` oluşturulduğundan beri ns.
    pub(super) fn now_ns(&self) -> u64 {
        self.0.base.elapsed().as_nanos() as u64
    }

    /// Bu saatin `t_ns` anının Unix zamanı (ms; iz kayıtları için).
    pub(super) fn unix_ms(&self, t_ns: u64) -> f64 {
        self.0.base_unix_ms + t_ns as f64 / 1e6
    }

    /// Ses `t_ns`'den sonra duyulmayacak (susturuldu / atıldı): o andan
    /// sonraki girdiler okunmaz. Kesme sürerken yeniden çağrılırsa erken
    /// olan kalır. Yazar devam edince ya da atmayı işleyince kapatır
    /// (`LevelTap::resume` / `on_flush`). Her iş parçacığından çağrılabilir.
    pub(super) fn cut(&self, t_ns: u64) {
        let s = &self.0;
        if s.cut_idx.load(Ordering::Acquire) == NO_CUT {
            s.cut_t.fetch_min(t_ns, Ordering::AcqRel);
        } else {
            s.cut_t.store(t_ns, Ordering::Release);
            s.cut_idx.store(NO_CUT, Ordering::Release);
        }
    }

    /// Halkaya en son yazılan girdi (testler için).
    #[cfg(test)]
    pub(super) fn last_entry(&self) -> Option<(u64, [f32; BANDS])> {
        let head = self.0.head.load(Ordering::Acquire);
        head.checked_sub(1)
            .and_then(|i| unpack(self.0.ring[i as usize & MASK].load(Ordering::Relaxed)))
    }

    #[cfg(test)]
    pub(super) fn at_ns(&self, t: u64) -> Option<[f32; BANDS]> {
        self.at(t)
    }

    /// `read`'in ns cinsinden çekirdeği (`base`ten beri).
    fn at(&self, t: u64) -> Option<[f32; BANDS]> {
        let s = &self.0;
        let head = s.head.load(Ordering::Acquire);
        if head == 0 {
            return None;
        }
        let cut_idx = s.cut_idx.load(Ordering::Acquire);
        let cut_t = s.cut_t.load(Ordering::Acquire);
        // Yazarın bir sonra ezeceği hücreye (`head - RING`) dokunma.
        let lowest = head.saturating_sub(RING as u64 - 1);
        let mut later: Option<(u64, [f32; BANDS])> = None;
        let mut found: Option<(u64, [f32; BANDS])> = None;
        let mut idx = head;
        while idx > lowest {
            idx -= 1;
            let Some(entry) = unpack(s.ring[idx as usize & MASK].load(Ordering::Relaxed)) else {
                break;
            };
            // Kesilmiş (duyulmayacak) ses: yok say.
            if idx < cut_idx && entry.0 > cut_t {
                continue;
            }
            // Zaman geriye giderken küçülmeli; büyürse halka sarmış demek.
            if later.is_some_and(|(tl, _)| entry.0 > tl) {
                break;
            }
            if entry.0 <= t {
                found = Some(entry);
                break;
            }
            later = Some(entry);
        }
        let (te, ve) = found?;
        if let Some((tl, vl)) = later {
            let gap = tl - te;
            if gap <= GAP_NS {
                let f = if gap == 0 { 0.0 } else { (t - te) as f32 / gap as f32 };
                return Some(std::array::from_fn(|k| ve[k] + (vl[k] - ve[k]) * f));
            }
        }
        // Sonraki girdi yok ya da kesinti var: son değer pencerenin ortasına
        // kadar sürer, sonra salımla söner.
        let hold = te + (WINDOW_NS / 2.0) as u64;
        let dt = t.saturating_sub(hold);
        if dt > SETTLE_NS {
            return None;
        }
        let k = (-(dt as f64) / (RELEASE_TAU_MS * 1e6)).exp() as f32;
        let out: [f32; BANDS] = std::array::from_fn(|i| ve[i] * k);
        if dt > 0 && out.iter().all(|v| *v < 1.0 / 255.0) {
            return None;
        }
        Some(out)
    }
}

/// 40 bit zaman (100 µs birim, +1: 0 "boş" demek) + 3 × 8 bit seviye.
fn pack(t_ns: u64, v: [f32; BANDS]) -> u64 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u64;
    let tu = ((t_ns / 100) + 1) & ((1 << 40) - 1);
    (tu << 24) | (q(v[0]) << 16) | (q(v[1]) << 8) | q(v[2])
}

fn unpack(e: u64) -> Option<(u64, [f32; BANDS])> {
    let tu = e >> 24;
    if tu == 0 {
        return None;
    }
    let f = |shift: u32| ((e >> shift) & 0xff) as f32 / 255.0;
    Some(((tu - 1) * 100, [f(16), f(8), f(0)]))
}

// ---------------------------------------------------------------------------
// Süzgeçler ve pencere çözümleyici
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Coef {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coef {
    /// RBJ "Audio EQ Cookbook" 2. derece alçak / yüksek geçiren.
    fn new(high: bool, fc: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * fc / SAMPLE_RATE;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high {
            ((1.0 + c) / 2.0, -(1.0 + c))
        } else {
            ((1.0 - c) / 2.0, 1.0 - c)
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b0 / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    fn pair(high: bool, fc: f64) -> [Coef; 2] {
        [Coef::new(high, fc, BUTTER4_Q[0]), Coef::new(high, fc, BUTTER4_Q[1])]
    }
}

#[derive(Clone, Copy, Default)]
struct State {
    s1: f64,
    s2: f64,
}

/// Transpoze doğrudan biçim II.
#[inline(always)]
fn step(c: &Coef, s: &mut State, x: f64) -> f64 {
    let y = c.b0 * x + s.s1;
    s.s1 = c.b1 * x - c.a1 * y + s.s2;
    s.s2 = c.b2 * x - c.a2 * y;
    y
}

/// Bir kanalın bütün süzgeç durumları.
#[derive(Clone, Copy, Default)]
struct Channel {
    bass: [State; 2],
    mid_hp: [State; 2],
    mid_lp: [State; 2],
    treble: [State; 2],
}

struct Analyzer {
    low_lp: [Coef; 2],
    low_hp: [Coef; 2],
    high_lp: [Coef; 2],
    high_hp: [Coef; 2],
    ch: [Channel; 2],
    /// Pencerenin bant başına kare-güç toplamı (iki kanal birlikte).
    acc: [f64; BANDS],
    /// Penceredeki kare sayısı.
    filled: usize,
    smooth: [f32; BANDS],
    attack: f32,
    release: f32,
}

impl Analyzer {
    fn new() -> Self {
        let coef = |tau_ms: f64| (1.0 - (-WINDOW_NS / (tau_ms * 1e6)).exp()) as f32;
        Self {
            low_lp: Coef::pair(false, CROSS_LOW_HZ),
            low_hp: Coef::pair(true, CROSS_LOW_HZ),
            high_lp: Coef::pair(false, CROSS_HIGH_HZ),
            high_hp: Coef::pair(true, CROSS_HIGH_HZ),
            ch: [Channel::default(); 2],
            acc: [0.0; BANDS],
            filled: 0,
            smooth: [0.0; BANDS],
            attack: coef(ATTACK_TAU_MS),
            release: coef(RELEASE_TAU_MS),
        }
    }

    fn reset(&mut self) {
        self.ch = [Channel::default(); 2];
        self.acc = [0.0; BANDS];
        self.filled = 0;
        self.smooth = [0.0; BANDS];
    }

    /// Karışık (L, R, L, R…) örnekleri işler; her pencere dolunca `emit`
    /// çağrılır: (bu parçanın başından pencere sonuna kadar kare sayısı,
    /// yumuşatılmış seviyeler).
    fn process(&mut self, samples: &[f64], mut emit: impl FnMut(usize, [f32; BANDS])) {
        let total = samples.len() / 2;
        let mut pos = 0;
        while pos < total {
            let n = (total - pos).min(WINDOW - self.filled);
            self.run(&samples[pos * 2..(pos + n) * 2]);
            self.filled += n;
            pos += n;
            if self.filled == WINDOW {
                let levels = self.finish();
                emit(pos, levels);
            }
        }
    }

    fn run(&mut self, frames: &[f64]) {
        for (c, ch) in self.ch.iter_mut().enumerate() {
            let mut a = [0.0f64; BANDS];
            for frame in frames.chunks_exact(2) {
                let x = frame[c] + ANTI_DENORMAL;
                let b = step(&self.low_lp[0], &mut ch.bass[0], x);
                let b = step(&self.low_lp[1], &mut ch.bass[1], b);
                let m = step(&self.low_hp[0], &mut ch.mid_hp[0], x);
                let m = step(&self.low_hp[1], &mut ch.mid_hp[1], m);
                let m = step(&self.high_lp[0], &mut ch.mid_lp[0], m);
                let m = step(&self.high_lp[1], &mut ch.mid_lp[1], m);
                let t = step(&self.high_hp[0], &mut ch.treble[0], x);
                let t = step(&self.high_hp[1], &mut ch.treble[1], t);
                a[0] += b * b;
                a[1] += m * m;
                a[2] += t * t;
            }
            for k in 0..BANDS {
                self.acc[k] += a[k];
            }
        }
    }

    fn finish(&mut self) -> [f32; BANDS] {
        let norm = 1.0 / (2.0 * WINDOW as f64);
        let mut out = [0.0f32; BANDS];
        let mut poisoned = false;
        for k in 0..BANDS {
            let p = self.acc[k] * norm;
            if !p.is_finite() {
                poisoned = true;
            }
            let db = 10.0 * p.max(1e-30).log10();
            let x = ((db - (TOP_DB[k] - RANGE_DB)) / RANGE_DB).clamp(0.0, 1.0) as f32;
            let target = x.powf(GAMMA);
            let a = if target > self.smooth[k] { self.attack } else { self.release };
            self.smooth[k] += (target - self.smooth[k]) * a;
            out[k] = self.smooth[k];
        }
        if poisoned {
            // Bozuk (NaN / sonsuz) örnek süzgeçleri sonsuza dek zehirlerdi.
            self.ch = [Channel::default(); 2];
        }
        self.acc = [0.0; BANDS];
        self.filled = 0;
        out
    }
}

// ---------------------------------------------------------------------------
// Zamanlama + yayın
// ---------------------------------------------------------------------------

/// Halkaya yazan taraf; izi de (geliştirme kancası) buradan yazar.
struct Publisher {
    shared: Arc<Shared>,
    count: u64,
    last_ns: u64,
    trace: Option<BufWriter<File>>,
    trace_rows: u32,
    /// Testlerde: yayınlanan her girdinin (zaman ns, çıkış karesi).
    #[cfg(test)]
    log: Vec<(u64, u64)>,
}

impl Publisher {
    fn new(shared: Arc<Shared>, trace: Option<BufWriter<File>>) -> Self {
        Self {
            shared,
            count: 0,
            last_ns: 0,
            trace,
            trace_rows: 0,
            #[cfg(test)]
            log: Vec::new(),
        }
    }

    /// `frame`: pencere merkezinin çıkıştaki kare numarası (iz / test).
    fn push(&mut self, t_ns: i64, v: [f32; BANDS], #[allow(unused)] frame: u64) {
        // Zaman tek yönlü artsın (düzeltmeler geri sıçratsa bile).
        let t = (t_ns.max(0) as u64).max(self.last_ns + 100);
        self.last_ns = t;
        let s = &self.shared;
        s.ring[self.count as usize & MASK].store(pack(t, v), Ordering::Relaxed);
        self.count += 1;
        s.head.store(self.count, Ordering::Release);
        #[cfg(test)]
        self.log.push((t, frame));

        if let Some(w) = &mut self.trace {
            let unix = s.base_unix_ms + t as f64 / 1e6;
            let delay = s.delay_ns.load(Ordering::Relaxed) as f64 / 1e6;
            let _ = writeln!(w, "{unix:.1},{:.4},{:.4},{:.4},{delay:.1}", v[0], v[1], v[2]);
            self.trace_rows += 1;
            if self.trace_rows % 64 == 0 {
                let _ = w.flush();
            }
        }
    }

    /// Süren kesmeyi kapatır: şimdiye dek yazılanlar kesmeye tabi, bundan
    /// sonrakiler değil. Yeni girdiler kesme anından sonraya gelebilsin
    /// diye tek yön sınırı geri çekilir.
    fn close_cut(&mut self) {
        if let Some(cut_t) = self.shared.pending_cut() {
            self.shared.cut_idx.store(self.count, Ordering::Release);
            self.last_ns = self.last_ns.min(cut_t);
        }
    }

    fn flush(&mut self) {
        if let Some(w) = &mut self.trace {
            let _ = w.flush();
        }
    }
}

/// Çaların yazdığı parçaları çözümler, duyulacakları anı çıkıştan alır ve
/// sonuçları `AudioLevels`a yayınlar. Yalnız çalar iş parçacığından
/// kullanılır (zaman: `AudioLevels` oluşturulduğundan beri ns).
pub struct LevelTap {
    an: Analyzer,
    publisher: Publisher,
    /// Aygıt tarafı gecikme (D), ns.
    device_ns: i64,
    /// Son yayınlanan pencerelerin çıkış karesi (pencere ortası) ve
    /// seviyeleri: çıkış susturulup devam edince yeniden zamanlanır.
    recent: VecDeque<(u64, [f32; BANDS])>,
    /// Son parçanın duyulma bitişi (kesinti sezmek için).
    last_end: Option<i64>,
}

impl LevelTap {
    /// `device_ns`: aygıt tarafı gecikmesi D (bkz. `device_delay_ns`).
    pub fn new(levels: &AudioLevels, device_ns: u64) -> Self {
        // Geliştirme kancası: OPENIFY_EQ_TRACE=<csv>: her pencere için
        // duyulma anı (Unix ms), üç seviye, tahmini gecikme. Gerçek çalmada
        // hoparlör çıkışının kaydıyla karşılaştırmak için.
        let trace = std::env::var_os("OPENIFY_EQ_TRACE")
            .and_then(|p| File::create(p).ok())
            .map(|f| {
                let mut w = BufWriter::new(f);
                let _ = writeln!(w, "audible_unix_ms,bass,mid,treble,delay_ms");
                w
            });
        Self {
            an: Analyzer::new(),
            publisher: Publisher::new(levels.0.clone(), trace),
            device_ns: device_ns as i64,
            recent: VecDeque::with_capacity(RECENT),
            last_end: None,
        }
    }

    /// Bir parça çıkışa yazılmadan HEMEN ÖNCE çağrılır. `samples`: karışık
    /// stereo, ses düzeyinden ÖNCE (bkz. `tap.rs`); `now_ns`: yazma anı;
    /// `pull_ns`: parçanın ilk karesinin ses kartına verileceği an
    /// (`OutputCtl::pull_ns`); `first_frame`: o karenin çıkıştaki numarası.
    pub fn analyze(&mut self, samples: &[f64], now_ns: u64, pull_ns: u64, first_frame: u64) {
        let frames = samples.len() / 2;
        if frames == 0 {
            return;
        }
        let start = pull_ns as i64 + self.device_ns;
        // Önceki sesle arada boşluk var (kuyruk boşalmıştı: başlangıç, ağ
        // beklemesi): yeni dönem, süzgeçler ve yumuşatma sıfırdan.
        if self.last_end.is_none_or(|end| start - end > GAP_NS as i64) {
            self.an.reset();
        }
        let publisher = &mut self.publisher;
        let recent = &mut self.recent;
        self.an.process(samples, |end_frames, levels| {
            let back = end_frames as f64 - WINDOW as f64 / 2.0;
            let frame = (first_frame as f64 + back).max(0.0) as u64;
            publisher.push(start + frames_ns(back), levels, frame);
            if recent.len() == RECENT {
                recent.pop_front();
            }
            recent.push_back((frame, levels));
        });
        self.last_end = Some(start + frames_ns(frames as f64));
        self.publisher
            .shared
            .delay_ns
            .store((start - now_ns as i64).max(0) as u64, Ordering::Relaxed);
    }

    /// Çıkış devam ediyor (duraklatmadan sonra). Kuyrukta bekleyen sesin
    /// girdileri kesilmişti: `read_frame`'den (sıradaki çalınacak kare)
    /// itibaren olanlar yeni anlarıyla yeniden yayınlanır. `pull_ns`:
    /// `read_frame`'in ses kartına verileceği an.
    pub fn resume(&mut self, pull_ns: u64, read_frame: u64) {
        if self.publisher.shared.pending_cut().is_none() {
            return;
        }
        self.publisher.close_cut();
        let base = pull_ns as i64 + self.device_ns;
        let mut last = None;
        for &(frame, levels) in &self.recent {
            if frame >= read_frame {
                let t = base + frames_ns((frame - read_frame) as f64);
                self.publisher.push(t, levels, frame);
                last = Some(t);
            }
        }
        self.last_end = last.map(|t| t + (WINDOW_NS / 2.0) as i64);
    }

    /// Kuyruk atıldı (sarma, şarkı değiştirme): ses kesintili, çözümleyici
    /// sıfırdan; kalan girdiler kesilmiş olmalı (`OutputCtl::flush` keser),
    /// değilse burada kesilir.
    pub fn on_flush(&mut self, now_ns: u64) {
        let shared = self.publisher.shared.clone();
        if shared.pending_cut().is_none() {
            AudioLevels(shared).cut(now_ns + self.device_ns as u64);
        }
        self.publisher.close_cut();
        self.recent.clear();
        self.an.reset();
        self.last_end = None;
        self.publisher.flush();
    }

    /// Çıkış durdu (iz dosyası diske).
    pub fn hold(&mut self) {
        self.publisher.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = SAMPLE_RATE;

    /// Her iki kanalda aynı sinüs (karışık).
    fn sine(freq: f64, amp: f64, secs: f64) -> Vec<f64> {
        let n = (FS * secs) as usize;
        let mut v = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = amp * (2.0 * std::f64::consts::PI * freq * i as f64 / FS).sin();
            v.push(s);
            v.push(s);
        }
        v
    }

    fn silence(secs: f64) -> Vec<f64> {
        vec![0.0; (FS * secs) as usize * 2]
    }

    /// Çözümleyiciden pencere pencere çıkan seviyeler.
    fn windows(samples: &[f64]) -> Vec<[f32; BANDS]> {
        let mut an = Analyzer::new();
        windows_with(&mut an, samples)
    }

    fn windows_with(an: &mut Analyzer, samples: &[f64]) -> Vec<[f32; BANDS]> {
        let mut out = Vec::new();
        an.process(samples, |_, l| out.push(l));
        out
    }

    fn last(w: &[[f32; BANDS]]) -> [f32; BANDS] {
        *w.last().expect("pencere yok")
    }

    #[test]
    fn bass_sine_is_bass() {
        let l = last(&windows(&sine(60.0, 0.5, 1.0)));
        assert!(l[0] > 0.9, "bas {l:?}");
        assert!(l[1] < 0.02 && l[2] < 0.02, "sızıntı {l:?}");
    }

    #[test]
    fn mid_sine_is_mid() {
        // (2 kHz kesişime (3 kHz) bir oktavdan yakın: orada tizin de
        // yanması doğru; tiz penceresi 10 dB daha duyarlı.)
        for f in [500.0, 1000.0] {
            let l = last(&windows(&sine(f, 0.5, 1.0)));
            assert!(l[1] > 0.9, "{f} Hz orta {l:?}");
            assert!(l[0] < 0.1 && l[2] < 0.12, "{f} Hz sızıntı {l:?}");
        }
    }

    #[test]
    fn treble_sine_is_treble() {
        for f in [6000.0, 8000.0, 12000.0] {
            let l = last(&windows(&sine(f, 0.5, 1.0)));
            assert!(l[2] > 0.9, "{f} Hz tiz {l:?}");
            assert!(l[0] < 0.02 && l[1] < 0.2, "{f} Hz sızıntı {l:?}");
        }
        let l = last(&windows(&sine(8000.0, 0.5, 1.0)));
        assert!(l[1] < 0.02, "8 kHz orta sızıntısı {l:?}");
    }

    #[test]
    fn silence_is_floor() {
        let w = windows(&silence(1.0));
        assert!(w.iter().all(|l| l.iter().all(|v| *v == 0.0)), "{:?}", last(&w));
        // Sınırlayıcının bıraktığı denormal artıklar da sessizlik sayılır.
        let tiny: Vec<f64> = (0..88200).map(|_| f64::MIN_POSITIVE).collect();
        let l = last(&windows(&tiny));
        assert!(l.iter().all(|v| *v == 0.0), "{l:?}");
    }

    #[test]
    fn follows_amplitude_in_db_steps() {
        // Her yarılama -6 dB: çubuk tek yönlü iner, adım ~6/30'a yakın.
        let mut prev = 2.0f32;
        let mut vals = Vec::new();
        for amp in [0.5, 0.25, 0.125, 0.0625, 0.03125] {
            let l = last(&windows(&sine(60.0, amp, 1.0)))[0];
            assert!(l < prev, "genlik {amp}: {l} >= {prev}");
            prev = l;
            vals.push(l);
        }
        // 0.5 genlik -9 dB (tam), sonra -15, -21, -27, -33 dB.
        assert!(vals[0] > 0.95, "{vals:?}");
        assert!(vals[2] > 0.3 && vals[2] < 0.6, "-21 dB {vals:?}");
        assert!(vals[4] < 0.2, "{vals:?}");
    }

    #[test]
    fn attack_fast_release_slow() {
        let mut an = Analyzer::new();
        windows_with(&mut an, &silence(0.3));
        let up = windows_with(&mut an, &sine(60.0, 0.5, 0.2));
        // Atak: 2-3 pencerede (≤ 70 ms) dolmaya yakın.
        assert!(up[2][0] > 0.85, "atak {:?}", &up[..4]);
        let down = windows_with(&mut an, &silence(1.5));
        let at = |ms: f64| down[(ms / (WINDOW_NS / 1e6)) as usize][0];
        // Salım: 100 ms'de hâlâ belirgin, 1 sn'de sönmüş.
        assert!(at(100.0) > 0.35 && at(100.0) < 0.85, "100 ms {}", at(100.0));
        assert!(at(1000.0) < 0.02, "1 sn {}", at(1000.0));
        // Tek yönlü sönüm.
        assert!(down.windows(2).all(|w| w[1][0] <= w[0][0]));
    }

    #[test]
    fn nan_input_does_not_poison() {
        let mut an = Analyzer::new();
        let mut bad = sine(60.0, 0.5, 0.1);
        bad[100] = f64::NAN;
        windows_with(&mut an, &bad);
        let l = last(&windows_with(&mut an, &sine(60.0, 0.5, 0.5)));
        assert!(l[0] > 0.9 && l[0].is_finite(), "{l:?}");
    }

    fn publisher(levels: &AudioLevels) -> Publisher {
        Publisher::new(levels.0.clone(), None)
    }

    /// Halka: bulma, enterpolasyon, tutma + sönüm, boşluk, `None`.
    #[test]
    fn ring_read_semantics() {
        let levels = AudioLevels::new();
        let mut p = publisher(&levels);
        assert_eq!(levels.at(1_000_000), None, "boş halka");
        let ms = |x: f64| (x * 1e6) as i64;
        // 100, 123.2, 146.4 ms'de 0, 1, 0.5 (bas)
        p.push(ms(100.0), [0.0, 0.2, 0.0], 0);
        p.push(ms(123.2), [1.0, 0.2, 0.0], 0);
        p.push(ms(146.4), [0.5, 0.2, 0.0], 0);
        assert_eq!(levels.at(ms(50.0) as u64), None, "ilk girdiden önce");
        let mid = levels.at(ms(111.6) as u64).unwrap();
        assert!((mid[0] - 0.5).abs() < 0.01, "ara değer {mid:?}");
        // Sondan sonra: pencere yarısı (11.6 ms) tutar, sonra söner.
        let hold = levels.at(ms(155.0) as u64).unwrap();
        assert!((hold[0] - 0.5).abs() < 0.01, "tutma {hold:?}");
        let later = levels.at(ms(158.0 + 160.0) as u64).unwrap();
        assert!(later[0] < 0.5 * 0.4 && later[0] > 0.5 * 0.3, "sönüm {later:?}");
        assert_eq!(levels.at(ms(146.4 + 1500.0) as u64), None, "durulunca");
        // Boşluk: 1 sn sonraki girdiye enterpolasyon yok, sönüm var.
        p.push(ms(1146.4), [1.0, 1.0, 1.0], 0);
        let gap = levels.at(ms(646.4) as u64).unwrap();
        assert!(gap[0] < 0.03 && gap[2] == 0.0, "boşlukta {gap:?}");
        assert_eq!(levels.at(ms(1000.0) as u64), None, "boşlukta durulmuş");
        let again = levels.at(ms(1146.4) as u64).unwrap();
        assert!(again[0] > 0.99, "yeni girdi {again:?}");
    }

    #[test]
    fn ring_wraps_without_confusion() {
        let levels = AudioLevels::new();
        let mut p = publisher(&levels);
        // 1000 girdi (halkanın 8 katı), değer = zamanla artan.
        for i in 0..1000i64 {
            p.push(i * 23_219_954, [(i % 256) as f32 / 255.0, 0.0, 0.0], 0);
        }
        let t = (990.5 * 23_219_954.0) as u64;
        let l = levels.at(t).unwrap();
        let expect = ((990 % 256) as f32 + 0.5) / 255.0;
        assert!((l[0] - expect).abs() < 0.01, "{l:?} beklenen {expect}");
        // Halkanın dışındaki eski zaman: bulunamaz.
        assert_eq!(levels.at(1_000_000_000), None);
    }

    /// Kesme: duyulmayacak girdiler okunmaz; kesme kapanınca yeni girdiler
    /// (kesme anından önceye düşse bile) geçerli.
    #[test]
    fn cut_hides_unheard_entries_and_new_ones_follow() {
        let levels = AudioLevels::new();
        let mut p = publisher(&levels);
        let ms = |x: f64| (x * 1e6) as i64;
        for k in 0..10 {
            p.push(ms(100.0 + 23.2 * k as f64), [1.0, 1.0, 1.0], 0);
        }
        // 150 ms'de sustu: 150'den sonrası yok, son geçerli girdiden söner.
        levels.cut(ms(150.0) as u64);
        let at = |x: f64| levels.at(ms(x) as u64);
        assert!(at(140.0).unwrap()[0] > 0.99);
        let fading = at(250.0).unwrap()[0];
        assert!(fading < 0.6 && fading > 0.3, "sönmeli {fading}");
        assert_eq!(at(1500.0), None);
        // İkinci kesme daha geç: erken olan kalır.
        levels.cut(ms(200.0) as u64);
        assert!(at(250.0).unwrap()[0] < 0.6);
        // Yazar kapatır (sarma işlendi), yeni ses 160 ms'den itibaren.
        p.close_cut();
        p.push(ms(160.0), [0.2, 0.2, 0.2], 0);
        p.push(ms(183.2), [0.4, 0.4, 0.4], 0);
        let v = at(171.6).unwrap()[0];
        assert!((v - 0.3).abs() < 0.01, "yeni girdiler arası {v}");
        // Eski, kesilmiş girdiler (200-310 ms) hâlâ gizli.
        let v = at(300.0).unwrap()[0];
        assert!(v < 0.4, "eski girdi göründü {v}");
    }

    /// Yazar çalışırken okuyanlar yırtık değer görmez (her hücre tek atom).
    #[test]
    fn concurrent_readers_never_see_torn_entries() {
        let levels = AudioLevels::new();
        let mut p = publisher(&levels);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let (l, stop) = (levels.clone(), stop.clone());
                std::thread::spawn(move || {
                    let mut reads = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let t = (reads * 7_919) % 120_000_000_000;
                        if let Some(v) = l.at(t) {
                            // Üç bant hep aynı değerle yazıldı: yırtılma bunu bozar.
                            assert!(
                                (v[0] - v[1]).abs() < 1e-4 && (v[1] - v[2]).abs() < 1e-4,
                                "yırtık {v:?}"
                            );
                        }
                        reads += 1;
                    }
                    reads
                })
            })
            .collect();
        for i in 0..200_000i64 {
            let v = (i % 256) as f32 / 255.0;
            p.push(i * 600_000, [v, v, v], 0);
        }
        stop.store(true, Ordering::Relaxed);
        let total: u64 = readers.into_iter().map(|r| r.join().unwrap()).sum();
        assert!(total > 1000, "okuyucular çalışmadı ({total})");
    }

    // -----------------------------------------------------------------------
    // Gecikme benzetimi: Openify çıkışı + aygıt, sanal zaman
    // -----------------------------------------------------------------------

    const PERIOD: u64 = 10_000_000;

    /// `output.rs`'in davranışı: yazar 1024 karelik paketleri halkaya koyar,
    /// kuyruk hedefi aşınca ses iş parçacığı uyandırana dek bekler. Aygıt
    /// 10 ms'de bir (kendi saatiyle: `drift`) bir dönemlik kare çeker; ses
    /// iş parçacığı her geri çağrıda sonraki çağrının anını tahmin edip
    /// "0. karenin sanal çekilme anı"nı (`anchor`) yayar. Çekilen kare
    /// `d_true` sonra duyulur.
    struct Sim {
        now: u64,
        next_tick: u64,
        written: u64,
        read: u64,
        target: u64,
        rate_per_tick: f64,
        carry: f64,
        d_true: f64,
        /// (ilk kare, kare sayısı, geri çağrı anı)
        pulls: Vec<(u64, u64, f64)>,
        fs_dev: f64,
        anchor: f64,
        running: bool,
    }

    impl Sim {
        fn new(drift: f64, d_true_ms: f64, target_ms: f64) -> Self {
            let fs = FS * (1.0 + drift);
            Sim {
                now: 0,
                next_tick: 3_300_000,
                written: 0,
                read: 0,
                target: (target_ms * FS / 1e3) as u64,
                rate_per_tick: fs * PERIOD as f64 / 1e9,
                carry: 0.0,
                d_true: d_true_ms * 1e6,
                pulls: Vec::new(),
                fs_dev: fs / 1e9,
                // Ses iş parçacığı akış açılınca hemen çalışır: ilk yazmadan
                // önce ilk geri çağrının anı bilinir.
                anchor: 3_300_000.0,
                running: true,
            }
        }

        fn advance_to(&mut self, t: u64) {
            while self.next_tick <= t {
                let tick = self.next_tick as f64;
                if self.running {
                    self.carry += self.rate_per_tick;
                    let k = self.carry.floor() as u64;
                    self.carry -= k as f64;
                    let n = k.min(self.written - self.read);
                    if n > 0 {
                        self.pulls.push((self.read, n, tick));
                        self.read += n;
                    }
                }
                // Ses iş parçacığı: sonraki çağrı tick + dönem.
                self.anchor = tick + PERIOD as f64 - frames_ns(self.read as f64) as f64;
                self.next_tick += PERIOD;
            }
            self.now = t;
        }

        /// `OutputCtl::pull_ns` ile aynı hesap.
        fn pull_ns(&self, frame: u64) -> u64 {
            let by_now = self.now as f64 + frames_ns((frame - self.read) as f64) as f64;
            let by_clock = self.anchor + frames_ns(frame as f64) as f64;
            by_clock.max(by_now) as u64
        }

        /// Paketi koy; kuyruk hedefin üstündeyse ses iş parçacığı uyandırana
        /// (bir geri çağrıya) dek bekle; uyanma + çözme 0,4 ms.
        fn write(&mut self, frames: u64) {
            self.written += frames;
            while self.running && self.written - self.read > self.target {
                self.advance_to(self.next_tick);
            }
            self.advance_to(self.now + 400_000);
        }

        /// `frame` karesinin gerçek duyulma anı (ns).
        fn audible(&self, frame: u64) -> Option<f64> {
            let i = match self.pulls.binary_search_by_key(&frame, |p| p.0) {
                Ok(i) => i,
                Err(i) => i.checked_sub(1)?,
            };
            let (first, n, t) = self.pulls[i];
            (frame < first + n).then(|| t + (frame - first) as f64 / self.fs_dev + self.d_true)
        }
    }

    /// Bir çalma benzetimi. Her çıkış karesi için halkada en son yayınlanan
    /// girdinin zamanı ile o karenin gerçek duyulma anı arasındaki fark:
    /// (çalma sn, hata ms). `pauses`: çalma saniyeleri; orada çıkış susar
    /// (kuyruk korunur), 1,3 sn sonra devam eder. `D`: modelin aygıt
    /// gecikmesi (gerçeği `d_true_ms`).
    fn simulate(secs: f64, drift: f64, d_true_ms: f64, d_model_ms: f64, pauses: &[f64]) -> Vec<(f64, f64)> {
        let levels = AudioLevels::new();
        let mut tap = LevelTap::new(&levels, (d_model_ms * 1e6) as u64);
        let mut sim = Sim::new(drift, d_true_ms, 120.0);
        let total = (FS * secs) as u64;
        let mut pauses = pauses.to_vec();
        pauses.reverse();
        let buf = vec![0.1; 1024 * 2];
        while sim.written < total {
            let first = sim.written;
            tap.analyze(&buf, sim.now, sim.pull_ns(first), first);
            sim.write(1024);
            if pauses.last().is_some_and(|p| sim.written as f64 / FS >= *p) {
                pauses.pop();
                // Duraklat: susar (bu benzetimde zarfsız), 1,3 sn bekler, sürer.
                sim.advance_to(sim.next_tick);
                sim.running = false;
                levels.cut(sim.now + (d_true_ms * 1e6) as u64);
                sim.advance_to(sim.now + 1_300_000_000);
                sim.running = true;
                tap.resume(sim.pull_ns(sim.read), sim.read);
            }
        }
        sim.running = true;
        sim.advance_to(sim.now + 2_000_000_000);
        // Her kare için son yayınlanan zaman (yeniden yayın eskiyi geçersiz kılar).
        let mut last: std::collections::BTreeMap<u64, u64> = Default::default();
        for &(t, frame) in &tap.publisher.log {
            last.insert(frame, t);
        }
        last.iter()
            .filter_map(|(&frame, &t)| {
                let truth = sim.audible(frame)?;
                Some((frame as f64 / FS, (t as f64 - truth) / 1e6))
            })
            .collect()
    }

    fn stats(errs: &[(f64, f64)], from_s: f64) -> (f64, f64, f64) {
        let v: Vec<f64> = errs.iter().filter(|(t, _)| *t >= from_s).map(|(_, e)| *e).collect();
        let max = v.iter().fold(0.0f64, |m, e| m.max(e.abs()));
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        (mean, sd, max)
    }

    #[test]
    fn timing_matches_true_playback_time() {
        let e = simulate(60.0, 0.0, 30.0, 30.0, &[]);
        let (mean, sd, max) = stats(&e, 0.0);
        println!("D doğru: ort {mean:.3} ms, sd {sd:.3}, en çok {max:.3} ({} pencere)", e.len());
        assert!(e.len() > 2500);
        assert!(max <= 0.5, "en çok {max}");
    }

    #[test]
    fn timing_does_not_drift() {
        // ±100 ppm (aygıt saati hızlı / yavaş), 10 dk kesintisiz: düzeltme
        // gerekmeden hata birikmez (her yazma kuyruğu yeniden ölçer).
        for drift in [100e-6, -100e-6] {
            let e = simulate(600.0, drift, 30.0, 30.0, &[]);
            let (mean, sd, max) = stats(&e, 0.0);
            let (_, _, late) = stats(&e, 540.0);
            println!("sürüklenme {drift:+e}: ort {mean:.3} ms, sd {sd:.3}, en çok {max:.3} (son dk {late:.3})");
            assert!(max <= 0.5, "drift {drift}: en çok {max}");
        }
    }

    #[test]
    fn wrong_device_delay_is_a_constant_offset() {
        // Gerçek aygıt gecikmesi tahminden 25 ms fazla (örn. USB DAC):
        // hata sabit -25 ms (görüntü sesten önce); büyümez.
        let e = simulate(60.0, 0.0, 55.0, 30.0, &[]);
        let (mean, sd, max) = stats(&e, 0.0);
        println!("D 25 ms eksik: ort {mean:.2} ms, sd {sd:.3}, en çok {max:.2}");
        assert!((mean + 25.0).abs() < 0.5 && sd < 0.5, "ort {mean} sd {sd}");
    }

    #[test]
    fn pauses_reschedule_queued_entries() {
        let e = simulate(120.0, 50e-6, 30.0, 30.0, &[20.0, 45.5, 46.0, 80.0]);
        let (mean, sd, max) = stats(&e, 0.0);
        println!("4 duraklatma, 50 ppm: ort {mean:.3} ms, sd {sd:.3}, en çok {max:.3}");
        assert!(max <= 0.5, "en çok {max}");
    }

    /// Duraklatma: çıkış susunca çubuklar söner, sonra `None`; devam edince
    /// kuyruktaki sesin girdileri yeniden görünür.
    #[test]
    fn pause_settles_then_resume_shows_queued_audio() {
        let levels = AudioLevels::new();
        let mut tap = LevelTap::new(&levels, 30_000_000);
        let base = levels.0.base;
        let at = |ms: u64| levels.read(base + std::time::Duration::from_millis(ms));
        // 2 sn çalma, 120 ms kuyruk: parça k, k·23,2 ms'de yazılır, kuyruk
        // sonrası çekilir.
        let audio = sine(60.0, 0.5, 2.0);
        let mut frame = 0u64;
        for c in audio.chunks(1024 * 2) {
            let now = frames_ns(frame as f64) as u64;
            tap.analyze(c, now, now + 120_000_000, frame);
            frame += (c.len() / 2) as u64;
        }
        let end = frames_ns(frame as f64) as u64 / 1_000_000; // ms, son yazma
        let playing = at(end).unwrap();
        assert!(playing[0] > 0.9, "{playing:?}");
        // Son yazmada duraklat: çalınan son kare ~`end - 120 ms`'deki.
        levels.cut((end + 30) * 1_000_000);
        tap.hold();
        let soon = at(end + 30 + 100).unwrap();
        assert!(soon[0] < playing[0] * 0.8 && soon[0] > 0.05, "sönüyor {soon:?}");
        assert!(at(end + 30 + 800).is_none_or(|l| l[0] < 0.02), "durulmadı");
        assert_eq!(at(end + 2500), None);
        // 3 sn sonra devam: kuyruktaki 120 ms yeniden görünür.
        let resume = end + 3000;
        let read = frame - (0.120 * FS) as u64;
        tap.resume(resume * 1_000_000, read);
        let back = at(resume + 30 + 50).unwrap();
        assert!(back[0] > 0.9, "devamda {back:?}");
    }

    // -----------------------------------------------------------------------
    // İşlemci maliyeti
    // -----------------------------------------------------------------------

    /// Müzik benzeri: gürültü + birkaç ton + vuruşlar + sessiz bölüm.
    fn music_like(secs: f64) -> Vec<f64> {
        let n = (FS * secs) as usize;
        let mut v = Vec::with_capacity(n * 2);
        let mut r = 0x9E3779B97F4A7C15u64;
        let mut lp = 0.0f64;
        for i in 0..n {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            let noise = (r >> 11) as f64 / (1u64 << 52) as f64 - 1.0;
            lp += 0.2 * (noise - lp);
            let t = i as f64 / FS;
            // Yarım saniyede bir tekme, son çeyrek sessiz.
            let beat = (-(t % 0.5) * 12.0).exp();
            let silent = (i * 4 > n * 3) as u8 as f64;
            let s = (0.30 * beat * (2.0 * std::f64::consts::PI * 55.0 * t).sin()
                + 0.12 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()
                + 0.05 * (2.0 * std::f64::consts::PI * 5000.0 * t).sin()
                + 0.15 * lp)
                * (1.0 - silent);
            v.push(s);
            v.push(s * 0.9);
        }
        v
    }

    /// Ölçüm testi (duvar saati; yük altında dalgalanır), bu yüzden normal
    /// takımda koşmaz. Çalıştırmak için, başka derleme / test yokken:
    /// `cargo test --release cpu_cost_per_second_of_audio -- --ignored --nocapture`
    #[test]
    #[ignore = "işlemci ölçümü, yük duyarlı; elle koş (bkz. yukarıdaki yorum)"]
    fn cpu_cost_per_second_of_audio() {
        let secs = 20.0;
        let audio = music_like(secs);
        let chunk = 1024 * 2; // Vorbis paketi ~1024 kare
        let levels = AudioLevels::new();
        let mut best = f64::MAX;
        // Duvar saati: paralel testler / başka derlemeler araya girer; en
        // iyi koşu (bol deneme) asıl maliyeti verir.
        for _ in 0..25 {
            let mut tap = LevelTap::new(&levels, 30_000_000);
            let mut t = 0u64;
            let mut frame = 0u64;
            let start = Instant::now();
            for c in audio.chunks(chunk) {
                tap.analyze(c, t, t + 120_000_000, frame);
                frame += (c.len() / 2) as u64;
                t += 23_219_954;
            }
            best = best.min(start.elapsed().as_secs_f64());
        }
        let per_sec = best / secs;
        println!(
            "CPU: {:.3} ms / 1 sn ses (44,1 kHz stereo f64) = tek çekirdeğin %{:.3}'ü ({} örnek / sn)",
            per_sec * 1e3,
            per_sec * 100.0,
            (FS * 2.0) as u64
        );
        if !cfg!(debug_assertions) {
            assert!(per_sec < 0.003, "çıta: < %0,3; ölçülen %{}", per_sec * 100.0);
        }
    }
}
