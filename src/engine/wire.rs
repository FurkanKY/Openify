//! Sayfalar (Adım 12) ve kütüphane yazmaları (Adım 13) motora burada bağlanır.
//!
//! Arayüz `Command::Library` ile ister, cevaplar `Event::Library` ile gelir.
//! `mod.rs`'e yalnız bu iki sarmalayıcı, bir iç sonuç türü ve birkaç çağrı
//! eklendi; işin kendisi burada (başka işlerin `mod.rs`'e dokunduğu
//! dosyada çakışma az olsun).
//!
//! Sayfalar: önbellekte varsa (bayat da olsa) hemen gönderilir; bayatsa ya da
//! hiç yoksa arkadan Spotify'dan alınır ve içerik değiştiyse yeniden
//! gönderilir (önce göster, sonra tazele). Sayfa önbelleği arayüzle
//! paylaşılır (`cached_artist` / `cached_album`): önbellekteki sayfa
//! tıklandığı karede, motora gidip gelmeden açılır.
//!
//! Yazmalar: işlem önce kalıcı kuyruğa (`edits.db`) yazılır, motorun kendi
//! kopyası (listeler, kütüphane, disk önbelleği) aynı saf fonksiyonlarla
//! (`patch_tracks`, `patch_library`) arayüzle birebir aynı biçimde
//! güncellenir, sonra Spotify'a gönderilir. Kuyruk bağlanınca, her işlemden
//! sonra ve zamanı gelen yeniden denemeler için düzenli olarak işlenir.
//!
//! Sınama koşusunda (`OPENIFY_TEST_DISPLAY` ya da `OPENIFY_TEST_EDITS`)
//! kuyruk bellekte durur ve sahte bir arka uca gider: arayüz sınamaları
//! kullanıcının Spotify hesabına HİÇBİR ŞEY yazamaz.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::t;
use anyhow::Result;

use super::{
    CollectionId, Engine, Event, Internal, Library, LibraryItem, NoticeKind, TrackRow,
    cache::Db,
    credits::{self, Credits, CreditsFailure},
    edits::{
        self, AddPosition, ApplyCtx, EditBackend, EditError, EditOp, EditOutcome, EditQueue,
        Enqueued, FailKind, ProcessReport, SpotifyBackend,
    },
    explore::{self, ExploreData, ExploreKey},
    friends::{self, Friend, FriendsFailure},
    pages::{self, AlbumPage, ArtistPage, Loaded, PageCache, TrackLinks},
    related::{self, Freshness, RelatedHint, RelatedList},
    search,
    stats::{self, AlbumPlaycounts, ArtistStats, StatsFailure},
};

/// Tam arama sayfasında bölüm başına sonuç.
const SEARCH_ALL_LIMIT: usize = 30;

/// Bir yazma kalıcı olarak başarısız olunca gösterilen şeridin metni. Arayüz,
/// yazılamayan işlem kalmayınca (bırakıldı ya da yeniden denendi) bu şeridi
/// kaldırır: başlıktaki simge de panel de yokken ona yönlendirmesin.
pub fn edit_failed_notice() -> &'static str {
    t!(EditFailedNotice)
}

// ── Arayüzle konuşma ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum LibraryCommand {
    /// Sanatçı sayfası (`LibraryEvent::Artist` ya da `PageFailed`).
    OpenArtist(String),
    /// Albüm sayfası (`LibraryEvent::Album` ya da `PageFailed`).
    OpenAlbum(String),
    /// Sanatçı ya da albüm adresinin üzerine gelindi: önbellekte yoksa
    /// (ya da bayatsa) şimdiden al. Başarısızlık söylenmez.
    Prefetch(String),
    /// Bu şarkıların albüm ve sanatçı adresleri (`LibraryEvent::Links`).
    ResolveLinks(Vec<String>),
    /// Tam arama sayfası: sonuç `Event::SearchResults` ile aynı `seq` ile.
    SearchAll {
        seq: u64,
        query: String,
    },
    /// Açılan sanatçının benzerleri (`LibraryEvent::Related`; bkz.
    /// `related.rs`). Yalnız ekrana gelen sanatçı için istenir (önceden alma
    /// yok); hata söylenmez. Ad ve popüler şarkı yalnız Deezer yedeği için.
    OpenRelated {
        uri: String,
        name: String,
        top_track: Option<String>,
    },
    /// Deezer'dan gelen benzer sanatçı kartına tıklandı (`deezer:artist:<id>`):
    /// Spotify adresini bul (`LibraryEvent::ArtistResolved`).
    ResolveArtist {
        from: String,
        name: String,
    },
    /// Keşfet / Ana sayfa verisi (`LibraryEvent::Explore`, olmazsa `ExploreFailed`;
    /// bkz. `explore.rs`). Yalnız ekrana gelen sayfa için istenir.
    OpenExplore(ExploreKey),
    /// Arkadaş etkinliğini şimdi al (`LibraryEvent::Friends` ya da
    /// `FriendsFailed`; bkz. `friends.rs`). Ne zaman sorulacağına arayüz karar
    /// verir (panel ve pencere görünürken 60 sn'de bir); bağlantı yoksa
    /// sıraya konmaz, `Offline` döner (arayüz bir sonraki turda yeniden sorar).
    OpenFriends,
    /// Şarkının künyesini al (`LibraryEvent::Credits`; salt okuma, bkz.
    /// `credits.rs`). Kullanıcı "Künyeyi göster"e basınca; bağlantı yoksa
    /// `Offline` döner.
    OpenCredits(String),
    /// Kütüphane yazması. `rows`: işlemdeki şarkıların bilgileri (listeye
    /// yerelde eklenebilsin diye); `item`: albüm kaydederken albümün kartı.
    Edit {
        op: EditOp,
        rows: Vec<TrackRow>,
        item: Option<LibraryItem>,
    },
    /// Başarısız yazmayı yeniden dene / bırak.
    RetryEdit(i64),
    DismissEdit(i64),
    /// Listeyi / albümü / sanatçıyı en üste sabitle ya da sabitlemeyi kaldır
    /// (`key`: `CollectionId::key()`). Yerel: `library_pins`; Spotify'a hiçbir
    /// şey yazılmaz (arayüz zaten iyimser günceller, cevap yok).
    Pin {
        key: String,
        pinned: bool,
    },
    /// Listenin şarkılarının eklenme anları (`LibraryEvent::AddedDates`;
    /// önbellekten okunur, ağa gitmez).
    AddedDates(CollectionId),
    // ── Kütüphane araçları (`tools/`); üçü de yerel, Spotify'a hiçbir şey gitmez ──
    /// Sayılan bir çalma ya da dinleme süresi eki (yerel çalma geçmişi).
    RecordPlay(super::tools::stats::PlayLog),
    /// İstatistik raporu (`LibraryEvent::PlayStats`).
    PlayStats(super::tools::stats::StatsRange),
    /// Kütüphaneyi bu dosyaya JSON olarak yaz (`BackupDone` / `BackupFailed`).
    ExportBackup(std::path::PathBuf),
    /// Şarkıları yerel olarak gizle (`on`) ya da göster (bkz. `hidden.rs`).
    /// Spotify'a hiçbir şey yazılmaz; cevap `LibraryEvent::Hidden`.
    SetHidden {
        songs: Vec<super::hidden::HiddenSong>,
        on: bool,
    },
}

#[derive(Debug, Clone)]
pub enum LibraryEvent {
    Artist {
        uri: String,
        page: Arc<ArtistPage>,
    },
    Album {
        uri: String,
        page: Arc<AlbumPage>,
    },
    /// Sanatçı sayfasının dinlenme sayıları (ayrı, isteğe bağlı istek).
    ArtistStats {
        uri: String,
        stats: Arc<ArtistStats>,
    },
    /// Albüm şarkılarının dinlenme sayıları (ayrı, isteğe bağlı istek).
    AlbumPlaycounts {
        uri: String,
        stats: Arc<AlbumPlaycounts>,
    },
    PageFailed {
        uri: String,
        error: String,
    },
    Links(Vec<TrackLinks>),
    /// Sanatçının benzerleri (boş olabilir: bölüm gösterilmez).
    Related {
        uri: String,
        list: Arc<RelatedList>,
    },
    /// Deezer kartının Spotify adresi (`None`: bulunamadı).
    ArtistResolved {
        from: String,
        uri: Option<String>,
    },
    /// Keşfet / Ana sayfa verisi (önbellekten ya da yeni indirilmiş).
    Explore {
        key: ExploreKey,
        data: ExploreData,
    },
    /// İndirilemedi (gösterilen bayat veri varsa o kalır).
    ExploreFailed {
        key: ExploreKey,
        error: String,
    },
    /// Arkadaş etkinliği (en yeni önde; boş olabilir).
    Friends(Arc<Vec<Friend>>),
    FriendsFailed(FriendsFailure),
    /// Şarkı künyesi (`LibraryCommand::OpenCredits` cevabı; boş olabilir).
    Credits {
        uri: String,
        result: Result<Arc<Credits>, CreditsFailure>,
    },
    /// Kuyruğun durumu: bekleyen yazma sayısı ve yazılamayanlar.
    EditStatus {
        pending: usize,
        failed: Vec<FailedEdit>,
    },
    /// Geçici adresle oluşturulan liste Spotify'da oluştu.
    Created {
        temp: String,
        uri: String,
    },
    /// `LibraryCommand::AddedDates` cevabı: şarkı adresi → eklenme anı (unix
    /// ms; tarihi bilinmeyen şarkı yok).
    AddedDates {
        id: CollectionId,
        dates: Arc<HashMap<String, i64>>,
    },
    /// `LibraryCommand::PlayStats` cevabı.
    PlayStats(Arc<super::tools::stats::StatsReport>),
    /// Yedek yazıldı.
    BackupDone(super::tools::backup::Summary),
    /// Yedek yazılamadı (ayrıntı günlükte; arayüz kendi cümlesini yazar).
    BackupFailed,
    /// Yerel olarak gizlenen şarkıların güncel dizini (açılışta ve her değişiklikte).
    Hidden(Arc<super::hidden::HiddenList>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FailedEdit {
    pub id: i64,
    pub op: EditOp,
    /// Ham hata metni (günlük için; ekranda gösterilmez).
    pub error: String,
    /// Hatanın türü: ekranda neden bununla söylenir.
    pub kind: FailKind,
}

/// Arka plan işlerinin motora dönen sonuçları.
pub(super) enum WireInternal {
    Page {
        uri: String,
        quiet: bool,
        cached: bool,
        result: Result<PageResult, String>,
    },
    Stats {
        uri: String,
        result: Result<StatsResult, StatsFailure>,
    },
    Links {
        asked: Vec<String>,
        result: Result<Vec<TrackLinks>, String>,
    },
    Related {
        uri: String,
        result: Result<Arc<RelatedList>, String>,
    },
    Explore {
        key: ExploreKey,
        result: Result<ExploreData, String>,
    },
    Friends(Result<Vec<Friend>, FriendsFailure>),
    Credits {
        uri: String,
        result: Result<Arc<Credits>, CreditsFailure>,
    },
    Resolved {
        from: String,
        result: Result<String, String>,
    },
    Processed(anyhow::Result<ProcessReport>),
}

pub(super) enum PageResult {
    Artist(Loaded<ArtistPage>),
    Album(Loaded<AlbumPage>),
}

/// Dinlenme sayılarının sonucu (sanatçı ya da albüm).
pub(super) enum StatsResult {
    Artist(Arc<ArtistStats>),
    Album(Arc<AlbumPlaycounts>),
}

/// Sayfa önbelleği: motor açar, arayüz de okur.
static PAGE_CACHE: OnceLock<Arc<PageCache>> = OnceLock::new();

fn page_cache() -> Option<Arc<PageCache>> {
    PAGE_CACHE.get().cloned()
}

/// Önbellekteki sanatçı sayfası (bayat olsa da) ve bayat mı; arayüz
/// tıklandığı karede açabilsin diye doğrudan okur.
pub fn cached_artist(uri: &str) -> Option<(Arc<ArtistPage>, bool)> {
    let c = pages::cached_artist(PAGE_CACHE.get()?, uri).ok()??;
    log::debug!(
        "{uri}: önbellekten ({} dk önce indirilmiş, bayat: {})",
        c.age_ms / 60_000,
        c.stale
    );
    Some((c.page, c.stale))
}

/// Önbellekteki benzer sanatçılar (gösterilecek kadar yeniyse) ve taze mi
/// (değilse arkadan tazelenir). Boş liste de döner: 24 saat sorulmaz.
pub fn cached_related(uri: &str) -> Option<(Arc<RelatedList>, bool)> {
    match related::cached(PAGE_CACHE.get()?, uri)? {
        (list, Freshness::Fresh) => Some((list, true)),
        (list, Freshness::Stale) => Some((list, false)),
        (_, Freshness::Expired) => None,
    }
}

/// Önbellekteki Keşfet / Ana sayfa verisi (7 günden eskisi yok) ve taze mi
/// (değilse arkadan tazelenir).
pub fn cached_explore(key: &ExploreKey) -> Option<(ExploreData, bool)> {
    let (data, state) = explore::cached(PAGE_CACHE.get()?, key)?;
    Some((data, state == explore::Freshness::Fresh))
}

/// Keşfet'teki "benzer sanatçılardan öneriler" (yalnız önbellekten; bkz.
/// `related::suggestions`).
pub fn suggested_artists(known: &HashSet<String>, limit: usize) -> Vec<pages::ArtistCard> {
    PAGE_CACHE
        .get()
        .map(|c| related::suggestions(c, known, limit))
        .unwrap_or_default()
}

/// Deezer kartının daha önce bulunmuş Spotify adresi.
pub fn resolved_artist(deezer_uri: &str) -> Option<String> {
    related::resolved(PAGE_CACHE.get()?, deezer_uri)
}

pub fn cached_album(uri: &str) -> Option<(Arc<AlbumPage>, bool)> {
    let c = pages::cached_album(PAGE_CACHE.get()?, uri).ok()??;
    log::debug!(
        "{uri}: önbellekten ({} dk önce indirilmiş, bayat: {})",
        c.age_ms / 60_000,
        c.stale
    );
    Some((c.page, c.stale))
}

// ── Sınama: sahte yazma ─────────────────────────────────────────────────

/// Sınama koşusunda yazmaların gideceği sahte arka uç.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fake {
    /// 400 ms sonra başarılı.
    Ok,
    /// 4 sn (ya da `slow-<sn>`) sonra başarılı (bekleyen durumu görülsün).
    Slow(u64),
    /// Hemen kalıcı hata (yazılamayanlar listesi görülsün); hatanın türü
    /// `fail` (izin yok), `fail-notfound` (yok), `fail-invalid` (geçersiz).
    Fail(FailKind),
}

/// `OPENIFY_TEST_EDITS=ok|slow[-sn]|fail[-notfound|-invalid]`; yoksa sınama
/// penceresinde (`OPENIFY_TEST_DISPLAY`) `ok`. Normal açılışta `None`: gerçek
/// Spotify.
fn test_edits() -> Option<Fake> {
    parse_test_edits(
        std::env::var("OPENIFY_TEST_EDITS").ok().as_deref(),
        std::env::var_os("OPENIFY_TEST_DISPLAY").is_some(),
    )
}

fn parse_test_edits(edits: Option<&str>, test_display: bool) -> Option<Fake> {
    match edits {
        Some("slow") => Some(Fake::Slow(4)),
        Some(s) if s.starts_with("slow-") => Some(Fake::Slow(
            s["slow-".len()..].parse().unwrap_or(4).clamp(1, 120),
        )),
        Some("fail-notfound") => Some(Fake::Fail(FailKind::NotFound)),
        Some("fail-invalid") => Some(Fake::Fail(FailKind::Invalid)),
        Some(s) if s.starts_with("fail") => Some(Fake::Fail(FailKind::Forbidden)),
        Some(_) => Some(Fake::Ok),
        None => test_display.then_some(Fake::Ok),
    }
}

/// Başarılı her yazmanın anını kaydeden sarmalayıcı (kuyruk işi engelleyici
/// iş parçacığında koşar; motor bu damgayı `refresh_is_stale` ile okur).
struct Stamped<'a, B> {
    inner: B,
    stamp: &'a AtomicI64,
}

impl<B: EditBackend + Send> EditBackend for Stamped<'_, B> {
    async fn apply(&self, op: &EditOp, ctx: ApplyCtx) -> Result<EditOutcome, EditError> {
        let done = self.inner.apply(op, ctx).await;
        if done.is_ok() {
            self.stamp.store(now_ms(), Ordering::Relaxed);
        }
        done
    }
}

struct FakeBackend(Fake);

impl EditBackend for FakeBackend {
    async fn apply(&self, op: &EditOp, _ctx: ApplyCtx) -> Result<EditOutcome, EditError> {
        op.validate()?;
        let wait = match self.0 {
            Fake::Ok => Duration::from_millis(400),
            Fake::Slow(secs) => Duration::from_secs(secs),
            Fake::Fail(_) => Duration::from_millis(200),
        };
        tokio::time::sleep(wait).await;
        if let Fake::Fail(kind) = self.0 {
            let why = "sınama: yazma bilerek reddedildi".to_string();
            return Err(match kind {
                FailKind::NotFound => EditError::NotFound(why),
                FailKind::Invalid => EditError::Invalid(why),
                _ => EditError::Forbidden(why),
            });
        }
        Ok(match op {
            EditOp::CreatePlaylist { temp_id, .. } => EditOutcome::Created {
                uri: format!(
                    "spotify:playlist:sinama{}",
                    temp_id.rsplit(':').next().unwrap_or("0")
                ),
            },
            _ => EditOutcome::Done,
        })
    }
}

