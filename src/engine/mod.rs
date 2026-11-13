//! Spotify motoru: ses çalma ve kütüphane.
//!
//! Openify'ın geri kalanı librespot'u doğrudan görmez; sadece buradaki
//! `Command` ve `Event` türlerini bilir. Motor kendi iş parçacığında, kendi
//! tokio çalışma ortamıyla çalışır:
//!
//!   arayüz ──Command──▶ motor (librespot) ──Event──▶ arayüz
//!
//! Spotify oturumu motorda yaşar. Ses, kütüphane ve çalma sırası burada
//! yönetilir; arayüz sadece gösterir ve komut verir.
//!
//! Açılış iki aşamalı: önce disk önbelleğindeki kütüphane hemen gösterilir
//! (Spotify'a bağlanmadan), sonra bağlanılıp arka planda tazelenir.

mod automix;
mod cache;
mod connect;
mod conns;
mod credits;
mod edits;
pub mod equalizer;
mod explore;
mod friends;
mod hidden;
mod levels;
mod library;
mod listening;
mod login;
mod lyrics;
pub mod opra;
mod pages;
mod related;
pub use login::{LoginFailReason, PlayBlock};
mod migrate;
mod output;
pub mod palette;
mod pathfinder;
mod playback;
mod queue;
mod radio;
mod recovery;
mod resume;
mod rules;
mod search;
mod seed;
mod seed_readme;
mod session;
mod tap;
pub mod tools;
mod traits;
mod wire;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    thread,
};

use bytes::Bytes;
use librespot::{
    core::{SpotifyUri, cache::Cache, session::Session},
    playback::{
        config::{Bitrate, PlayerConfig},
        mixer::NoOpVolume,
        player::{Player, PlayerEvent},
    },
};
use tokio::sync::mpsc;

pub use cache::cover_path;
pub use connect::{
    ConnectCommand, ConnectDriver, ConnectStatus, RemoteNow, apply_connect_identity,
};
use connect::{ConnectInternal, ConnectPort};
pub use levels::AudioLevels;
pub use library::{
    CollectionId, Library, LibraryItem, LibraryMeta, TrackRow, artist_item, folder_contains,
    folder_id, mosaic_urls,
};
pub use output::{MAX_OVERLAP_MS, OutputCtl};
pub use queue::{QueueEntry, QueueSource, QueueView, RepeatMode};
// Dinleme çekirdeği (bkz. `radio.rs`, `hidden.rs`, `listening.rs`).
pub use hidden::{HiddenList, HiddenSong};
pub use radio::{
    RadioInfo, parse as parse_radio, seed_ok as radio_seed_ok, test_fixture as radio_test_fixture,
};
pub use rules::{BLOCKED_MAX, Rules, Traits, fold as fold_name};
#[cfg(test)]
pub use search::from_json as search_from_json;
pub use search::{
    SearchHit, SearchResults, TestSearch, test_palette_search, test_results as test_search_results,
};
// Sayfalar ve kütüphane yazmaları (Adım 12–13, bkz. `wire.rs`).
pub use edits::{
    AddPosition, DESCRIPTION_MAX, EditOp, FailKind, folder_start_marker, new_folder_id,
    new_temp_id, same_edit,
};
pub use pages::{AlbumCard, AlbumPage, ArtistCard, ArtistPage, ReleaseKind, TrackLinks, home};
pub use wire::{
    FailedEdit, LibraryCommand, LibraryEvent, cached_album, cached_artist, edit_failed_notice,
    patch_library, patch_tracks, touched_collection,
};
// Benzer sanatçılar (Adım 12, 7 Kasım; bkz. `related.rs`).
pub use related::RelatedList;
pub use wire::suggested_artists;
// Arkadaş etkinliği (bkz. `friends.rs`).
pub use friends::{Friend, FriendsFailure, PlayPlan, parse as parse_friends, play_plan};
// Keşfet ve Ana sayfa (Adım 12, 7 Kasım; bkz. `explore.rs`).
pub use explore::{CardKind, CategoryList, ExploreCard, ExploreData, ExploreKey, Feed};
pub use lyrics::{
    LyricLine, Lyrics, Source as LyricsSource, SyncKind, parse_spotify as parse_spotify_lyrics,
};
// Şarkı künyesi (bkz. `credits.rs`).
pub use credits::{Credits, CreditsFailure, RoleKind as CreditRoleKind, parse as parse_credits};

use crate::auth;
use cache::Db;
use playback::{Deck, Gate, LibrespotPort, Playback, PlayerSignal, TransportCmd};
use queue::{Queue, UserItem};
use session::{Install, SessionHost, first_connect};

/// librespot'un ses önbelleği: çalınan şarkıların şifreli ses dosyaları.
pub fn audio_cache() -> Option<Cache> {
    let dir = cache::audio_dir()?;
    Cache::new(None, None, Some(dir), Some(cache::AUDIO_CACHE_LIMIT))
        .map_err(|e| log::warn!("Ses önbelleği açılamadı: {e}"))
        .ok()
}

/// Varsayılan ses aygıtı (hoparlör, kulaklık) var mı?
pub fn has_output_device() -> bool {
    output::has_device()
}

// Ayarlar sayfası (Adım 15): çıkış aygıtı ve önbellek.
use crate::t;
pub use cache::{CLEARABLE, clear_subdirs, clearing_allowed, dir_size, settings_cache_dir};
pub use output::{
    default_output_device_name, output_device_names, set_preferred_device as set_output_device,
};

/// Ayarlardan: ses kalitesi (96 / 160 / 320 kbps) ve ses düzeyi eşitleme.
/// Çalışırken de çağrılabilir; etkisi sıradaki şarkıdan başlar (bkz.
/// `vendor/librespot-playback/OPENIFY.md`).
pub fn apply_audio(kbps: u16, normalise: bool) {
    use librespot::playback::config::live;
    live::set_bitrate(Some(match kbps {
        0..=96 => Bitrate::Bitrate96,
        97..=160 => Bitrate::Bitrate160,
        _ => Bitrate::Bitrate320,
    }));
    live::set_normalisation(Some(normalise));
}

/// Eşitleme düzeyi (Sessiz / Normal / Yüksek): çaların ön kazancı, dB olarak,
/// Normal'e (−14 LUFS) göre fark (bkz. `Settings::normalise_pregain_db`). Etkisi
/// sıradaki şarkıdan başlar, `apply_audio` gibi.
pub fn apply_normalisation_level(pregain_db: f64) {
    librespot::playback::config::live::set_normalisation_pregain_db(Some(pregain_db));
}

