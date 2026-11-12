//! Openify'ın kendi ses çıkışı: librespot'un rodio çıkışının yerine.
//!
//! # Neden
//!
//! librespot-playback 0.8.0'ın rodio çıkışı (`audio_backend/rodio.rs`)
//! yazılan her paketi rodio'nun kuyruğuna ekler, kuyruk 26 paketi aşınca 10
//! ms'lik uykularla bekler: kararlı durumda ~0,6 sn ses kuyrukta durur.
//!   - Duraklat: `stop` = `sleep_until_end` + `pause`; kuyruk sonuna kadar
//!     çalar (~0,6 sn), çalar iş parçacığı bu sırada bloklanır.
//!   - Sarma ve kullanıcının şarkı değiştirmesi çıkışa hiç dokunmaz (gapless
//!     açık): eski sesin ~0,6 sn'si arayüz yeni konumu gösterirken çalar.
//!   - Ses düzeyi kuyruğun önünde uygulanıyordu: ~0,6 sn sonra duyulur.
//!
//! rodio tutamağı librespot'ta gizli olduğu için sarmalayarak düzeltilemez;
//! `Player::new`'in `sink_builder`'ıyla kendi çıkışımızı veriyoruz.
//!
//! # Yapı
//!
//! ```text
//!   çalar ──write──▶ Writer ──▶ halka (kilitsiz; tek yazar, tek okur)
//!                                      │
//!   ses kartı ◀── rodio karıştırıcı ◀── RingSource (zarf, ses düzeyi)
//!   motor ──duck / flush / set_gain──▶ OutputCtl (atomikler)
//! ```
//!
//! - Halka kare (stereo örnek çifti) sayar; `written` / `read` hiç geri
//!   gitmeyen toplam sayaçlar, kuyruk = fark. Hücreler `AtomicU32` (f32
//!   bitleri): güvensiz kod yok, ses iş parçacığı hiç kilit almaz, bellek
//!   ayırmaz, beklemez.
//! - `write` paketi koyar, sonra kuyruk HEDEFE (kare cinsinden, varsayılan
//!   120 ms, `OPENIFY_SINK_MS`) inene dek bekler (`park`; ses iş parçacığı
//!   uyandırır, 10 ms'lik yoklama yok).
//! - Ses iş parçacığı (`RingSource`) hiçbir zaman `None` döndürmez
//!   (karıştırıcı biten kaynağı atar); veri yoksa sıfır çalar, akış ortasında
//!   olduysa bir "xrun" sayar ve hedefi büyütür.
//!
//! # Komutlar (motor iş parçacığından, beklemeden)
//!
//! - `duck`: duraklatmadan HEMEN önce: ses ~8 ms'lik inişle susar (çalar
//!   iş parçacığının `pause`'u işlemesini beklemez). Kuyruk atılmaz; devam
//!   edince kaldığı kareden, bit bit kesintisiz sürer (8 ms'lik girişle).
//! - `flush`: kullanıcının sarması / şarkı değiştirmesinden HEMEN önce:
//!   kuyruktaki eski ses ~8 ms'lik inişle atılır.
//! - `set_gain`: ses düzeyi çıkışta, ~10 ms'lik rampayla (kuyruğun önünde
//!   değil).
//!
//! Şarkı kendiliğinden bittiğinde (`EndOfTrack` → sıradaki) hiçbiri yapılmaz:
//! boşluksuz geçişte önceki şarkının kuyruktaki sonu çalmalı.
//!
//! # Eski ses nasıl ayıklanıyor (kapı)
//!
//! Komutlar çalara kanaldan gider, çalar onları kendi döngüsünde işler. Çalar
//! iş parçacığı `flush`tan sonra, sarmayı işlemeden önce bir paket daha
//! (eski konumdan) yazabilir. Bunu kesinleştirmek için motor her
//! `flush`/`duck`tan hemen sonra çalara bir İŞARET gönderir
//! (`PlayerPort::mark(no)`: çalar bunu olduğu gibi
//! `SessionClientChanged { client_id: "openify-mark", client_name: no }`
//! olayı olarak geri yollar; `VolumeChanged` değil, çünkü Spotify Connect
//! (Spirc) telefondan gelen ses düzeyini o olayla bildirir ve sayısı işaretle
//! çakışıp kapıyı erken açabilirdi). Çıkış, çaların olay
//! kanalına abone; `flush`tan sonra paketleri, önce işareti, ARDINDAN
//! komutun sonucu olan bir olayı (`Seeked`, `PlayRequestIdChanged`, …)
//! görene dek atar. Olaylar yazmayla aynı iş parçacığında, yazmadan önce
//! gönderildiği için sıra kesindir. (İşaret olmasa, `flush`tan önce
//! işlenmiş eski bir komutun olayı kapıyı erken açabilirdi.)
//!
//! # Zaman: ses ne zaman duyulacak
//!
//! Ses kartı örnekleri geri çağrılarla, patlamalar halinde çeker (WASAPI
//! paylaşımlı kipte ~10 ms'de bir). Ses iş parçacığı her patlamanın başında
//! saati okur, bir sonraki patlamanın anını tahmin eder ve "0. karenin sanal
//! çekilme anı"nı (`anchor`) yayar: F. kare `anchor + F / 44 100`'de çekilir,
//! `+ D` (aygıt tarafı gecikmesi, `levels.rs`) sonra duyulur. Tek bir atomik
//! değer, yırtılma yok; kuyruk ne kadar uzun olursa olsun hata birikmez.
//!
//! # Şarkı sonu payı (kuyruk kuyruğu)
//!
//! 7 Kasım soak'unda önyüklenmiş şarkının ilk paketi diskte 2,65 sn takıldı
//! (NTFS onarımı); 120 ms'lik kuyruk boşaldı. Motor sıradakini önyükleyince
//! (`set_tail(true)`) hedef ~4 sn'ye çıkar: çalar şarkının son saniyelerini
//! önden çözer, geçişte 3,5 sn'ye dek bir takılma duyulmaz. Yeni şarkı kendi
//! sesiyle bu payı doldurunca hedef yine 120 ms'ye iner (kuyruk kendiliğinden
//! erir). Kullanıcı komutları (duraklat, sar, şarkı değiştir, ses düzeyi)
//! kuyruğun boyundan bağımsız, çıkışta uygulandığı için yine anında.
//!
//! Kuyruk derinleşince çaların `TrackChanged`'i duyulan sınırdan ~4 sn önce
//! gelir; motor sınırın karesini `begin_handover` ile alır, arayüze şarkı
//! değişimini o kare DUYULUNCA bildirir (bkz. `handover.rs`).
//!
//! # Aygıt gidince / varsayılan aygıt değişince
//!
//! - Aygıt giderse (akış hatası, Windows "artık yok" bildirimi) yazar hemen
//!   hata döner, çalar duraklar; kuyruk ve konum korunur (duyulan son kare).
//!   Çal'a basınca o anki varsayılan aygıt açılır, kaldığı kareden sürer.
//! - Kullanıcı Windows'ta varsayılan aygıtı değiştirirse (eski aygıt hâlâ
//!   etkin) çalarken yeni aygıta geçilir: yeni akış önce bekleme kipinde açılır,
//!   eskisi 8 ms'lik inişle bırakır, yenisi aynı kareden 8 ms'lik girişle
//!   sürer (`RingSource::epoch`).
//!
//! # Dosyalar
//!
//! - `mod.rs`: bu başlık, ortak sabitler, `OutputCtl` (motorun tutamağı ve
//!   paylaşılan atomikler), iz yazıcısı.
//! - `writer.rs`: `Writer` (çalar iş parçacığı: kapı, söz saati tabanı, halkaya
//!   yazma, kuyruğu bekleme).
//! - `source.rs`: `RingSource` (ses iş parçacığı: halkadan okuma, zarf, ses
//!   düzeyi).
//! - `device.rs`: `Device` (ses kartı akışı, aygıt değişimi) ve aygıt seçimi.
//! - `eq.rs`: 10 bantlı ekolayzır (`EqCtl` ayar paylaşımı, `Eq` süzgeç).
//! - `beat.rs`, `automix.rs`: AutoMix çözümlemesi (`BeatTap`) ve kararı / sessizlik
//!   atlama (bkz. "AutoMix" başlığı altındaki notlar, `ARCHITECTURE.md`).
//! - `karaoke.rs`: vokal azaltma (orta / yan; `KaraokeCtl`, `Karaoke`).
//! - `watch.rs`: Windows varsayılan aygıt bildirimleri.
//! - `tests.rs`: aygıtsız, sanal saatli testler ve altın test.