// ── Motor tarafının durumu ──────────────────────────────────────────────

#[derive(Default)]
pub(super) struct Wire {
    queue: Option<Arc<EditQueue>>,
    fake: Option<Fake>,
    /// Kuyruk şu an işleniyor; bu sırada yeni işlem geldiyse bitince yeniden.
    processing: bool,
    again: bool,
    /// En erken yeniden deneme zamanı (unix ms).
    next_due: Option<i64>,
    /// İstenmiş, henüz gelmemiş sayfalar ve bağlar (aynısı iki kez gitmesin).
    loading: HashSet<String>,
    links_asked: HashSet<String>,
    /// Dinlenme sayıları bu oturumda sorulmuş sanatçı / albümler (sayfa
    /// açılışı başına en çok bir ek istek; başarısızsa oturumda yeniden denenmez).
    stats_asked: HashSet<String>,
    /// Benzerleri istenmiş, henüz gelmemiş sanatçılar; adresi aranan Deezer kartları.
    related_loading: HashSet<String>,
    explore_loading: HashSet<ExploreKey>,
    /// Arkadaş etkinliği isteği yolda (aynı anda tek istek).
    friends_loading: bool,
    resolving: HashSet<String>,
    /// Kuyruk işi → işlemdeki şarkıların bilgileri ve (albüm kaydında) kartı.
    /// Kuyruk yalnız işlemin kendisini saklıyor; yenilenen veriye bekleyen
    /// işlemler yeniden uygulanırken (`PendingEdit`) bunlar gerekiyor.
    rows: HashMap<i64, EditRows>,
    /// Silinen listeler (geri getirmek için): adres → (yeri, kartı). Ekrandaki
    /// (iyimser) hâl için; Spotify'daki hâl için `deleted_truth`.
    deleted: HashMap<String, (usize, LibraryItem)>,
    /// Aynısı, Spotify'a yazılmış (önbelleğe işlenmiş) silmeler için.
    deleted_truth: HashMap<String, (usize, LibraryItem)>,
    /// Son başarılı yazmanın ve son kütüphane tazelemesinin başlangıcının anı
    /// (unix ms; 0 = hiç). Bkz. `refresh_is_stale`.
    last_write_ms: Arc<AtomicI64>,
    refresh_started_ms: AtomicI64,
}

/// Bir yazma işleminin şarkı bilgileri ve albüm kartı.
#[derive(Debug, Clone, Default)]
struct EditRows {
    rows: Vec<TrackRow>,
    item: Option<LibraryItem>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Şarkı adresi → eklenme anı. Tarihi bilinmeyen şarkı haritada yok; aynı
/// şarkı listede iki kez geçiyorsa ilk satırın tarihi.
pub(super) fn added_dates_of(entries: &[super::library::Entry]) -> HashMap<String, i64> {
    let mut dates = HashMap::new();
    for e in entries {
        if let Some(at) = e.added_at {
            dates.entry(e.track.clone()).or_insert(at);
        }
    }
    dates
}

impl Engine {
    /// Sayfa önbelleğini ve yazma kuyruğunu açar (motor başlarken bir kez).
    pub(super) fn open_wire(&mut self) {
        match PageCache::open_default() {
            Ok(cache) => {
                let cache = Arc::new(cache);
                let _ = PAGE_CACHE.set(cache.clone());
                // 60 günden eski sayfalar silinir (arka planda).
                self.bg.spawn_blocking(move || match cache.prune() {
                    Ok(n) if n > 0 => log::debug!("sayfa önbelleği: {n} eski sayfa silindi"),
                    Ok(_) => {}
                    Err(e) => log::warn!("Sayfa önbelleği temizlenemedi: {e:#}"),
                });
            }
            Err(e) => {
                log::warn!("Sayfa önbelleği açılamadı, sayfalar her seferinde indirilecek: {e:#}")
            }
        }
        self.wire.fake = test_edits();
        let queue = if self.wire.fake.is_some() {
            log::info!(
                "Sınama: yazmalar bellekte, sahte arka uca gidiyor ({:?})",
                self.wire.fake
            );
            EditQueue::open_in_memory()
        } else {
            EditQueue::open_default()
        };
        match queue {
            Ok(q) => self.wire.queue = Some(Arc::new(q)),
            Err(e) => log::warn!("Yazma kuyruğu açılamadı: {e:#}"),
        }
        self.emit_edit_status();
    }

    /// Bağlandı (ya da yeniden bağlandı): bekleyen yazmaları gönder.
    pub(super) fn wire_connected(&mut self) {
        self.process_edits();
    }

    /// Düzenli yoklama: zamanı gelen yeniden deneme varsa kuyruğu işle.
    pub(super) fn wire_tick(&mut self) {
        if !self.wire.processing && self.wire.next_due.is_some_and(|t| t <= now_ms()) {
            self.process_edits();
        }
    }

    pub(super) fn on_library_command(&mut self, command: LibraryCommand) {
        match command {
            LibraryCommand::OpenArtist(uri) | LibraryCommand::OpenAlbum(uri) => {
                self.open_page(uri, false)
            }
            LibraryCommand::Prefetch(uri) => self.open_page(uri, true),
            LibraryCommand::ResolveLinks(uris) => self.resolve_links(uris),
            LibraryCommand::SearchAll { seq, query } => self.search_all(seq, query),
            LibraryCommand::OpenRelated {
                uri,
                name,
                top_track,
            } => self.open_related(RelatedHint {
                uri,
                name,
                top_track,
            }),
            LibraryCommand::ResolveArtist { from, name } => self.resolve_artist(from, name),
            LibraryCommand::OpenExplore(key) => self.open_explore(key),
            LibraryCommand::OpenFriends => self.open_friends(),
            LibraryCommand::OpenCredits(uri) => self.open_credits(uri),
            LibraryCommand::Edit { op, rows, item } => self.edit(op, rows, item),
            LibraryCommand::RetryEdit(id) => {
                if let Some(queue) = &self.wire.queue {
                    let _ = queue.retry(id);
                }
                self.emit_edit_status();
                self.process_edits();
            }
            LibraryCommand::DismissEdit(id) => self.dismiss_edit(id),
            LibraryCommand::Pin { key, pinned } => self.pin_item(key, pinned),
            LibraryCommand::AddedDates(id) => self.added_dates(id),
            LibraryCommand::RecordPlay(log) => self.tools_record_play(log),
            LibraryCommand::PlayStats(range) => self.tools_play_stats(range),
            LibraryCommand::ExportBackup(path) => self.tools_export_backup(path),
            LibraryCommand::SetHidden { songs, on } => self.set_hidden(songs, on),
        }
    }

    pub(super) fn on_wire_internal(&mut self, result: WireInternal) {
        match result {
            WireInternal::Page {
                uri,
                quiet,
                cached,
                result,
            } => {
                self.wire.loading.remove(&uri);
                let complete = match &result {
                    Ok(PageResult::Artist(l)) => l.complete,
                    Ok(PageResult::Album(l)) => l.complete,
                    Err(_) => true,
                };
                if !complete {
                    log::debug!(
                        "{uri}: sayfa eksik geldi (bazı şarkı bilgileri yok), önbelleğe yazılmadı"
                    );
                }
                match result {
                    Ok(PageResult::Artist(l)) if l.changed || !cached => {
                        self.emit(Event::Library(LibraryEvent::Artist { uri, page: l.page }))
                    }
                    Ok(PageResult::Album(l)) if l.changed || !cached => {
                        self.emit(Event::Library(LibraryEvent::Album { uri, page: l.page }))
                    }
                    Ok(_) => {}
                    Err(error) => {
                        log::warn!("Sayfa alınamadı ({uri}): {error}");
                        if !quiet && !cached {
                            self.emit(Event::Library(LibraryEvent::PageFailed { uri, error }));
                        }
                    }
                }
            }
            WireInternal::Stats { uri, result } => match result {
                Ok(StatsResult::Artist(stats)) => {
                    self.emit(Event::Library(LibraryEvent::ArtistStats { uri, stats }))
                }
                Ok(StatsResult::Album(stats)) => {
                    self.emit(Event::Library(LibraryEvent::AlbumPlaycounts { uri, stats }))
                }
                // Ekranda söylenmez: sayılar yoksa sayfa bugünkü gibi kalır.
                Err(e) => log::info!("dinlenme sayıları alınamadı ({uri}): {e:?}"),
            },
            WireInternal::Links { asked, result } => match result {
                Ok(links) => {
                    if !links.is_empty() {
                        self.emit(Event::Library(LibraryEvent::Links(links)));
                    }
                }
                Err(e) => {
                    log::warn!("Şarkı bağları alınamadı: {e}");
                    // Sonra yeniden istenebilsin.
                    for uri in asked {
                        self.wire.links_asked.remove(&uri);
                    }
                }
            },
            WireInternal::Related { uri, result } => {
                self.wire.related_loading.remove(&uri);
                match result {
                    Ok(list) => self.emit(Event::Library(LibraryEvent::Related { uri, list })),
                    // Ekranda söylenmez: bayat liste varsa o kalır, yoksa bölüm yok.
                    Err(e) => log::info!("benzer sanatçılar alınamadı ({uri}): {e}"),
                }
            }
            WireInternal::Explore { key, result } => {
                self.wire.explore_loading.remove(&key);
                match result {
                    Ok(data) => self.emit(Event::Library(LibraryEvent::Explore { key, data })),
                    Err(error) => {
                        log::info!("keşfet verisi alınamadı ({key:?}): {error}");
                        self.emit(Event::Library(LibraryEvent::ExploreFailed { key, error }));
                    }
                }
            }
            WireInternal::Friends(result) => {
                self.wire.friends_loading = false;
                match result {
                    Ok(list) => self.emit(Event::Library(LibraryEvent::Friends(Arc::new(list)))),
                    Err(why) => self.emit(Event::Library(LibraryEvent::FriendsFailed(why))),
                }
            }
            WireInternal::Credits { uri, result } => {
                self.emit(Event::Library(LibraryEvent::Credits { uri, result }))
            }
            WireInternal::Resolved { from, result } => {
                self.wire.resolving.remove(&from);
                let uri = result
                    .map_err(|e| log::info!("Deezer sanatçısı Spotify'da bulunamadı ({from}): {e}"))
                    .ok();
                self.emit(Event::Library(LibraryEvent::ArtistResolved { from, uri }));
            }
            WireInternal::Processed(report) => {
                self.wire.processing = false;
                match report {
                    Ok(report) => self.on_processed(report),
                    Err(e) => log::warn!("Yazma kuyruğu işlenemedi: {e:#}"),
                }
                self.emit_edit_status();
                if std::mem::take(&mut self.wire.again) {
                    self.process_edits();
                }
            }
        }
    }

    // ── Sayfalar ────────────────────────────────────────────────────────