/// "Kaldığı yerden sür" ayarı: kapalıysa açılışta kayıtlı sıra geri
/// yüklenmez (kayıt yazılmaya devam eder; ayar açılınca kullanılabilir).
static RESUME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_resume(on: bool) {
    RESUME.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Şarkı dosyasının ağdan indirme ayarı (ilk şarkı açılmadan, bir kez).
///
/// librespot bir dosyayı açarken ilk istekte en az `minimum_download_size`
/// indirir; önyüklemede (şarkı sonundan 30 sn önce) yalnız bu baş iner,
/// indirme döngüsü sonra kimse okumadıkça uyur. Varsayılan 64 KiB, 320
/// kbps'de ~1,6 sn ses: sıradaki şarkı başlayınca ağ isteği bu sürede
/// dönmezse kuyruk boşalırdı (7 Kasım tanısı). 512 KiB ~13 sn: geçişte
/// ağın yavaş / bağlantının yeniden kurulması duyulmaz. Bedeli: atlanan
/// şarkı başına ~450 KiB fazla indirme ve rastgele erişimde (sarma)
/// istek başına en az 512 KiB; librespot indirmeleri tek tek yaptığından
/// yavaş ağda şarkının ilk saniyelerindeki sarma bu baş inene dek bekler.
/// Bekleme süresi (`download_timeout`) varsayılandaki gibi 8 sn kalır.
fn configure_fetch() {
    use librespot::audio::AudioFetchParams;
    const MIN_DOWNLOAD: usize = 512 * 1024;
    let params = AudioFetchParams {
        minimum_download_size: MIN_DOWNLOAD,
        ..AudioFetchParams::default()
    };
    // Yeniden bağlanınca ikinci kez çağrılır: ilk değer kalır.
    let _ = AudioFetchParams::set(params);
}

/// Çaları Openify'ın kendi ses çıkışıyla kurar (bkz. `output.rs`; aygıt
/// çalar iş parçacığında açılır). Çalara `NoOpVolume` verilir: ses düzeyi
/// çıkışta uygulanır (`OutputCtl::set_gain`, başlangıçta `gain`), seviye
/// çubukları ses düzeyinden önceki sese bakar (bkz. `tap.rs`).
pub fn new_player(
    config: PlayerConfig,
    session: Session,
    levels: AudioLevels,
    gain: f32,
    crossfade_ms: u32,
) -> (Arc<Player>, Arc<OutputCtl>) {
    configure_fetch();
    // Halka geçiş süresine göre kurulur (kapalıyken bugünkü 2 MiB).
    let out = OutputCtl::new_for(levels, gain, crossfade_ms);
    let sink_out = out.clone();
    let player = Player::new(config, session, Box::new(NoOpVolume), move || {
        Box::new(tap::LevelSink::open(sink_out))
    });
    // İşaret / kapı için çıkış çaların olaylarını dinler; ilk komuttan önce.
    out.attach_events(player.get_player_event_channel());
    (player, out)
}

/// Arayüzün motora gönderdiği komutlar.
#[derive(Debug, Clone)]
pub enum Command {
    /// Tek bir şarkıyı çal (sıra yok).
    Load(SpotifyUri),
    /// Bir listenin şarkılarını getir (`CollectionLoaded` ile gelir).
    OpenCollection(CollectionId),
    /// Yüklenmiş bir listeyi `index`'teki şarkıdan çalmaya başla; şarkı
    /// bitince listede sıradakine geçilir.
    PlayCollection {
        id: CollectionId,
        index: usize,
    },
    /// Bir sanatçının (ya da başka bir Spotify bağlamının) şarkılarını çal.
    PlayContext(String),
    /// Arkadaş etkinliğinden: bağlamı (liste, albüm, sanatçı) `track` şarkısından
    /// başlayarak çal. `hint`: şarkının bağlamdaki sırası (aynı şarkı birden
    /// çok yerdeyse en yakını seçilir). Bağlam alınamazsa ya da şarkı içinde
    /// yoksa yalnız şarkı çalınır (bkz. `friends::start_position`).
    PlayContextAt {
        uri: String,
        track: String,
        hint: Option<usize>,
    },
    /// Spotify'ın tamamında ara; sonuç `SearchResults` ile, aynı `seq` ile gelir.
    Search {
        seq: u64,
        query: String,
    },
    /// Kütüphaneyi hemen Spotify'dan tazele.
    RefreshLibrary,
    /// Bu resimleri indir; hazır olanlar `ImageReady` ile bildirilir.
    FetchImages(Vec<String>),
    /// Bu şarkının sözünü getir (önbellek → Spotify → LRCLIB); `Event::Lyrics`
    /// ile gelir, yoksa `lyrics: None` (bkz. `lyrics.rs`).
    FetchLyrics(String),
    /// Sıradaki şarkının sözünü önceden önbelleğe al (olay yok).
    PrefetchLyrics(String),
    Next,
    Previous,
    Play,
    Pause,
    /// Milisaniye cinsinden konuma git.
    Seek(u32),
    /// 0.0 (sessiz) ile 1.0 (tam) arası.
    SetVolume(f32),
    /// Şarkılar arası geçiş (crossfade): milisaniye, 0 kapalı, en çok 12 000.
    /// Yalnız kendiliğinden geçişte, farklı albümler arasında uygulanır
    /// (bkz. `playback/crossfade.rs`). Bağlantı gerekmez: motor değeri tutar,
    /// çıkış kurulurken halka ona göre boyutlanır.
    SetCrossfade(u32),
    /// AutoMix (akıllı geçiş): açıkken geçişin yeri ve uzunluğu şarkıların
    /// çözümlemesinden seçilir; kapalıyken `SetCrossfade` süresi geçerli (bkz.
    /// `playback/crossfade.rs`, `output/automix.rs`).
    SetAutoMix(bool),
    /// Ekolayzer (bkz. `equalizer.rs`, `output/eq.rs`): açık mı, 10 bandın
    /// kazancı (31,5 Hz … 16 kHz, dB) ve elle ön kazanç (dB). Bağlantı
    /// gerekmez: motor ayarı tutar, çıkış kurulunca (ve her yeniden
    /// kurulunca) uygular; duyulan değişim 8 blokta süzülür.
    SetEq {
        enabled: bool,
        gains_db: [f32; 10],
        preamp_db: f32,
    },
    /// Karaoke / vokal azaltma (bkz. `output/karaoke.rs`): açık mı ve orta
    /// kanalın konuşma bandı ne kadar bastırılsın (0..1). Bağlantı gerekmez:
    /// motor ayarı tutar, çıkış kurulunca uygular; açma / kapama / miktar
    /// değişimi birkaç blokta süzülür (tıkırtı yok). Mono kaynakta etkisizdir.
    SetKaraoke {
        enabled: bool,
        amount: f32,
    },

    // ── Sıra, karıştırma, tekrar (bkz. `queue.rs`). Hepsi bağlanmadan da
    // çalışır (yalnız sırayı düzenler); sıra değişince `QueueChanged` ve
    // `PlaybackEvent::Queue` gelir. "Görünür liste": önce kullanıcı sırası,
    // sonra bağlamın kalanı, `QueueView::upcoming` ile aynı sıra.
    /// Şarkıları kullanıcı sırasının sonuna ekle ("sıraya ekle"); bağlamdan
    /// önce çalarlar.
    Enqueue(Vec<TrackRow>),
    /// "Sonra çal": verilen sırayla kullanıcı sırasının başına.
    PlayNext(Vec<TrackRow>),
    /// Görünür listeden `i`. satırı kaldır.
    RemoveFromQueue(usize),
    /// Görünür listede `from`. satırı `to`. yere taşı (sürükle bırak).
    MoveInQueue(usize, usize),
    /// Kullanıcı sırasını boşalt (bağlam kalır).
    ClearQueue,
    /// Görünür listedeki `i`. şarkıya atla ve çal.
    PlayQueueItem(usize),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    /// Karıştırma ve sıra kuralları (ayarlar sayfası ve sağ tık menüsü):
    /// kara listedeki sanatçılar, açık sözlüler, aynı şarkının sürümleri.
    /// Çalma sırasının kalanı hemen yeniden kurulur; kullanıcının kendi
    /// seçtiği şarkılara dokunulmaz (bkz. `queue.rs`).
    SetQueueRules(Rules),
    /// Şarkı bilgileri elde olan bir listeyi (albüm / sanatçı sayfası, web
    /// verisi) `index`'ten çalmaya başla. `context`: kütüphane listesiyse
    /// kimliği (çalan satırın vurgusu için), değilse `None`.
    PlayTracks {
        context: Option<CollectionId>,
        tracks: Arc<Vec<TrackRow>>,
        index: usize,
    },
    /// Kaldığı yer kaydını diske yaz ve bitince haber ver (kapanıştan önce;
    /// bkz. `EngineHandle::flush_state`).
    Flush(std::sync::mpsc::Sender<()>),
    /// Radyo: şarkı / sanatçı / albüm / liste tohumundan sonsuz benzer şarkı
    /// akışı başlat (bkz. `radio.rs`). `title`: sıra başlığında görünecek ad
    /// (tohumun adı; boşsa Spotify'ın istasyon başlığı). Salt okuma; başarısız
    /// olursa bir `Problem` bildirimi gelir.
    StartRadio {
        seed: String,
        title: String,
    },
    /// Sayfalar ve kütüphane yazmaları (bkz. `wire.rs`).
    Library(LibraryCommand),
    /// Kullanıcı "Giriş yap"a bastı: tarayıcıda Spotify girişini aç. Yalnız
    /// giriş gerekiyorsa (`Event::NeedsLogin` / `LoginFailed`) işe yarar;
    /// tarayıcı başka hiçbir yolla açılmaz (bkz. `login.rs`).
    StartLogin,
    /// Tarayıcıda girişi bekleyen süreci bırak (`Event::NeedsLogin`'e döner).
    CancelLogin,
    /// Spotify Connect (varsayılan AÇIK, bkz. `connect.rs`): aç / kapat,
    /// aygıt adı, "buraya aktar". Bağlantı gerekmez; durum
    /// `PlaybackEvent::Connect` ile gelir.
    Connect(ConnectCommand),
}

/// Sıra ve kaldığı yerden sürme olayları. Ayrı kanalda
/// (`EngineHandle::playback`): `Event`e katılması arayüzde kapsamlı
/// eşleşmeleri kırardı; arayüz bağlanırken tek kanala indirmek istenirse
/// `Event::Playback(PlaybackEvent)` eklemek yeter.
#[derive(Debug, Clone)]
pub enum PlaybackEvent {
    /// Çalan ve sıradakiler (her sıra değişikliğinde; bilgisi sonradan
    /// bulunan şarkılar için yeniden).
    Queue(QueueView),
    /// Açılışta bulunan kayıtlı durum: şarkı çalmıyor, duraklatılmış gibi
    /// gösterilmeli; Çal'a basılınca kaldığı konumdan yüklenir.
    Restored(Box<Restored>),
    /// Spotify Connect durumu (çalmayı kim yürütüyor; başka aygıtta çalıyorsa
    /// hangisi). Yalnız değişince gelir; Connect kapalıyken hiç gelmez.
    Connect(ConnectStatus),
}

/// Açılışta geri yüklenen çalma durumu.
#[derive(Debug, Clone)]
pub struct Restored {
    /// Kaldığı şarkı (`Event::TrackChanged` yerine geçer).
    pub track: TrackInfo,
    /// Kaldığı konum (`Event::Paused { position_ms }` yerine geçer).
    pub position_ms: u32,
    /// `Event::QueueChanged` yerine geçer.
    pub queue: QueueState,
    pub view: QueueView,
    /// Kapak baytları ve vurgu rengi (diskte varsa; `Event::Cover` yerine geçer).
    pub cover: Option<(Vec<u8>, Option<palette::Accent>)>,
}

/// Motorun arayüze bildirdiği olaylar.
#[derive(Debug, Clone)]
pub enum Event {
    Connecting,
    Connected {
        username: String,
    },
    /// Spotify'a ulaşılamıyor (ağ yok): çevrimdışı; motor geri çekilmeyle
    /// kendiliğinden yeniden dener.
    ConnectFailed(String),
    /// Kayıtlı giriş yok ya da Spotify reddetti: kullanıcı "Giriş yap"a
    /// basmalı (`Command::StartLogin`). Tarayıcı kendiliğinden açılmaz.
    NeedsLogin,
    /// Tarayıcıda giriş bekleniyor (`Command::CancelLogin` ile bırakılır;
    /// 3 dakikada tamamlanmazsa `LoginFailed`).
    LoggingIn,
    /// Giriş tamamlanamadı (neden KODU; yazısı arayüzde). Yeniden
    /// `StartLogin` denenebilir.
    LoginFailed(LoginFailReason),
    /// Hesap ücretsiz: librespot yalnız Premium'la ses akıtır. Çalma
    /// komutları ileti verip düşer.
    NeedsPremium,
    /// Motor iş parçacığı beklenmedik biçimde öldü (ya da komut kanalı kapandı).
    /// Arayüz "Yeniden başlat" gösterir (`EngineHandle::restart`). Metin: neden
    /// (kayıt için; kullanıcıya gösterilmez).
    EngineStopped(String),
    /// Çalma komutu düştü çünkü giriş / Premium gerekiyor (neden KODU;
    /// yazısı arayüzde, `msg::play_block_text`).
    PlaybackBlocked(PlayBlock),
    LibraryLoaded(Library),
    LibraryFailed(String),
    CollectionLoaded {
        id: CollectionId,
        tracks: Arc<Vec<TrackRow>>,
    },
    CollectionFailed {
        id: CollectionId,
        error: String,
    },
    /// Yerel arama için kütüphanedeki bütün şarkılar.
    SearchIndex(Arc<Vec<TrackRow>>),
    SearchResults {
        seq: u64,
        results: Arc<SearchResults>,
    },
    SearchFailed {
        seq: u64,
        error: String,
    },
    /// Çalma sırası değişti: hangi listenin kaçıncı şarkısı çalıyor.
    QueueChanged(QueueState),
    /// Yeni şarkının bilgileri geldi (kapak ayrıca `Cover` ile gelir).
    TrackChanged(TrackInfo),
    /// Kapak resminin baytları (JPEG) ve kapaktan türetilen vurgu rengi.
    Cover {
        track_uri: String,
        bytes: Vec<u8>,
        accent: Option<palette::Accent>,
    },
    /// İstenen resim diske indi (`cache::cover_path` ile bulunur).
    ImageReady(String),
    /// `FetchLyrics` cevabı; hiçbir kaynakta söz yoksa `None`.
    Lyrics {
        uri: String,
        lyrics: Option<Arc<lyrics::Lyrics>>,
    },
    Loading,
    Playing {
        position_ms: u32,
    },
    Paused {
        position_ms: u32,
    },
    /// Çalma sürerken konum değişti (sarma ya da düzeltme).
    Position {
        position_ms: u32,
    },
    /// Sıradaki şarkı kalmadı.
    Ended,
    /// Spotify şarkıyı vermedi ve atlanacak başka şarkı yok.
    Unavailable,
    /// Spotify bağlantısı koptu; kayıtlı girişle yeniden bağlanılıyor.
    Reconnecting,
    /// Kullanıcıya gösterilecek kısa bilgi.
    Notice {
        kind: NoticeKind,
        message: String,
    },
    /// Spotify'a bağlanıldı ama ses çıkışı (hoparlör, kulaklık) bulunamadı.
    NoAudioOutput,
    /// Sayfalar, şarkı bağları ve yazma kuyruğunun durumu (bkz. `wire.rs`).
    Library(LibraryEvent),
}

/// Bildirimin türü: arayüz rengini ve bildirimin ne zaman kalkacağını
/// buna göre seçer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Sürüyor ("yeniden deneniyor…"): sorun çözülünce kendiliğinden kalkar.
    Working,
    /// Olup bitti, bilgin olsun: birkaç saniye görünür.
    Info,
    /// Kullanıcının bilmesi gereken sorun: bir sonraki işleme kadar kalır.
    Problem,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueState {
    pub context: Option<CollectionId>,
    pub index: usize,
    pub has_next: bool,
    pub has_previous: bool,
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub duration_ms: u32,
    pub cover_url: Option<String>,
}

/// Çalışan motora bağlantı. Düşürülünce motor da kapanır.
///
/// Olay kanalları motor iş parçacığından bağımsız tutulur (tutamak gönderici
/// uçları da saklar): motor ölürse kanal kapanmaz, `Event::EngineStopped`
/// gelir ve `restart` aynı kanallarla yeni bir motor başlatır (arayüzün
/// dinleyicileri değişmez).
pub struct EngineHandle {
    commands: std::sync::Mutex<mpsc::UnboundedSender<Command>>,
    pub events: async_channel::Receiver<Event>,
    /// Sıra görünümü ve açılışta geri yüklenen durum (bkz. `PlaybackEvent`).
    pub playback: async_channel::Receiver<PlaybackEvent>,
    levels: AudioLevels,
    event_tx: async_channel::Sender<Event>,
    playback_tx: async_channel::Sender<PlaybackEvent>,
    /// Motor iş parçacığı öldü (çöktü ya da komut kanalı kapandı) ve
    /// arayüze haber verildi.
    stopped: Arc<std::sync::atomic::AtomicBool>,
}

impl EngineHandle {
    pub fn send(&self, command: Command) {
        let sent = self
            .commands
            .lock()
            .map(|tx| tx.send(command).is_ok())
            .unwrap_or(false);
        // Komut kanalı kapalı = motor iş parçacığı yok. Çökme yolu zaten
        // haber verdiyse (`stopped`) bir daha söyleme; vermediyse (iş
        // parçacığı başka biçimde bitti) arayüz sessiz zombi kalmasın.
        if !sent && !self.stopped.swap(true, std::sync::atomic::Ordering::SeqCst) {
            log::error!("Motor komut kanalı kapalı: motor durmuş");
            let _ = self
                .event_tx
                .try_send(Event::EngineStopped("command channel closed".into()));
        }
    }

    /// Motor durduysa (bkz. `Event::EngineStopped`) yeni bir motor başlatır:
    /// aynı olay kanalları, aynı ses seviyesi tutamağı. Çalışan motorda
    /// bir şey yapmaz. Çağıran, durumunu (ses düzeyi gibi) yeniden göndermeli.
    pub fn restart(&self) {
        if !self.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            log::warn!("Motor çalışıyor, yeniden başlatma yok sayıldı");
            return;
        }
        log::warn!("Motor yeniden başlatılıyor");
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        if let Ok(mut tx) = self.commands.lock() {
            *tx = command_tx;
        }
        self.stopped
            .store(false, std::sync::atomic::Ordering::SeqCst);
        spawn_engine(
            command_rx,
            self.event_tx.clone(),
            self.playback_tx.clone(),
            self.levels.clone(),
            self.stopped.clone(),
        );
    }

    /// Kaldığı yer kaydının diske yazılmasını bekler (en çok 1,5 sn):
    /// uygulama kapanırken, pencere kapanmadan önce çağrılır. Motor
    /// durumu zaten pause / şarkı değişimi / her sn yazar; bu, son
    /// saniyelerin de kaybolmamasını sağlar.
    pub fn flush_state(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sent = self
            .commands
            .lock()
            .map(|c| c.send(Command::Flush(tx)).is_ok())
            .unwrap_or(false);
        if sent {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(1500));
        }
    }

    /// Duyulan sesin bas / orta / tiz seviyesi (ekolayzer çubukları için).
    /// Ucuz kopyalanır, her iş parçacığından okunur, kilitsizdir; motor
    /// yeniden bağlansa da aynı tutamak geçerli kalır.
    ///
    /// ```ignore
    /// let levels = engine.levels();            // bir kez al, görünümde sakla
    /// // çizim sırasında, ekrana gelecek an için bir kare ileriye bakarak:
    /// match levels.read(Instant::now() + Duration::from_millis(16)) {
    ///     Some([bass, mid, treble]) => { /* her biri 0..1: çubuk yükseklikleri */ }
    ///     None => { /* ses yok: dinlenme yüksekliği, kare üretmeyi bırak */ }
    /// }
    /// ```
    /// Seviyeler ses düzeyinden bağımsızdır (sessize alınca çubuklar
    /// düzleşmez) ve gecikmeye göre ayarlıdır: `read(now)` o an
    /// hoparlörden DUYULAN sesin seviyesini verir (bkz. `levels.rs`).
    pub fn levels(&self) -> AudioLevels {
        self.levels.clone()
    }
}