mod automix;
mod beat;
mod device;
pub(in crate::engine) mod eq;
mod karaoke;
mod ring;
mod source;
mod watch;
mod writer;
mod xfade;

pub use automix::{EdgeView, FALLBACK_MS as AUTOMIX_FALLBACK_MS};
pub use beat::{Analysis, BeatTap, ENV_SECS, Edge};
#[cfg(test)]
pub use beat::{HAS_HEAD, HAS_TAIL};
pub(in crate::engine) use device::Device;
pub use device::{
    default_output_device_name, has_device, output_device_names, set_preferred_device,
};
pub(in crate::engine) use writer::Writer;
pub use xfade::{Curve, MAX_OVERLAP_MS, Transition, TransitionSpec};

use ring::RingBuf;

use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU32, AtomicU64, Ordering::*},
        mpsc,
    },
    thread::{self, Thread},
    time::{Duration, Instant},
};

use librespot::playback::player::{PlayerEvent, PlayerEventChannel};

use super::levels::AudioLevels;

/// Kanal sayısı ve örnekleme hızı (librespot'un sabitleri; `tap.rs`
/// eşitliği derleme zamanında doğrular).
pub const CHANNELS: usize = 2;
pub const RATE: u32 = 44_100;

/// Halka kapasitesi (kare, 2'nin kuvveti): 262 144 kare = 5,94 sn (2 MiB).
/// En derin hedef şarkı sonu payı (4 sn) + bir paket; fazlası yalnız çıkış
/// bekletilirken dolar. Geçiş (crossfade) açıkken halka büyür (`ring_frames`).
const CAP_FRAMES: u64 = 1 << 18;
/// Halkanın hedefin ötesinde bırakılan payı (kare): bir paket + yazıcının
/// bekleme taneciği. Şarkı sonu payı `kapasite − RING_SLACK`i aşmaz.
const RING_SLACK: u64 = 16_384;
/// En büyük halka (kare): 12 sn bindirme + 4 sn pay + paket için 2^20 kare =
/// 23,8 sn (8 MiB).
const MAX_RING_FRAMES: u64 = 1 << 20;
/// Yazar, ses iş parçacığının okuma ucunun (`read`) bu kadar ilerisinden
/// öncesini DEĞİŞTİRMEZ (bindirme; bkz. `writer.rs`): ses iş parçacığı bir
/// blok (64) + iniş zarfı (353) kadar önde okur, yazıcı iş parçacığı
/// ölçülmeden kesilirse (zamanlayıcı) pay: 8 192 kare = 186 ms.
const GUARD_FRAMES: u64 = 8_192;
/// Bundan kısa bindirme yapılmaz (boşluksuz geçişe düşülür): 2 048 kare = 46 ms.
const MIN_OVERLAP_FRAMES: u64 = 2_048;
/// Ses iş parçacığının bir seferde halkadan aldığı kare (1,45 ms). Küçük:
/// komutlara tepki ve zaman tahmini bu tanecikle.
const BLOCK: usize = 64;
/// İniş / çıkış zarfı: 353 kare = 8 ms (doğrusal).
pub const FADE_FRAMES: u32 = 353;
/// Ses düzeyi rampası: 441 kare = 10 ms.
const GAIN_RAMP_FRAMES: u32 = 441;

/// Çıkış kapısının işaretinin olay kimliği (`SessionClientChanged.client_id`).
/// İşaret, çaların olay kanalından geri gelen ve yalnız bu kimlikle tanınan bir
/// olaydır; `VolumeChanged` KULLANILMAZ çünkü Spirc (Spotify Connect) telefondan
/// gelen ses düzeyini aynı olayla bildirir ve kapıyı erken açabilirdi. Sıra
/// numarası `client_name`de ondalık yazıdır (64 bit; eskiden `u16`a sığması gerekirdi).
pub(in crate::engine) const MARK_CLIENT_ID: &str = "openify-mark";

/// Bir olay çıkış kapısının işareti mi? İşaretin sıra numarası.
pub(in crate::engine) fn mark_of(event: &PlayerEvent) -> Option<u64> {
    match event {
        PlayerEvent::SessionClientChanged {
            client_id,
            client_name,
            ..
        } if client_id == MARK_CLIENT_ID => client_name.parse().ok(),
        _ => None,
    }
}

/// `mark`ın çaldan geri geldiği olay (sınamalarda çaların işlevini taklit eder).
#[cfg(test)]
pub(in crate::engine) fn mark_event(mark: u64) -> PlayerEvent {
    PlayerEvent::SessionClientChanged {
        client_id: MARK_CLIENT_ID.to_string(),
        client_name: mark.to_string(),
        client_brand_name: String::new(),
        client_model_name: String::new(),
    }
}

const DEFAULT_TARGET_MS: u64 = 120;
const MIN_TARGET_MS: u64 = 20;
const MAX_TARGET_MS: u64 = 1000;
/// Her xrun'da hedef bu kadar büyür, en çok `XRUN_TARGET_LIMIT_MS`'e (ya da
/// başlangıç hedefine) kadar.
const XRUN_GROW_MS: u64 = 40;
const XRUN_TARGET_LIMIT_MS: u64 = 400;
/// Şarkı sonu payı: sıradaki önyüklenince hedef (bkz. başlık). 3,5 sn'lik
/// takılma + motorun komut gidiş dönüşü + bir paket için pay.
const TAIL_MS: u64 = 4000;
const _: () = assert!(TAIL_MS * RATE as u64 / 1000 + 8192 < CAP_FRAMES);
/// Yazar iki paket arasında (kuyruğu beklemesi hariç) bundan uzun çözdüyse
/// izde `producer_gap`.
const PRODUCER_GAP_NS: u64 = 200_000_000;
/// Varsayılan aygıt değişince yeni aygıta geçmeden önce beklenen süre: eski
/// aygıt aslında gidiyorsa (kulaklık çıktı) bu arada akış hatası / "artık
/// yok" bildirimi gelir; o durumda geçilmez, duraklatılır.
const REROUTE_GRACE: Duration = Duration::from_millis(250);
/// Geçişte eski akışın inişi bitirmesi için en çok bu kadar beklenir.
const RELEASE_WAIT: Duration = Duration::from_millis(60);