    fn open_page(&mut self, uri: String, quiet: bool) {
        let artist = uri.starts_with("spotify:artist:");
        if !artist && !uri.starts_with("spotify:album:") {
            return;
        }
        // Dinlenme sayıları sayfadan ayrı gelir: önbellekteyse bu karede
        // gösterilir, değilse tek bir ek istek arkada yapılır. Önceden almada
        // (üzerine gelme) istenmez.
        if !quiet {
            self.ensure_stats(&uri);
        }
        let cache = page_cache();
        let mut cached = false;
        if !quiet {
            let hit = if artist {
                cached_artist(&uri).map(|(page, stale)| {
                    (
                        stale,
                        LibraryEvent::Artist {
                            uri: uri.clone(),
                            page,
                        },
                    )
                })
            } else {
                cached_album(&uri).map(|(page, stale)| {
                    (
                        stale,
                        LibraryEvent::Album {
                            uri: uri.clone(),
                            page,
                        },
                    )
                })
            };
            if let Some((stale, event)) = hit {
                self.emit(Event::Library(event));
                if !stale {
                    return;
                }
                cached = true;
            }
        } else {
            let fresh = if artist {
                cached_artist(&uri).map(|(_, stale)| !stale)
            } else {
                cached_album(&uri).map(|(_, stale)| !stale)
            };
            match fresh {
                Some(true) => return,
                Some(false) => cached = true,
                None => {}
            }
        }
        if self.wire.loading.contains(&uri) {
            return;
        }
        let (Some(conn), Some(db), Some(cache)) = (&self.host.conn, &self.db, cache) else {
            if !quiet && !cached {
                self.emit(Event::Library(LibraryEvent::PageFailed {
                    uri,
                    error: "network: bağlanılamadı".into(),
                }));
            }
            return;
        };
        self.wire.loading.insert(uri.clone());
        let session = conn.session.clone();
        let db = db.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = if artist {
                pages::load_artist(&session, &db, &cache, &uri)
                    .await
                    .map(PageResult::Artist)
            } else {
                pages::load_album(&session, &db, &cache, &uri)
                    .await
                    .map(PageResult::Album)
            };
            let result = result.map_err(|e| format!("{e:#}"));
            let _ = internal.send(Internal::Wire(WireInternal::Page {
                uri,
                quiet,
                cached,
                result,
            }));
        });
    }

    /// Dinlenme sayıları: önce önbellek (anında, bu karede), sonra oturumda
    /// bir kez tek istek. Hata ekranda söylenmez; sayılar yoksa sayfa bugünkü
    /// gibi çalışır.
    fn ensure_stats(&mut self, uri: &str) {
        let artist = uri.starts_with("spotify:artist:");
        let Some(cache) = page_cache() else { return };
        let hook = stats::fixture_active();
        if hook {
            // Sınama: ağa ve oturuma hiç gidilmez; fikstür bu karede dolar.
            if !self.wire.stats_asked.insert(uri.to_string()) {
                return;
            }
            if artist {
                if let Some(stats) = stats::fixture_artist() {
                    self.emit(Event::Library(LibraryEvent::ArtistStats {
                        uri: uri.to_string(),
                        stats,
                    }));
                }
            } else if let Some(stats) = stats::fixture_album() {
                self.emit(Event::Library(LibraryEvent::AlbumPlaycounts {
                    uri: uri.to_string(),
                    stats,
                }));
            }
            return;
        }
        if artist {
            if let Some(hit) = stats::cached_artist(&cache, uri) {
                self.emit(Event::Library(LibraryEvent::ArtistStats {
                    uri: uri.to_string(),
                    stats: hit.page.clone(),
                }));
                if !hit.stale {
                    return;
                }
            }
        } else if let Some(hit) = stats::cached_album(&cache, uri) {
            self.emit(Event::Library(LibraryEvent::AlbumPlaycounts {
                uri: uri.to_string(),
                stats: hit.page.clone(),
            }));
            if !hit.stale {
                return;
            }
        }
        if !self.wire.stats_asked.insert(uri.to_string()) {
            return;
        }
        let (Some(conn), db) = (&self.host.conn, self.db.clone()) else {
            self.wire.stats_asked.remove(uri);
            return;
        };
        let session = conn.session.clone();
        let internal = self.internal.clone();
        let uri = uri.to_string();
        self.bg.spawn(async move {
            let result = if artist {
                stats::load_artist(&session, db.as_deref(), &cache, &uri)
                    .await
                    .map(StatsResult::Artist)
            } else {
                stats::load_album(&session, db.as_deref(), &cache, &uri)
                    .await
                    .map(StatsResult::Album)
            };
            let _ = internal.send(Internal::Wire(WireInternal::Stats { uri, result }));
        });
    }

    fn resolve_links(&mut self, uris: Vec<String>) {
        let Some(cache) = page_cache() else { return };
        let want: Vec<String> = uris
            .into_iter()
            .filter(|u| u.starts_with("spotify:track:") && !self.wire.links_asked.contains(u))
            .collect();
        if want.is_empty() {
            return;
        }
        // Bir kez sorulan bir daha sorulmaz (bulunamayan da: yerel dosya vb.).
        self.wire.links_asked.extend(want.iter().cloned());
        let found = cache.links(&want);
        let have: HashSet<&str> = found.iter().map(|l| l.uri.as_str()).collect();
        let missing: Vec<String> = want
            .iter()
            .filter(|u| !have.contains(u.as_str()))
            .cloned()
            .collect();
        if !found.is_empty() {
            self.emit(Event::Library(LibraryEvent::Links(found)));
        }
        if missing.is_empty() {
            return;
        }
        let Some(conn) = &self.host.conn else {
            // Bağlanınca sorulsun (açılışta görünen satırlar bağlantıdan önce
            // çiziliyor).
            for uri in &missing {
                self.wire.links_asked.remove(uri);
            }
            self.pending
                .push(super::Command::Library(LibraryCommand::ResolveLinks(
                    missing,
                )));
            return;
        };
        let session = conn.session.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = pages::load_links(&session, &cache, &missing)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = internal.send(Internal::Wire(WireInternal::Links {
                asked: missing,
                result,
            }));
        });
    }

    /// Açılan sanatçının benzerleri: önbellekteki (taze ya da bayat) hemen
    /// gider; taze değilse arkadan istenir (sanatçı başına aynı anda tek istek).
    fn open_related(&mut self, hint: RelatedHint) {
        if !hint.uri.starts_with("spotify:artist:") {
            return;
        }
        let Some(cache) = page_cache() else { return };
        if let Some((list, fresh)) = cached_related(&hint.uri) {
            self.emit(Event::Library(LibraryEvent::Related {
                uri: hint.uri.clone(),
                list,
            }));
            if fresh {
                return;
            }
        }
        if self.wire.related_loading.contains(&hint.uri) {
            return;
        }
        let Some(conn) = &self.host.conn else {
            // Bağlanınca sorulsun.
            self.pending
                .push(super::Command::Library(LibraryCommand::OpenRelated {
                    uri: hint.uri,
                    name: hint.name,
                    top_track: hint.top_track,
                }));
            return;
        };
        let session = conn.session.clone();
        let db = self.db.clone();
        let internal = self.internal.clone();
        self.wire.related_loading.insert(hint.uri.clone());
        self.bg.spawn(async move {
            let result = related::load(&session, db.as_deref(), &cache, &hint)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = internal.send(Internal::Wire(WireInternal::Related {
                uri: hint.uri,
                result,
            }));
        });
    }

    /// Keşfet / Ana sayfa verisi: önbellekteki (taze ya da bayat) hemen gider;
    /// taze değilse arkadan istenir (anahtar başına aynı anda tek istek).
    /// Bağlantı yoksa bağlanınca sorulur.
    fn open_explore(&mut self, key: ExploreKey) {
        let Some(cache) = page_cache() else {
            self.emit(Event::Library(LibraryEvent::ExploreFailed {
                key,
                error: "önbellek yok".into(),
            }));
            return;
        };
        if let Some((data, fresh)) = cached_explore(&key) {
            self.emit(Event::Library(LibraryEvent::Explore {
                key: key.clone(),
                data,
            }));
            if fresh {
                return;
            }
        }
        if self.wire.explore_loading.contains(&key) {
            return;
        }
        let Some(conn) = &self.host.conn else {
            self.pending
                .push(super::Command::Library(LibraryCommand::OpenExplore(key)));
            return;
        };
        let session = conn.session.clone();
        let db = self.db.clone();
        let internal = self.internal.clone();
        self.wire.explore_loading.insert(key.clone());
        self.bg.spawn(async move {
            let result = explore::load(&session, db.as_deref(), &cache, &key)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = internal.send(Internal::Wire(WireInternal::Explore { key, result }));
        });
    }

    /// Sabitleme: yerel tabloya yazılır (`library_pins`), motorun kütüphane
    /// kopyası aynı anda güncellenir (sonraki tazeleme / yayın sabitlemeyi
    /// kaybetmesin). Arayüz iyimser günceller: olay göndermeye gerek yok.
    fn pin_item(&mut self, key: String, pinned: bool) {
        let Some(db) = &self.db else { return };
        let written = db
            .next_pin_time(now_ms())
            .and_then(|at| db.set_pinned(&key, pinned, at));
        if let Err(e) = written {
            log::warn!("Sabitleme yazılamadı ({key}): {e:#}");
            return;
        }
        if let Some(library) = self.library.as_mut() {
            library.meta.pinned.retain(|k| *k != key);
            if pinned {
                library.meta.pinned.push(key);
            }
        }
    }

    /// Listenin şarkılarının eklenme anları, önbellekten (ağa çıkılmaz).
    fn added_dates(&mut self, id: CollectionId) {
        let Some(db) = &self.db else { return };
        let entries = match db.collection_entries(&id) {
            Ok(Some(entries)) => entries,
            Ok(None) => return,
            Err(e) => {
                log::warn!("Eklenme tarihleri okunamadı ({}): {e:#}", id.key());
                return;
            }
        };
        let dates = added_dates_of(&entries);
        self.emit(Event::Library(LibraryEvent::AddedDates {
            id,
            dates: Arc::new(dates),
        }));
    }

    /// Arkadaş etkinliği: bağlıysa arkadan alınır (aynı anda tek istek), değilse
    /// `Offline`. Önbellek yok: sonuç yalnız arayüzün belleğinde durur.
    fn open_friends(&mut self) {
        if self.wire.friends_loading {
            return;
        }
        let Some(conn) = &self.host.conn else {
            self.emit(Event::Library(LibraryEvent::FriendsFailed(
                FriendsFailure::Offline,
            )));
            return;
        };
        let session = conn.session.clone();
        let internal = self.internal.clone();
        self.wire.friends_loading = true;
        self.bg.spawn(async move {
            let result = friends::load(&session).await;
            let _ = internal.send(Internal::Wire(WireInternal::Friends(result)));
        });
    }

    /// Şarkı künyesi (salt okuma): bağlıysa arkadan alınır, değilse `Offline`.
    /// Kullanıcı düğmeye bastığında tek istek; sonuç arayüzün belleğinde durur.
    fn open_credits(&mut self, uri: String) {
        let Some(conn) = &self.host.conn else {
            self.emit(Event::Library(LibraryEvent::Credits {
                uri,
                result: Err(CreditsFailure::Offline),
            }));
            return;
        };
        let session = conn.session.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            let result = credits::load(&session, &uri).await;
            let _ = internal.send(Internal::Wire(WireInternal::Credits { uri, result }));
        });
    }

    /// Deezer kartının Spotify adresi: önbellekte yoksa bir aramayla.
    fn resolve_artist(&mut self, from: String, name: String) {
        if let Some(uri) = resolved_artist(&from) {
            self.emit(Event::Library(LibraryEvent::ArtistResolved {
                from,
                uri: Some(uri),
            }));
            return;
        }
        let (Some(conn), Some(cache)) = (&self.host.conn, page_cache()) else {
            self.emit(Event::Library(LibraryEvent::ArtistResolved {
                from,
                uri: None,
            }));
            return;
        };
        if self.wire.resolving.contains(&from) {
            return;
        }
        let session = conn.session.clone();
        let db = self.db.clone();
        let internal = self.internal.clone();
        self.wire.resolving.insert(from.clone());
        self.bg.spawn(async move {
            let result =
                related::resolve_deezer(&session, &cache, db.as_deref(), &from, &name).await;
            let result = result.map_err(|e| format!("{e:#}"));
            let _ = internal.send(Internal::Wire(WireInternal::Resolved { from, result }));
        });
    }

    /// Tam arama sayfası: paletteki aramanın daha çok sonuçlu hâli.
    fn search_all(&self, seq: u64, query: String) {
        let Some(conn) = &self.host.conn else {
            self.emit(Event::SearchFailed {
                seq,
                error: "network: Spotify'a henüz bağlanılmadı".into(),
            });
            return;
        };
        let session = conn.session.clone();
        let db = self.db.clone();
        let internal = self.internal.clone();
        self.bg.spawn(async move {
            // Hash'i (kayıtlı, eskiyse yenisi) ve kapıyı pathfinder istemcisi yönetir.
            let result = search::run(&session, db.as_deref(), &query, SEARCH_ALL_LIMIT).await;
            let _ = internal.send(Internal::Search {
                seq,
                result: result.map_err(anyhow::Error::from),
            });
        });
    }

    // ── Yazmalar ────────────────────────────────────────────────────────

    fn edit(&mut self, op: EditOp, rows: Vec<TrackRow>, item: Option<LibraryItem>) {
        let Some(queue) = self.wire.queue.clone() else {
            self.notice(NoticeKind::Problem, t!(EditQueueFailed));
            return;
        };
        match queue.enqueue(&op) {
            Ok(Enqueued::Queued(id)) => {
                self.wire.rows.insert(
                    id,
                    EditRows {
                        rows: rows.clone(),
                        item: item.clone(),
                    },
                );
            }
            Ok(Enqueued::Cancelled | Enqueued::Duplicate(_)) => {}
            Err(e) => {
                log::warn!("Yazma kuyruğa alınamadı: {e}");
                self.notice(NoticeKind::Problem, t!(EditSaveFailed));
                return;
            }
        }
        self.patch_local(&op, &rows, item.as_ref());
        self.emit_edit_status();
        self.process_edits();
    }

    /// Motorun ekrandaki kopyasını (bellek) işleme göre günceller: arayüzle
    /// aynı fonksiyonlar, aynı sonuç. Disk önbelleği DEĞİŞMEZ: o, Spotify'daki
    /// son bilinen hâldir; işlem Spotify'a ulaşınca `persist_done` onu
    /// yetiştirir. Spotify'dan yeni veri gelince bekleyen işlemler ona
    /// yeniden uygulanır (`view_tracks`, `view_library`), böylece liste bir an
    /// eski hâline dönmez; işlem bırakılırsa ekran bu önbelleğe döner.
    fn patch_local(&mut self, op: &EditOp, rows: &[TrackRow], item: Option<&LibraryItem>) {
        if let Some(id) = touched_collection(op) {
            // Bellekte yoksa dokunulmaz: açılınca önbellekten ve bekleyen
            // işlemlerden kurulur (yeni oluşturulan liste hariç).
            let current = match self.collections.get(&id) {
                Some(t) => Some((**t).clone()),
                None if matches!(op, EditOp::CreatePlaylist { .. }) => Some(Vec::new()),
                None => None,
            };
            if let Some(mut list) = current
                && patch_tracks(op, rows, &mut list)
            {
                self.collections.insert(id, Arc::new(list));
            }
        }
        if let Some(lib) = self.library.as_mut() {
            patch_library(op, item, lib, &mut self.wire.deleted);
        }
    }

    fn process_edits(&mut self) {
        let Some(queue) = self.wire.queue.clone() else {
            return;
        };
        if self.wire.processing {
            self.wire.again = true;
            return;
        }
        let internal = self.internal.clone();
        let now = now_ms();
        let handle = self.bg.clone();
        let stamp = self.wire.last_write_ms.clone();
        match self.wire.fake {
            Some(mode) => {
                self.bg.spawn_blocking(move || {
                    let backend = Stamped {
                        inner: FakeBackend(mode),
                        stamp: &stamp,
                    };
                    let report = handle.block_on(queue.process_due(&backend, now));
                    let _ = internal.send(Internal::Wire(WireInternal::Processed(report)));
                });
            }
            None => {
                let Some(conn) = &self.host.conn else { return };
                let session = conn.session.clone();
                self.bg.spawn_blocking(move || {
                    let backend = Stamped {
                        inner: SpotifyBackend { session: &session },
                        stamp: &stamp,
                    };
                    let report = handle.block_on(queue.process_due(&backend, now));
                    let _ = internal.send(Internal::Wire(WireInternal::Processed(report)));
                });
            }
        }
        self.wire.processing = true;
    }

    /// Kütüphane tazelemesi başladı (bkz. `refresh_is_stale`).
    pub(super) fn mark_refresh_started(&self) {
        self.wire
            .refresh_started_ms
            .store(now_ms(), Ordering::Relaxed);
    }

    /// Süren tazeleme, o başladıktan sonra Spotify'a bir yazma ulaşmadan ÖNCE
    /// okunmuş olabilir mi? Öyleyse sonucu (örn. beğenmediğin şarkı hâlâ
    /// beğenili) eskidir ve ekrana verilmemeli; yenisi istenir. Bekleyen
    /// (henüz yazılmamış) işlemler bununla ilgili değil: onlar yeniden uygulanır.
    pub(super) fn refresh_is_stale(&self) -> bool {
        let started = self.wire.refresh_started_ms.load(Ordering::Relaxed);
        started > 0 && self.wire.last_write_ms.load(Ordering::Relaxed) >= started
    }

    fn on_processed(&mut self, report: ProcessReport) {
        self.wire.next_due = report.next_due_ms;
        let mut refresh = false;
        for (k, (id, outcome)) in report.done.iter().enumerate() {
            let held = self.wire.rows.remove(id).unwrap_or_default();
            let Some(op) = report.done_ops.get(k) else {
                continue;
            };
            match (op, outcome) {
                (EditOp::CreatePlaylist { .. }, EditOutcome::Created { uri }) => {
                    // Ekranın tanımadığı bir oluşturma (önceki oturumdan kalan):
                    // kütüphane Spotify'dan tazelensin.
                    refresh |= !self.created(op, uri.clone(), &held);
                }
                (
                    EditOp::CreateFolder { .. }
                    | EditOp::RestoreFolder { .. }
                    | EditOp::RenameFolder { .. }
                    | EditOp::DeleteFolder { .. }
                    | EditOp::MovePlaylistToFolder { .. },
                    _,
                ) => {
                    self.persist_done(op, &held);
                    // Kök listenin sırası (klasörlerin `at`ı, iç içe yerleşim)
                    // yazmadan sonra kesin hâliyle okunur: iyimser kopya
                    // yaklaşıktır, bu tek istek onu düzeltir.
                    refresh = true;
                }
                _ => self.persist_done(op, &held),
            }
        }
        if refresh {
            self.refresh_library();
        }
        // Bitmiş / bırakılmış işlemlerin şarkı bilgileri artık gerekmiyor.
        if let Some(queue) = &self.wire.queue {
            let alive: HashSet<i64> = queue
                .pending()
                .unwrap_or_default()
                .into_iter()
                .chain(queue.failed().unwrap_or_default())
                .map(|e| e.id)
                .collect();
            self.wire.rows.retain(|id, _| alive.contains(id));
        }
        if !report.failed.is_empty() {
            self.notice(NoticeKind::Problem, edit_failed_notice());
        }
    }

    /// Spotify'a yazılan işlemi önbelleğe (Spotify'daki son bilinen hâle)
    /// işler. Sınamada hiçbir şey yazılmaz: önbellek Spotify'dakinden
    /// ayrışmasın.
    fn persist_done(&mut self, op: &EditOp, held: &EditRows) {
        if self.wire.fake.is_some() {
            return;
        }
        let Some(db) = self.db.clone() else { return };
        let rows = if held.rows.is_empty() {
            rows_from_cache(&db, op)
        } else {
            held.rows.clone()
        };
        if let Err(e) = persist_op(
            &db,
            op,
            &rows,
            held.item.clone().or_else(|| follow_card(op)).as_ref(),
            &mut self.wire.deleted_truth,
        ) {
            log::warn!(
                "Yazılan değişiklik önbelleğe işlenemedi (sonraki tazelemede düzelir): {e:#}"
            );
        }
    }

    /// Geçici adresli liste oluştu: motorun kopyasında gerçek adrese çevir.
    /// Ekran bu listeyi tanıyorsa (aynı oturumda oluşturulmuş) `true`.
    fn created(&mut self, op: &EditOp, uri: String, held: &EditRows) -> bool {
        let EditOp::CreatePlaylist {
            temp_id,
            name,
            uris,
            ..
        } = op
        else {
            return false;
        };
        let temp = temp_id.clone();
        let temp_id = CollectionId::Playlist(temp.clone());
        let real_id = CollectionId::Playlist(uri.clone());
        if let Some(tracks) = self.collections.remove(&temp_id) {
            self.collections.insert(real_id.clone(), tracks);
        }
        if self.opened.remove(&temp_id) {
            self.opened.insert(real_id.clone());
        }
        let mut known = false;
        if let Some(lib) = self.library.as_mut() {
            for item in lib.playlists.iter_mut().filter(|p| p.id == temp_id) {
                item.id = real_id.clone();
                known = true;
            }
        }
        // Önbellek: Spotify'da oluşan liste (adı ve ilk şarkılarıyla). Sınamada
        // (sahte arka uç) diske yazılmaz.
        if self.wire.fake.is_none()
            && let Some(db) = &self.db
            && let Err(e) = persist_created(db, &real_id, name, uris, &held.rows)
        {
            log::warn!("Oluşan liste önbelleğe işlenemedi (sonraki tazelemede düzelir): {e:#}");
        }
        self.emit(Event::Library(LibraryEvent::Created { temp, uri }));
        known
    }

    /// Yazılamayan işlemi bırak: işlem kuyruktan ve ekrandan gider, ekran
    /// Spotify'daki son bilinen hâle (artı hâlâ bekleyen başka işlemlere) döner.
    fn dismiss_edit(&mut self, id: i64) {
        let Some(queue) = self.wire.queue.clone() else {
            return;
        };
        let removed = queue.dismiss_cascade(id).unwrap_or_else(|e| {
            log::warn!("Değişiklik bırakılamadı: {e:#}");
            Vec::new()
        });
        for e in &removed {
            self.wire.rows.remove(&e.id);
        }
        self.emit_edit_status();
        if removed.is_empty() {
            return;
        }
        let ops: Vec<EditOp> = removed.into_iter().map(|e| e.op).collect();
        self.rebuild_views(&ops);
    }

    /// Bırakılan işlemlerin değiştirdiği liste ve kütüphane yeniden kurulur:
    /// önbellekteki hâl + kalan bekleyen işlemler, sonra gösterilir. Gerçek
    /// Spotify'da ayrıca bir tazeleme de başlar (işlem Spotify'a ulaşmış
    /// olabilirdi).
    fn rebuild_views(&mut self, ops: &[EditOp]) {
        let mut ids: Vec<CollectionId> = Vec::new();
        for id in ops.iter().filter_map(touched_collection) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        for id in ids {
            self.collections.remove(&id);
            self.complete.remove(&id);
            if self.opened.contains(&id) && !edits::is_temp_id(id.key()) {
                // Önbellekten (ve bekleyen işlemlerle) yeniden gösterir.
                self.open_collection(id);
            }
        }
        if ops.iter().any(affects_library) {
            let truth = self
                .db
                .as_ref()
                .and_then(|db| db.load_library().ok().flatten());
            if let Some(truth) = truth {
                let view = self.view_library(truth);
                if self.library.as_ref() != Some(&view) {
                    self.library = Some(view.clone());
                    self.emit(Event::LibraryLoaded(view));
                }
            }
        }
        if self.wire.fake.is_none() {
            self.refresh_library();
        }
    }

    // ── Spotify'dan yenilenen veriye bekleyen işlemleri yeniden uygula ──

    /// Kuyrukta bekleyen ve yazılamayan işlemler (kayıt sırasıyla), `keep`
    /// ile süzülmüş. Bunlar ekranda iyimser uygulanmış ama Spotify'da henüz
    /// yok: Spotify'dan yeni okunan veride görünmezler.
    fn pending_edits(&self, keep: impl Fn(&EditOp) -> bool) -> Vec<PendingEdit> {
        let Some(queue) = &self.wire.queue else {
            return Vec::new();
        };
        let mut all = queue.pending().unwrap_or_default();
        all.extend(queue.failed().unwrap_or_default());
        all.retain(|e| keep(&e.op));
        all.sort_by_key(|e| e.id);
        all.into_iter()
            .map(|e| {
                let held = self.wire.rows.get(&e.id);
                // Önceki oturumdan kalan işlemin şarkı bilgileri bellekte yok:
                // önbellekten.
                let rows = match held {
                    Some(h) if !h.rows.is_empty() => h.rows.clone(),
                    _ => self
                        .db
                        .as_ref()
                        .map(|db| rows_from_cache(db, &e.op))
                        .unwrap_or_default(),
                };
                PendingEdit {
                    rows,
                    item: held
                        .and_then(|h| h.item.clone())
                        .or_else(|| follow_card(&e.op)),
                    unsure: e.in_flight || e.maybe_applied,
                    op: e.op,
                }
            })
            .collect()
    }

    /// Spotify'dan (önbellekten) okunan liste + bekleyen işlemler = gösterilecek
    /// liste.
    pub(super) fn view_tracks(
        &self,
        id: &CollectionId,
        truth: Arc<Vec<TrackRow>>,
    ) -> Arc<Vec<TrackRow>> {
        let edits = self.pending_edits(|op| touched_collection(op).as_ref() == Some(id));
        if edits.is_empty() {
            return truth;
        }
        let mut list = (*truth).clone();
        if replay_tracks(id, &edits, &mut list) {
            Arc::new(list)
        } else {
            truth
        }
    }

    /// Aynısı kütüphane için (liste adları / sayıları, albümler, beğenilen sayısı).
    pub(super) fn view_library(&self, truth: Library) -> Library {
        let mut lib = self.replayed_library(truth);
        // Kapağı olmayan listelere önbellekteki şarkılarından kapak / mozaik.
        if let Some(db) = &self.db {
            super::library::with_mosaics(db, &mut lib);
            // Sabitlemeler yerel ve her an değişebilir: tazelemenin okuduğu
            // (daha eski) liste yerine tablonun şimdiki hâli.
            if let Ok(pinned) = db.pinned_keys() {
                lib.meta.pinned = pinned;
            }
        }
        lib
    }

    fn replayed_library(&self, truth: Library) -> Library {
        let edits = self.pending_edits(affects_library);
        if edits.is_empty() {
            return truth;
        }
        let liked: Option<HashSet<String>> = if edits.iter().any(|e| is_track_like(&e.op)) {
            self.db
                .as_ref()
                .and_then(|db| db.collection_uris(&CollectionId::Liked).ok().flatten())
                .map(|u| u.into_iter().collect())
        } else {
            None
        };
        let mut lib = truth;
        let mut deleted = self.wire.deleted_truth.clone();
        replay_library(&edits, &mut lib, &mut deleted, liked.as_ref());
        lib
    }

    fn emit_edit_status(&self) {
        let Some(queue) = &self.wire.queue else {
            return;
        };
        let pending = queue.pending().map(|p| p.len()).unwrap_or(0);
        let failed = queue
            .failed()
            .unwrap_or_default()
            .into_iter()
            .map(|e| FailedEdit {
                id: e.id,
                op: e.op,
                error: e.last_error.unwrap_or_default(),
                kind: e.error_kind.unwrap_or(FailKind::Unknown),
            })
            .collect();
        self.emit(Event::Library(LibraryEvent::EditStatus { pending, failed }));
    }
}