/// Motoru arka planda başlatır. Spotify'a bağlanma da arka planda yapılır;
/// kayıtlı giriş yoksa ya da reddedildiyse `Event::NeedsLogin` gelir (tarayıcı
/// kendiliğinden açılmaz; `Command::StartLogin` bekler). Bağlanınca kütüphane
/// kendiliğinden yüklenir.
pub fn start() -> EngineHandle {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = async_channel::unbounded();
    let (playback_tx, playback_rx) = async_channel::unbounded();
    let levels = AudioLevels::new();
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    spawn_engine(
        command_rx,
        event_tx.clone(),
        playback_tx.clone(),
        levels.clone(),
        stopped.clone(),
    );

    EngineHandle {
        commands: std::sync::Mutex::new(command_tx),
        events: event_rx,
        playback: playback_rx,
        levels,
        event_tx,
        playback_tx,
        stopped,
    }
}

/// Motor iş parçacığını kurar. Çökerse (panik) iş parçacığı sessizce ölmez:
/// panik kancası günlüğe yazar (`crash.rs`), burada da yakalanıp arayüze
/// `Event::EngineStopped` gider.
fn spawn_engine(
    command_rx: mpsc::UnboundedReceiver<Command>,
    event_tx: async_channel::Sender<Event>,
    playback_tx: async_channel::Sender<PlaybackEvent>,
    levels: AudioLevels,
    stopped: Arc<std::sync::atomic::AtomicBool>,
) {
    let notify = event_tx.clone();
    thread::Builder::new()
        .name("openify-engine".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // İki ayrı tokio ortamı:
                //   - ana ortam: Spotify oturumunun bağlantısı (şarkı anahtarı
                //     cevapları dahil) ve motorun komut döngüsü;
                //   - arka plan ortamı: kütüphane tazeleme, şarkı bilgileri,
                //     SQLite yazımı, kapaklar, arama.
                // librespot şarkı anahtarının cevabını yalnızca 1,5 sn bekliyor.
                // Arka plan işi aynı ortamdaki işçileri meşgul edince cevap
                // zamanında okunamıyor, şarkı çalınamıyordu (açılıştaki tazeleme
                // sırasında çalınca). Ağır iş artık bağlantıyı bekletemez.
                let background = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(3)
                    .thread_name("openify-bg")
                    .enable_all()
                    .build()
                    .expect("arka plan için tokio başlatılamadı");
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("openify-net")
                    .enable_all()
                    .build()
                    .expect("motor için tokio başlatılamadı");
                runtime.block_on(run(
                    command_rx,
                    event_tx,
                    playback_tx,
                    background.handle().clone(),
                    levels,
                ));
                drop(background);
            }));
            if let Err(payload) = outcome {
                let reason = crate::crash::panic_message(payload.as_ref());
                log::error!("Motor iş parçacığı çöktü: {reason}");
                stopped.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = notify.try_send(Event::EngineStopped(reason));
            }
        })
        .expect("motor iş parçacığı başlatılamadı");
}