/// Yazar en çok bu kadar uyur, sonra koşulu yeniden yoklar (kaçan bir
/// uyandırmaya karşı emniyet; normalde ses iş parçacığı uyandırır).
const PARK: Duration = Duration::from_millis(20);
/// Çıkış bu kadar süre hiç ilerlemezse (aygıt çıktı, akış öldü) `write`
/// hata döner; çalar duraklar, iş parçacığı sonsuza dek takılmaz.
const STALL: Duration = Duration::from_secs(2);
/// Kapı kapalıyken en çok bu kadar kare atılır; komutun sonucu hiç gelmezse
/// (çalar hatası) kapı kendiliğinden açılır.
const GATE_LIMIT_FRAMES: u64 = RATE as u64;
/// Ses iş parçacığının iki `fill` arası bundan uzunsa yeni bir geri çağrı
/// (patlama) başlamıştır.
const BURST_GAP_NS: u64 = 1_000_000;
const DEFAULT_PERIOD_NS: f64 = 10_000_000.0;
/// İz dosyasına düzenli satır aralığı.
const TRACE_TICK: Duration = Duration::from_millis(100);

/// `device_notice` değerleri.
const DEVICE_OK: u8 = 0;
const DEVICE_LOST: u8 = 1;
const DEVICE_NONE: u8 = 2;

/// Motorun kullanıcıya söyleyeceği aygıt sorunu (`take_device_notice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceNotice {
    /// Çalarken aygıt gitti; çalar duraklatıldı, konum korundu.
    Lost,
    /// Çal'a basıldı ama açılacak ses aygıtı yok.
    NoDevice,
}

/// Boşluksuz geçişin durumu (`OutputCtl::handover_state`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Handover {
    /// Geçiş yok ya da bir `flush` iptal etti.
    None,
    /// Önceki şarkının sonu hâlâ çalıyor; sınır en erken `eta` sonra duyulur.
    Pending { eta: Duration },
    /// Yeni şarkının ilk karesi duyuldu; şu an duyulan konumu.
    Audible { new_ms: u32 },
}

fn frames_to_ns(frames: f64) -> f64 {
    frames * 1e9 / RATE as f64
}

fn ms_to_frames(ms: u64) -> u64 {
    ms * RATE as u64 / 1000
}

/// `crossfade_ms` milisaniyelik geçişe yetecek halka kapasitesi (kare): geçiş
/// kapalıyken bugünkü `CAP_FRAMES`; açıkken bindirme + şarkı sonu payı + paket
/// payını alan en küçük 2'nin kuvveti (5 sn için 2^19 = 4 MiB, 12 sn için
/// 2^20 = 8 MiB).
pub fn ring_frames(crossfade_ms: u32) -> u64 {
    if crossfade_ms == 0 {
        return CAP_FRAMES;
    }
    let want = ms_to_frames(crossfade_ms.min(MAX_OVERLAP_MS) as u64 + TAIL_MS) + RING_SLACK;
    want.next_power_of_two().clamp(CAP_FRAMES, MAX_RING_FRAMES)
}

// ---------------------------------------------------------------------------
// Ortak durum
// ---------------------------------------------------------------------------

/// Çıkışın paylaşılan durumu ve motorun tutamağı. Bütün yöntemler
/// beklemeden döner; her iş parçacığından çağrılabilir.
pub struct OutputCtl {
    /// Karışık stereo örnekler: 1. yuva (kurulurken `ring_frames` kadar). Geçiş
    /// açılınca ikinci, daha büyük bir yuva (`big`) eklenebilir (bkz. `reserve`).
    ring: RingBuf,
    /// 2. yuva: motor iş parçacığında ayrılır (`reserve`), bir kez konur.
    ///    Ses iş parçacığı yalnız `get` çağırır (atomik okuma).
    big: OnceLock<RingBuf>,
    /// `big`e geçişin ilk karesi (`u64::MAX`: geçilmedi). Bu kareden öncesi 1.
    /// yuvada, sonrası `big`de; yazar koyar (bkz. `Writer::grow`), ses iş
    /// parçacığı blok başına bir okur.
    big_from: AtomicU64,
    /// Yazarın sıradaki karesinin halkadaki yeri (yazma ucu): bindirme sürerken
    /// `written`ın gerisinde (bkz. `Writer::push`); yoksa `written`a eşit.
    head: AtomicU64,
    /// Motorun sıradaki kendiliğinden geçiş için verdiği `TransitionSpec`
    /// (bkz. `arm_transition`; bit düzeni `xfade.rs`).
    armed: AtomicU64,
    /// Süren geçiş (`Transition`, tek atomik; 0 = yok).
    transition: AtomicU64,
    /// AutoMix: gelen şarkının baş ucu (`EdgeView::pack`; 0 = bilinmiyor); motor
    /// `arm_automix` ile `armed` ile birlikte verir, `armed` ile birlikte silinir.
    armed_in: AtomicU64,
    /// AutoMix açık: çalar iş parçacığı şarkıları çözümler (`BeatTap`) ve sessizlik
    /// atlar. Kapalıyken ses yolu hiç değişmez.
    automix: AtomicBool,
    /// Çözümlenen şarkılar (çalar iş parçacığı koyar, motor `take_analyses` ile alır).
    analyses: Mutex<Vec<(String, Analysis)>>,
    /// Testler: bindirmenin değiştirdiği kare ile o andaki `read` arasındaki
    /// en küçük fark (`GUARD_FRAMES`ten küçük olmamalı).
    #[cfg(test)]
    min_lead: AtomicU64,
    /// Şimdiye dek yazılan / ses iş parçacığının aldığı kare (toplam).
    written: AtomicU64,
    read: AtomicU64,
    /// Kuyruk hedefi (kare); xrun olunca büyür.
    target: AtomicU64,
    target_limit: u64,
    /// Şarkı sonu payı açık: hedef en az `TAIL_MS` (bkz. başlık); her
    /// açılışta kuşak artar.
    tail: AtomicBool,
    tail_gen: AtomicU64,
    /// Boşluksuz geçişin sınırı: yeni şarkının ilk karesi (`written`
    /// ekseni; `i64::MIN`: geçiş yok ya da `flush` iptal etti) ve geçişten
    /// önce çalan şarkının tabanı (bkz. `begin_handover`).
    ho_boundary: AtomicI64,
    ho_old_base: AtomicI64,
    /// Ses aygıtı gitti (akış hatası / Windows bildirimi): yazar hata döner.
    device_lost: AtomicBool,
    /// Motora söylenecek aygıt sorunu (`DEVICE_*`; motor `take_device_notice`
    /// ile alır).
    device_notice: AtomicU8,
    /// Halkadan okumaya yetkili ses kaynağı (`RingSource::epoch`) ve inişini
    /// bitirip bırakan en son kaynak: aygıt değişiminde iki akış aynı anda
    /// okumasın.
    source_gen: AtomicU64,
    released_gen: AtomicU64,
    /// Açık akışın açıldığı andaki varsayılan aygıt sayacı (`watch`).
    route_gen: AtomicU64,
    /// Çalar `start` / `stop`.
    running: AtomicBool,
    /// Motorun `duck`'ı (çalar `start`'ı ya da motorun `unduck`'ı kaldırır).
    duck: AtomicBool,
    /// Son `duck`'ın işaret numarası.
    duck_mark: AtomicU64,
    /// İşaret sayacı (`flush` ve `duck` paylaşır; 0 = hiç).
    marks: AtomicU64,
    /// Son `flush`'ın işaret numarası / yazarın onayladığı.
    flush_req: AtomicU64,
    flush_ack: AtomicU64,
    /// Onaylanan `flush`ta bu kareden öncesi atılır (yazar koyar).
    discard_to: AtomicU64,
    /// Ses düzeyi hedefi (f32 bitleri).
    gain: AtomicU32,
    /// Çaların son yazdığı paketin kare sayısı (konum düzeltmesi için).
    last_packet: AtomicU64,
    /// Ses iş parçacığı bekletmede (zarf 0, kuyruğa dokunmuyor).
    held: AtomicBool,
    /// F. karenin çekilme anı = `anchor + F / RATE` (ns, `levels` saati).
    anchor: AtomicI64,
    /// Çalan şarkının 0. ms'sinin halkadaki kare numarası (`written`
    /// sayacıyla aynı eksen; `i64::MIN`: bilinmiyor). Yazar, çaların konum
    /// bildiren olaylarından koyar (bkz. `Writer::sync`); arayüzün söz
    /// saati buradan (`audible_track_ms`).
    track_base: AtomicI64,
    xruns: AtomicU64,
    underrun_frames: AtomicU64,
    played: AtomicU64,
    /// Ses iş parçacığının kaydettiği son anlar (iz için; 0 = hiç).
    t_silent: AtomicU64,
    t_audible: AtomicU64,
    t_gain: AtomicU64,
    t_xrun: AtomicU64,
    /// Yazar uyuyor mu, kim (ses iş parçacığı uyandırır).
    writer_waiting: AtomicBool,
    writer: OnceLock<Thread>,
    /// Çaların olay kanalı; yazar ilk fırsatta alır.
    events: Mutex<Option<PlayerEventChannel>>,
    levels: AudioLevels,
    /// Ekolayzır ayarı: motor yazar, ses iş parçacığı okur (bkz. `eq.rs`).
    eq: eq::EqCtl,
    /// Karaoke ayarı: motor yazar, ses iş parçacığı okur (bkz. `karaoke.rs`).
    karaoke: karaoke::KaraokeCtl,
    /// Aygıt tarafı gecikmesi (geri çağrıdan hoparlöre), ns.
    device_ns: u64,
    /// İz satırları: (an, olay, ek değer ms; yoksa NaN).
    trace: Option<mpsc::Sender<(u64, &'static str, f64)>>,
    /// Testlerde sanal saat.
    #[cfg(test)]
    clock: Option<Arc<AtomicU64>>,
}

impl OutputCtl {
    /// Geçişsiz (2 MiB'lık halkalı) çıkış; testler.
    #[cfg(test)]
    pub fn new(levels: AudioLevels, gain: f32) -> Arc<Self> {
        Self::new_for(levels, gain, 0)
    }