// ── Saf güncellemeler (motor ve arayüz aynısını kullanır) ───────────────

/// İşlemin şarkı listesini değiştirdiği liste (yoksa `None`).
pub fn touched_collection(op: &EditOp) -> Option<CollectionId> {
    match op {
        EditOp::Save { uri } | EditOp::Unsave { uri } if uri.starts_with("spotify:track:") => {
            Some(CollectionId::Liked)
        }
        EditOp::AddToPlaylist { playlist, .. }
        | EditOp::RemoveFromPlaylist { playlist, .. }
        | EditOp::MovePlaylistItems { playlist, .. } => {
            Some(CollectionId::Playlist(playlist.clone()))
        }
        EditOp::CreatePlaylist { temp_id, .. } => Some(CollectionId::Playlist(temp_id.clone())),
        _ => None,
    }
}

/// Şarkı listesini işleme göre günceller (`rows`: işlemdeki şarkıların
/// bilgileri). Bir şey değiştiyse `true`.
pub fn patch_tracks(op: &EditOp, rows: &[TrackRow], list: &mut Vec<TrackRow>) -> bool {
    let row_of = |uri: &str| rows.iter().find(|r| r.uri == uri).cloned();
    match op {
        // Beğenilenler en yeni önde.
        EditOp::Save { uri } => {
            if list.iter().any(|t| &t.uri == uri) {
                return false;
            }
            let Some(row) = row_of(uri) else { return false };
            list.insert(0, row);
            true
        }
        EditOp::Unsave { uri } => {
            let before = list.len();
            list.retain(|t| &t.uri != uri);
            list.len() != before
        }
        EditOp::AddToPlaylist { uris, position, .. } => {
            let new: Vec<TrackRow> = uris.iter().filter_map(|u| row_of(u)).collect();
            if new.is_empty() {
                return false;
            }
            let at = match position {
                AddPosition::First => 0,
                AddPosition::Last => list.len(),
                AddPosition::At(i) => (*i as usize).min(list.len()),
            };
            for (k, row) in new.into_iter().enumerate() {
                list.insert(at + k, row);
            }
            true
        }
        EditOp::RemoveFromPlaylist { uris, indices, .. } => {
            let state: Vec<String> = list.iter().map(|t| t.uri.clone()).collect();
            let gone = edits::resolve_rows(&state, uris, indices, false);
            for (ix, _) in &gone {
                list.remove(*ix as usize);
            }
            !gone.is_empty()
        }
        EditOp::MovePlaylistItems {
            from_index,
            length,
            to_index,
            ..
        } => edits::apply_move(
            list,
            *from_index as usize,
            *length as usize,
            *to_index as usize,
        ),
        EditOp::CreatePlaylist { uris, .. } => {
            *list = uris.iter().filter_map(|u| row_of(u)).collect();
            true
        }
        _ => false,
    }
}

/// "12 şarkı" → 12 (kendi listelerinin açıklaması).
fn count_of(subtitle: &str) -> Option<usize> {
    crate::i18n::canon::plain_song_count(subtitle)
}

/// Kütüphaneyi (raftaki listeler, albümler, beğenilen sayısı) işleme göre
/// günceller. `item`: kaydedilen albümün kartı; `deleted`: silinen listelerin
/// yerleri (geri getirmek için). Bir şey değiştiyse `true`.
pub fn patch_library(
    op: &EditOp,
    item: Option<&LibraryItem>,
    lib: &mut Library,
    deleted: &mut HashMap<String, (usize, LibraryItem)>,
) -> bool {
    let playlist_pos = |lib: &Library, uri: &str| {
        lib.playlists
            .iter()
            .position(|p| matches!(&p.id, CollectionId::Playlist(u) if u == uri))
    };
    let recount = |lib: &mut Library, uri: &str, delta: isize| {
        if let Some(p) = playlist_pos(lib, uri)
            && let Some(n) = count_of(&lib.playlists[p].subtitle)
        {
            lib.playlists[p].subtitle =
                crate::i18n::canon::songs((n as isize + delta).max(0) as usize);
            return true;
        }
        false
    };
    match op {
        EditOp::Save { uri } if uri.starts_with("spotify:track:") => {
            lib.liked_count += 1;
            true
        }
        EditOp::Unsave { uri } if uri.starts_with("spotify:track:") => {
            lib.liked_count = lib.liked_count.saturating_sub(1);
            true
        }
        EditOp::Save { uri } => {
            let id = CollectionId::Album(uri.clone());
            if lib.albums.iter().any(|a| a.id == id) {
                return false;
            }
            let Some(item) = item else { return false };
            lib.albums.insert(0, LibraryItem { id, ..item.clone() });
            true
        }
        EditOp::Unsave { uri } => {
            let before = lib.albums.len();
            lib.albums.retain(|a| a.id.key() != uri);
            lib.albums.len() != before
        }
        // Takip: kart bekleyen işlemle gelir (ad ve portre); en son takip edilen önde.
        EditOp::Follow { uri } => {
            let id = CollectionId::Artist(uri.clone());
            if lib.artists.iter().any(|a| a.id == id) {
                return false;
            }
            let Some(item) = item else { return false };
            lib.artists.insert(0, LibraryItem { id, ..item.clone() });
            true
        }
        EditOp::Unfollow { uri } => {
            let before = lib.artists.len();
            lib.artists.retain(|a| a.id.key() != uri);
            lib.artists.len() != before
        }
        EditOp::AddToPlaylist { playlist, uris, .. } => recount(lib, playlist, uris.len() as isize),
        EditOp::RemoveFromPlaylist { playlist, uris, .. } => {
            recount(lib, playlist, -(uris.len() as isize))
        }
        EditOp::CreatePlaylist {
            temp_id,
            name,
            uris,
            ..
        } => {
            lib.playlists.insert(
                0,
                LibraryItem {
                    id: CollectionId::Playlist(temp_id.clone()),
                    name: name.clone(),
                    subtitle: crate::i18n::canon::songs(uris.len()),
                    image: String::new(),
                },
            );
            true
        }
        EditOp::RenamePlaylist { playlist, name, .. } => match playlist_pos(lib, playlist) {
            Some(p) => {
                lib.playlists[p].name = name.clone();
                true
            }
            None => false,
        },
        EditOp::DeletePlaylist { playlist } => match playlist_pos(lib, playlist) {
            Some(p) => {
                let item = lib.playlists.remove(p);
                deleted.insert(playlist.clone(), (p, item));
                true
            }
            None => false,
        },
        EditOp::RestorePlaylist { playlist } => {
            if playlist_pos(lib, playlist).is_some() {
                return false;
            }
            match deleted.remove(playlist) {
                Some((p, item)) => {
                    lib.playlists.insert(p.min(lib.playlists.len()), item);
                    true
                }
                None => false,
            }
        }
        EditOp::MovePlaylistItems { .. } => false,
        EditOp::SetPlaylistDescription {
            playlist,
            description,
            ..
        } => {
            let old = lib.meta.descriptions.get(playlist).cloned();
            if description.is_empty() {
                lib.meta.descriptions.remove(playlist);
            } else {
                lib.meta
                    .descriptions
                    .insert(playlist.clone(), description.clone());
            }
            old.as_deref().unwrap_or("") != description.as_str()
        }
        EditOp::SetPlaylistCollaborative {
            playlist,
            collaborative,
            ..
        } => {
            let was = lib.meta.collaborative.contains(playlist);
            if *collaborative {
                lib.meta.collaborative.insert(playlist.clone());
            } else {
                lib.meta.collaborative.remove(playlist);
            }
            was != *collaborative
        }
        EditOp::CreateFolder { key, name } => {
            if lib.meta.folders.iter().any(|f| &f.key == key) {
                return false;
            }
            lib.meta.folders.push(crate::engine::library::Folder {
                key: key.clone(),
                name: name.clone(),
                parent: None,
                at: lib.playlists.len(),
            });
            true
        }
        EditOp::RenameFolder { folder, name, .. } => {
            let Some(new_key) = edits::renamed_folder_key(folder, name) else {
                return false;
            };
            if new_key == *folder {
                return false;
            }
            let mut changed = false;
            for f in &mut lib.meta.folders {
                if f.key == *folder {
                    f.key = new_key.clone();
                    f.name = name.clone();
                    changed = true;
                }
                if f.parent.as_deref() == Some(folder.as_str()) {
                    f.parent = Some(new_key.clone());
                    changed = true;
                }
            }
            for parent in lib.meta.parents.values_mut() {
                if parent.as_str() == folder.as_str() {
                    *parent = new_key.clone();
                }
            }
            changed
        }
        EditOp::DeleteFolder { folder, .. } => {
            let Some(ix) = lib.meta.folders.iter().position(|f| &f.key == folder) else {
                return false;
            };
            let gone = lib.meta.folders.remove(ix);
            // İçindekiler bir dış klasöre (yoksa köke) çıkar.
            for f in &mut lib.meta.folders {
                if f.parent.as_deref() == Some(folder.as_str()) {
                    f.parent = gone.parent.clone();
                }
            }
            for parent in lib.meta.parents.values_mut() {
                if parent.as_str() == folder.as_str() {
                    *parent = gone.parent.clone().unwrap_or_default();
                }
            }
            lib.meta.parents.retain(|_, p| !p.is_empty());
            true
        }
        EditOp::RestoreFolder {
            folder,
            name,
            before,
            contents,
        } => {
            if lib.meta.folders.iter().any(|f| &f.key == folder) {
                return false;
            }
            let at = before
                .as_ref()
                .and_then(|b| lib.playlists.iter().position(|p| p.id.key() == b))
                .unwrap_or(lib.playlists.len());
            let parent = before
                .as_ref()
                .and_then(|b| lib.meta.parents.get(b).cloned());
            lib.meta.folders.push(crate::engine::library::Folder {
                key: folder.clone(),
                name: name.clone(),
                parent,
                at,
            });
            // Aynı `at`ta sıra bozulmasın: klasörler `at`a göre sıralı kalır.
            lib.meta.folders.sort_by_key(|f| f.at);
            for child in contents {
                lib.meta.parents.insert(child.clone(), folder.clone());
            }
            true
        }
        EditOp::MovePlaylistToFolder {
            playlist, folder, ..
        } => {
            let old = lib.meta.parents.get(playlist).cloned();
            if old == *folder {
                return false;
            }
            match folder {
                Some(folder) => {
                    lib.meta.parents.insert(playlist.clone(), folder.clone());
                }
                None => {
                    lib.meta.parents.remove(playlist);
                }
            }
            true
        }
    }
}

// ── Bekleyen işlemleri yenilenen veriye yeniden uygulama (saf) ──────────

/// Kuyrukta bekleyen ya da yazılamayan, ekranda iyimser uygulanmış ama
/// Spotify'da henüz olmayan bir işlem. Spotify'dan yeni okunan veriye
/// `replay_tracks` / `replay_library` ile yeniden uygulanır.
#[derive(Debug, Clone)]
pub struct PendingEdit {
    pub op: EditOp,
    /// İşlemdeki şarkıların bilgileri (`patch_tracks` için).
    pub rows: Vec<TrackRow>,
    /// Albüm kaydında albümün kartı.
    pub item: Option<LibraryItem>,
    /// Sonucu belirsiz: şu an gönderiliyor ya da önceki deneme sunucuya
    /// ulaşmış olabilir. Spotify'dan okunan veri bu işlemin etkisini içeriyor
    /// olabilir; sıra / ekleme gibi tekrarı zarar verecek işlemler uygulanmış
    /// mı diye bakılır (arka ucun yeniden denemede yaptığı denetimin aynısı).
    pub unsure: bool,
}

/// İşlem kütüphane görünümünü (liste adları, sayılar, albümler) etkiler mi?
pub fn affects_library(op: &EditOp) -> bool {
    !matches!(op, EditOp::MovePlaylistItems { .. })
}

/// Şarkı beğenme / vazgeçme mi (beğenilen sayısını etkiler)?
fn is_track_like(op: &EditOp) -> bool {
    matches!(op, EditOp::Save { uri } | EditOp::Unsave { uri } if uri.starts_with("spotify:track:"))
}

/// Sonucu belirsiz bir işlemi listeye uygular: zaten uygulanmışsa dokunmaz.
fn patch_tracks_unsure(op: &EditOp, rows: &[TrackRow], list: &mut Vec<TrackRow>) -> bool {
    let state: Vec<String> = list.iter().map(|t| t.uri.clone()).collect();
    match op {
        // Eklenenler istenen yerde zaten duruyorsa eklenmiş.
        EditOp::AddToPlaylist { uris, position, .. } => {
            let at = match position {
                AddPosition::First => 0,
                AddPosition::Last => state.len().saturating_sub(uris.len()),
                AddPosition::At(i) => (*i as usize).min(state.len()),
            };
            if edits::move_applied(&state, at, uris) {
                return false;
            }
            patch_tracks(op, rows, list)
        }
        // Sırası bilinen satır artık orada değilse çıkarılmış sayılır (başka
        // kopya silinmez).
        EditOp::RemoveFromPlaylist { uris, indices, .. } => {
            let gone = edits::resolve_rows(&state, uris, indices, true);
            for (ix, _) in &gone {
                list.remove(*ix as usize);
            }
            !gone.is_empty()
        }
        EditOp::MovePlaylistItems { to_index, uris, .. } => {
            if edits::move_applied(&state, *to_index as usize, uris) {
                return false;
            }
            patch_tracks(op, rows, list)
        }
        _ => patch_tracks(op, rows, list),
    }
}