/// Arka planda biten işlerin sonuçları motora bu yolla döner.
enum Internal {
    Library(anyhow::Result<library::Refresh>),
    Tracks(CollectionId, anyhow::Result<Arc<Vec<TrackRow>>>),
    /// Bir listenin içeriği Spotify'da değişmişti; önbellek güncellendi.
    Synced(CollectionId),
    /// Bilgisi eksik şarkılar tamamlandı: arama dizini yenilensin.
    Filled,
    Search {
        seq: u64,
        result: anyhow::Result<SearchResults>,
    },
    Context(anyhow::Result<Vec<SpotifyUri>>),
    /// `PlayContextAt` cevabı: bağlamın şarkıları (alınamadıysa hata).
    ContextAt {
        track: String,
        hint: Option<usize>,
        result: anyhow::Result<Vec<SpotifyUri>>,
    },
    Reconnected(Result<Session, auth::ConnectError>),
    /// Tarayıcı girişinin sonucu (bkz. `Engine::start_login`).
    LoginDone(Result<Session, auth::LoginError>),
    /// Sıradaki şarkıların önbellekten bulunan bilgileri.
    Meta(Vec<TrackRow>),
    /// Sayfa, bağ ve yazma işlerinin sonuçları (bkz. `wire.rs`).
    Wire(wire::WireInternal),
    /// Açık sözlüleri atlamak için eksik şarkı özellikleri tamamlandı.
    Traits(HashMap<String, Traits>),
    /// Radyo yanıtı (başlangıç ya da devam; bkz. `listening.rs`).
    Radio(listening::RadioBatch),
    /// Spotify Connect işleri (bkz. `connect.rs`).
    Connect(ConnectInternal),
}

struct Engine {
    /// Ağır işlerin çalıştığı ayrı tokio ortamı (bkz. `start`).
    bg: tokio::runtime::Handle,
    /// Spotify oturumu ve bağlantı / giriş / yeniden bağlanma durumu (bkz. `session.rs`).
    host: SessionHost,
    db: Option<Arc<Db>>,
    events: async_channel::Sender<Event>,
    /// Sıra görünümü ve geri yüklenen durum (ayrı kanal, bkz. `PlaybackEvent`).
    playback: async_channel::Sender<PlaybackEvent>,
    internal: mpsc::UnboundedSender<Internal>,
    /// Son bilinen kütüphane (albümleri yeniden istememek için).
    library: Option<Library>,
    /// Bellekteki liste içerikleri ve bunlardan eksiksiz olanlar.
    collections: HashMap<CollectionId, Arc<Vec<TrackRow>>>,
    complete: HashSet<CollectionId>,
    /// Arayüzün açtığı listeler: değişirlerse yeni hali gönderilir.
    opened: HashSet<CollectionId>,
    /// Çalma sırası modeli (bağlam + kullanıcı sırası + karıştırma + tekrar).
    queue: Queue,
    /// Sıradaki şarkıların bilgileri: sıraya eklenenlerden ve önbellekten.
    meta: HashMap<String, TrackRow>,
    /// Bilgisi önbellekten istenmiş şarkılar (aynısı tekrar istenmesin).
    meta_requested: HashSet<String>,
    /// Karıştırma ve sıra kuralları (ayarlardan; bkz. `rules.rs`). Çalma sırası
    /// yeniden kurulunca, kaldığı yer geri yüklenince de bunlar geçerli.
    rules: Rules,
    /// Açık söz / popülerlik bilgisi okunmuş ya da istenmiş şarkılar (aynısı
    /// tekrar sorulmasın) ve tamamlama isteği sürüyor mu.
    traits_known: HashSet<String>,
    traits_filling: bool,
    /// Yerel olarak gizlenen şarkılar (bkz. `hidden.rs`) ve radyonun devam
    /// zamanlaması (bkz. `radio.rs`).
    hidden: Arc<HiddenList>,
    radio: radio::Feed,
    /// Kaldığı yer kaydının yazıcısı; büyük kaydın son yazılan sürümü.
    saver: Option<resume::Saver>,
    saved_gen: Option<u64>,
    /// Bağlanmadan önce gelen çalma komutları; bağlanınca çalıştırılır.
    pending: Vec<Command>,
    /// Süren tazelemeyi kullanıcı mı istedi (sonucu söylenir)?
    user_refresh: bool,
    /// Çalma durumu: çalar + çıkış (deste), konum, geri yüklenen şarkı (bkz.
    /// `playback/`).
    pb: Playback,
    /// Ses seviyesi tutamağı: çalar her kurulduğunda çıkışa takılır.
    levels: AudioLevels,
    /// Ekolayzer ayarı (arayüzden; çıkış her kurulduğunda uygulanır).
    eq: equalizer::EqSetting,
    /// Karaoke ayarı (açık mı, miktar 0..1; çıkış her kurulduğunda uygulanır).
    karaoke: (bool, f32),
    /// Sayfalar ve yazma kuyruğu (bkz. `wire.rs`).
    wire: wire::Wire,
    /// Spotify Connect yaşam döngüsü (varsayılan açık; bkz. `connect.rs`).
    connect: connect::ConnectHost,
}