    /// `gain`: başlangıç ses düzeyi (0..1, doğrusal çarpan). `crossfade_ms`:
    /// ayarlardaki geçiş süresi; halka baştan ona yetecek boyda kurulur (geçiş
    /// kapalıyken bugünkü 2 MiB).
    pub fn new_for(levels: AudioLevels, gain: f32, crossfade_ms: u32) -> Arc<Self> {
        Self::build(levels, gain, ring_frames(crossfade_ms), None)
    }

    fn build(
        levels: AudioLevels,
        gain: f32,
        ring_frames: u64,
        #[allow(unused)] clock: Option<Arc<AtomicU64>>,
    ) -> Arc<Self> {
        let target_ms = std::env::var("OPENIFY_SINK_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| (MIN_TARGET_MS..=MAX_TARGET_MS).contains(v))
            .unwrap_or(DEFAULT_TARGET_MS);
        // Geliştirme kancası: OPENIFY_SINK_TRACE=<csv> (bkz. `trace_loop`).
        let trace_file =
            std::env::var_os("OPENIFY_SINK_TRACE").and_then(|p| match File::create(&p) {
                Ok(f) => Some(f),
                Err(e) => {
                    log::warn!(
                        "OPENIFY_SINK_TRACE açılamadı ({}): {e}",
                        p.to_string_lossy()
                    );
                    None
                }
            });
        let (trace_tx, trace_rx) = match trace_file {
            Some(f) => {
                let (tx, rx) = mpsc::channel();
                (Some(tx), Some((rx, f)))
            }
            None => (None, None),
        };
        let ctl = Arc::new(Self {
            ring: RingBuf::new(ring_frames),
            big: OnceLock::new(),
            big_from: AtomicU64::new(u64::MAX),
            head: AtomicU64::new(0),
            armed: AtomicU64::new(0),
            transition: AtomicU64::new(0),
            armed_in: AtomicU64::new(0),
            automix: AtomicBool::new(false),
            analyses: Mutex::new(Vec::new()),
            #[cfg(test)]
            min_lead: AtomicU64::new(u64::MAX),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
            target: AtomicU64::new(ms_to_frames(target_ms)),
            target_limit: ms_to_frames(target_ms.max(XRUN_TARGET_LIMIT_MS)),
            tail: AtomicBool::new(false),
            tail_gen: AtomicU64::new(0),
            ho_boundary: AtomicI64::new(i64::MIN),
            ho_old_base: AtomicI64::new(i64::MIN),
            device_lost: AtomicBool::new(false),
            device_notice: AtomicU8::new(DEVICE_OK),
            source_gen: AtomicU64::new(1),
            released_gen: AtomicU64::new(0),
            route_gen: AtomicU64::new(watch::default_gen()),
            running: AtomicBool::new(false),
            duck: AtomicBool::new(false),
            duck_mark: AtomicU64::new(0),
            marks: AtomicU64::new(0),
            flush_req: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            discard_to: AtomicU64::new(0),
            gain: AtomicU32::new(gain.clamp(0.0, 1.0).to_bits()),
            last_packet: AtomicU64::new(0),
            held: AtomicBool::new(true),
            anchor: AtomicI64::new(i64::MIN),
            track_base: AtomicI64::new(i64::MIN),
            xruns: AtomicU64::new(0),
            underrun_frames: AtomicU64::new(0),
            played: AtomicU64::new(0),
            t_silent: AtomicU64::new(0),
            t_audible: AtomicU64::new(0),
            t_gain: AtomicU64::new(0),
            t_xrun: AtomicU64::new(0),
            writer_waiting: AtomicBool::new(false),
            writer: OnceLock::new(),
            events: Mutex::new(None),
            device_ns: super::levels::device_delay_ns(),
            levels,
            eq: eq::EqCtl::new(),
            karaoke: karaoke::KaraokeCtl::new(),
            trace: trace_tx,
            #[cfg(test)]
            clock,
        });
        // Arayüz söz saatini `AudioLevels` tutamağından okur (bağlantı
        // yenilenince yeni çıkış eskisinin yerine geçer).
        ctl.levels.attach_output(Arc::downgrade(&ctl));
        if let Some((rx, file)) = trace_rx {
            let weak = Arc::downgrade(&ctl);
            let _ = thread::Builder::new()
                .name("openify-sink-trace".into())
                .spawn(move || trace_loop(weak, rx, BufWriter::new(file)));
        }
        ctl
    }

    /// Testler için: sanal saatli, aygıtsız çıkış.
    #[cfg(test)]
    pub(super) fn with_clock(levels: AudioLevels, clock: Arc<AtomicU64>) -> Arc<Self> {
        Self::build(levels, 1.0, CAP_FRAMES, Some(clock))
    }

    /// Testler için: sanal saatli, `crossfade_ms`'e göre boyutlanmış halkalı.
    #[cfg(test)]
    pub(super) fn with_clock_for(
        levels: AudioLevels,
        clock: Arc<AtomicU64>,
        crossfade_ms: u32,
    ) -> Arc<Self> {
        Self::build(levels, 1.0, ring_frames(crossfade_ms), Some(clock))
    }

    /// Çaların olay kanalını çıkışa bağlar (kapı ve işaretler için; bkz.
    /// başlık). `Player::new`'den hemen sonra, ilk komuttan önce çağrılmalı.
    pub fn attach_events(&self, events: PlayerEventChannel) {
        if let Ok(mut slot) = self.events.lock() {
            *slot = Some(events);
        }
    }

    pub(super) fn levels(&self) -> &AudioLevels {
        &self.levels
    }

    pub(super) fn now_ns(&self) -> u64 {
        #[cfg(test)]
        if let Some(c) = &self.clock {
            return c.load(Relaxed);
        }
        self.levels.now_ns()
    }

    /// Duraklatmadan hemen önce: ses beklemeden, ~8 ms'lik inişle susar.
    /// Dönen işaret `PlayerPort::mark` ile çalara verilmeli (çalar onu
    /// `SessionClientChanged` olarak geri yollar, bkz. `MARK_CLIENT_ID`),
    /// SONRA `player.pause()`.
    pub fn duck(&self) -> u64 {
        let mark = self.marks.fetch_add(1, SeqCst) + 1;
        self.duck_mark.store(mark, SeqCst);
        self.duck.store(true, SeqCst);
        self.cut_levels();
        self.wake_writer();
        self.trace("duck");
        mark
    }

    /// `duck`'ı kaldırır (kullanıcı yeniden çalınca). Çıkış duruyorsa
    /// (`stop`) çalar `start` diyene dek sessiz kalır.
    pub fn unduck(&self) {
        if self.duck.swap(false, SeqCst) {
            self.trace("unduck");
        }
    }

    /// Sınama: çıkış şu an susturulmuş mu (`duck`)?
    #[cfg(test)]
    pub(in crate::engine) fn is_ducked(&self) -> bool {
        self.duck.load(SeqCst)
    }

    /// Sınama: `flush` istendi mi (en son işaret)?
    #[cfg(test)]
    pub(in crate::engine) fn flush_requested(&self) -> u64 {
        self.flush_req.load(SeqCst)
    }

    /// Kullanıcının sarmasından / şarkı değiştirmesinden hemen önce: kuyruktaki
    /// eski ses ~8 ms'lik inişle atılır. Dönen işaret
    /// `PlayerPort::mark` ile verilmeli, SONRA komut.
    pub fn flush(&self) -> u64 {
        let mark = self.marks.fetch_add(1, SeqCst) + 1;
        self.flush_req.store(mark, SeqCst);
        // Önceki şarkının kuyruktaki sonu atılıyor: süren geçiş yok sayılır.
        self.ho_boundary.store(i64::MIN, SeqCst);
        self.cut_levels();
        self.wake_writer();
        self.trace("flush");
        mark
    }

    /// Ses düzeyi (0..1, doğrusal çarpan; ses kartına ~10 ms'lik rampayla).
    pub fn set_gain(&self, gain: f32) {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.gain.swap(gain.to_bits(), SeqCst) != gain.to_bits() {
            self.trace("volume");
        }
    }

    /// Ekolayzır ayarı (motor iş parçacığı; beklemez). `gains_db`: 10 bandın
    /// kazancı (31,5 Hz … 16 kHz, dB, ±24 ile sınırlı), `preamp_db`: elle ön
    /// kazanç. Çıkışa uygulanan ön kazanç `preamp_db − max(0, bileşik yanıtın
    /// tepesi)` (bkz. `eq.rs`): komşu artı bantların etekleri de hesaba
    /// girer, çıkış girişi aşmaz. Katsayılar burada hesaplanır, ses iş
    /// parçacığı 8 blokta ona süzülür; kapalıyken (ya da hepsi 0 dB, ön
    /// kazanç 0 dB iken) çıkış bit bit değişmez. Motor `Command::SetEq` ile
    /// çağırır (bkz. `engine::Engine::apply_eq`).
    pub fn set_eq(&self, enabled: bool, gains_db: [f32; eq::BANDS], preamp_db: f32) {
        self.eq.set(enabled, &gains_db, preamp_db);
    }

    /// Karaoke (vokal azaltma; bkz. `karaoke.rs`): orta kanalın 150 Hz – 8 kHz
    /// bandı `amount` (0..1) oranında bastırılır, yan kanal ve bas korunur.
    /// Motor iş parçacığı; beklemez. Kapalıyken (ya da `amount` 0) çıkış bit bit
    /// değişmez; mono kaynakta etkisizdir. Motor `Command::SetKaraoke` ile çağırır.
    pub fn set_karaoke(&self, enabled: bool, amount: f32) {
        self.karaoke.set(enabled, amount);
    }

    /// Kuyrukta çalınmayı bekleyen kare (atılacak eski ses hariç).
    pub fn queued_frames(&self) -> u64 {
        if self.flush_pending() {
            return 0;
        }
        self.queued_raw()
    }

    /// Ses kartına giden (kuyruktan çalınan) toplam kare.
    #[allow(dead_code)] // tanı / ölçüm için dışa açık
    pub fn played_frames(&self) -> u64 {
        self.played.load(Relaxed)
    }

    /// Akış ortasında kuyruğun boşaldığı (ses kesildiği) kez sayısı.
    pub fn xruns(&self) -> u64 {
        self.xruns.load(Relaxed)
    }

    /// Çaların bildirdiği konuma (`Paused` / devamdaki `Playing`:
    /// son ÇÖZÜLEN paketin başı) eklenince DUYULAN konumu veren fark (ms).
    /// Kuyruktaki, henüz çalınmamış ses kadar geridedir.
    pub fn audible_offset_ms(&self) -> i64 {
        if self.flush_pending() {
            return 0;
        }
        // Yazma ucu: bindirme sürerken yeni şarkının çözülmüş son karesi
        // `written`ın gerisinde (`head`), yoksa ikisi aynı.
        let w = self.head.load(SeqCst);
        let mut r = self.read.load(SeqCst).max(self.discard_to.load(SeqCst));
        if self.fading_out() {
            // İniş daha bitmedi: en çok bir zarf boyu daha çalınacak.
            r += FADE_FRAMES as u64;
        }
        let queued = w.saturating_sub(r) as i64;
        let last = self.last_packet.load(SeqCst) as i64;
        (last - queued) * 1000 / RATE as i64
    }

    /// `audible_offset_ms` uygulanmış konum.
    pub fn audible_position_ms(&self, reported_ms: u32) -> u32 {
        (reported_ms as i64 + self.audible_offset_ms()).clamp(0, u32::MAX as i64) as u32
    }

    /// Şu an hoparlörden DUYULAN yerin çalan şarkıdaki konumu (ms; arayüzün
    /// söz saati). Ses kartının saatinden (`anchor`) ve aygıt gecikmesinden
    /// hesaplanır: kuyruğun boyu, boşluksuz geçişte önceki şarkının kuyrukta
    /// kalan sonu, sarmadaki gecikme hepsi içinde. Boşluksuz geçişte yeni
    /// şarkı henüz duyulmuyorsa EKSİ değer. Bilinmiyorsa (`flush` sürüyor,
    /// şarkı başlamadı, ses iş parçacığı çalışmadı) `None`.
    pub fn audible_track_ms(&self) -> Option<i64> {
        let base = self.track_base.load(SeqCst);
        if base == i64::MIN || self.flush_pending() {
            return None;
        }
        let heard = self.heard_frame()?;
        Some(((heard - base as f64) * 1000.0 / RATE as f64).round() as i64)
    }

    /// Şu an hoparlörden duyulan kare (kesirli, `written` ekseni). Ses kartı
    /// saati bilinmiyorsa (ses iş parçacığı hiç çalışmadı) `None`.
    fn heard_frame(&self) -> Option<f64> {
        let anchor = self.anchor.load(SeqCst);
        if anchor == i64::MIN {
            return None;
        }
        // Henüz çekilmemiş kare duyulamaz (veri beklenirken saat ilerlese de).
        let next = self.read.load(SeqCst).max(self.discard_to.load(SeqCst)) as f64;
        if self.device_lost.load(SeqCst) {
            // Aygıt gitti: saat durdu, son çekilen kare.
            return Some(next);
        }
        let now = self.now_ns() as f64;
        let pulled = (now - self.device_ns as f64 - anchor as f64) * RATE as f64 / 1e9;
        Some(pulled.min(next))
    }

    /// Duraklatma / susturma istendi ama ses iş parçacığı inişi henüz
    /// bitirmedi (aygıt gittiyse iniş hiç olmayacak: kuyruk olduğu yerde).
    fn fading_out(&self) -> bool {
        let holding = !self.running.load(SeqCst) || self.duck.load(SeqCst);
        holding && !self.held.load(SeqCst) && !self.device_lost.load(SeqCst)
    }

    /// Yazarın sıradaki karesinin (`written`) şarkıdaki konumu `position_ms`.
    pub(super) fn set_track_base(&self, position_ms: u32) {
        let written = self.written.load(SeqCst) as i64;
        let base = written - ms_to_frames(position_ms as u64) as i64;
        self.track_base.store(base, SeqCst);
    }

    // ── Şarkı sonu payı ve boşluksuz geçiş ──────────────────────────────

    /// Şarkı sonu payı (bkz. başlık). Motor sıradakini önyükleyince açar,
    /// kullanıcı başka şarkı yükleyince kapatır; boşluksuz geçişten sonra
    /// yazar, yeni şarkı payı kendi sesiyle doldurunca kendisi kapatır.
    pub fn set_tail(&self, on: bool) {
        if on {
            self.tail_gen.fetch_add(1, SeqCst);
        } else {
            // Pay kapandı (başka şarkı yüklendi ya da yeni şarkı payı doldurdu):
            // önceden verilmiş geçiş isteği bu geçişe aitti, bitti.
            self.armed.store(0, SeqCst);
            self.armed_in.store(0, SeqCst);
        }
        if self.tail.swap(on, SeqCst) != on {
            self.trace(if on { "tail_on" } else { "tail_off" });
            // Yazar hedefin altında uyuyorsa hemen doldurmaya başlasın.
            self.wake_writer();
        }
    }

    /// Şarkı kendiliğinden bitti (`EndOfTrack`): şimdiye dek yazılanın hepsi
    /// biten şarkının; sıradaki şarkı tam bu kareden başlar. Çalar `EndOfTrack`
    /// ile sıradaki `load` arasında hiç yazmaz, yani kare kesin. Önceki geçiş
    /// henüz duyulmadıysa (çok kısa şarkı) arayüzün gösterdiği şarkının tabanı
    /// korunur.
    pub fn begin_handover(&self) {
        let boundary = self.written.load(SeqCst) as i64;
        if self.ho_boundary.load(SeqCst) == i64::MIN {
            self.ho_old_base.store(self.track_base.load(SeqCst), SeqCst);
        }
        self.ho_boundary.store(boundary, SeqCst);
        self.trace("handover");
    }

    /// Motor geçişi arayüze bildirdi (ya da bıraktı).
    pub fn end_handover(&self) {
        self.ho_boundary.store(i64::MIN, SeqCst);
    }

    /// Bindirmeli geçişte arayüzün şarkı değişimi yeni şarkının İLK duyulan
    /// karesinde (`Transition::start`) olur, giden şarkının bittiği karede
    /// değil: süren bir boşluksuz geçiş varsa sınırı oraya çeker (`flush`
    /// aynı anda iptal ettiyse dokunmaz).
    pub(super) fn move_handover_boundary(&self, frame: u64) {
        let _ = self
            .ho_boundary
            .try_update(SeqCst, SeqCst, |b| (b != i64::MIN).then_some(frame as i64));
    }

    pub fn handover_state(&self) -> Handover {
        let boundary = self.ho_boundary.load(SeqCst);
        if boundary == i64::MIN {
            return Handover::None;
        }
        match self.heard_frame() {
            Some(heard) if heard >= boundary as f64 => Handover::Audible {
                new_ms: ((heard - boundary as f64) * 1000.0 / RATE as f64).round() as u32,
            },
            _ => {
                let now = self.now_ns();
                let read = self.read.load(SeqCst).max(self.discard_to.load(SeqCst));
                let anchor = self.anchor.load(SeqCst);
                // Sınır karesi çekildiyse (aygıtın tamponunda) saatten; henüz
                // çekilmediyse `pull_ns` (şimdiden önce olamaz). `pull_ns`
                // çekilmiş kareyi de "şimdi" sayar: tahmin D'de takılı kalırdı.
                let pulled_at = if boundary as u64 <= read && anchor != i64::MIN {
                    (anchor as f64 + frames_to_ns(boundary as f64)).max(0.0) as u64
                } else {
                    self.pull_ns(boundary as u64, now)
                };
                let at = pulled_at + self.device_ns;
                Handover::Pending {
                    eta: Duration::from_nanos(at.saturating_sub(now)),
                }
            }
        }
    }

    /// Geçiş sürerken (yeni şarkının sınırı henüz duyulmadı) arayüzün hâlâ
    /// gösterdiği ÖNCEKİ şarkıdaki duyulan konum. `paused`: duraklatma
    /// konumu (iniş bitince duracağı kare, bkz. `audible_offset_ms`). Sınır
    /// duyulduysa ya da geçiş yoksa `None`.
    pub fn handover_old_ms(&self, paused: bool) -> Option<u32> {
        let boundary = self.ho_boundary.load(SeqCst);
        let base = self.ho_old_base.load(SeqCst);
        if boundary == i64::MIN || base == i64::MIN {
            return None;
        }
        let next = self.read.load(SeqCst).max(self.discard_to.load(SeqCst));
        let frame = if paused {
            (next
                + if self.fading_out() {
                    FADE_FRAMES as u64
                } else {
                    0
                }) as f64
        } else {
            self.heard_frame().unwrap_or(next as f64)
        };
        if frame >= boundary as f64 {
            return None;
        }
        Some(((frame - base as f64).max(0.0) * 1000.0 / RATE as f64).round() as u32)
    }

    // ── Geçiş (crossfade) ve büyüyebilen halka ──────────────────────────

    /// Sıradaki kendiliğinden geçiş için istenen bindirme (`Playback`, geçişi
    /// başlatan yükleme komutundan ve şarkı sonu payını açarken ÖNCE verir).
    /// Yazar, yeni şarkının ilk paketinde alır ve geçişi kurar; `overlap` 0 ise
    /// geçiş boşluksuz. Payın derinliğini de belirler (`tail_frames`).
    /// Uygulama `arm_automix` kullanır; bu kısa yol yalnız testlerde.
    #[cfg(test)]
    pub fn arm_transition(&self, spec: TransitionSpec) {
        self.arm_automix(spec, None);
    }

    /// `arm_transition` + AutoMix için gelen şarkının (önbellekteki) baş ucu.
    pub fn arm_automix(&self, spec: TransitionSpec, incoming: Option<EdgeView>) {
        self.armed_in
            .store(incoming.map_or(0, |v| v.pack()), SeqCst);
        self.armed.store(spec.pack(), SeqCst);
        // Yazar hedefin altında uyuyorsa yeni derinliği hemen görsün.
        self.wake_writer();
    }

    /// Verilmiş isteğin gelen şarkı baş ucu (yazar ve testler).
    pub(in crate::engine) fn armed_incoming(&self) -> Option<EdgeView> {
        EdgeView::unpack(self.armed_in.load(SeqCst))
    }

    /// AutoMix'i aç / kapat (motor; beklemez). Kapalıyken çalar iş parçacığında
    /// hiçbir çözümleme ve sessizlik atlama çalışmaz.
    pub fn set_automix(&self, on: bool) {
        self.automix.store(on, SeqCst);
    }

    pub(in crate::engine) fn automix_on(&self) -> bool {
        self.automix.load(Relaxed)
    }

    /// Çalar iş parçacığı: bir şarkının çözümlemesi hazır.
    pub(super) fn publish_analysis(&self, uri: String, analysis: Analysis) {
        if let Ok(mut q) = self.analyses.lock() {
            // Motor almazsa (kopuk) şişmesin.
            if q.len() >= 16 {
                q.remove(0);
            }
            q.push((uri, analysis));
        }
    }

    /// Testler: çalar iş parçacığının yayınını taklit eder.
    #[cfg(test)]
    pub(in crate::engine) fn publish_analysis_for_test(&self, uri: &str, analysis: Analysis) {
        self.publish_analysis(uri.to_owned(), analysis);
    }

    /// Hazır çözümlemeleri alır (motor iş parçacığı).
    pub fn take_analyses(&self) -> Vec<(String, Analysis)> {
        self.analyses
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    /// Şu an verilmiş istek (yazar ve testler).
    pub(in crate::engine) fn armed_spec(&self) -> TransitionSpec {
        TransitionSpec::unpack(self.armed.load(SeqCst))
    }

    /// Testler: halka büyütüldü mü (ikinci yuva ayrıldı mı)?
    #[cfg(test)]
    pub(in crate::engine) fn has_second_slot(&self) -> bool {
        self.big.get().is_some()
    }

    /// Süren / sürecek geçiş: yeni şarkının 0. karesi `start`'ta, `overlap`
    /// kare boyunca eskiyle karışır. Bindirme tamamen çalındıysa (`read`
    /// bitişi geçti) ya da yoksa `None`.
    #[allow(dead_code)] // tanı / ölçüm ve testler için dışa açık
    pub fn transition(&self) -> Option<Transition> {
        let t = Transition::unpack(self.transition.load(SeqCst))?;
        (self.read.load(SeqCst) < t.start + t.overlap as u64).then_some(t)
    }

    /// Yazar: geçiş kuruldu (`overlap` 0 ise yok sayılır).
    pub(super) fn begin_transition(&self, t: Transition) {
        self.transition.store(t.pack(), SeqCst);
        self.trace_arg("xfade", t.overlap as f64 * 1000.0 / RATE as f64);
    }

    /// Yazar: geçiş bitti ya da iptal (`flush`).
    pub(super) fn end_transition(&self) {
        self.transition.store(0, SeqCst);
    }

    /// Halkayı `frames` kareye yetecek büyüklüğe getirir (motor iş parçacığı;
    /// çalarken de güvenli, ses ve çalar iş parçacığı beklemez). Yetmiyorsa
    /// ikinci yuva (en büyük boy, bir kez) burada ayrılır; yazar sonraki
    /// pakette ona geçer (`Writer::grow`): eski yuvadaki kuyruk yerinde kalır,
    /// yeni kareler ikincisine yazılır, ses iş parçacığı ikisini de sırayla
    /// okur. Geçiş kapalıyken ya da halka yetiyorsa hiçbir şey ayrılmaz. İkinci
    /// yuva bağlantı yenilenene (yeni `OutputCtl`) dek kalır; ayarlardan açılışta
    /// geçiş açıksa halka baştan doğru boyda kurulur (`new_for`), ikinci yuva
    /// ayrılmaz.
    pub fn reserve(&self, frames: u64) {
        if frames > self.write_cap() && self.big.get().is_none() {
            let _ = self.big.set(RingBuf::new(MAX_RING_FRAMES));
            self.wake_writer();
        }
    }

    /// `frame` numaralı karenin yuvası.
    fn ring_for(&self, frame: u64) -> &RingBuf {
        match self.big.get() {
            Some(big) if frame >= self.big_from.load(SeqCst) => big,
            _ => &self.ring,
        }
    }

    /// Yazılacak yeni kareler için halka kapasitesi (kare).
    fn write_cap(&self) -> u64 {
        match self.big.get() {
            Some(big) if self.big_from.load(SeqCst) != u64::MAX => big.frames(),
            _ => self.ring.frames(),
        }
    }

    // ── Aygıt ───────────────────────────────────────────────────────────

    /// Aygıt gitti (akış hatası ya da Windows bildirimi): yazar beklemeden
    /// hata döner, çalar duraklar.
    pub(super) fn signal_device_lost(&self, why: &str) {
        if !self.device_lost.swap(true, SeqCst) {
            log::warn!("Ses aygıtı ayrıldı ({why}); çalma duraklatılıyor");
            self.trace("device_lost");
        }
        self.wake_writer();
    }

    pub(super) fn is_device_lost(&self) -> bool {
        self.device_lost.load(SeqCst)
    }

    /// Kullanıcıya söylenecek aygıt sorunu (bir kez döner).
    pub fn take_device_notice(&self) -> Option<DeviceNotice> {
        match self.device_notice.swap(DEVICE_OK, SeqCst) {
            DEVICE_LOST => Some(DeviceNotice::Lost),
            DEVICE_NONE => Some(DeviceNotice::NoDevice),
            _ => None,
        }
    }

    /// Varsayılan aygıt değişeli `REROUTE_GRACE` geçti, açık akış eskisinde.
    fn reroute_due(&self) -> bool {
        watch::default_gen() != self.route_gen.load(SeqCst)
            && watch::since_default_change() >= REROUTE_GRACE
    }

    // ── iç ────────────────────────────────────────────────────────────

    /// Bir `flush` istendi ama yazar henüz onaylamadı.
    fn flush_pending(&self) -> bool {
        self.flush_req.load(SeqCst) != self.flush_ack.load(SeqCst)
    }

    /// `written - max(read, discard_to)`: çalınacak kare.
    fn queued_raw(&self) -> u64 {
        let w = self.written.load(SeqCst);
        let r = self.read.load(SeqCst).max(self.discard_to.load(SeqCst));
        w.saturating_sub(r)
    }

    /// Kuyruk hedefi: xrun'la büyüyen taban, şarkı sonu payı açıksa en az
    /// `tail_frames`.
    fn target_frames(&self) -> u64 {
        let base = self.target.load(Relaxed);
        if self.tail.load(Relaxed) {
            base.max(self.tail_frames())
        } else {
            base
        }
    }

    /// Şarkı sonu payının derinliği (kare): `TAIL_MS` + sıradaki geçişin
    /// bindirmesi (giden şarkının son `overlap` karesi, yeni şarkının ilk
    /// paketi gelene dek halkada durmalı), halkaya sığacak kadar.
    fn tail_frames(&self) -> u64 {
        let overlap = TransitionSpec::unpack(self.armed.load(Relaxed)).overlap as u64;
        (ms_to_frames(TAIL_MS) + overlap).min(self.write_cap().saturating_sub(RING_SLACK))
    }

    /// `frame` numaralı karenin ses kartına verileceği an (ns, `levels`
    /// saati). Atılmayı bekleyen eski ses sayılmaz. Ses iş parçacığı henüz
    /// çalışmıyorsa (ya da durduysa) "şimdi + öndeki kuyruk"a düşer.
    pub(super) fn pull_ns(&self, frame: u64, now_ns: u64) -> u64 {
        let r = self.read.load(SeqCst);
        let d = self.discard_to.load(SeqCst);
        // Onaylanmış ama henüz atılmamış eski kareler önde sayılmaz.
        let frame = frame.saturating_sub(d.saturating_sub(r));
        let ahead = frame.saturating_sub(r) as f64;
        let by_now = now_ns as f64 + frames_to_ns(ahead);
        let anchor = self.anchor.load(SeqCst);
        let by_clock = if anchor == i64::MIN {
            f64::MIN
        } else {
            anchor as f64 + frames_to_ns(frame as f64)
        };
        by_clock.max(by_now).max(0.0) as u64
    }

    /// Şu an çalan sesin en son duyulacağı an: ses iş parçacığı bir zarf
    /// boyu daha çalar, aygıt `D` sonra hoparlöre verir. Seviye çubukları
    /// bundan sonrasını göstermesin.
    fn cut_levels(&self) {
        let now = self.now_ns();
        let r = self.read.load(SeqCst);
        let until = self.pull_ns(r + FADE_FRAMES as u64, now) + self.device_ns;
        self.levels.cut(until);
    }

    fn wake_writer(&self) {
        if let Some(t) = self.writer.get() {
            t.unpark();
        }
    }

    /// İz dosyasına (varsa) bir komut satırı: motorun aldığı kullanıcı
    /// komutu (`pause`, `play`, `seek`, `load`), ölçümde çıkışın tepkisiyle
    /// karşılaştırmak için.
    pub fn note(&self, name: &'static str) {
        self.trace(name);
    }

    fn trace(&self, name: &'static str) {
        self.trace_arg(name, f64::NAN);
    }

    /// Ek değerli iz satırı (`arg_ms` sütunu).
    fn trace_arg(&self, name: &'static str, arg_ms: f64) {
        if let Some(tx) = &self.trace {
            let _ = tx.send((self.now_ns(), name, arg_ms));
        }
    }
}

// ---------------------------------------------------------------------------
// İz (OPENIFY_SINK_TRACE)
// ---------------------------------------------------------------------------

/// İz yazıcısı: her 100 ms'de bir `tick` satırı, her komutta bir satır;
/// ses iş parçacığının kaydettiği anlar (`silent`: iniş bitti, ses sustu;
/// `audible`: bekletmeden sonra ilk gerçek kare; `gain`: yeni ses düzeyi
/// alındı; `xrun`: kesinti bitti) kendi zamanlarıyla, en geç bir tik sonra
/// yazılır (satırlar zamana göre sıralı olmayabilir).
///
/// `t_unix_ms,event,queued_ms,xruns,gain,target_ms,underrun_ms,arg_ms`
///
/// `target_ms` xrun'la büyüyen taban hedeftir; şarkı sonu payı ayrı satırlarla
/// (`tail_on` / `tail_off`). `arg_ms` yalnız ek değerli satırlarda dolu
/// (`producer_gap`: çözücünün iki paket arası süresi; `reroute`: geçiş süresi).
fn trace_loop(
    ctl: Weak<OutputCtl>,
    rx: mpsc::Receiver<(u64, &'static str, f64)>,
    mut w: BufWriter<File>,
) {
    let _ = writeln!(
        w,
        "t_unix_ms,event,queued_ms,xruns,gain,target_ms,underrun_ms,arg_ms"
    );
    let mut seen = [0u64; 4];
    let mut next_tick = Instant::now() + TRACE_TICK;
    let mut ticks = 0u32;
    loop {
        let msg = rx.recv_timeout(next_tick.saturating_duration_since(Instant::now()));
        let Some(ctl) = ctl.upgrade() else { break };
        let row = |w: &mut BufWriter<File>, t: u64, name: &str, arg: f64| {
            let ms = |frames: u64| frames as f64 * 1000.0 / RATE as f64;
            let arg = if arg.is_finite() {
                format!("{arg:.1}")
            } else {
                String::new()
            };
            let _ = writeln!(
                w,
                "{:.2},{name},{:.1},{},{:.4},{:.0},{:.1},{arg}",
                ctl.levels.unix_ms(t),
                ms(ctl.queued_frames()),
                ctl.xruns(),
                f32::from_bits(ctl.gain.load(Relaxed)),
                ms(ctl.target.load(Relaxed)),
                ms(ctl.underrun_frames.load(Relaxed)),
            );
        };
        match msg {
            Ok((t, name, arg)) => row(&mut w, t, name, arg),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let marks = [
                    (&ctl.t_silent, "silent"),
                    (&ctl.t_audible, "audible"),
                    (&ctl.t_gain, "gain"),
                    (&ctl.t_xrun, "xrun"),
                ];
                for (i, (atom, name)) in marks.into_iter().enumerate() {
                    let t = atom.load(Relaxed);
                    if t != 0 && t != seen[i] {
                        seen[i] = t;
                        row(&mut w, t, name, f64::NAN);
                    }
                }
                row(&mut w, ctl.now_ns(), "tick", f64::NAN);
                next_tick += TRACE_TICK;
                ticks += 1;
                if ticks.is_multiple_of(10) {
                    let _ = w.flush();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = w.flush();
}

#[cfg(test)]
mod beat_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod xfade_tests;
#[cfg(test)]
pub(in crate::engine) use source::RingSource;