/// Spotify'dan yeni okunan `id` listesine, bekleyen işlemleri kayıt sırasıyla
/// yeniden uygular (motorla arayüzün ortak `patch_tracks`'ıyla). Bir şey
/// değiştiyse `true`.
pub fn replay_tracks(id: &CollectionId, edits: &[PendingEdit], list: &mut Vec<TrackRow>) -> bool {
    let mut changed = false;
    for e in edits
        .iter()
        .filter(|e| touched_collection(&e.op).as_ref() == Some(id))
    {
        changed |= if e.unsure {
            patch_tracks_unsure(&e.op, &e.rows, list)
        } else {
            patch_tracks(&e.op, &e.rows, list)
        };
    }
    changed
}

/// Aynısı kütüphane için (`patch_library`). `liked`: Spotify'daki beğenilen
/// şarkıların adresleri (biliniyorsa): beğenilen sayısı buna göre ±1 değil,
/// gerçekten değişince ayarlanır (Spotify'da zaten beğenili şarkıyı beğenmek
/// sayıyı şişirmez). Sonucu belirsiz ekleme / çıkarmada liste şarkı sayısına
/// dokunulmaz (uygulanıp uygulanmadığı bilinmiyor; sonraki tazeleme düzeltir).
pub fn replay_library(
    edits: &[PendingEdit],
    lib: &mut Library,
    deleted: &mut HashMap<String, (usize, LibraryItem)>,
    liked: Option<&HashSet<String>>,
) -> bool {
    let mut liked = liked.cloned();
    let mut changed = false;
    for e in edits {
        match (&e.op, liked.as_mut()) {
            (EditOp::Save { uri } | EditOp::Unsave { uri }, Some(set))
                if uri.starts_with("spotify:track:") =>
            {
                let save = matches!(e.op, EditOp::Save { .. });
                let moved = if save {
                    set.insert(uri.clone())
                } else {
                    set.remove(uri)
                };
                if moved {
                    lib.liked_count = if save {
                        lib.liked_count + 1
                    } else {
                        lib.liked_count.saturating_sub(1)
                    };
                    changed = true;
                }
            }
            (EditOp::AddToPlaylist { .. } | EditOp::RemoveFromPlaylist { .. }, _) if e.unsure => {}
            _ => changed |= patch_library(&e.op, e.item.as_ref(), lib, deleted),
        }
    }
    changed
}

/// Takip işleminin sanatçı kartı, sayfa önbelleğinden (bellekte tutulmuyorsa:
/// önceki oturumdan kalan işlem). Sayfa önbellekte yoksa `None`: kart
/// sonraki tazelemede Spotify'dan gelir.
fn follow_card(op: &EditOp) -> Option<LibraryItem> {
    let EditOp::Follow { uri } = op else {
        return None;
    };
    let (page, _) = cached_artist(uri)?;
    Some(super::library::artist_item(uri, &page.name, &page.image))
}

/// İşlemdeki şarkıların bilgileri önbellekte (bellekte tutulmuyorsa: önceki
/// oturumdan kalan işlem).
fn rows_from_cache(db: &Db, op: &EditOp) -> Vec<TrackRow> {
    let uris: Vec<String> = match op {
        EditOp::Save { uri } if uri.starts_with("spotify:track:") => vec![uri.clone()],
        EditOp::AddToPlaylist { uris, .. } | EditOp::CreatePlaylist { uris, .. } => uris.clone(),
        _ => return Vec::new(),
    };
    let mut found = db.tracks(&uris).unwrap_or_default();
    uris.iter().filter_map(|u| found.remove(u)).collect()
}

// ── Yazılan işlemi önbelleğe işleme (Spotify'daki son bilinen hâl) ──────

/// Yalnız adresi bilinen, boş satır (listeye uygulanıp yalnız adresleri
/// kaydedilir; bilgisi önbellekte eksik şarkılar listeden düşmesin diye).
fn stub_row(uri: &str) -> TrackRow {
    TrackRow {
        uri: uri.to_string(),
        name: String::new(),
        artists: String::new(),
        album: String::new(),
        duration_ms: 0,
        image: String::new(),
    }
}

/// İşlemi önbellekteki liste (adresler) üzerinde uygular ve kaydeder;
/// sürüm numarası korunur (yazma Spotify'a ulaşınca sürüm değişir, sonraki
/// tazeleme gerçeği getirir). Yeni uzunluk, liste değiştiyse.
fn persist_tracks(
    db: &Db,
    id: &CollectionId,
    op: &EditOp,
    rows: &[TrackRow],
) -> Result<Option<usize>> {
    let Some(uris) = db.collection_uris(id)? else {
        return Ok(None);
    };
    // Sıra numaralı işlemler ekrandaki (yalnız şarkı) dizilimde anlamlı;
    // önbellekte şarkı olmayan öğe (bölüm, yerel dosya) varsa kaydırır:
    // yazmayalım, sonraki tazeleme getirir.
    let by_index = matches!(
        op,
        EditOp::RemoveFromPlaylist { .. }
            | EditOp::MovePlaylistItems { .. }
            | EditOp::AddToPlaylist {
                position: AddPosition::At(_),
                ..
            }
    );
    if by_index && uris.iter().any(|u| !u.starts_with("spotify:track:")) {
        return Ok(None);
    }
    let mut list: Vec<TrackRow> = uris.iter().map(|u| stub_row(u)).collect();
    if !patch_tracks(op, rows, &mut list) {
        return Ok(None);
    }
    let uris: Vec<String> = list.into_iter().map(|t| t.uri).collect();
    db.save_tracks(rows)?;
    let revision = db.revision(id)?;
    db.save_collection(id, revision.as_deref(), &uris)?;
    Ok(Some(uris.len()))
}

/// Spotify'a yazılan işlemi önbelleğe işler (liste şarkıları + kütüphane).
fn persist_op(
    db: &Db,
    op: &EditOp,
    rows: &[TrackRow],
    item: Option<&LibraryItem>,
    deleted: &mut HashMap<String, (usize, LibraryItem)>,
) -> Result<()> {
    let mut liked_len = None;
    if let Some(id) = touched_collection(op)
        && !matches!(op, EditOp::CreatePlaylist { .. })
        && !edits::is_temp_id(id.key())
    {
        let len = persist_tracks(db, &id, op, rows)?;
        if id == CollectionId::Liked {
            liked_len = len;
        }
    }
    if let Some(mut lib) = db.load_library()? {
        let changed = if is_track_like(op) {
            // Beğenilen sayısı listeden: ±1 değil, gerçek uzunluk (liste
            // değişmediyse sayı da değişmez).
            liked_len.is_some_and(|n| std::mem::replace(&mut lib.liked_count, n) != n)
        } else {
            patch_library(op, item, &mut lib, deleted)
        };
        if changed {
            db.save_library(&lib)?;
        }
    }
    Ok(())
}