async fn run(
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: async_channel::Sender<Event>,
    playback: async_channel::Sender<PlaybackEvent>,
    bg: tokio::runtime::Handle,
    levels: AudioLevels,
) {
    let db = Db::open_default()
        .map_err(|e| log::warn!("Önbellek açılamadı, önbelleksiz devam: {e:#}"))
        .ok()
        .map(Arc::new);
    log::debug!("aşama {} ms: önbellek açıldı", crate::since_start());
    bg.spawn_blocking(cache::prune_covers);

    let (internal_tx, mut internal_rx) = mpsc::unbounded_channel();
    let mut engine = Engine::new(bg, db, events.clone(), playback, internal_tx, levels);
    engine.open_wire();
    log::debug!(
        "aşama {} ms: sayfa önbelleği ve yazma kuyruğu açıldı",
        crate::since_start()
    );
    // Yerel olarak gizlenen şarkılar: kaldığı yer geri yüklenmeden önce bilinmeli.
    engine.init_hidden();

    // 1. aşama: önbellekteki kütüphaneyi hemen göster, bu sırada bağlan.
    engine.show_cached_library();
    // Kaldığı yer: şarkı ve sıra hemen gösterilir, çalmaz; Spotify'a hiçbir
    // şey gitmez (anahtar isteği ancak kullanıcı Çal'a basınca).
    engine.restore_playback();
    // Açılışta seçili gelen liste (beğenilenler) hemen arkasından: arayüz
    // ilk karesinde gösterebilsin. Arama dizini ondan sonra, arka planda.
    engine.open_collection(CollectionId::Liked);
    log::debug!(
        "aşama {} ms: açılış listesi gönderildi",
        crate::since_start()
    );
    engine.send_search_index();
    engine.set_link(login::LinkState::Connecting);
    // Bağlanamazsa (internet yok) artan aralıklarla yeniden dener (bkz.
    // `recovery::connect_backoff_for`); önceden tek denemede bırakıyordu,
    // internet gelse de uygulama yeniden açılana kadar çevrimdışı kalıyordu.
    // Kayıtlı giriş yoksa ya da reddedildiyse DENEMEZ ve tarayıcıyı da açmaz:
    // `NeedsLogin` olur, kullanıcının "Giriş yap"ını (`Command::StartLogin`)
    // bekler.
    type Connecting =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<Session, auth::ConnectError>>>>;
    let mut connect: Connecting = Box::pin(first_connect());
    let mut failures = 0u32;
    let session = loop {
        tokio::select! {
            result = &mut connect => {
                // Biten bekleme bir daha yoklanmasın.
                connect = Box::pin(std::future::pending());
                match result {
                    Ok(session) => break session,
                    Err(auth::ConnectError::Offline(e)) => {
                        engine.on_offline(&e);
                        let delay = recovery::connect_backoff_for(failures);
                        failures = failures.saturating_add(1);
                        connect = Box::pin(async move {
                            tokio::time::sleep(delay).await;
                            first_connect().await
                        });
                    }
                    // Giriş gerekli / hesap ücretsiz: kullanıcı eylemi beklenir.
                    Err(refused) => engine.on_refused(refused),
                }
            },
            command = commands.recv() => match command {
                // "Yenile" / "yeniden dene": giriş gerekiyorsa girişi başlatır
                // (kullanıcı eylemi); çevrimdışıyken ya da hesap ücretsizken
                // hemen yeniden bağlanmayı dener.
                Some(Command::RefreshLibrary) => {
                    if engine.host.link.can_start_login() {
                        engine.start_login();
                    } else if engine.host.link != login::LinkState::LoggingIn {
                        engine.set_link(login::LinkState::Connecting);
                        failures = 0;
                        connect = Box::pin(first_connect());
                    }
                }
                Some(command) => engine.on_command(command),
                None => return,
            },
            Some(result) = internal_rx.recv() => engine.on_internal(result),
        }
        // Tarayıcı girişi tamamlandı: oturumla devam.
        if let Some(session) = engine.host.new_session.take() {
            break session;
        }
    };

    // 2. aşama: bağlıyken (ya da ses çıkışı yokken sadece önbellekle) çalış.
    let mut player_events = engine.install_session(session, Install::First);
    let mut health = tokio::time::interval(HEALTH_CHECK_INTERVAL);
    let mut save_tick = tokio::time::interval(PERSIST_INTERVAL);
    loop {
        // Yükleme takılırsa (oturum sessizce ölmüş olabilir) uyanılacak an.
        let stall = engine
            .pb
            .stall_deadline(engine.queue.current_uri(), engine.host.reconnecting)
            .map(tokio::time::Instant::from_std);
        tokio::select! {
            _ = async {
                match stall {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => engine.with_pb(|pb, cx| pb.on_load_stall(cx)),
            _ = health.tick() => {
                engine.check_health();
                // Zamanı gelen yazma denemeleri (bkz. `wire.rs`).
                engine.wire_tick();
            }
            _ = save_tick.tick() => engine.persist_tick(),
            command = commands.recv() => match command {
                Some(command) => engine.on_command(command),
                // Arayüz kapandı.
                None => break,
            },
            Some(event) = async {
                match player_events.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => engine.on_player_event(event),
            Some(result) = internal_rx.recv() => engine.on_internal(result),
        }
        // Çaları olmayan motora (ses çıkışı yoktu, hesap ücretsiz çıkmıştı)
        // oturum geldi: çaları şimdi kur.
        if let Some(session) = engine.host.new_session.take() {
            player_events = engine.install_session(session, Install::First);
        }
    }

    // Kapanış: son konumu yaz ve yazımın bitmesini bekle.
    engine.persist();
    if let Some(saver) = &engine.saver {
        saver.flush(std::time::Duration::from_millis(1500));
    }
    // Connect açıksa Spirc önce kapanır (Spotify'dan aygıt silinir); kapalıyken
    // anında döner.
    engine.connect_close().await;
    if let Some(deck) = &engine.pb.deck {
        deck.stop();
    }
    if let Some(conn) = &engine.host.conn {
        conn.session.shutdown();
    }
}

impl Engine {
    fn new(
        bg: tokio::runtime::Handle,
        db: Option<Arc<Db>>,
        events: async_channel::Sender<Event>,
        playback: async_channel::Sender<PlaybackEvent>,
        internal: mpsc::UnboundedSender<Internal>,
        levels: AudioLevels,
    ) -> Self {
        let (events_tx, playback_tx) = (events.clone(), playback.clone());
        Self {
            bg,
            host: SessionHost::new(),
            db,
            events,
            playback,
            internal,
            library: None,
            collections: HashMap::new(),
            complete: HashSet::new(),
            opened: HashSet::new(),
            queue: Queue::new(),
            meta: HashMap::new(),
            meta_requested: HashSet::new(),
            rules: Rules::default(),
            traits_known: HashSet::new(),
            traits_filling: false,
            hidden: Arc::new(HiddenList::default()),
            radio: radio::Feed::default(),
            saver: None,
            saved_gen: None,
            pending: Vec::new(),
            user_refresh: false,
            pb: Playback::new(
                Gate::new(events_tx, playback_tx),
                librespot::playback::config::live::key_failures(),
            ),
            levels,
            eq: equalizer::EqSetting::default(),
            karaoke: (false, 1.0),
            wire: wire::Wire::default(),
            connect: connect::ConnectHost::new(),
        }
    }

    /// Çalma olayları boşluksuz geçişte sınır duyulana dek tutulabilir
    /// (bkz. `playback/handover.rs`); geri kalanı olduğu gibi geçer.
    fn emit(&self, event: Event) {
        self.pb.gate.send(event);
    }

    fn show_cached_library(&mut self) {
        let Some(db) = &self.db else { return };
        match db.load_library() {
            Ok(Some(library)) => {
                // Önbellek Spotify'daki son bilinen hâl; önceki oturumdan
                // kalan bekleyen yazmalar üstüne uygulanır (bkz. `wire.rs`).
                let library = self.view_library(library);
                self.library = Some(library.clone());
                self.emit(Event::LibraryLoaded(library));
            }
            Ok(None) => {}
            Err(e) => log::warn!("Önbellekteki kütüphane okunamadı: {e:#}"),
        }
    }

    /// AutoMix'i açar / kapatır; açılırken önbellekteki çözümlemeler belleğe alınır.
    fn set_automix(&mut self, on: bool) {
        if on && self.pb.beats.is_empty() {
            self.load_beats();
        }
        self.with_pb(|pb, cx| pb.set_automix(cx, on));
    }

    /// Önbellekteki en yeni çözümlemeleri belleğe alır (AutoMix açıkken).
    fn load_beats(&mut self) {
        let Some(db) = &self.db else { return };
        match db.load_analyses(automix::LOAD_MAX) {
            Ok(rows) => self.pb.beats.extend(rows),
            Err(e) => log::warn!("AutoMix çözümlemeleri okunamadı: {e:#}"),
        }
    }

    /// Çalınan şarkının çözümlemesini önbelleğe yazar (arka planda).
    pub(in crate::engine) fn save_analysis(&self, uri: String, analysis: output::Analysis) {
        let Some(db) = self.db.clone() else { return };
        self.bg.spawn_blocking(move || {
            if let Err(e) = db.save_analysis(&uri, &analysis) {
                log::warn!("AutoMix çözümlemesi yazılamadı: {e:#}");
            }
        });
    }

    /// Tutulan AutoMix kipini çıkışa verir (çıkış yoksa bağlanınca).
    fn apply_automix(&self) {
        if let Some(deck) = &self.pb.deck {
            deck.out.set_automix(self.pb.automix);
        }
    }

    /// Ekolayzer ayarını tutar ve (çıkış varsa) uygular.
    fn set_eq(&mut self, setting: equalizer::EqSetting) {
        self.eq = setting;
        self.apply_eq();
    }

    /// Tutulan ekolayzer ayarını çıkışa verir (çıkış yoksa bağlanınca).
    fn apply_eq(&self) {
        if let Some(deck) = &self.pb.deck {
            deck.out
                .set_eq(self.eq.enabled, self.eq.gains_db, self.eq.preamp_db);
        }
    }

    /// Karaoke ayarını tutar ve (çıkış varsa) uygular.
    fn set_karaoke(&mut self, enabled: bool, amount: f32) {
        self.karaoke = (
            enabled,
            if amount.is_finite() {
                amount.clamp(0.0, 1.0)
            } else {
                0.0
            },
        );
        self.apply_karaoke();
    }

    /// Tutulan karaoke ayarını çıkışa verir (çıkış yoksa bağlanınca).
    fn apply_karaoke(&self) {
        if let Some(deck) = &self.pb.deck {
            deck.out.set_karaoke(self.karaoke.0, self.karaoke.1);
        }
    }

    fn on_command(&mut self, command: Command) {
        // Boşluksuz geçiş duyulmadan: komut arayüzün gösterdiği şarkıya göre.
        let Some(command) = self.with_pb(|pb, cx| pb.handover_command(cx, command)) else {
            return;
        };
        match command {
            Command::OpenCollection(id) => return self.open_collection(id),
            // Sayfalar ve yazmalar bağlantıyı kendileri bekler (bkz. `wire.rs`).
            Command::Library(c) => return self.on_library_command(c),
            Command::SetVolume(v) => return self.transport(TransportCmd::SetVolume(v)),
            Command::SetCrossfade(ms) => return self.with_pb(|pb, cx| pb.set_crossfade(cx, ms)),
            Command::SetAutoMix(on) => return self.set_automix(on),
            Command::SetEq {
                enabled,
                gains_db,
                preamp_db,
            } => {
                return self.set_eq(
                    equalizer::EqSetting {
                        enabled,
                        gains_db,
                        preamp_db,
                    }
                    .sanitized(),
                );
            }
            Command::SetKaraoke { enabled, amount } => return self.set_karaoke(enabled, amount),
            Command::Connect(c) => return self.on_connect_command(c),
            // Sıra düzenleme bağlantı istemez: yalnız modeli değiştirir.
            Command::Enqueue(rows) => return self.enqueue(rows, false),
            Command::PlayNext(rows) => return self.enqueue(rows, true),
            Command::RemoveFromQueue(i) => return self.edit_queue(|q| q.remove(i)),
            Command::MoveInQueue(from, to) => return self.edit_queue(|q| q.move_item(from, to)),
            Command::ClearQueue => {
                return self.edit_queue(|q| {
                    q.clear_user();
                    true
                });
            }
            Command::SetShuffle(on) => {
                return self.edit_queue(|q| {
                    q.set_shuffle(on);
                    true
                });
            }
            Command::SetRepeat(mode) => {
                return self.edit_queue(|q| {
                    q.set_repeat(mode);
                    true
                });
            }
            Command::SetQueueRules(rules) => return self.set_queue_rules(rules),
            Command::FetchLyrics(uri) => {
                if self.host.conn.is_some() {
                    self.fetch_lyrics(uri, false);
                } else {
                    self.pending.push(Command::FetchLyrics(uri)); // bağlanınca
                }
                return;
            }
            Command::PrefetchLyrics(uri) => return self.fetch_lyrics(uri, true),
            Command::Flush(ack) => {
                self.persist();
                if let Some(saver) = &self.saver {
                    saver.flush(std::time::Duration::from_secs(1));
                }
                let _ = ack.send(());
                return;
            }
            // Geri yüklenen şarkı çalara yüklü değil: sarma yalnız kayıtlı
            // konumu değiştirir, duraklatma zaten duraklatılmış.
            Command::Seek(ms) if self.pb.restored.is_some() => {
                return self.transport(TransportCmd::Seek(ms));
            }
            Command::Pause if self.pb.restored.is_some() => return,
            // Giriş: tarayıcı yalnız bu komutlarla (kullanıcı eylemi) açılır.
            Command::StartLogin => return self.start_login(),
            Command::CancelLogin => return self.cancel_login(),
            // "Yeniden dene" / "Yenile": giriş gerekiyorsa girişi başlatır; süren
            // girişi bozmaz; çalarsız motor (çevrimdışı, ücretsiz hesap, ses
            // çıkışı yok) yeniden bağlanıp dener.
            Command::RefreshLibrary if self.host.link.can_start_login() => {
                return self.start_login();
            }
            Command::RefreshLibrary if self.host.link == login::LinkState::LoggingIn => return,
            Command::RefreshLibrary if self.host.conn.is_none() => return self.retry_connect(),
            _ => {}
        }

        // Giriş / Premium gerekirken çalma komutları ne olduğunu söyleyip düşer
        // (bekletilmez: giriş yapılınca kendiliğinden çalmaya başlamasın).
        if matches!(
            command,
            Command::Load(_)
                | Command::PlayCollection { .. }
                | Command::PlayTracks { .. }
                | Command::PlayQueueItem(_)
                | Command::PlayContext(_)
                | Command::PlayContextAt { .. }
                | Command::StartRadio { .. }
                | Command::Play
                | Command::Next
                | Command::Previous
        ) && let Some(block) = self.host.link.playback_block()
        {
            self.emit(Event::PlaybackBlocked(block));
            return;
        }

        // Geri kalanı çalarla ilgili: bağlanmadıysak sonraya bırak.
        if self.host.conn.is_none() {
            self.pending.push(command);
            return;
        }
        match command {
            Command::Load(uri) => {
                if let Ok(uri) = uri.to_uri() {
                    self.start_queue(None, vec![uri], None, 0);
                }
            }
            Command::PlayCollection { id, index } => {
                let Some(tracks) = self.collections.get(&id).cloned() else {
                    return;
                };
                let uris = tracks.iter().map(|t| t.uri.clone()).collect();
                self.start_queue(Some(id), uris, Some(tracks), index);
            }
            Command::PlayTracks {
                context,
                tracks,
                index,
            } => {
                let uris = tracks.iter().map(|t| t.uri.clone()).collect();
                self.start_queue(context, uris, Some(tracks), index);
            }
            Command::PlayQueueItem(i) => self.transport(TransportCmd::JumpTo(i)),
            Command::PlayContext(uri) => self.play_context(uri),
            Command::PlayContextAt { uri, track, hint } => self.play_context_at(uri, track, hint),
            Command::StartRadio { seed, title } => self.start_radio(seed, title),
            Command::Search { seq, query } => self.search(seq, query),
            Command::RefreshLibrary => {
                // Kullanıcı istedi: ne olduğunu söyle (kendiliğinden yapılan
                // tazelemeler sessiz).
                self.user_refresh = true;
                self.notice(NoticeKind::Working, t!(NoticeRefreshing));
                self.refresh_library();
            }
            Command::FetchImages(urls) => self.fetch_images(urls),
            Command::Next => self.transport(TransportCmd::Next),
            Command::Previous => self.transport(TransportCmd::Previous),
            Command::Play => self.transport(TransportCmd::Play),
            Command::Pause => self.transport(TransportCmd::Pause),
            Command::Seek(ms) => self.transport(TransportCmd::Seek(ms)),
            Command::Enqueue(_)
            | Command::PlayNext(_)
            | Command::RemoveFromQueue(_)
            | Command::MoveInQueue(..)
            | Command::ClearQueue
            | Command::SetShuffle(_)
            | Command::SetRepeat(_)
            | Command::SetQueueRules(_)
            | Command::Flush(_)
            | Command::FetchLyrics(_)
            | Command::PrefetchLyrics(_)
            | Command::OpenCollection(_)
            | Command::Library(_)
            | Command::StartLogin
            | Command::CancelLogin
            | Command::SetEq { .. }
            | Command::SetKaraoke { .. }
            | Command::Connect(_)
            | Command::SetCrossfade(_)
            | Command::SetAutoMix(_)
            | Command::SetVolume(_) => unreachable!(),
        }
    }

    fn on_internal(&mut self, result: Internal) {
        match result {
            Internal::Wire(result) => self.on_wire_internal(result),
            Internal::Library(Ok(_)) if self.refresh_is_stale() => {
                // Bu tazeleme, bir yazma Spotify'a ulaşmadan önce okundu: sonucu
                // eski (örn. yeni beğenmediğin şarkı hâlâ beğenili). Gösterme,
                // yenisini iste (bekleyen "Kütüphane güncel." bildirimi kalır).
                log::debug!("Kütüphane tazelemesi yazmadan önce okunmuştu, yenileniyor");
                self.refresh_library();
            }
            Internal::Library(Ok(refresh)) => {
                if std::mem::take(&mut self.user_refresh) {
                    self.notice(NoticeKind::Info, t!(NoticeLibraryUpToDate));
                }
                // Spotify'dan gelen hâl, henüz Spotify'a ulaşmamış (bekleyen /
                // yazılamayan) işlemlerle birlikte gösterilir: yoksa beğenmeme
                // gibi bir yazma beklerken kütüphane bir an eski hâline dönerdi.
                let view = self.view_library(refresh.library);
                // Gösterilenle aynıysa arayüzü boşuna yeniden çizdirme.
                if self.library.as_ref() != Some(&view) {
                    self.library = Some(view.clone());
                    self.emit(Event::LibraryLoaded(view));
                }
                if refresh.liked_changed {
                    self.invalidate(CollectionId::Liked);
                }
                self.sync_changed(refresh.changed);
            }
            Internal::Library(Err(e)) => {
                if std::mem::take(&mut self.user_refresh) {
                    self.notice(NoticeKind::Problem, t!(NoticeRefreshFailed));
                }
                // Önbellekten bir kütüphane gösteriliyorsa onunla devam et.
                if self.library.is_none() {
                    self.emit(Event::LibraryFailed(format!("{e:#}")));
                } else {
                    log::warn!("Kütüphane tazelenemedi: {e:#}");
                }
            }
            Internal::Tracks(id, Ok(tracks)) => {
                // Bekleyen yazmalar yeniden uygulanır (bkz. `Internal::Library`).
                let tracks = self.view_tracks(&id, tracks);
                self.collections.insert(id.clone(), tracks.clone());
                self.complete.insert(id.clone());
                self.emit(Event::CollectionLoaded { id, tracks });
            }
            Internal::Tracks(id, Err(e)) => {
                if !self.collections.contains_key(&id) {
                    self.emit(Event::CollectionFailed {
                        id,
                        error: format!("{e:#}"),
                    });
                }
            }
            Internal::Synced(id) => self.invalidate(id),
            Internal::Radio(batch) => self.on_radio_batch(batch),
            Internal::Filled => self.send_search_index(),
            Internal::Search { seq, result } => match result {
                Ok(results) => self.emit(Event::SearchResults {
                    seq,
                    results: Arc::new(results),
                }),
                Err(e) => self.emit(Event::SearchFailed {
                    seq,
                    error: format!("{e:#}"),
                }),
            },
            Internal::Context(Ok(uris)) => {
                let uris = uris.iter().filter_map(|u| u.to_uri().ok()).collect();
                self.start_queue(None, uris, None, 0);
            }
            Internal::ContextAt {
                track,
                hint,
                result,
            } => {
                let all: Vec<String> = match result {
                    Ok(uris) => uris.iter().filter_map(|u| u.to_uri().ok()).collect(),
                    Err(e) => {
                        log::info!("Arkadaşın bağlamı alınamadı, yalnız şarkı çalınacak: {e:#}");
                        Vec::new()
                    }
                };
                match friends::start_position(&all, &track, hint) {
                    Some(at) => self.start_queue(None, all, None, at),
                    // Şarkı bağlamda yok (ya da bağlam alınamadı): yanlış şarkıdan
                    // başlamak yerine şarkı tek başına.
                    None => self.start_queue(None, vec![track], None, 0),
                }
            }
            Internal::Context(Err(e)) => {
                log::warn!("Sanatçı çalınamadı: {e:#}");
                self.notice(NoticeKind::Problem, t!(NoticeArtistTracksFailed));
            }
            Internal::Reconnected(result) => self.on_reconnected(result),
            Internal::Connect(message) => self.on_connect_internal(message),
            Internal::LoginDone(result) => self.on_login_done(result),
            Internal::Meta(rows) => {
                for row in rows {
                    self.meta.insert(row.uri.clone(), row);
                }
                self.emit_view();
            }
            Internal::Traits(found) => {
                self.traits_filling = false;
                if self.queue.merge_traits(&found) {
                    self.queue_changed();
                }
                // Sırada hâlâ özelliği bilinmeyen şarkılar varsa sıradaki grup.
                self.fill_traits();
            }
        }
    }

    fn notice(&self, kind: NoticeKind, message: &str) {
        self.emit(Event::Notice {
            kind,
            message: message.into(),
        });
    }

    /// Arama dizinini (bütün listelerdeki şarkılar) arka planda okur.
    ///
    /// Açılışta bu okuma (~5.900 şarkı) motorun akışında yapılınca açık
    /// listenin isteği arkasında bekliyor ve liste ~0,3 sn geç geliyordu.
    fn send_search_index(&self) {
        let Some(db) = self.db.clone() else { return };
        let events = self.events.clone();
        self.bg
            .spawn_blocking(move || match db.all_library_tracks() {
                Ok(tracks) => {
                    let _ = events.try_send(Event::SearchIndex(Arc::new(tracks)));
                }
                Err(e) => log::warn!("Arama dizini okunamadı: {e:#}"),
            });
    }

    /// Hash, yeniden keşif ve kapı `pathfinder` istemcisinde.
    fn search(&self, seq: u64, query: String) {
        let Some(conn) = &self.host.conn else {
            self.emit(Event::SearchFailed {
                seq,
                error: "Spotify'a henüz bağlanılmadı".into(),
            });
            return;
        };
        let session = conn.session.clone();
        let db = self.db.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = search::run(&session, db.as_deref(), &query, SEARCH_LIMIT)
                .await
                .map_err(anyhow::Error::from);
            let _ = internal.send(Internal::Search { seq, result });
        });
    }

    /// Sanatçı gibi bir bağlamın şarkılarını (context-resolve) çal.
    fn play_context(&self, uri: String) {
        let Some(conn) = &self.host.conn else { return };
        let session = conn.session.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = async {
                let context = session.spclient().get_context(&uri).await?;
                let uris: Vec<SpotifyUri> = context
                    .pages
                    .iter()
                    .flat_map(|p| p.tracks.iter())
                    .filter_map(|t| SpotifyUri::from_uri(t.uri()).ok())
                    .collect();
                anyhow::ensure!(!uris.is_empty(), "çalınacak şarkı bulunamadı");
                Ok(uris)
            }
            .await;
            let _ = internal.send(Internal::Context(result));
        });
    }

    /// Arkadaş etkinliği: bağlamın şarkılarını al, `track`'ten başlayarak çal
    /// (cevap `Internal::ContextAt`).
    fn play_context_at(&self, uri: String, track: String, hint: Option<usize>) {
        let Some(conn) = &self.host.conn else { return };
        let session = conn.session.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = async {
                let context = session.spclient().get_context(&uri).await?;
                Ok(context
                    .pages
                    .iter()
                    .flat_map(|p| p.tracks.iter())
                    .filter_map(|t| SpotifyUri::from_uri(t.uri()).ok())
                    .collect::<Vec<_>>())
            }
            .await;
            let _ = internal.send(Internal::ContextAt {
                track,
                hint,
                result,
            });
        });
    }

    /// Bir listenin önbellekteki hali değişti: açıksa yenisini gönder.
    fn invalidate(&mut self, id: CollectionId) {
        self.collections.remove(&id);
        self.complete.remove(&id);
        if self.opened.contains(&id) {
            self.fetch_collection(id);
        }
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        let Some(signal) = PlayerSignal::from_librespot(event) else {
            return;
        };
        self.with_pb(|pb, cx| pb.on_signal(cx, signal));
    }

    // ── Sıra ────────────────────────────────────────────────────────────

    /// Yeni bir bağlam başlat: `index`'teki şarkı çalar, karıştırma / tekrar
    /// ve kullanıcı sırası korunur (bkz. `KEEP_USER_QUEUE`).
    fn start_queue(
        &mut self,
        context: Option<CollectionId>,
        uris: Vec<String>,
        rows: Option<Arc<Vec<TrackRow>>>,
        index: usize,
    ) {
        if self.host.conn.is_none() {
            return;
        }
        let (info, traits) = self.rule_data(&uris, rows.is_some());
        let Some(uri) = self.queue.restart_with(
            context,
            uris,
            rows,
            index,
            fresh_seed(),
            KEEP_USER_QUEUE,
            info,
            traits,
        ) else {
            return;
        };
        self.fill_traits();
        self.transport(TransportCmd::Start {
            uri,
            position_ms: 0,
        });
    }

    /// Kullanıcı sırasına ekle (`front`: sonra çal). Çözülemeyen adresler
    /// sıraya alınmaz.
    fn enqueue(&mut self, rows: Vec<TrackRow>, front: bool) {
        let rows: Vec<TrackRow> = rows
            .into_iter()
            .filter(|t| SpotifyUri::from_uri(&t.uri).is_ok())
            .collect();
        if rows.is_empty() {
            return;
        }
        // Telefon sürüyorsa sıra onun: Spirc'in sırasına eklenir (bkz. `connect.rs`).
        if self.connect_enqueue(&rows.iter().map(|t| t.uri.clone()).collect::<Vec<_>>()) {
            return;
        }
        if self.meta.len() > META_LIMIT {
            self.meta.clear();
            self.meta_requested.clear();
        }
        for row in &rows {
            self.meta.insert(row.uri.clone(), row.clone());
        }
        let items = rows
            .into_iter()
            .map(|t| UserItem {
                uri: t.uri.clone(),
                track: Some(t),
            })
            .collect();
        if front {
            self.queue.play_next(items);
        } else {
            self.queue.enqueue(items);
        }
        self.queue_changed();
    }

    /// Sıra modelini düzenler; değiştiyse arayüze ve kayda yansıtır.
    fn edit_queue(&mut self, edit: impl FnOnce(&mut Queue) -> bool) {
        if edit(&mut self.queue) {
            self.queue_changed();
        }
    }

    // ── Karıştırma ve sıra kuralları (bkz. `rules.rs`) ──────────────────

    /// Bir bağlam başlarken kuralların bakacağı bilgi: satırlar (`rows`
    /// verilmediyse önbellekten; bulunamayan şarkı boş satırdır, elenmez) ve
    /// özellikler (açık söz / popülerlik; önbellekte olmayan bilinmiyor).
    /// Önbellek okuması motor iş parçacığında, birkaç milisaniye.
    fn rule_data(
        &mut self,
        uris: &[String],
        have_rows: bool,
    ) -> (Option<Arc<Vec<TrackRow>>>, Vec<Traits>) {
        let Some(db) = self.db.clone().filter(|_| !self.rules.is_empty()) else {
            return (None, Vec::new());
        };
        let info = (!have_rows).then(|| {
            let found = db.tracks(uris).unwrap_or_default();
            Arc::new(
                uris.iter()
                    .map(|u| found.get(u).cloned().unwrap_or_else(|| blank_row(u)))
                    .collect::<Vec<_>>(),
            )
        });
        let traits = if self.rules.needs_traits() {
            let found = db.track_traits(uris).unwrap_or_default();
            self.traits_known.extend(found.keys().cloned());
            uris.iter()
                .map(|u| found.get(u).copied().unwrap_or_default())
                .collect()
        } else {
            Vec::new()
        };
        (info, traits)
    }

    /// Mevcut sıra için `rule_data`'yı yükleyip uygular (kaldığı yer geri
    /// yüklendi, kurallar değişti).
    fn apply_rule_data(&mut self) {
        if self.rules.is_empty() {
            return;
        }
        let uris = self.queue.ctx_uris().to_vec();
        if uris.is_empty() {
            return;
        }
        let needs_info = self.queue.needs_info();
        let (info, traits) = self.rule_data(&uris, !needs_info);
        if let Some(info) = info {
            self.queue.attach_info(info);
        }
        if !traits.is_empty() {
            self.queue.set_traits(traits);
        }
    }

    /// Ayarlardan ya da menüden yeni kurallar geldi.
    fn set_queue_rules(&mut self, rules: Rules) {
        if rules == self.rules {
            return;
        }
        self.rules = rules.clone();
        // Bilgiyi önce yükle (kurallar eskiyken ya da yokken uygulanması
        // sıranın tek seferde yeniden kurulmasına engel olmaz: `set_rules` en sonda).
        if !rules.is_empty() {
            self.apply_rule_data();
        }
        self.queue.set_rules(rules);
        self.queue_changed();
        self.fill_traits();
    }

    /// "Açık sözlüleri atla" açıkken sıradaki şarkıların açık söz işareti
    /// bilinmiyorsa (özellik eklenmeden önce önbelleğe alınmış şarkılar)
    /// Spotify'dan arka planda, sıradakilerden başlayarak tamamlanır.
    fn fill_traits(&mut self) {
        if !self.rules.skip_explicit || self.traits_filling {
            return;
        }
        let (Some(db), Some(conn)) = (self.db.clone(), &self.host.conn) else {
            return;
        };
        let known = &self.traits_known;
        let missing = self
            .queue
            .untraited_upcoming(&|u| known.contains(u), TRAITS_FILL_BATCH);
        if missing.is_empty() {
            return;
        }
        self.traits_known.extend(missing.iter().cloned());
        self.traits_filling = true;
        let session = conn.session.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let rows = library::fetch_tracks(&session, missing.clone()).await;
            // `save_tracks` bu şarkıların özelliklerini de yazar (bkz. `traits.rs`).
            let found = tokio::task::spawn_blocking(move || {
                if !rows.is_empty()
                    && let Err(e) = db.save_tracks(&rows)
                {
                    log::warn!("Tamamlanan şarkı bilgileri yazılamadı: {e:#}");
                }
                db.track_traits(&missing).unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            // Gruplar arası nefes: Spotify'ı boğma.
            tokio::time::sleep(TRAITS_FILL_PAUSE).await;
            let _ = internal.send(Internal::Traits(found));
        });
    }

    /// Sıra değişti: arayüze bildir, önyüklemeyi yenile, kaydet.
    fn queue_changed(&mut self) {
        self.emit(Event::QueueChanged(queue_state(&self.queue)));
        self.emit_view();
        self.pb.refresh_preload(&self.queue);
        self.persist();
    }

    /// Sıra görünümünü gönderir; bilgisi bulunamayan şarkıların bilgisini
    /// önbellekten ister (gelince görünüm yenilenir).
    fn emit_view(&mut self) {
        let view = self.queue.view(&|uri| self.meta.get(uri).cloned());
        let missing = Queue::missing_meta(&view);
        self.pb.gate.send_view(PlaybackEvent::Queue(view));
        self.request_meta(missing);
    }

    fn request_meta(&mut self, uris: Vec<String>) {
        let Some(db) = self.db.clone() else { return };
        let uris: Vec<String> = uris
            .into_iter()
            .filter(|u| self.meta_requested.insert(u.clone()))
            .collect();
        if uris.is_empty() {
            return;
        }
        let internal = self.internal.clone();
        self.bg.spawn_blocking(move || match db.tracks(&uris) {
            Ok(found) if !found.is_empty() => {
                let _ = internal.send(Internal::Meta(found.into_values().collect()));
            }
            Ok(_) => {}
            Err(e) => log::warn!("Sıradaki şarkıların bilgisi okunamadı: {e:#}"),
        });
    }

    // ── Kütüphane ───────────────────────────────────────────────────────

    fn refresh_library(&self) {
        let (Some(conn), Some(db)) = (&self.host.conn, &self.db) else {
            return;
        };
        let session = conn.session.clone();
        let db = db.clone();
        self.mark_refresh_started();
        // Önceki hâl: önbellekteki (Spotify'daki son bilinen) kütüphane.
        // Ekrandaki (`self.library`) bekleyen yazmalarla değişmiş olabilir;
        // o, Spotify'dan alınmış gibi önbelleğe geri yazılmamalı.
        let previous = db.load_library().ok().flatten();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = library::refresh_library(&session, &db, previous.as_ref()).await;
            let _ = internal.send(Internal::Library(result));
        });
    }

    /// Sürümü değişen listeleri sırayla indir, sonra bilgisi eksik bütün
    /// şarkıları tamamla. Hepsi arka planda; arayüz beklemez.
    fn sync_changed(&self, changed: Vec<(CollectionId, Vec<u8>)>) {
        let (Some(conn), Some(db)) = (&self.host.conn, &self.db) else {
            return;
        };
        let session = conn.session.clone();
        let db = db.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let count = changed.len();
            for (id, revision) in changed {
                match library::sync_playlist(&session, &db, &id, &revision).await {
                    Ok(()) => {
                        let _ = internal.send(Internal::Synced(id));
                    }
                    Err(e) => log::warn!("{} güncellenemedi: {e:#}", id.key()),
                }
            }
            if count > 0 {
                log::info!("{count} liste güncellendi");
            }
            match library::fill_missing_tracks(&session, &db).await {
                Ok(n) if n > 0 || count > 0 => {
                    log::info!("{n} şarkının bilgisi önbelleğe eklendi");
                    let _ = internal.send(Internal::Filled);
                }
                Ok(_) => {}
                Err(e) => log::warn!("Şarkı bilgileri tamamlanamadı: {e:#}"),
            }
        });
    }

    fn open_collection(&mut self, id: CollectionId) {
        self.opened.insert(id.clone());

        if let Some(tracks) = self.collections.get(&id) {
            self.emit(Event::CollectionLoaded {
                id,
                tracks: tracks.clone(),
            });
            return;
        }

        // Henüz Spotify'da oluşmamış (geçici adresli) liste: gösterilecek tek
        // şey bekleyen oluşturma işlemi (önceki oturumdan kalmış olabilir);
        // Spotify'dan istenecek bir şey yok.
        if edits::is_temp_id(id.key()) {
            let tracks = self.view_tracks(&id, Arc::new(Vec::new()));
            self.collections.insert(id.clone(), tracks.clone());
            self.emit(Event::CollectionLoaded { id, tracks });
            return;
        }

        // Önbellekte varsa hemen göster.
        if let Some(db) = &self.db {
            match library::cached_tracks(db, &id) {
                // Şarkı bilgileri henüz yoksa boş liste gösterme; gelene kadar
                // arayüz "Yükleniyor" desin.
                Ok(Some((tracks, complete))) if complete || !tracks.is_empty() => {
                    // Önbellek Spotify'daki son bilinen hâl: bekleyen yazmalar
                    // (önceki oturumdan kalanlar dahil) üstüne uygulanır.
                    let tracks = self.view_tracks(&id, tracks);
                    self.collections.insert(id.clone(), tracks.clone());
                    if complete {
                        self.complete.insert(id.clone());
                    }
                    self.emit(Event::CollectionLoaded {
                        id: id.clone(),
                        tracks,
                    });
                    if complete {
                        return;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => {}
                Err(e) => log::warn!("Önbellekteki liste okunamadı: {e:#}"),
            }
        }
        // Eksikse (ya da hiç yoksa) Spotify'dan tamamla; bağlı değilsek
        // bağlanınca `attach` tamamlar.
        self.fetch_collection(id);
    }

    fn fetch_collection(&self, id: CollectionId) {
        let (Some(conn), Some(db)) = (&self.host.conn, &self.db) else {
            return;
        };
        let session = conn.session.clone();
        let db = db.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = library::load_tracks(&session, &db, &id).await;
            let _ = internal.send(Internal::Tracks(id, result));
        });
    }

    fn fetch_cover(&self, track_uri: String, url: String) {
        let session = self.host.conn.as_ref().map(|c| c.session.clone());
        // Kapak da şarkıyla birlikte, boşluksuz geçişte sınır duyulunca gitsin
        // (önce giderse arayüz, henüz çalmayan şarkının kapağını atar).
        let events = self.pb.gate.clone();
        self.bg.spawn(async move {
            // Diskte varsa ağa hiç çıkma (eski tam boy dosya varsa küçültülür).
            let on_disk = {
                let url = url.clone();
                tokio::task::spawn_blocking(move || cache::read_cover(&url))
                    .await
                    .ok()
                    .flatten()
            };
            let bytes = match on_disk {
                Some(bytes) => bytes,
                None => {
                    let Some(session) = session else { return };
                    // Geçici ağ hatasında birkaç kez dene: çalan şarkı kapaksız kalmasın.
                    let mut downloaded = None;
                    for attempt in 0..COVER_ATTEMPTS {
                        match download(&session, &url).await {
                            Ok(bytes) => {
                                downloaded = Some(bytes);
                                break;
                            }
                            Err(e) => {
                                log::warn!("Kapak indirilemedi (deneme {}): {e:#}", attempt + 1);
                                if attempt + 1 < COVER_ATTEMPTS {
                                    tokio::time::sleep(std::time::Duration::from_millis(
                                        800 * (attempt as u64 + 1),
                                    ))
                                    .await;
                                }
                            }
                        }
                    }
                    let Some(bytes) = downloaded else { return };
                    // Diske küçültülmüş yazılır; çalar da küçültülmüş halini alır.
                    let url = url.clone();
                    tokio::task::spawn_blocking(move || cache::store_cover(&url, &bytes))
                        .await
                        .unwrap_or_default()
                }
            };
            if bytes.is_empty() {
                return;
            }
            // Vurgu rengi için resmi çözmek birkaç ms sürer: ayrı iş parçacığında.
            let (bytes, accent) = tokio::task::spawn_blocking(move || {
                let accent = palette::accent_from_image(&bytes);
                (bytes, accent)
            })
            .await
            .unwrap_or_default();
            events.send(Event::Cover {
                track_uri,
                bytes,
                accent,
            });
        });
    }

    /// Kenar çubuğu ve listelerdeki küçük kapaklar: aynı anda en fazla 6
    /// indirme, diskte olanlar atlanır.
    /// Şarkı sözü (bkz. `lyrics.rs`): çalan, sıradaki ya da önbellekteki bir
    /// şarkının adı / sanatçısı / süresi / albümü elde olmalı.
    fn fetch_lyrics(&self, uri: String, prefetch: bool) {
        let job = match (&self.host.conn, &self.db) {
            (Some(_), Some(_)) => self.lyrics_track(&uri),
            _ => None,
        };
        let (Some(conn), Some(db), Some((track, album))) = (&self.host.conn, &self.db, job) else {
            if !prefetch {
                self.emit(Event::Lyrics { uri, lyrics: None });
            }
            return;
        };
        let session = conn.session.clone();
        let db = db.clone();
        let events = self.events.clone();
        self.bg.spawn(async move {
            if prefetch {
                lyrics::prefetch(&session, &db, &track, album.as_deref()).await;
                return;
            }
            let lyrics = lyrics::get(&session, &db, &track, album.as_deref()).await;
            let _ = events.try_send(Event::Lyrics { uri, lyrics });
        });
    }

    /// Söz aramak için şarkının bilgileri ve albümü; adı bilinmiyorsa `None`.
    fn lyrics_track(&self, uri: &str) -> Option<(TrackInfo, Option<String>)> {
        let row = (self.queue.current_uri() == Some(uri))
            .then(|| self.queue.current_row())
            .flatten()
            .or_else(|| self.meta.get(uri).cloned())
            .or_else(|| {
                self.db
                    .as_ref()?
                    .tracks(&[uri.to_owned()])
                    .ok()?
                    .remove(uri)
            });
        let info = match (&self.pb.track_info, row.as_ref()) {
            (Some(t), _) if t.uri == uri && !t.name.is_empty() => t.clone(),
            (_, Some(r)) => TrackInfo {
                uri: uri.to_owned(),
                name: r.name.clone(),
                artists: r.artists.clone(),
                duration_ms: r.duration_ms,
                cover_url: None,
            },
            _ => return None,
        };
        let album = row.map(|r| r.album).filter(|a| !a.is_empty());
        Some((info, album))
    }

    fn fetch_images(&self, urls: Vec<String>) {
        let Some(conn) = &self.host.conn else { return };
        let session = conn.session.clone();
        let events = self.events.clone();
        self.bg.spawn(async move {
            use futures::StreamExt;
            futures::stream::iter(urls)
                .for_each_concurrent(6, |url| {
                    let session = session.clone();
                    let events = events.clone();
                    async move {
                        // Diskte (küçültülmüş halde) var mı? Eski sürümden kalan
                        // tam boy dosya varsa ağa çıkmadan küçültülür.
                        let on_disk = {
                            let url = url.clone();
                            tokio::task::spawn_blocking(move || cache::ensure_cover(&url))
                                .await
                                .unwrap_or(false)
                        };
                        if !on_disk {
                            match download(&session, &url).await {
                                Ok(bytes) => {
                                    let url = url.clone();
                                    let _ = tokio::task::spawn_blocking(move || {
                                        cache::store_cover(&url, &bytes)
                                    })
                                    .await;
                                }
                                // Haber verilmez: arayüz bir süre sonra aynı resmi
                                // yeniden ister (bkz. `ui::images`).
                                Err(e) => {
                                    log::debug!("Resim indirilemedi ({url}): {e:#}");
                                    return;
                                }
                            }
                        }
                        let _ = events.try_send(Event::ImageReady(url));
                    }
                })
                .await;
        });
    }
}