/// Spotify'da oluşan listeyi önbelleğe işler (adı ve ilk şarkılarıyla).
fn persist_created(
    db: &Db,
    real: &CollectionId,
    name: &str,
    uris: &[String],
    rows: &[TrackRow],
) -> Result<()> {
    db.save_tracks(rows)?;
    db.save_collection(real, None, uris)?;
    if let Some(mut lib) = db.load_library()?
        && !lib.playlists.iter().any(|p| &p.id == real)
    {
        lib.playlists.insert(
            0,
            LibraryItem {
                id: real.clone(),
                name: name.to_string(),
                subtitle: crate::i18n::canon::songs(uris.len()),
                image: String::new(),
            },
        );
        db.save_library(&lib)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: u32) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{n:022}"),
            name: format!("Şarkı {n}"),
            artists: "Sanatçı".into(),
            album: "Albüm".into(),
            duration_ms: 1000,
            image: String::new(),
        }
    }

    fn uris(list: &[TrackRow]) -> Vec<u32> {
        list.iter().map(|t| t.uri[14..].parse().unwrap()).collect()
    }

    #[test]
    fn like_and_unlike_patch_the_liked_list() {
        let mut list = vec![row(1), row(2)];
        assert!(patch_tracks(
            &EditOp::Save { uri: row(3).uri },
            &[row(3)],
            &mut list
        ));
        assert_eq!(uris(&list), [3, 1, 2]);
        // Zaten varsa iki kez eklenmez.
        assert!(!patch_tracks(
            &EditOp::Save { uri: row(3).uri },
            &[row(3)],
            &mut list
        ));
        assert!(patch_tracks(
            &EditOp::Unsave { uri: row(1).uri },
            &[],
            &mut list
        ));
        assert_eq!(uris(&list), [3, 2]);
    }

    #[test]
    fn add_remove_and_move_rows_like_spotify() {
        let mut list = vec![row(1), row(2), row(1), row(3)];
        let p = "spotify:playlist:x".to_string();
        // Tek satır: 2. sıradaki kopya gider, ilki kalır.
        let rm = EditOp::RemoveFromPlaylist {
            playlist: p.clone(),
            uris: vec![row(1).uri],
            indices: vec![2],
        };
        assert!(patch_tracks(&rm, &[], &mut list));
        assert_eq!(uris(&list), [1, 2, 3]);
        // Geri alma: aynı yere döner.
        let undo = rm.inverse().unwrap();
        assert!(patch_tracks(&undo, &[row(1)], &mut list));
        assert_eq!(uris(&list), [1, 2, 1, 3]);
        let add = EditOp::AddToPlaylist {
            playlist: p.clone(),
            uris: vec![row(5).uri, row(6).uri],
            position: AddPosition::Last,
        };
        assert!(patch_tracks(&add, &[row(5), row(6)], &mut list));
        assert_eq!(uris(&list), [1, 2, 1, 3, 5, 6]);
        let mv = EditOp::MovePlaylistItems {
            playlist: p,
            from_index: 4,
            length: 2,
            to_index: 0,
            uris: vec![],
        };
        assert!(patch_tracks(&mv, &[], &mut list));
        assert_eq!(uris(&list), [5, 6, 1, 2, 1, 3]);
        // Taşımanın tersi eski sıraya döndürür.
        assert!(patch_tracks(&mv.inverse().unwrap(), &[], &mut list));
        assert_eq!(uris(&list), [1, 2, 1, 3, 5, 6]);
    }

    #[test]
    fn library_follows_create_rename_delete_restore() {
        let mut lib = Library::default();
        let mut deleted = HashMap::new();
        let create = EditOp::CreatePlaylist {
            temp_id: "local:playlist:1".into(),
            name: "Gece".into(),
            uris: vec![row(1).uri],
            created: None,
        };
        assert!(patch_library(&create, None, &mut lib, &mut deleted));
        assert_eq!(lib.playlists[0].subtitle, "1 şarkı");
        let p = "local:playlist:1".to_string();
        let add = EditOp::AddToPlaylist {
            playlist: p.clone(),
            uris: vec![row(2).uri],
            position: AddPosition::Last,
        };
        assert!(patch_library(&add, None, &mut lib, &mut deleted));
        assert_eq!(lib.playlists[0].subtitle, "2 şarkı");
        let rename = EditOp::RenamePlaylist {
            playlist: p.clone(),
            name: "Sabah".into(),
            previous_name: Some("Gece".into()),
        };
        assert!(patch_library(&rename, None, &mut lib, &mut deleted));
        assert!(patch_library(
            &rename.inverse().unwrap(),
            None,
            &mut lib,
            &mut deleted
        ));
        assert_eq!(lib.playlists[0].name, "Gece");
        let delete = EditOp::DeletePlaylist {
            playlist: p.clone(),
        };
        assert!(patch_library(&delete, None, &mut lib, &mut deleted));
        assert!(lib.playlists.is_empty());
        assert!(patch_library(
            &delete.inverse().unwrap(),
            None,
            &mut lib,
            &mut deleted
        ));
        assert_eq!(lib.playlists[0].name, "Gece");
        // Beğenilen sayısı.
        assert!(patch_library(
            &EditOp::Save { uri: row(9).uri },
            None,
            &mut lib,
            &mut deleted
        ));
        assert_eq!(lib.liked_count, 1);
    }

    #[test]
    fn test_runs_never_reach_spotify() {
        // Sınama penceresi açıkken yazmalar sahte arka uca gider.
        // (Ortam değişkeni bu süreçte yoksa `None`: gerçek açılış.)
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_none()
            && std::env::var_os("OPENIFY_TEST_EDITS").is_none()
        {
            assert_eq!(test_edits(), None);
        }
    }

    // ── Bekleyen işlemleri yenilenen veriye yeniden uygulama (saf) ──────

    fn pend(op: EditOp, rows: Vec<TrackRow>) -> PendingEdit {
        PendingEdit {
            op,
            rows,
            item: None,
            unsure: false,
        }
    }

    fn unsure(op: EditOp, rows: Vec<TrackRow>) -> PendingEdit {
        PendingEdit {
            op,
            rows,
            item: None,
            unsure: true,
        }
    }

    fn pl(n: u32) -> String {
        format!("spotify:playlist:{n:022}")
    }

    #[test]
    fn replay_reapplies_pending_edits_in_order_to_fresh_data() {
        // Spotify'dan taze: [1 2 3 4]. Bekleyen: 2'yi beğenmeme, 9'u beğenme.
        let mut fresh = vec![row(1), row(2), row(3), row(4)];
        let edits = vec![
            pend(EditOp::Unsave { uri: row(2).uri }, vec![row(2)]),
            pend(EditOp::Save { uri: row(9).uri }, vec![row(9)]),
        ];
        assert!(replay_tracks(&CollectionId::Liked, &edits, &mut fresh));
        assert_eq!(uris(&fresh), [9, 1, 3, 4]);
        // Başka listenin işlemi bu listeye dokunmaz.
        let mut other = vec![row(1), row(2)];
        let for_list = vec![pend(
            EditOp::AddToPlaylist {
                playlist: pl(1),
                uris: vec![row(7).uri],
                position: AddPosition::Last,
            },
            vec![row(7)],
        )];
        assert!(!replay_tracks(&CollectionId::Liked, &for_list, &mut other));
        assert_eq!(uris(&other), [1, 2]);
        assert!(replay_tracks(
            &CollectionId::Playlist(pl(1)),
            &for_list,
            &mut other
        ));
        assert_eq!(uris(&other), [1, 2, 7]);
        // Sıra önemli: önce ekle sonra taşı, taşıma eklenmişi görür.
        let mut list = vec![row(1), row(2)];
        let edits = vec![
            pend(
                EditOp::AddToPlaylist {
                    playlist: pl(1),
                    uris: vec![row(3).uri],
                    position: AddPosition::Last,
                },
                vec![row(3)],
            ),
            pend(
                EditOp::MovePlaylistItems {
                    playlist: pl(1),
                    from_index: 2,
                    length: 1,
                    to_index: 0,
                    uris: vec![row(3).uri],
                },
                vec![],
            ),
        ];
        replay_tracks(&CollectionId::Playlist(pl(1)), &edits, &mut list);
        assert_eq!(uris(&list), [3, 1, 2]);
    }

    #[test]
    fn replay_is_idempotent_for_like_and_unlike() {
        // Spotify'da zaten uygulanmış olsa bile beğen / vazgeç ikinci kez bozmaz.
        let mut list = vec![row(1), row(3)];
        let edits = vec![
            pend(EditOp::Unsave { uri: row(2).uri }, vec![row(2)]),
            pend(EditOp::Save { uri: row(3).uri }, vec![row(3)]),
        ];
        assert!(!replay_tracks(&CollectionId::Liked, &edits, &mut list));
        assert_eq!(uris(&list), [1, 3]);
    }

    #[test]
    fn unsure_edits_are_not_applied_twice() {
        let p = pl(1);
        let id = CollectionId::Playlist(p.clone());
        // Ekleme: Spotify verisi eklenenleri zaten sonda gösteriyor → tekrar eklenmez.
        let add = EditOp::AddToPlaylist {
            playlist: p.clone(),
            uris: vec![row(5).uri, row(6).uri],
            position: AddPosition::Last,
        };
        let mut list = vec![row(1), row(2), row(5), row(6)];
        assert!(!replay_tracks(
            &id,
            &[unsure(add.clone(), vec![row(5), row(6)])],
            &mut list
        ));
        assert_eq!(uris(&list), [1, 2, 5, 6]);
        // Henüz yoksa eklenir.
        let mut list = vec![row(1), row(2)];
        assert!(replay_tracks(
            &id,
            &[unsure(add.clone(), vec![row(5), row(6)])],
            &mut list
        ));
        assert_eq!(uris(&list), [1, 2, 5, 6]);
        // Kesin (gönderilmemiş) ekleme aynı veride yine eklerdi: ayrım bu.
        let mut list = vec![row(1), row(2), row(5), row(6)];
        assert!(replay_tracks(
            &id,
            &[pend(add, vec![row(5), row(6)])],
            &mut list
        ));
        assert_eq!(list.len(), 6);
        // Çıkarma: sırası bilinen satır artık orada değilse çıkarılmış sayılır,
        // başka kopya silinmez.
        let rm = EditOp::RemoveFromPlaylist {
            playlist: p.clone(),
            uris: vec![row(1).uri],
            indices: vec![2],
        };
        let mut list = vec![row(1), row(2), row(3)];
        assert!(!replay_tracks(
            &id,
            &[unsure(rm.clone(), vec![])],
            &mut list
        ));
        assert_eq!(uris(&list), [1, 2, 3]);
        let mut list = vec![row(2), row(3), row(1)];
        assert!(replay_tracks(&id, &[unsure(rm, vec![])], &mut list));
        assert_eq!(uris(&list), [2, 3]);
        // Taşıma: bloğun hedefte durduğu veri zaten taşınmış.
        let mv = EditOp::MovePlaylistItems {
            playlist: p,
            from_index: 3,
            length: 1,
            to_index: 0,
            uris: vec![row(4).uri],
        };
        let mut list = vec![row(4), row(1), row(2), row(3)];
        assert!(!replay_tracks(
            &id,
            &[unsure(mv.clone(), vec![])],
            &mut list
        ));
        assert_eq!(uris(&list), [4, 1, 2, 3]);
        let mut list = vec![row(1), row(2), row(3), row(4)];
        assert!(replay_tracks(&id, &[unsure(mv, vec![])], &mut list));
        assert_eq!(uris(&list), [4, 1, 2, 3]);
    }

    fn lib_with(count: usize) -> Library {
        Library {
            liked_count: count,
            albums: Vec::new(),
            playlists: vec![LibraryItem {
                id: CollectionId::Playlist(pl(1)),
                name: "Gece".into(),
                subtitle: "3 şarkı".into(),
                image: String::new(),
            }],
            artists: Vec::new(),
            ..Default::default()
        }
    }

    fn art(n: u32) -> String {
        format!("spotify:artist:{n:022}")
    }

    fn art_card(n: u32, name: &str) -> LibraryItem {
        super::super::library::artist_item(&art(n), name, "img")
    }

    #[test]
    fn follow_and_unfollow_patch_the_library_and_undo_restores_it() {
        let mut lib = lib_with(0);
        let mut deleted = HashMap::new();
        let follow = EditOp::Follow { uri: art(1) };
        // Kart olmadan (ad / portre bilinmiyor) kütüphaneye boş kart konmaz.
        assert!(!patch_library(&follow, None, &mut lib, &mut deleted));
        assert!(lib.artists.is_empty());
        // Takip: en son takip edilen başta; ikinci kez eklenmez.
        assert!(patch_library(
            &follow,
            Some(&art_card(1, "Tarkan")),
            &mut lib,
            &mut deleted
        ));
        assert!(patch_library(
            &EditOp::Follow { uri: art(2) },
            Some(&art_card(2, "Sezen")),
            &mut lib,
            &mut deleted
        ));
        assert!(!patch_library(
            &follow,
            Some(&art_card(1, "Tarkan")),
            &mut lib,
            &mut deleted
        ));
        let names: Vec<&str> = lib.artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Sezen", "Tarkan"]);
        // Takip, şarkı sayısını ya da listeleri değiştirmez.
        assert_eq!(
            (lib.liked_count, lib.playlists.len(), lib.albums.len()),
            (0, 1, 0)
        );
        // Takipten çık ve geri al (kart eski hâliyle başa döner).
        let unfollow = follow.inverse().unwrap();
        assert!(patch_library(&unfollow, None, &mut lib, &mut deleted));
        assert_eq!(lib.artists.len(), 1);
        assert!(
            !patch_library(&unfollow, None, &mut lib, &mut deleted),
            "ikinci kez bir şey değişmez"
        );
        assert!(patch_library(
            &unfollow.inverse().unwrap(),
            Some(&art_card(1, "Tarkan")),
            &mut lib,
            &mut deleted
        ));
        assert_eq!(lib.artists[0].name, "Tarkan");
        // Takip bir şarkı listesine dokunmaz (arayüz liste yamasından geçirmez).
        assert_eq!(touched_collection(&follow), None);
        assert!(affects_library(&follow));
    }

    #[test]
    fn pending_follows_are_replayed_on_top_of_refreshed_data() {
        // Spotify'dan gelen tazeleme henüz yazılmamış takibi / bırakmayı bilmez:
        // bekleyen işlemler üstüne uygulanır, kütüphane bir an eski hâline dönmez.
        let mut truth = lib_with(0);
        truth.artists = vec![art_card(1, "Tarkan"), art_card(2, "Sezen")];
        let mut deleted = HashMap::new();
        let edits = vec![
            PendingEdit {
                op: EditOp::Follow { uri: art(3) },
                rows: vec![],
                item: Some(art_card(3, "Ajda")),
                unsure: false,
            },
            pend(EditOp::Unfollow { uri: art(2) }, vec![]),
            // Sonucu belirsiz takip de uygulanır (yazma idempotent: iki kez zarar vermez).
            PendingEdit {
                op: EditOp::Follow { uri: art(1) },
                rows: vec![],
                item: Some(art_card(1, "Tarkan")),
                unsure: true,
            },
        ];
        assert!(replay_library(&edits, &mut truth, &mut deleted, None));
        let names: Vec<&str> = truth.artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Ajda", "Tarkan"]);
    }

    #[test]
    fn written_follows_catch_the_cache_up_and_keep_other_sets() {
        let db = seeded_db();
        let mut deleted = HashMap::new();
        let follow = EditOp::Follow { uri: art(7) };
        persist_op(&db, &follow, &[], Some(&art_card(7, "Ajda")), &mut deleted).unwrap();
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(lib.artists.len(), 1);
        assert_eq!(
            (lib.artists[0].name.as_str(), lib.artists[0].image.as_str()),
            ("Ajda", "img")
        );
        // Kütüphanenin geri kalanı (beğenilen sayısı, liste) olduğu gibi.
        assert_eq!((lib.liked_count, lib.playlists.len()), (5, 1));
        // Beğenilen şarkı listesi etkilenmez.
        assert_eq!(db_uris(&db, &CollectionId::Liked), [1, 2, 3, 4, 5]);
        // Başka bir yazma (liste adı) takip edilenleri silmez.
        let rename = EditOp::RenamePlaylist {
            playlist: pl(1),
            name: "Sabah".into(),
            previous_name: None,
        };
        persist_op(&db, &rename, &[], None, &mut deleted).unwrap();
        assert_eq!(db.load_library().unwrap().unwrap().artists.len(), 1);
        persist_op(&db, &follow.inverse().unwrap(), &[], None, &mut deleted).unwrap();
        assert!(db.load_library().unwrap().unwrap().artists.is_empty());
    }

    #[test]
    fn replay_library_counts_liked_from_the_real_set() {
        let liked: HashSet<String> = [row(1).uri, row(2).uri].into_iter().collect();
        let mut lib = lib_with(2);
        let mut deleted = HashMap::new();
        let edits = vec![
            pend(EditOp::Unsave { uri: row(2).uri }, vec![row(2)]),
            // Zaten beğenili olanı beğenmek sayıyı şişirmez.
            pend(EditOp::Save { uri: row(1).uri }, vec![row(1)]),
            pend(EditOp::Save { uri: row(8).uri }, vec![row(8)]),
        ];
        assert!(replay_library(&edits, &mut lib, &mut deleted, Some(&liked)));
        assert_eq!(lib.liked_count, 2);
        // Beğenilen kümesi bilinmiyorsa eski (±1) davranış.
        let mut lib = lib_with(2);
        replay_library(&edits[..1], &mut lib, &mut deleted, None);
        assert_eq!(lib.liked_count, 1);
    }

    #[test]
    fn replay_library_follows_list_ops_and_skips_counts_for_unsure_adds() {
        let mut lib = lib_with(0);
        let mut deleted = HashMap::new();
        let add = EditOp::AddToPlaylist {
            playlist: pl(1),
            uris: vec![row(5).uri],
            position: AddPosition::Last,
        };
        let rename = EditOp::RenamePlaylist {
            playlist: pl(1),
            name: "Sabah".into(),
            previous_name: Some("Gece".into()),
        };
        // Kesin ekleme sayıyı artırır, belirsiz olan artırmaz (uygulanıp uygulanmadığı bilinmiyor).
        replay_library(
            &[pend(add.clone(), vec![row(5)]), pend(rename, vec![])],
            &mut lib,
            &mut deleted,
            None,
        );
        assert_eq!(
            (
                lib.playlists[0].name.as_str(),
                lib.playlists[0].subtitle.as_str()
            ),
            ("Sabah", "4 şarkı")
        );
        let mut lib = lib_with(0);
        replay_library(&[unsure(add, vec![row(5)])], &mut lib, &mut deleted, None);
        assert_eq!(lib.playlists[0].subtitle, "3 şarkı");
        // Sil + geri getir bekliyor: ikisi sırayla uygulanınca liste yerinde.
        let mut lib = lib_with(0);
        let del = EditOp::DeletePlaylist { playlist: pl(1) };
        replay_library(&[pend(del.clone(), vec![])], &mut lib, &mut deleted, None);
        assert!(lib.playlists.is_empty());
        let mut lib = lib_with(0);
        let mut deleted = HashMap::new();
        replay_library(
            &[
                pend(del.clone(), vec![]),
                pend(del.inverse().unwrap(), vec![]),
            ],
            &mut lib,
            &mut deleted,
            None,
        );
        assert_eq!(lib.playlists.len(), 1);
        // Albüm kaydı: kart bekleyen işlemle gelir.
        let card = LibraryItem {
            id: CollectionId::Album("spotify:album:a".into()),
            name: "A".into(),
            subtitle: "B".into(),
            image: String::new(),
        };
        let mut lib = lib_with(0);
        let save = PendingEdit {
            op: EditOp::Save {
                uri: "spotify:album:a".into(),
            },
            rows: vec![],
            item: Some(card),
            unsure: false,
        };
        assert!(replay_library(&[save], &mut lib, &mut deleted, None));
        assert_eq!(lib.albums.len(), 1);
        assert!(affects_library(&EditOp::Unsave {
            uri: "spotify:album:a".into()
        }));
        assert!(!affects_library(&EditOp::MovePlaylistItems {
            playlist: pl(1),
            from_index: 0,
            length: 1,
            to_index: 1,
            uris: vec![]
        }));
    }

    // ── Önbelleğe işleme (Spotify'daki son bilinen hâl) ─────────────────

    fn seeded_db() -> Arc<Db> {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let rows: Vec<TrackRow> = (1..=5).map(row).collect();
        db.save_tracks(&rows).unwrap();
        let liked: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
        db.save_collection(&CollectionId::Liked, None, &liked)
            .unwrap();
        db.save_collection(
            &CollectionId::Playlist(pl(1)),
            Some(b"rev1"),
            &[row(1).uri, row(2).uri, row(3).uri],
        )
        .unwrap();
        db.save_library(&lib_with(5)).unwrap();
        db
    }

    fn db_uris(db: &Db, id: &CollectionId) -> Vec<u32> {
        db.collection_uris(id)
            .unwrap()
            .unwrap()
            .iter()
            .map(|u| u[14..].parse().unwrap())
            .collect()
    }

    #[test]
    fn written_edits_catch_the_cache_up_keeping_the_revision() {
        let db = seeded_db();
        let mut deleted = HashMap::new();
        let liked = CollectionId::Liked;
        let list = CollectionId::Playlist(pl(1));
        // Beğenmeme: liste ve beğenilen sayısı (listenin gerçek uzunluğu).
        persist_op(
            &db,
            &EditOp::Unsave { uri: row(2).uri },
            &[],
            None,
            &mut deleted,
        )
        .unwrap();
        assert_eq!(db_uris(&db, &liked), [1, 3, 4, 5]);
        assert_eq!(db.load_library().unwrap().unwrap().liked_count, 4);
        // Zaten beğenili şarkıyı beğenmek: liste de sayı da değişmez.
        persist_op(
            &db,
            &EditOp::Save { uri: row(1).uri },
            &[row(1)],
            None,
            &mut deleted,
        )
        .unwrap();
        assert_eq!(db_uris(&db, &liked), [1, 3, 4, 5]);
        assert_eq!(db.load_library().unwrap().unwrap().liked_count, 4);
        // Yeni beğeni başa eklenir, şarkı bilgisi de kaydedilir.
        persist_op(
            &db,
            &EditOp::Save { uri: row(9).uri },
            &[row(9)],
            None,
            &mut deleted,
        )
        .unwrap();
        assert_eq!(db_uris(&db, &liked), [9, 1, 3, 4, 5]);
        assert!(db.tracks(&[row(9).uri]).unwrap().contains_key(&row(9).uri));
        assert_eq!(db.load_library().unwrap().unwrap().liked_count, 5);
        // Liste işlemleri: sürüm numarası korunur (sonraki tazeleme gerçeği getirir).
        let add = EditOp::AddToPlaylist {
            playlist: pl(1),
            uris: vec![row(5).uri],
            position: AddPosition::At(1),
        };
        persist_op(&db, &add, &[row(5)], None, &mut deleted).unwrap();
        assert_eq!(db_uris(&db, &list), [1, 5, 2, 3]);
        assert_eq!(db.revision(&list).unwrap().as_deref(), Some(&b"rev1"[..]));
        assert_eq!(
            db.load_library().unwrap().unwrap().playlists[0].subtitle,
            "4 şarkı"
        );
        let mv = EditOp::MovePlaylistItems {
            playlist: pl(1),
            from_index: 1,
            length: 1,
            to_index: 3,
            uris: vec![row(5).uri],
        };
        persist_op(&db, &mv, &[], None, &mut deleted).unwrap();
        assert_eq!(db_uris(&db, &list), [1, 2, 3, 5]);
        let rm = EditOp::RemoveFromPlaylist {
            playlist: pl(1),
            uris: vec![row(2).uri],
            indices: vec![1],
        };
        persist_op(&db, &rm, &[], None, &mut deleted).unwrap();
        assert_eq!(db_uris(&db, &list), [1, 3, 5]);
        // Ad, silme, geri getirme.
        let rename = EditOp::RenamePlaylist {
            playlist: pl(1),
            name: "Sabah".into(),
            previous_name: None,
        };
        persist_op(&db, &rename, &[], None, &mut deleted).unwrap();
        assert_eq!(
            db.load_library().unwrap().unwrap().playlists[0].name,
            "Sabah"
        );
        let del = EditOp::DeletePlaylist { playlist: pl(1) };
        persist_op(&db, &del, &[], None, &mut deleted).unwrap();
        assert!(db.load_library().unwrap().unwrap().playlists.is_empty());
        persist_op(&db, &del.inverse().unwrap(), &[], None, &mut deleted).unwrap();
        assert_eq!(db.load_library().unwrap().unwrap().playlists.len(), 1);
    }

    #[test]
    fn cache_catch_up_is_careful_with_unknown_rows_and_index_ops() {
        let db = seeded_db();
        let mut deleted = HashMap::new();
        // Bilgisi önbellekte olmayan şarkı da kaybolmaz (yalnız adresler uygulanır).
        db.save_collection(
            &CollectionId::Liked,
            None,
            &[row(1).uri, row(70).uri, row(2).uri],
        )
        .unwrap();
        persist_op(
            &db,
            &EditOp::Unsave { uri: row(2).uri },
            &[],
            None,
            &mut deleted,
        )
        .unwrap();
        assert_eq!(db_uris(&db, &CollectionId::Liked), [1, 70]);
        // Listede şarkı olmayan öğe (bölüm / yerel dosya) varsa sıra numaralı
        // işlem yazılmaz (indeksler kayar): sonraki tazeleme getirir.
        let list = CollectionId::Playlist(pl(1));
        db.save_collection(
            &list,
            Some(b"r"),
            &["spotify:episode:e".into(), row(1).uri, row(2).uri],
        )
        .unwrap();
        let mv = EditOp::MovePlaylistItems {
            playlist: pl(1),
            from_index: 1,
            length: 1,
            to_index: 2,
            uris: vec![],
        };
        persist_op(&db, &mv, &[], None, &mut deleted).unwrap();
        assert_eq!(db.collection_uris(&list).unwrap().unwrap().len(), 3);
        assert_eq!(db.collection_uris(&list).unwrap().unwrap()[1], row(1).uri);
        // Hiç kaydedilmemiş liste: dokunulmaz, hata değil.
        let add = EditOp::AddToPlaylist {
            playlist: pl(9),
            uris: vec![row(1).uri],
            position: AddPosition::Last,
        };
        persist_op(&db, &add, &[row(1)], None, &mut deleted).unwrap();
        assert!(
            db.collection_uris(&CollectionId::Playlist(pl(9)))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn created_list_lands_in_the_cache_with_its_real_address() {
        let db = seeded_db();
        let real = CollectionId::Playlist(pl(7));
        persist_created(
            &db,
            &real,
            "Yeni",
            &[row(1).uri, row(2).uri],
            &[row(1), row(2)],
        )
        .unwrap();
        assert_eq!(db_uris(&db, &real), [1, 2]);
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (
                lib.playlists[0].id.clone(),
                lib.playlists[0].subtitle.as_str()
            ),
            (real.clone(), "2 şarkı")
        );
        // İki kez yazılsa kütüphanede tek kez durur.
        persist_created(&db, &real, "Yeni", &[row(1).uri, row(2).uri], &[]).unwrap();
        assert_eq!(db.load_library().unwrap().unwrap().playlists.len(), 2);
    }

    // ── Motor akışı: bekleyen yazma + yenileme + hata + bırakma ─────────

    use super::super::{Internal, library::Refresh};
    use tokio::sync::mpsc;

    struct Rig {
        engine: Engine,
        events: async_channel::Receiver<Event>,
        internal: mpsc::UnboundedReceiver<Internal>,
        db: Arc<Db>,
        _rt: tokio::runtime::Runtime,
    }

    /// Bağlantısız, bellekte kuyruklu, sahte arka uçlu motor (ağ yok, Spotify'a
    /// ve diske hiçbir şey gitmez); kuyruğu testler elle sürer.
    fn rig(fake: Option<Fake>) -> Rig {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (events_tx, events) = async_channel::unbounded();
        let (playback_tx, _playback) = async_channel::unbounded();
        let (internal_tx, internal) = mpsc::unbounded_channel();
        let db = seeded_db();
        let mut engine = Engine::new(
            rt.handle().clone(),
            Some(db.clone()),
            events_tx,
            playback_tx,
            internal_tx,
            super::super::AudioLevels::new(),
        );
        engine.wire.queue = Some(Arc::new(EditQueue::open_in_memory().unwrap()));
        engine.wire.fake = fake;
        // Kuyruğu arka plan iş parçacığı değil test sürsün.
        engine.wire.processing = true;
        engine.show_cached_library();
        engine.open_collection(CollectionId::Liked);
        Rig {
            engine,
            events,
            internal,
            db,
            _rt: rt,
        }
    }

    fn drain(rig: &Rig) -> Vec<Event> {
        std::iter::from_fn(|| rig.events.try_recv().ok()).collect()
    }

    fn loaded(events: &[Event], id: &CollectionId) -> Option<Vec<u32>> {
        events.iter().rev().find_map(|e| match e {
            Event::CollectionLoaded { id: i, tracks } if i == id => Some(uris(tracks)),
            _ => None,
        })
    }

    fn lib_event(events: &[Event]) -> Option<Library> {
        events.iter().rev().find_map(|e| match e {
            Event::LibraryLoaded(l) => Some(l.clone()),
            _ => None,
        })
    }

    fn status(events: &[Event]) -> Option<(usize, Vec<FailedEdit>)> {
        events.iter().rev().find_map(|e| match e {
            Event::Library(LibraryEvent::EditStatus { pending, failed }) => {
                Some((*pending, failed.clone()))
            }
            _ => None,
        })
    }

    fn truth_liked() -> Arc<Vec<TrackRow>> {
        Arc::new((1..=5).map(row).collect())
    }

    fn send_edit(rig: &mut Rig, op: EditOp, rows: Vec<TrackRow>) {
        rig.engine.on_library_command(LibraryCommand::Edit {
            op,
            rows,
            item: None,
        });
    }

    /// Kuyruğu sahte arka uçla işleyip sonucu motora iletir.
    fn run_queue(rig: &mut Rig, mode: Fake) {
        assert_eq!(rig.engine.wire.fake, Some(mode));
        // Gerçek akış: engelleyici iş parçacığında sahte arka uç (damgalayıcıyla),
        // sonucu bekle ve motora ilet.
        rig.engine.wire.processing = false;
        rig.engine.process_edits();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let report = loop {
            match rig.internal.try_recv() {
                Ok(Internal::Wire(WireInternal::Processed(report))) => break report,
                Ok(_) => {}
                Err(_) => {
                    assert!(std::time::Instant::now() < deadline, "kuyruk işi bitmedi");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        };
        rig.engine
            .on_internal(Internal::Wire(WireInternal::Processed(report)));
        // `on_internal` bekleyen işleme "tekrar" demiş olabilir: kuyruk yine test sürsün.
        rig.engine.wire.processing = true;
    }

    #[test]
    fn refresh_read_before_a_write_finished_is_dropped_and_redone() {
        let mut rig = rig(Some(Fake::Ok));
        drain(&rig);
        // Hiç tazeleme başlamadıysa bir şey eskimiş sayılmaz.
        assert!(!rig.engine.refresh_is_stale());
        // Tazeleme başladı, sonra bir yazma Spotify'a ulaştı.
        rig.engine.mark_refresh_started();
        std::thread::sleep(Duration::from_millis(5));
        send_edit(&mut rig, EditOp::Unsave { uri: row(2).uri }, vec![row(2)]);
        run_queue(&mut rig, Fake::Ok);
        drain(&rig);
        assert!(rig.engine.refresh_is_stale());
        // Tazelemenin eski verisi (şarkı hâlâ beğenili, sayı 5) ekrana verilmez.
        let refresh = Refresh {
            library: lib_with(5),
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        let ev = drain(&rig);
        assert!(lib_event(&ev).is_none(), "{ev:?}");
        assert_eq!(rig.engine.library.as_ref().unwrap().liked_count, 4);
        // Yazmadan SONRA başlayan tazelemenin sonucu geçerli: ekrana verilir.
        std::thread::sleep(Duration::from_millis(5));
        rig.engine.mark_refresh_started();
        assert!(!rig.engine.refresh_is_stale());
        rig.engine.library = Some(lib_with(9));
        let refresh = Refresh {
            library: lib_with(4),
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        assert_eq!(lib_event(&drain(&rig)).unwrap().liked_count, 4);
    }

    #[test]
    fn refresh_while_unlike_is_pending_does_not_bring_the_row_back() {
        let mut rig = rig(Some(Fake::Slow(4)));
        drain(&rig);
        send_edit(&mut rig, EditOp::Unsave { uri: row(2).uri }, vec![row(2)]);
        assert_eq!(status(&drain(&rig)).map(|s| s.0), Some(1));
        // Spotify'dan taze (henüz yazılmamış) veri: 5 şarkı, sayı 5.
        let refresh = Refresh {
            library: lib_with(5),
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        let ev = drain(&rig);
        // Eski hâl (5) hiç gösterilmez; ekranla aynı (4) olduğundan olay bile gitmez.
        assert!(lib_event(&ev).is_none_or(|l| l.liked_count == 4), "{ev:?}");
        assert_eq!(rig.engine.library.as_ref().unwrap().liked_count, 4);
        // Liste yenilenince de satır geri gelmez; sıra korunur.
        rig.engine
            .on_internal(Internal::Tracks(CollectionId::Liked, Ok(truth_liked())));
        let ev = drain(&rig);
        assert_eq!(
            loaded(&ev, &CollectionId::Liked),
            Some(vec![1, 3, 4, 5]),
            "{ev:?}"
        );
        assert_eq!(
            uris(&rig.engine.collections[&CollectionId::Liked]),
            [1, 3, 4, 5]
        );
        // Ekrandaki kopya eskiyse (arayüzün göreceği durum) yenileme onu düzeltir:
        // sayı 5'e değil 4'e gider.
        rig.engine.library = Some(lib_with(5));
        let refresh = Refresh {
            library: lib_with(5),
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        assert_eq!(lib_event(&drain(&rig)).unwrap().liked_count, 4);
    }

    #[test]
    fn refresh_keeps_failed_edits_applied_until_retried_or_dismissed() {
        let mut rig = rig(Some(Fake::Fail(FailKind::Forbidden)));
        drain(&rig);
        send_edit(&mut rig, EditOp::Unsave { uri: row(2).uri }, vec![row(2)]);
        run_queue(&mut rig, Fake::Fail(FailKind::Forbidden));
        let ev = drain(&rig);
        let (pending, failed) = status(&ev).unwrap();
        assert_eq!((pending, failed.len()), (0, 1));
        assert_eq!(failed[0].kind, FailKind::Forbidden);
        assert_eq!(failed[0].op, EditOp::Unsave { uri: row(2).uri });
        // Yazılamayan işlem iyimser hâlde kalır; yenileme onu geri getirmez.
        rig.engine
            .on_internal(Internal::Tracks(CollectionId::Liked, Ok(truth_liked())));
        assert_eq!(
            loaded(&drain(&rig), &CollectionId::Liked),
            Some(vec![1, 3, 4, 5])
        );
        // "Yeniden dene": yine bekliyor.
        let id = failed[0].id;
        rig.engine.on_library_command(LibraryCommand::RetryEdit(id));
        let (pending, failed) = status(&drain(&rig)).unwrap();
        assert_eq!((pending, failed.len()), (1, 0));
        rig.engine
            .on_internal(Internal::Tracks(CollectionId::Liked, Ok(truth_liked())));
        assert_eq!(
            loaded(&drain(&rig), &CollectionId::Liked),
            Some(vec![1, 3, 4, 5])
        );
    }

    #[test]
    fn dismissing_a_failed_unlike_restores_the_row_and_the_count() {
        let mut rig = rig(Some(Fake::Fail(FailKind::Forbidden)));
        drain(&rig);
        send_edit(&mut rig, EditOp::Unsave { uri: row(2).uri }, vec![row(2)]);
        assert_eq!(
            uris(&rig.engine.collections[&CollectionId::Liked]),
            [1, 3, 4, 5]
        );
        assert_eq!(rig.engine.library.as_ref().unwrap().liked_count, 4);
        run_queue(&mut rig, Fake::Fail(FailKind::Forbidden));
        let id = status(&drain(&rig)).unwrap().1[0].id;
        // Bırak: satır ORİJİNAL yerinde (2. sıra) geri gelir, sayı 5, kuyruk boş.
        rig.engine
            .on_library_command(LibraryCommand::DismissEdit(id));
        let ev = drain(&rig);
        assert_eq!(
            loaded(&ev, &CollectionId::Liked),
            Some(vec![1, 2, 3, 4, 5]),
            "{ev:?}"
        );
        assert_eq!(lib_event(&ev).unwrap().liked_count, 5);
        let (pending, failed) = status(&ev).unwrap();
        assert_eq!((pending, failed.len()), (0, 0));
        assert_eq!(
            uris(&rig.engine.collections[&CollectionId::Liked]),
            [1, 2, 3, 4, 5]
        );
        // Sınamada önbellek (Spotify'daki hâl) hiç değişmedi.
        assert_eq!(db_uris(&rig.db, &CollectionId::Liked), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn dismissing_keeps_other_pending_edits_applied() {
        let mut rig = rig(Some(Fake::Fail(FailKind::NotFound)));
        drain(&rig);
        send_edit(&mut rig, EditOp::Unsave { uri: row(2).uri }, vec![row(2)]);
        run_queue(&mut rig, Fake::Fail(FailKind::NotFound));
        let id = status(&drain(&rig)).unwrap().1[0].id;
        // Başka bir beğenmeme bekliyor (işlem gelince arka planda gitmeyecek).
        send_edit(&mut rig, EditOp::Unsave { uri: row(4).uri }, vec![row(4)]);
        drain(&rig);
        rig.engine
            .on_library_command(LibraryCommand::DismissEdit(id));
        let ev = drain(&rig);
        // 2 geri geldi, 4 hâlâ yok (bekleyen işlem yeniden uygulandı).
        assert_eq!(
            loaded(&ev, &CollectionId::Liked),
            Some(vec![1, 2, 3, 5]),
            "{ev:?}"
        );
        assert_eq!(lib_event(&ev).unwrap().liked_count, 4);
    }

    #[test]
    fn dismissing_playlist_edits_restores_list_name_and_count() {
        let list = CollectionId::Playlist(pl(1));
        let mut rig = rig(Some(Fake::Fail(FailKind::Forbidden)));
        rig.engine.open_collection(list.clone());
        drain(&rig);
        let add = EditOp::AddToPlaylist {
            playlist: pl(1),
            uris: vec![row(5).uri],
            position: AddPosition::Last,
        };
        let rename = EditOp::RenamePlaylist {
            playlist: pl(1),
            name: "Sabah".into(),
            previous_name: Some("Gece".into()),
        };
        send_edit(&mut rig, add, vec![row(5)]);
        send_edit(&mut rig, rename, vec![]);
        assert_eq!(uris(&rig.engine.collections[&list]), [1, 2, 3, 5]);
        run_queue(&mut rig, Fake::Fail(FailKind::Forbidden));
        let failed = status(&drain(&rig)).unwrap().1;
        assert_eq!(failed.len(), 2);
        for f in &failed {
            rig.engine
                .on_library_command(LibraryCommand::DismissEdit(f.id));
        }
        let ev = drain(&rig);
        assert_eq!(loaded(&ev, &list), Some(vec![1, 2, 3]), "{ev:?}");
        let lib = lib_event(&ev).unwrap();
        assert_eq!(
            (
                lib.playlists[0].name.as_str(),
                lib.playlists[0].subtitle.as_str()
            ),
            ("Gece", "3 şarkı")
        );
    }

    #[test]
    fn dismissing_a_failed_create_removes_the_temp_list_and_its_dependents() {
        let temp = edits::new_temp_id();
        let mut rig = rig(Some(Fake::Fail(FailKind::Forbidden)));
        drain(&rig);
        let create = EditOp::CreatePlaylist {
            temp_id: temp.clone(),
            name: "Yeni".into(),
            uris: vec![row(1).uri],
            created: None,
        };
        send_edit(&mut rig, create, vec![row(1)]);
        let add = EditOp::AddToPlaylist {
            playlist: temp.clone(),
            uris: vec![row(2).uri],
            position: AddPosition::Last,
        };
        send_edit(&mut rig, add, vec![row(2)]);
        assert_eq!(
            rig.engine.library.as_ref().unwrap().playlists[0].name,
            "Yeni"
        );
        run_queue(&mut rig, Fake::Fail(FailKind::Forbidden));
        let failed = status(&drain(&rig)).unwrap().1;
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[1].kind, FailKind::ParentFailed);
        // Oluşturmayı bırakmak ona bağlı yazılamayanı da bırakır.
        let create_id = failed[0].id;
        rig.engine
            .on_library_command(LibraryCommand::DismissEdit(create_id));
        let ev = drain(&rig);
        let (pending, failed) = status(&ev).unwrap();
        assert_eq!((pending, failed.len()), (0, 0));
        let lib = lib_event(&ev).unwrap();
        assert!(
            lib.playlists
                .iter()
                .all(|p| p.id != CollectionId::Playlist(temp.clone())),
            "{lib:?}"
        );
        assert!(
            !rig.engine
                .collections
                .contains_key(&CollectionId::Playlist(temp))
        );
    }

    #[test]
    fn test_runs_never_write_the_cache_or_reach_spotify() {
        // Sahte arka uç başarıyla "yazdı": önbellek (Spotify'daki hâl) değişmez.
        let mut rig = rig(Some(Fake::Ok));
        drain(&rig);
        send_edit(&mut rig, EditOp::Unsave { uri: row(2).uri }, vec![row(2)]);
        run_queue(&mut rig, Fake::Ok);
        let (pending, failed) = status(&drain(&rig)).unwrap();
        assert_eq!((pending, failed.len()), (0, 0));
        assert_eq!(db_uris(&rig.db, &CollectionId::Liked), [1, 2, 3, 4, 5]);
        assert_eq!(rig.db.load_library().unwrap().unwrap().liked_count, 5);
        // Ekranda ise işlem uygulanmış kalır.
        assert_eq!(
            uris(&rig.engine.collections[&CollectionId::Liked]),
            [1, 3, 4, 5]
        );
        // Bağlantı yokken sahte olmayan kuyruk hiçbir şey göndermez (işleme başlamaz).
        let mut real = rig_without_fake();
        real.engine.wire.processing = false;
        real.engine.process_edits();
        assert!(
            !real.engine.wire.processing,
            "bağlantısız gerçek kuyruk işlenmemeli"
        );
    }

    fn rig_without_fake() -> Rig {
        rig(None)
    }

    #[test]
    fn written_edit_updates_the_cache_only_outside_test_runs() {
        // Gerçek (sahte olmayan) yolda Spotify'a ulaşan işlem önbelleğe işlenir.
        let mut rig = rig(None);
        drain(&rig);
        let op = EditOp::Unsave { uri: row(2).uri };
        let report = ProcessReport {
            done: vec![(1, EditOutcome::Done)],
            done_ops: vec![op],
            ..ProcessReport::default()
        };
        rig.engine.wire.processing = true;
        rig.engine
            .on_internal(Internal::Wire(WireInternal::Processed(Ok(report))));
        assert_eq!(db_uris(&rig.db, &CollectionId::Liked), [1, 3, 4, 5]);
        assert_eq!(rig.db.load_library().unwrap().unwrap().liked_count, 4);
    }

    #[test]
    fn unfinished_create_from_a_previous_session_opens_without_asking_spotify() {
        let mut rig = rig(Some(Fake::Slow(4)));
        drain(&rig);
        let temp = edits::new_temp_id();
        let queue = rig.engine.wire.queue.clone().unwrap();
        queue
            .enqueue(&EditOp::CreatePlaylist {
                temp_id: temp.clone(),
                name: "Yarım".into(),
                uris: vec![row(3).uri, row(4).uri],
                created: None,
            })
            .unwrap();
        // Kütüphanede geçici adresiyle görünür, açılınca şarkıları gelir.
        rig.engine.show_cached_library();
        let lib = lib_event(&drain(&rig)).unwrap();
        assert_eq!(lib.playlists[0].id, CollectionId::Playlist(temp.clone()));
        let id = CollectionId::Playlist(temp);
        rig.engine.open_collection(id.clone());
        assert_eq!(loaded(&drain(&rig), &id), Some(vec![3, 4]));
    }

    #[test]
    fn edits_left_from_a_previous_session_show_on_the_cached_data() {
        // Önceki oturumdan kalan beğeni: motorun belleğinde şarkı bilgisi yok,
        // önbellekten alınır; açılışta Spotify'dan önceki hâl değil, bekleyenli hâl görünür.
        let mut rig = rig(Some(Fake::Slow(4)));
        drain(&rig);
        let queue = rig.engine.wire.queue.clone().unwrap();
        rig.db.save_tracks(&[row(8)]).unwrap();
        queue.enqueue(&EditOp::Save { uri: row(8).uri }).unwrap();
        queue.enqueue(&EditOp::Unsave { uri: row(1).uri }).unwrap();
        queue.enqueue(&EditOp::Unsave { uri: row(2).uri }).unwrap();
        rig.engine.collections.clear();
        rig.engine.open_collection(CollectionId::Liked);
        assert_eq!(
            loaded(&drain(&rig), &CollectionId::Liked),
            Some(vec![8, 3, 4, 5])
        );
        rig.engine.show_cached_library();
        // Beğenilen sayısı: 5 + 1 - 2.
        assert_eq!(lib_event(&drain(&rig)).unwrap().liked_count, 4);
    }

    // ── Sanatçı takibi, motor düzeyinde ─────────────────────────────────

    fn send_follow(rig: &mut Rig, n: u32, name: &str, on: bool) {
        let op = if on {
            EditOp::Follow { uri: art(n) }
        } else {
            EditOp::Unfollow { uri: art(n) }
        };
        rig.engine.on_library_command(LibraryCommand::Edit {
            op,
            rows: vec![],
            item: Some(art_card(n, name)),
        });
    }

    fn artist_names(lib: &Library) -> Vec<&str> {
        lib.artists.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn follow_shows_at_once_and_a_refresh_before_the_write_does_not_take_it_back() {
        let mut rig = rig(Some(Fake::Slow(4)));
        drain(&rig);
        send_follow(&mut rig, 1, "Tarkan", true);
        // Anında kütüphanede; yazma henüz Spotify'a gitmedi (bekliyor).
        let ev = drain(&rig);
        assert_eq!(
            artist_names(rig.engine.library.as_ref().expect("kütüphane")),
            ["Tarkan"]
        );
        assert_eq!(status(&ev).map(|s| s.0), Some(1));
        // Spotify'dan gelen taze veri takibi henüz bilmiyor: bekleyen işlem üstüne
        // uygulanır, sanatçı kaybolmaz.
        let refresh = Refresh {
            library: lib_with(5),
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        assert_eq!(
            artist_names(rig.engine.library.as_ref().unwrap()),
            ["Tarkan"]
        );
        // Aynı karta ikinci "takip et" kuyruğa ikinci kez girmez.
        send_follow(&mut rig, 1, "Tarkan", true);
        assert_eq!(
            rig.engine
                .wire
                .queue
                .as_ref()
                .unwrap()
                .pending()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn follow_then_undo_before_the_write_cancels_out() {
        let mut rig = rig(Some(Fake::Slow(4)));
        drain(&rig);
        send_follow(&mut rig, 1, "Tarkan", true);
        send_follow(&mut rig, 1, "Tarkan", false);
        let ev = drain(&rig);
        // Hiçbir şey yazılmayacak, ekran eski hâlde.
        assert!(rig.engine.library.as_ref().unwrap().artists.is_empty());
        assert_eq!(status(&ev).map(|s| s.0), Some(0));
        assert!(
            rig.engine
                .wire
                .queue
                .as_ref()
                .unwrap()
                .pending()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_follow_written_to_spotify_catches_the_cache_up_and_survives_a_refresh() {
        // Sınama dışı yol: Spotify'a ulaşan takip önbelleğe işlenir.
        let mut rig = rig(None);
        drain(&rig);
        let follow = EditOp::Follow { uri: art(2) };
        rig.engine.wire.rows.insert(
            7,
            EditRows {
                rows: vec![],
                item: Some(art_card(2, "Sezen")),
            },
        );
        let report = ProcessReport {
            done: vec![(7, EditOutcome::Done)],
            done_ops: vec![follow],
            ..ProcessReport::default()
        };
        rig.engine.wire.processing = true;
        rig.engine
            .on_internal(Internal::Wire(WireInternal::Processed(Ok(report))));
        let cached = rig.db.load_library().unwrap().unwrap();
        assert_eq!(artist_names(&cached), ["Sezen"]);
        // Önbelleğin geri kalanı değişmedi.
        assert_eq!((cached.liked_count, cached.playlists.len()), (5, 1));
        // Açılışta (önbellekten) sanatçı görünür.
        rig.engine.show_cached_library();
        assert_eq!(artist_names(&lib_event(&drain(&rig)).unwrap()), ["Sezen"]);
    }

    #[test]
    fn refused_follow_stays_visible_until_dismissed_then_the_card_goes_away() {
        let mut rig = rig(Some(Fake::Fail(FailKind::Forbidden)));
        drain(&rig);
        send_follow(&mut rig, 3, "Ajda", true);
        run_queue(&mut rig, Fake::Fail(FailKind::Forbidden));
        let ev = drain(&rig);
        let (pending, failed) = status(&ev).unwrap();
        assert_eq!((pending, failed.len()), (0, 1));
        assert_eq!(failed[0].kind, FailKind::Forbidden);
        assert_eq!(failed[0].op, EditOp::Follow { uri: art(3) });
        // Reddedilen takip iyimser hâlde kalır, yenileme onu silmez.
        let refresh = Refresh {
            library: lib_with(5),
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        assert_eq!(artist_names(rig.engine.library.as_ref().unwrap()), ["Ajda"]);
        // "Bırak": kart Spotify'daki son bilinen hâle döner (sanatçı yok).
        rig.engine
            .on_library_command(LibraryCommand::DismissEdit(failed[0].id));
        let ev = drain(&rig);
        assert!(lib_event(&ev).unwrap().artists.is_empty(), "{ev:?}");
        assert_eq!(status(&ev).map(|s| s.1.len()), Some(0));
    }

    #[test]
    fn unfollow_hides_the_card_and_a_refresh_does_not_bring_it_back() {
        let mut rig = rig(Some(Fake::Slow(4)));
        // Spotify'daki hâl: iki sanatçı.
        let mut truth = lib_with(5);
        truth.artists = vec![art_card(1, "Tarkan"), art_card(2, "Sezen")];
        rig.db.save_library(&truth).unwrap();
        rig.engine.show_cached_library();
        assert_eq!(
            artist_names(&lib_event(&drain(&rig)).unwrap()),
            ["Tarkan", "Sezen"]
        );
        send_follow(&mut rig, 1, "Tarkan", false);
        drain(&rig);
        assert_eq!(
            artist_names(rig.engine.library.as_ref().unwrap()),
            ["Sezen"]
        );
        let refresh = Refresh {
            library: truth,
            changed: vec![],
            liked_changed: false,
        };
        rig.engine.on_internal(Internal::Library(Ok(refresh)));
        assert_eq!(
            artist_names(rig.engine.library.as_ref().unwrap()),
            ["Sezen"]
        );
    }

    /// Ekran sınaması için tohum önbelleği: takip edilen sanatçılı bir kütüphane,
    /// bir sanatçı sayfası ve portre dosyaları. Kullanıcının gerçek önbelleğine
    /// DOKUNMAZ: klasör adı "tohum" içermeli. Elle:
    /// `OPENIFY_TEST_CACHE_DIR=<...\openify-tohum-x> cargo test --release seed_screenshot_cache -- --ignored`
    #[test]
    #[ignore = "ekran sınaması için tohum önbelleği yazar (OPENIFY_TEST_CACHE_DIR, adı 'tohum' içeren klasör)"]
    fn seed_screenshot_cache() {
        let dir =
            std::env::var_os("OPENIFY_TEST_CACHE_DIR").expect("OPENIFY_TEST_CACHE_DIR verilmeli");
        let dir = std::path::PathBuf::from(dir);
        assert!(
            dir.to_string_lossy().contains("tohum"),
            "yalnız adında 'tohum' geçen klasöre yazılır: {dir:?}"
        );
        std::fs::create_dir_all(&dir).unwrap();

        let id = |n: u32| format!("spotify:artist:seed{n:018}");
        let image = |n: u32| format!("https://i.scdn.co/image/seed-artist-{n}");
        let names = [
            "Aylin Kaya",
            "Nova Reyes",
            "The Midnight Orchard",
            "Selin Aras",
            "Kuzey Rüzgârı",
            "Mira & The Lanterns",
            "Deniz Altın",
            "Orchestre Philharmonique de Montpellier",
            "Ece Yıldız",
            "Lowtide",
            "Jonah Whitfield",
            "Ömer Çelik",
            // 13.: kütüphanede YOK (takip edilmeyen sanatçı sayfası için).
            "Elif Sönmez",
        ];
        // Yumuşak degrade portreler (ağsız; her sanatçıya ayrı ton).
        let portrait = |n: u32| {
            let hue = (n * 37 % 360) as f32;
            let hsv = |h: f32, s: f32, v: f32| {
                let c = v * s;
                let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
                let (r, g, b) = match (h % 360.) as u32 / 60 {
                    0 => (c, x, 0.),
                    1 => (x, c, 0.),
                    2 => (0., c, x),
                    3 => (0., x, c),
                    4 => (x, 0., c),
                    _ => (c, 0., x),
                };
                let m = v - c;
                [
                    ((r + m) * 255.) as u8,
                    ((g + m) * 255.) as u8,
                    ((b + m) * 255.) as u8,
                ]
            };
            let (a, b) = (hsv(hue, 0.55, 0.42), hsv(hue + 45., 0.45, 0.9));
            let img = image::RgbImage::from_fn(320, 320, |x, y| {
                // Üstte hafif "omuz" ışığı: portre gibi, düz degrade değil.
                let t = (x + y) as f32 / 640.;
                let (dx, dy) = (x as f32 - 160., y as f32 - 200.);
                let glow = (1. - (dx * dx + dy * dy).sqrt() / 190.).clamp(0., 1.);
                let mix = |i: usize| {
                    ((a[i] as f32 * (1. - t) + b[i] as f32 * t) * (0.75 + 0.35 * glow)).min(255.)
                        as u8
                };
                image::Rgb([mix(0), mix(1), mix(2)])
            });
            let mut out = Vec::new();
            img.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut out, 88,
            ))
            .unwrap();
            out
        };
        for n in 1..=names.len() as u32 {
            super::super::cache::write_cover(&image(n), &portrait(n));
        }

        let item =
            |n: u32| super::super::library::artist_item(&id(n), names[n as usize - 1], &image(n));
        let playlist = |n: u32, name: &str, subtitle: &str| LibraryItem {
            id: CollectionId::Playlist(format!("spotify:playlist:seed{n:018}")),
            name: name.into(),
            subtitle: subtitle.into(),
            image: String::new(),
        };
        let lib = Library {
            liked_count: 128,
            albums: vec![],
            playlists: vec![
                playlist(1, "Gece Yolculuğu", "24 şarkı"),
                playlist(2, "Pazar Sabahı", "18 şarkı"),
                playlist(3, "Daily Mix 1", "Spotify · 50 şarkı"),
            ],
            artists: (1..names.len() as u32).map(item).collect(),
            ..Default::default()
        };
        let db = Db::open_default().unwrap();
        db.save_library(&lib).unwrap();

        // Sanatçı sayfası (takip edilen ilk sanatçı + takip edilmeyen "Selin Aras"ın da sayfası).
        let track = |n: u32, name: &str| TrackRow {
            uri: format!("spotify:track:seed{n:018}"),
            name: name.into(),
            artists: names[0].into(),
            album: "Gece Radyosu".into(),
            duration_ms: 180_000 + n * 7_000,
            image: image(n),
        };
        let album = |n: u32, name: &str, year: u16, kind: pages::ReleaseKind| pages::AlbumCard {
            uri: format!("spotify:album:seed{n:018}"),
            name: name.into(),
            artists: names[0].into(),
            year,
            kind,
            image: image(n + 3),
        };
        let cards: Vec<pages::ArtistCard> = [4, 7, 9, 5]
            .iter()
            .map(|&n| pages::ArtistCard {
                uri: id(n),
                name: names[n as usize - 1].into(),
                image: image(n),
            })
            .collect();
        let page = |n: u32| {
            Arc::new(ArtistPage {
                uri: id(n),
                name: names[n as usize - 1].into(),
                image: image(n),
                popularity: 64,
                bio: String::new(),
                top_tracks: vec![
                    track(1, "Gece Radyosu"),
                    track(2, "Cam Kenarı"),
                    track(3, "Yağmurdan Sonra"),
                    track(4, "Lambalar"),
                    track(5, "Uzak Şehir"),
                ],
                albums: vec![
                    album(1, "Gece Radyosu", 2024, pages::ReleaseKind::Album),
                    album(2, "Kıyıdan Notlar", 2021, pages::ReleaseKind::Album),
                ],
                singles: vec![album(3, "Cam Kenarı", 2025, pages::ReleaseKind::Single)],
                compilations: vec![],
                appears_on: vec![],
                related: cards.clone(),
            })
        };
        let cache = PageCache::open_default().unwrap();
        for n in [1, 4, 13] {
            cache
                .put(&format!("artist:{}", id(n)), &page(n), now_ms())
                .unwrap();
        }
        println!(
            "tohum yazıldı: {dir:?}; sanatçı sayfaları: {} (takipte), {} (takipte), {} (takipte değil)",
            id(1),
            id(4),
            id(13)
        );
    }

    /// Sabitleme yerel: tabloya yazılır, motorun kopyası güncellenir, kuyruğa
    /// (Spotify'a giden yazmalara) hiçbir şey girmez; kaldırma tersine çevirir.
    #[test]
    fn pinning_is_local_and_survives_a_library_rewrite() {
        let mut rig = rig(None);
        let key = pl(1);
        rig.engine.on_library_command(LibraryCommand::Pin {
            key: key.clone(),
            pinned: true,
        });
        rig.engine.on_library_command(LibraryCommand::Pin {
            key: "liked".into(),
            pinned: true,
        });
        assert_eq!(
            rig.db.pinned_keys().unwrap(),
            [key.clone(), "liked".to_string()]
        );
        let mine = rig.engine.library.as_ref().unwrap().meta.pinned.clone();
        assert_eq!(mine, [key.clone(), "liked".to_string()]);
        assert!(
            rig.engine
                .wire
                .queue
                .as_ref()
                .unwrap()
                .pending()
                .unwrap()
                .is_empty(),
            "Spotify'a yazma yok"
        );
        // Kütüphane baştan yazılsa da (tazeleme) sabitlemeler yerinde.
        rig.db.save_library(&lib_with(5)).unwrap();
        assert_eq!(rig.db.load_library().unwrap().unwrap().meta.pinned, mine);
        rig.engine.on_library_command(LibraryCommand::Pin {
            key: key.clone(),
            pinned: false,
        });
        assert_eq!(rig.db.pinned_keys().unwrap(), ["liked"]);
        assert_eq!(rig.engine.library.as_ref().unwrap().meta.pinned, ["liked"]);
    }

    /// Eklenme anları önbellekten çıkar ve olay olarak gider; tarihi
    /// bilinmeyen şarkı haritada yok, listede olmayan liste cevapsız kalır.
    #[test]
    fn added_dates_are_read_from_the_cache() {
        let mut rig = rig(None);
        let entries = [
            super::super::library::Entry {
                track: row(1).uri,
                added_at: Some(1_700_000_000_000),
            },
            super::super::library::Entry::untimed(row(2).uri),
            super::super::library::Entry {
                track: row(3).uri,
                added_at: Some(1_600_000_000_000),
            },
            super::super::library::Entry {
                track: row(1).uri,
                added_at: Some(5),
            },
        ];
        let dates = added_dates_of(&entries);
        assert_eq!(dates.len(), 2);
        assert_eq!(dates[&row(1).uri], 1_700_000_000_000, "ilk satır");
        rig.db
            .save_collection_entries(&CollectionId::Playlist(pl(1)), Some(b"r"), &entries[..3])
            .unwrap();
        drain(&rig);
        rig.engine
            .on_library_command(LibraryCommand::AddedDates(CollectionId::Playlist(pl(1))));
        let got = drain(&rig).into_iter().find_map(|e| match e {
            Event::Library(LibraryEvent::AddedDates { id, dates }) => Some((id, dates)),
            _ => None,
        });
        let (id, dates) = got.expect("AddedDates olayı");
        assert_eq!(id, CollectionId::Playlist(pl(1)));
        assert_eq!(dates.len(), 2);
        rig.engine
            .on_library_command(LibraryCommand::AddedDates(CollectionId::Playlist(pl(99))));
        assert!(
            drain(&rig)
                .iter()
                .all(|e| !matches!(e, Event::Library(LibraryEvent::AddedDates { .. })))
        );
    }

    #[test]
    fn test_edit_modes_parse_and_default_to_the_fake_backend() {
        assert_eq!(
            parse_test_edits(None, false),
            None,
            "normal açılış: gerçek Spotify"
        );
        assert_eq!(
            parse_test_edits(None, true),
            Some(Fake::Ok),
            "sınama penceresi: sahte"
        );
        assert_eq!(parse_test_edits(Some("slow"), false), Some(Fake::Slow(4)));
        assert_eq!(
            parse_test_edits(Some("slow-15"), true),
            Some(Fake::Slow(15))
        );
        assert_eq!(parse_test_edits(Some("slow-x"), true), Some(Fake::Slow(4)));
        assert_eq!(parse_test_edits(Some("ok"), false), Some(Fake::Ok));
        assert_eq!(
            parse_test_edits(Some("fail"), false),
            Some(Fake::Fail(FailKind::Forbidden))
        );
        assert_eq!(
            parse_test_edits(Some("fail-notfound"), true),
            Some(Fake::Fail(FailKind::NotFound))
        );
        assert_eq!(
            parse_test_edits(Some("fail-invalid"), true),
            Some(Fake::Fail(FailKind::Invalid))
        );
        // Tanınmayan değer bile sahteye gider (yanlış yazım gerçeğe yazdırmaz).
        assert_eq!(parse_test_edits(Some("kaçırdım"), false), Some(Fake::Ok));
    }
}