/// Çalan şarkının kapağı için en çok kaç kez indirilmeyi deneriz.
const COVER_ATTEMPTS: u32 = 3;

/// Oturumun sağlığı bu aralıkla kontrol edilir.
const HEALTH_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
/// Yeni bir bağlam başlayınca kullanıcı sırası (sıraya eklenenler) korunur mu?
/// Spotify'da korunuyor.
const KEEP_USER_QUEUE: bool = true;
/// Açık sözlüleri atla için eksik özellikler bu kadarlık gruplarla (6 istek)
/// tamamlanır; gruplar arası bekleme Spotify'ı boğmamak için.
const TRAITS_FILL_BATCH: usize = 600;
const TRAITS_FILL_PAUSE: std::time::Duration = std::time::Duration::from_millis(800);
/// Kaldığı yer kaydı çalarken bu aralıkla yazılır (çökmede en çok ~1 sn
/// kayıp; tek satırlık küçük yazım).
const PERSIST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
/// Bellekteki şarkı bilgisi önbelleğinin sınırı.
const META_LIMIT: usize = 5_000;
/// Spotify aramasında her türden istenen sonuç sayısı.
const SEARCH_LIMIT: usize = 10;

/// Önbellekte bilgisi bulunamayan şarkının kurallara verilen boş satırı
/// (sanatçısı bilinmediği için hiçbir kural onu elemez).
fn blank_row(uri: &str) -> TrackRow {
    TrackRow {
        uri: uri.to_string(),
        name: String::new(),
        artists: String::new(),
        album: String::new(),
        duration_ms: 0,
        image: String::new(),
    }
}

/// Sıranın arayüzdeki eski görünümü: hangi listenin kaçıncı (özgün sıradaki)
/// şarkısı çalıyor. Çalan şarkı kullanıcı sırasından geldiyse bağlam yok.
fn queue_state(queue: &Queue) -> QueueState {
    let index = queue.context_index();
    QueueState {
        context: index.and_then(|_| queue.context().cloned()),
        index: index.unwrap_or(0),
        has_next: queue.has_next(),
        has_previous: queue.has_previous(),
    }
}

/// Karıştırma tohumu: her yeni bağlamda farklı (ama bağlam içinde kararlı).
fn fresh_seed() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5EED);
    nanos ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
}

async fn download(session: &Session, url: &str) -> anyhow::Result<Vec<u8>> {
    let request = http::Request::get(url).body(Bytes::new())?;
    Ok(session.http_client().request_body(request).await?.to_vec())
}

#[cfg(test)]
mod tests;
