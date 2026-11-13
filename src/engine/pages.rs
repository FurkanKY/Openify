//! Sayfa verisi (Adım 12): sanatçı sayfası, albüm sayfası, ana sayfa.
//!
//! Spotify'a giden yol kütüphanenin zaten kullandığı yol: librespot'un
//! `extended-metadata` toplu isteği (ARTIST_V4 / ALBUM_V4 / TRACK_V4). Web API
//! ve GraphQL (pathfinder) gerekmedi. Bir sanatçı sayfası en çok üç tur istektir:
//!
//!   1. sanatçı            (tek istek)
//!   2. albümler + benzer sanatçılar + popüler şarkılar (üçü aynı anda;
//!      her biri tek toplu istek, şarkılar zaten önbellekteyse hiç gitmez)
//!
//! Önbellek (`PageCache`): sayfalar JSON olarak kendi SQLite dosyasında
//! (`pages.db`, kütüphane önbelleğinden ayrı: onun şema yükseltmesi sayfaları
//! silmez) ve en çok `MEMORY_CAP` tanesi bellekte `Arc` olarak durur. Bellekteki
//! sayfa çözümleme bile istemeden döner; diskteki sayfa birkaç yüz mikrosaniyede.
//!
//! Tazelik: Spotify'ın bu verilerde sürüm numarası yok; bu yüzden TTL.
//!   - sanatçı: 6 saat taze, sonra "bayat" (yine anında gösterilir, arkadan tazelenir)
//!   - albüm:   7 gün taze
//!   - 60 günden eski sayfa hiç döndürülmez (ve `prune` siler)
//!     Bayat sayfa gösterilirken arka planda `load_*` çağrılır; içerik aynıysa
//!     `changed == false` döner ve arayüz yeniden çizmez (zıplama yok).

use std::{
    any::Any,
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use librespot::{
    core::{SpotifyUri, session::Session},
    metadata::{Album, Artist, Metadata, album::AlbumType, image::ImageSize},
    protocol::{
        extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery},
        extension_kind::ExtensionKind,
        metadata::{Album as AlbumMessage, Artist as ArtistMessage},
    },
};
use protobuf::Message;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{
    cache::{self, Db},
    conns::Conns,
    library::{
        CollectionId, Library, LibraryItem, TrackRow, fetch_tracks, image_url, join_artists,
        pick_cover, with_retry,
    },
    migrate::{self, Step},
};

/// Tek toplu istekte istenen en çok varlık.
const BATCH: usize = 100;
/// Sanatçı sayfasında grup başına en çok sürüm (hepsi ~100 lük tek istekte).
const MAX_ALBUMS: usize = 50;
const MAX_SINGLES: usize = 40;
const MAX_COMPILATIONS: usize = 10;
const MAX_APPEARS_ON: usize = 12;
const MAX_RELATED: usize = 20;
const TOP_TRACKS: usize = 10;

/// Sayfa bu kadar sürece "taze" (arka planda yeniden isteme).
const ARTIST_FRESH_MS: i64 = 6 * 3600 * 1000;
const ALBUM_FRESH_MS: i64 = 7 * 24 * 3600 * 1000;
/// Dinlenme sayıları (`stats.rs`) bir gün taze.
const STATS_FRESH_MS: i64 = super::stats::STATS_FRESH_MS;
/// Bundan eski sayfa hiç gösterilmez.
const HARD_EXPIRE_MS: i64 = 60 * 24 * 3600 * 1000;
/// Bellekte tutulan en çok sayfa.
const MEMORY_CAP: usize = 64;
/// Sayfa biçimi değişirse artır: eski satırlar yok sayılıp silinir.
const PAGE_FORMAT: i64 = 1;

// ── Sayfa türleri ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseKind {
    Album,
    Single,
    Ep,
    Compilation,
    /// "Şunlarda da var": sanatçının başkasının albümünde geçtiği yerler.
    AppearsOn,
}

impl ReleaseKind {
    fn from_album_type(t: AlbumType) -> Self {
        match t {
            AlbumType::SINGLE => Self::Single,
            AlbumType::EP => Self::Ep,
            AlbumType::COMPILATION => Self::Compilation,
            _ => Self::Album,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumCard {
    pub uri: String,
    pub name: String,
    /// "Sanatçı A, Sanatçı B"
    pub artists: String,
    /// Yayın yılı; bilinmiyorsa 0.
    pub year: u16,
    pub kind: ReleaseKind,
    /// Kapak (300 px); yoksa boş.
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistCard {
    pub uri: String,
    pub name: String,
    /// Portre; yoksa boş (arayüz yer tutucu çizer).
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistPage {
    pub uri: String,
    pub name: String,
    pub image: String,
    /// Spotify'ın 0..100 popülerlik puanı.
    pub popularity: u8,
    /// Etiketsiz düz metin; yoksa boş.
    pub bio: String,
    pub top_tracks: Vec<TrackRow>,
    pub albums: Vec<AlbumCard>,
    pub singles: Vec<AlbumCard>,
    pub compilations: Vec<AlbumCard>,
    pub appears_on: Vec<AlbumCard>,
    pub related: Vec<ArtistCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumPage {
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistCard>,
    pub year: u16,
    pub kind: ReleaseKind,
    pub label: String,
    /// Büyük kapak (640 px); yoksa boş.
    pub image: String,
    pub tracks: Vec<TrackRow>,
    /// Şarkı sürelerinin toplamı.
    pub total_ms: u64,
}

/// Ana sayfa: ağsız, kütüphane + yerel geçmişten anında kurulur.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomePage {
    /// Son çalınan bağlamlar, en yeni önde.
    pub recent: Vec<LibraryItem>,
    /// Spotify'ın senin için hazırladığı listeler (Mix'ler, Discover Weekly...).
    pub made_for_you: Vec<LibraryItem>,
    pub liked_count: usize,
}

/// Önbellekten dönen sayfa.
#[derive(Debug)]
pub struct Cached<T> {
    pub page: Arc<T>,
    /// Sayfanın kaç ms önce indirildiği.
    pub age_ms: i64,
    /// Taze değil: göster, ama arkadan `load_*` ile tazele.
    pub stale: bool,
}

/// `load_*` sonucu.
#[derive(Debug)]
pub struct Loaded<T> {
    pub page: Arc<T>,
    /// Önbellektekinden farklı mı (arayüz yalnız o zaman yeniden çizer).
    pub changed: bool,
    /// Sayfanın bütün şarkı bilgileri geldi mi; gelmediyse önbelleğe yazılmadı.
    pub complete: bool,
}

// ── Önbellek ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Artist,
    Album,
    /// Benzer sanatçılar ve Deezer eşlemeleri (`related.rs`; kendi tazelik
    /// kuralları orada, burada yalnız 7 günlük "taze" sınırı).
    Related,
    /// Keşfet ve Ana sayfa verisi (`explore.rs`; tazelik kuralları orada).
    Explore,
    /// Dinlenme sayıları (`stats.rs`): bir gün taze, sonra arkadan tazelenir.
    Stats,
}

impl Kind {
    fn fresh_ms(self) -> i64 {
        match self {
            Self::Artist => ARTIST_FRESH_MS,
            Self::Album => ALBUM_FRESH_MS,
            Self::Related => super::related::FRESH_MS,
            Self::Explore => super::explore::STALE_MAX_MS,
            Self::Stats => STATS_FRESH_MS,
        }
    }
}

struct MemEntry {
    fetched_ms: i64,
    payload_len: usize,
    value: Arc<dyn Any + Send + Sync>,
}

/// Sayfa önbelleği şeması (`migrate.rs`): bütün veri yeniden indirilebilir, bu
/// yüzden bir göç gerekirse tabloyu silmek de meşrudur; şimdilik tek adım var.
///   1  temel şema (sürümsüz eski dosyalar da bu adımda devralınır)
const MIGRATIONS: [Step; 1] = [Step {
    to: 1,
    name: "baseline",
    up: baseline,
}];

fn baseline(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS pages (
             key        TEXT PRIMARY KEY,
             format     INTEGER NOT NULL,
             fetched_ms INTEGER NOT NULL,
             payload    TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS track_links (
             uri     TEXT PRIMARY KEY,
             payload TEXT NOT NULL
         );",
    )
}

/// Sayfa önbelleği (`pages.db`): tek yazıcı + okuma havuzu (bkz. `conns.rs`),
/// önünde bellek önbelleği.
pub struct PageCache {
    conns: Conns,
    memory: Mutex<HashMap<String, MemEntry>>,
}

impl PageCache {
    pub fn open_default() -> Result<Self> {
        let dir = cache::cache_dir().context("Önbellek klasörü bulunamadı")?;
        fs::create_dir_all(&dir)?;
        Self::open(&dir.join("pages.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        let writer =
            Self::init_writer(Connection::open(path).context("Sayfa önbelleği açılamadı")?)?;
        Ok(Self::with(Conns::file(path, writer)))
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self::with(Conns::memory(Self::init_writer(
            Connection::open_in_memory()?,
        )?)))
    }

    fn init_writer(mut conn: Connection) -> Result<Connection> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate::migrate(&mut conn, &MIGRATIONS)
            .context("Sayfa önbelleği şeması yükseltilemedi")?;
        // Biçimi eski satırlar bir daha okunmaz (satırın kendi `format`ı var:
        // şema sürümünden bağımsız, sayfa türü değişince `PAGE_FORMAT` artar).
        conn.execute("DELETE FROM pages WHERE format != ?1", [PAGE_FORMAT])?;
        Ok(conn)
    }

    fn with(conns: Conns) -> Self {
        Self {
            conns,
            memory: Mutex::new(HashMap::new()),
        }
    }

    /// Yazıcı bağlantı.
    fn w(&self) -> MutexGuard<'_, Connection> {
        self.conns.w()
    }

    /// Okuma bağlantısı (yazıcıyı beklemez).
    fn r(&self) -> MutexGuard<'_, Connection> {
        self.conns.r()
    }

    fn mem(&self) -> std::sync::MutexGuard<'_, HashMap<String, MemEntry>> {
        self.memory.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn get<T>(&self, kind: Kind, key: &str, now_ms: i64) -> Result<Option<Cached<T>>>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        let hit = |fetched_ms: i64, page: Arc<T>| {
            let age_ms = (now_ms - fetched_ms).max(0);
            (age_ms < HARD_EXPIRE_MS).then_some(Cached {
                page,
                age_ms,
                stale: age_ms >= kind.fresh_ms(),
            })
        };
        // Bellek: çözümleme yok.
        if let Some(e) = self.mem().get(key)
            && let Ok(page) = e.value.clone().downcast::<T>()
        {
            return Ok(hit(e.fetched_ms, page));
        }
        let row: Option<(i64, String)> = self
            .r()
            .query_row(
                "SELECT fetched_ms, payload FROM pages WHERE key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((fetched_ms, payload)) = row else {
            return Ok(None);
        };
        let Ok(page) = serde_json::from_str::<T>(&payload) else {
            // Bozuk satır: yok say (bir sonraki yazış üzerine yazar).
            return Ok(None);
        };
        let page = Arc::new(page);
        self.remember(key, fetched_ms, payload.len(), page.clone());
        Ok(hit(fetched_ms, page))
    }

    fn remember<T: Send + Sync + 'static>(
        &self,
        key: &str,
        fetched_ms: i64,
        payload_len: usize,
        page: Arc<T>,
    ) {
        let mut mem = self.mem();
        if mem.len() >= MEMORY_CAP
            && !mem.contains_key(key)
            && let Some(oldest) = mem
                .iter()
                .min_by_key(|(_, e)| e.fetched_ms)
                .map(|(k, _)| k.clone())
        {
            mem.remove(&oldest);
        }
        mem.insert(
            key.to_string(),
            MemEntry {
                fetched_ms,
                payload_len,
                value: page,
            },
        );
    }

    /// Sayfayı yazar; önceki içerikten farklı mı döner.
    pub(super) fn put<T>(&self, key: &str, page: &Arc<T>, now_ms: i64) -> Result<bool>
    where
        T: Serialize + Send + Sync + 'static,
    {
        let payload = serde_json::to_string(page.as_ref())?;
        // Eski içeriği okuma ve yazma aynı yazıcı kilidinde (arada başka yazma girmez).
        let changed = {
            let conn = self.w();
            let previous: Option<String> = conn
                .query_row("SELECT payload FROM pages WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?;
            conn.execute(
                "INSERT OR REPLACE INTO pages (key, format, fetched_ms, payload)
                 VALUES (?1, ?2, ?3, ?4)",
                params![key, PAGE_FORMAT, now_ms, payload],
            )?;
            previous.as_deref() != Some(payload.as_str())
        };
        self.remember(key, now_ms, payload.len(), page.clone());
        Ok(changed)
    }

    /// Anahtarı `prefix` ile başlayan satırların anahtarları (bellek dışı:
    /// diskteki satırlar; önbellekte ne var diye ağsız bakmak için).
    pub(super) fn keys_with_prefix(&self, prefix: &str) -> Vec<String> {
        let conn = self.r();
        let Ok(mut stmt) =
            conn.prepare("SELECT key FROM pages WHERE substr(key, 1, length(?1)) = ?1")
        else {
            return Vec::new();
        };
        stmt.query_map([prefix], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    #[allow(dead_code)] // elle tazeleme için; henüz arayüzde yok
    pub fn invalidate(&self, key: &str) -> Result<()> {
        self.mem().remove(key);
        self.w()
            .execute("DELETE FROM pages WHERE key = ?1", [key])?;
        Ok(())
    }

    /// Çok eski sayfaları siler; silinen sayısı döner.
    pub fn prune(&self) -> Result<usize> {
        self.prune_at(now_ms())
    }

    fn prune_at(&self, now_ms: i64) -> Result<usize> {
        let cutoff = now_ms - HARD_EXPIRE_MS;
        self.mem().retain(|_, e| e.fetched_ms > cutoff);
        Ok(self
            .w()
            .execute("DELETE FROM pages WHERE fetched_ms <= ?1", [cutoff])?)
    }

    /// Bellekteki sayfaların JSON boyutu toplamı (bellek ölçümü için).
    #[allow(dead_code)] // bellek ölçümü için
    pub fn memory_bytes(&self) -> usize {
        self.mem().values().map(|e| e.payload_len).sum()
    }
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn artist_key(uri: &str) -> String {
    format!("artist:{uri}")
}

fn album_key(uri: &str) -> String {
    format!("album:{uri}")
}

/// Sanatçı sayfası önbellekteyse (bayat olsa da) anında verir.
pub fn cached_artist(cache: &PageCache, uri: &str) -> Result<Option<Cached<ArtistPage>>> {
    cache.get(Kind::Artist, &artist_key(uri), now_ms())
}

pub fn cached_album(cache: &PageCache, uri: &str) -> Result<Option<Cached<AlbumPage>>> {
    cache.get(Kind::Album, &album_key(uri), now_ms())
}

// ── Ana sayfa (ağsız) ───────────────────────────────────────────────────

/// Ana sayfayı kütüphaneden ve yerel çalma geçmişinden (`Settings::recent`,
/// en yeni önde) kurar. Ağ yok: anında. Kütüphanede bulunmayan bağlamlar
/// (silinmiş liste, kütüphanede olmayan albüm) atlanır.
pub fn home(library: &Library, recent_keys: &[String], limit: usize) -> HomePage {
    let mut recent = Vec::new();
    for key in recent_keys {
        if recent.len() >= limit {
            break;
        }
        let Some(id) = CollectionId::from_key(key) else {
            continue;
        };
        let item = match &id {
            CollectionId::Liked => Some(LibraryItem {
                id: CollectionId::Liked,
                name: crate::t!(LikedSongs).into(),
                subtitle: crate::i18n::canon::songs(library.liked_count),
                image: String::new(),
            }),
            CollectionId::Playlist(_) => library.playlists.iter().find(|p| p.id == id).cloned(),
            CollectionId::Album(_) => library.albums.iter().find(|a| a.id == id).cloned(),
            CollectionId::Smart(_) => None,
            // Takip edilen sanatçı bir şarkı listesi değil: "son çalınan" kartı olmaz.
            CollectionId::Artist(_) => None,
        };
        if let Some(item) = item {
            recent.push(item);
        }
    }
    HomePage {
        recent,
        made_for_you: library
            .playlists
            .iter()
            .filter(|p| is_spotify_made(p))
            .cloned()
            .collect(),
        liked_count: library.liked_count,
    }
}

/// Spotify'ın kendi hazırladığı liste mi? `library.rs` bunların açıklamasını
/// "Spotify · N şarkı" diye yazıyor (sahibi "spotify" olanlar).
pub fn is_spotify_made(item: &LibraryItem) -> bool {
    matches!(item.id, CollectionId::Playlist(_)) && item.subtitle.starts_with("Spotify ·")
}

// ── Sanatçı sayfası ─────────────────────────────────────────────────────

/// Sanatçı sayfasını Spotify'dan indirir ve önbelleğe yazar.
pub async fn load_artist(
    session: &Session,
    db: &Db,
    cache: &PageCache,
    uri: &str,
) -> Result<Loaded<ArtistPage>> {
    let started = std::time::Instant::now();
    let spotify_uri = SpotifyUri::from_uri(uri)?;
    if !matches!(spotify_uri, SpotifyUri::Artist { .. }) {
        return Err(anyhow!("Sanatçı adresi değil: {uri}"));
    }
    let one = [uri.to_string()];
    let mut found =
        with_retry(|| fetch_extended::<ArtistMessage>(session, ExtensionKind::ARTIST_V4, &one))
            .await
            .context("Sanatçı alınamadı")?;
    let (_, message) = found
        .pop()
        .ok_or_else(|| anyhow!("Sanatçı bulunamadı: {uri}"))?;
    let artist = Artist::parse(&message, &spotify_uri)?;

    let plan = ArtistPlan::new(&artist, &session.country());
    let (top, releases, related) = futures::join!(
        top_tracks(session, db, &plan.top_uris),
        fetch_cards(session, &plan.release_uris),
        fetch_artist_cards(session, &plan.related_uris),
    );
    let (top_tracks, complete) = top;
    let releases = releases.unwrap_or_else(|e| {
        log::warn!("{uri}: albümler alınamadı: {e:#}");
        HashMap::new()
    });
    let related = related.unwrap_or_else(|e| {
        log::warn!("{uri}: benzer sanatçılar alınamadı: {e:#}");
        Vec::new()
    });

    let page = Arc::new(assemble_artist(
        &artist, &plan, top_tracks, &releases, related,
    ));
    let complete = complete && (plan.release_uris.is_empty() || !releases.is_empty());
    let changed = if complete {
        cache.put(&artist_key(uri), &page, now_ms())?
    } else {
        true
    };
    log::debug!(
        "sanatçı sayfası {uri}: {} ms",
        started.elapsed().as_millis()
    );
    Ok(Loaded {
        page,
        changed,
        complete,
    })
}

/// Hangi adreslerin isteneceği (Spotify'ın sanatçı mesajından).
#[derive(Debug, Default)]
struct ArtistPlan {
    top_uris: Vec<String>,
    /// (adres, grup) ; grup sayfadaki yerini belirler.
    release_uris: Vec<String>,
    groups: HashMap<String, ReleaseGroup>,
    related_uris: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseGroup {
    Albums,
    Singles,
    Compilations,
    AppearsOn,
}

impl ArtistPlan {
    fn new(artist: &Artist, country: &str) -> Self {
        let mut plan = Self {
            top_uris: artist
                .top_tracks
                .for_country(country)
                .iter()
                .take(TOP_TRACKS)
                .filter_map(|u| u.to_uri().ok())
                .collect(),
            ..Self::default()
        };
        let collect = |it: &mut dyn Iterator<Item = &SpotifyUri>| -> Vec<String> {
            it.filter_map(|u| u.to_uri().ok()).collect()
        };
        let groups = [
            (
                collect(&mut artist.albums_current()),
                ReleaseGroup::Albums,
                MAX_ALBUMS,
            ),
            (
                collect(&mut artist.singles_current()),
                ReleaseGroup::Singles,
                MAX_SINGLES,
            ),
            (
                collect(&mut artist.compilations_current()),
                ReleaseGroup::Compilations,
                MAX_COMPILATIONS,
            ),
            (
                collect(&mut artist.appears_on_albums_current()),
                ReleaseGroup::AppearsOn,
                MAX_APPEARS_ON,
            ),
        ];
        for (uris, group, cap) in groups {
            let mut taken = 0;
            for uri in uris {
                if taken >= cap {
                    break;
                }
                // Aynı sürüm iki grupta geçiyorsa bir kez istenir.
                if plan.groups.contains_key(&uri) {
                    continue;
                }
                plan.groups.insert(uri.clone(), group);
                plan.release_uris.push(uri);
                taken += 1;
            }
        }
        plan.related_uris = artist
            .related
            .iter()
            .take(MAX_RELATED)
            .filter_map(|a| a.id.to_uri().ok())
            .collect();
        plan
    }
}

/// Spotify mesajı + indirilen kartlardan sayfayı kurar (saf: testlenir).
fn assemble_artist(
    artist: &Artist,
    plan: &ArtistPlan,
    top_tracks: Vec<TrackRow>,
    releases: &HashMap<String, AlbumCard>,
    related: Vec<ArtistCard>,
) -> ArtistPage {
    let mut page = ArtistPage {
        uri: artist.id.to_uri().unwrap_or_default(),
        name: artist.name.clone(),
        image: artist_portrait(artist),
        popularity: artist.popularity.clamp(0, 100) as u8,
        bio: artist
            .biographies
            .first()
            .map(|b| strip_html(&b.text))
            .unwrap_or_default(),
        top_tracks,
        albums: Vec::new(),
        singles: Vec::new(),
        compilations: Vec::new(),
        appears_on: Vec::new(),
        related,
    };
    for uri in &plan.release_uris {
        let (Some(card), Some(group)) = (releases.get(uri), plan.groups.get(uri)) else {
            continue;
        };
        let mut card = card.clone();
        let list = match group {
            ReleaseGroup::Albums => &mut page.albums,
            ReleaseGroup::Singles => &mut page.singles,
            ReleaseGroup::Compilations => &mut page.compilations,
            ReleaseGroup::AppearsOn => {
                card.kind = ReleaseKind::AppearsOn;
                &mut page.appears_on
            }
        };
        list.push(card);
    }
    // Yeniden yayınlar ve karışık sıra için en yeni önde (aynı yılda ad sırası).
    for list in [&mut page.albums, &mut page.singles, &mut page.compilations] {
        sort_releases(list);
    }
    page
}

/// En yeni yıl önde; aynı yılda sıra korunur (kararlı sıralama).
fn sort_releases(list: &mut [AlbumCard]) {
    list.sort_by_key(|a| std::cmp::Reverse(a.year));
}

pub(super) fn artist_portrait(artist: &Artist) -> String {
    let big = artist
        .portrait_group
        .iter()
        .find(|i| i.size == ImageSize::LARGE)
        .or_else(|| artist.portrait_group.first())
        .or_else(|| artist.portraits.iter().find(|i| i.size == ImageSize::LARGE))
        .or_else(|| artist.portraits.first());
    big.map(|i| image_url(&i.id.0)).unwrap_or_default()
}

/// Popüler şarkılar: önce SQLite şarkı önbelleği, eksikler tek toplu istekte.
/// İkinci değer: hepsinin bilgisi geldi mi.
async fn top_tracks(session: &Session, db: &Db, uris: &[String]) -> (Vec<TrackRow>, bool) {
    match rows_for(session, db, uris).await {
        Ok(rows) => {
            let complete = rows.len() == uris.len();
            (rows, complete)
        }
        Err(e) => {
            log::warn!("şarkı bilgileri alınamadı: {e:#}");
            (Vec::new(), uris.is_empty())
        }
    }
}

/// Verilen sırada şarkı satırları; önbellekte olmayanlar Spotify'dan alınır
/// ve `tracks` tablosuna yazılır (kütüphane aramasıyla paylaşılır).
async fn rows_for(session: &Session, db: &Db, uris: &[String]) -> Result<Vec<TrackRow>> {
    let mut found = db.tracks(uris)?;
    let missing: Vec<String> = uris
        .iter()
        .filter(|u| !found.contains_key(*u))
        .cloned()
        .collect();
    if !missing.is_empty() {
        let fetched = fetch_tracks(session, missing).await;
        db.save_tracks(&fetched)?;
        found.extend(fetched.into_iter().map(|r| (r.uri.clone(), r)));
    }
    Ok(uris.iter().filter_map(|u| found.remove(u)).collect())
}

async fn fetch_cards(session: &Session, uris: &[String]) -> Result<HashMap<String, AlbumCard>> {
    let mut cards = HashMap::new();
    for chunk in uris.chunks(BATCH) {
        let found =
            with_retry(|| fetch_extended::<AlbumMessage>(session, ExtensionKind::ALBUM_V4, chunk))
                .await?;
        for (uri, message) in found {
            let Ok(spotify_uri) = SpotifyUri::from_uri(&uri) else {
                continue;
            };
            match Album::parse(&message, &spotify_uri) {
                Ok(album) => {
                    cards.insert(uri.clone(), album_card(&uri, &album));
                }
                Err(e) => log::warn!("{uri} çözülemedi: {e}"),
            }
        }
    }
    Ok(cards)
}

fn album_card(uri: &str, album: &Album) -> AlbumCard {
    AlbumCard {
        uri: uri.to_string(),
        name: album.name.clone(),
        artists: join_artists(album.artists.iter().map(|a| a.name.as_str())),
        year: album_year(album),
        kind: ReleaseKind::from_album_type(album.album_type),
        image: pick_cover(&album.covers, ImageSize::DEFAULT),
    }
}

fn album_year(album: &Album) -> u16 {
    album.date.year().clamp(0, u16::MAX as i32) as u16
}

pub(super) async fn fetch_artist_cards(
    session: &Session,
    uris: &[String],
) -> Result<Vec<ArtistCard>> {
    if uris.is_empty() {
        return Ok(Vec::new());
    }
    let found =
        with_retry(|| fetch_extended::<ArtistMessage>(session, ExtensionKind::ARTIST_V4, uris))
            .await?;
    let mut by_uri: HashMap<String, ArtistCard> = HashMap::new();
    for (uri, message) in found {
        let Ok(spotify_uri) = SpotifyUri::from_uri(&uri) else {
            continue;
        };
        if let Ok(artist) = Artist::parse(&message, &spotify_uri) {
            by_uri.insert(
                uri.clone(),
                ArtistCard {
                    uri,
                    name: artist.name.clone(),
                    image: artist_portrait(&artist),
                },
            );
        }
    }
    // Spotify'ın "benzerlik" sırası korunur.
    Ok(uris.iter().filter_map(|u| by_uri.remove(u)).collect())
}

// ── Albüm sayfası ───────────────────────────────────────────────────────

pub async fn load_album(
    session: &Session,
    db: &Db,
    cache: &PageCache,
    uri: &str,
) -> Result<Loaded<AlbumPage>> {
    let started = std::time::Instant::now();
    let spotify_uri = SpotifyUri::from_uri(uri)?;
    if !matches!(spotify_uri, SpotifyUri::Album { .. }) {
        return Err(anyhow!("Albüm adresi değil: {uri}"));
    }
    let one = [uri.to_string()];
    let mut found =
        with_retry(|| fetch_extended::<AlbumMessage>(session, ExtensionKind::ALBUM_V4, &one))
            .await
            .context("Albüm alınamadı")?;
    let (_, message) = found
        .pop()
        .ok_or_else(|| anyhow!("Albüm bulunamadı: {uri}"))?;
    let album = Album::parse(&message, &spotify_uri)?;

    let track_uris: Vec<String> = album.tracks().filter_map(|u| u.to_uri().ok()).collect();
    let tracks = rows_for(session, db, &track_uris).await?;
    let complete = tracks.len() == track_uris.len();

    let page = Arc::new(assemble_album(uri, &album, tracks));
    let changed = if complete {
        cache.put(&album_key(uri), &page, now_ms())?
    } else {
        true
    };
    log::debug!("albüm sayfası {uri}: {} ms", started.elapsed().as_millis());
    Ok(Loaded {
        page,
        changed,
        complete,
    })
}

fn assemble_album(uri: &str, album: &Album, tracks: Vec<TrackRow>) -> AlbumPage {
    AlbumPage {
        uri: uri.to_string(),
        name: album.name.clone(),
        artists: album
            .artists
            .iter()
            .filter_map(|a| {
                Some(ArtistCard {
                    uri: a.id.to_uri().ok()?,
                    name: a.name.clone(),
                    image: String::new(),
                })
            })
            .collect(),
        year: album_year(album),
        kind: ReleaseKind::from_album_type(album.album_type),
        label: album.label.clone(),
        image: pick_cover(&album.covers, ImageSize::LARGE),
        total_ms: total_ms(&tracks),
        tracks,
    }
}

fn total_ms(tracks: &[TrackRow]) -> u64 {
    tracks.iter().map(|t| u64::from(t.duration_ms)).sum()
}

/// "47 dk" ya da "1 sa 12 dk" (Spotify'ın albüm üst bilgisindeki biçim).
#[cfg_attr(not(test), allow(dead_code))] // arayüz kendi biçimini kullanıyor (library::fmt_total)
pub fn duration_label(ms: u64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    if minutes >= 60 {
        format!("{} sa {} dk", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} dk")
    }
}

// ── Toplu istek ─────────────────────────────────────────────────────────

/// `extended-metadata` toplu isteği: verilen türde, adres başına bir mesaj.
/// Sıra korunmaz; bilgisi gelmeyen ya da çözülemeyen adresler atlanır.
pub(super) async fn fetch_extended<M: Message>(
    session: &Session,
    kind: ExtensionKind,
    uris: &[String],
) -> Result<Vec<(String, M)>> {
    let request = BatchedEntityRequest {
        entity_request: uris
            .iter()
            .map(|uri| EntityRequest {
                entity_uri: uri.clone(),
                query: vec![ExtensionQuery {
                    extension_kind: kind.into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let response = session.spclient().get_extended_metadata(request).await?;
    let mut out = Vec::new();
    for array in &response.extended_metadata {
        for data in &array.extension_data {
            let Some(any) = data.extension_data.as_ref() else {
                continue;
            };
            if let Ok(message) = M::parse_from_bytes(&any.value) {
                out.push((data.entity_uri.clone(), message));
            }
        }
    }
    Ok(out)
}

// ── Şarkıdan sanatçıya / albüme bağlar ──────────────────────────────────
//
// Şarkı satırı (`TrackRow`) yalnız adları taşıyor; satırdaki sanatçı ve
// albüm adlarının tıklanabilmesi için adresleri gerekir. Adresler ayrı
// tutulur (şarkı önbelleğinin şeması değişmesin, binlerce şarkı yeniden
// indirilmesin): görünen satırlar için toplu istekle (100'lük) bir kez
// alınır, `pages.db`'de kalıcıdır.

/// Bir şarkının albümü ve sanatçıları (adres, ad), Spotify'daki sırayla.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackLinks {
    pub uri: String,
    pub album: Option<(String, String)>,
    pub artists: Vec<(String, String)>,
}

impl PageCache {
    /// Önbellekteki bağlar (bulunanlar).
    pub fn links(&self, uris: &[String]) -> Vec<TrackLinks> {
        let conn = self.r();
        let Ok(mut stmt) = conn.prepare_cached("SELECT payload FROM track_links WHERE uri = ?1")
        else {
            return Vec::new();
        };
        uris.iter()
            .filter_map(|uri| {
                let payload: String = stmt.query_row([uri], |r| r.get(0)).ok()?;
                serde_json::from_str(&payload).ok()
            })
            .collect()
    }

    pub fn put_links(&self, links: &[TrackLinks]) -> Result<()> {
        let mut conn = self.w();
        let tx = conn.transaction()?;
        {
            let mut insert =
                tx.prepare("INSERT OR REPLACE INTO track_links (uri, payload) VALUES (?1, ?2)")?;
            for l in links {
                insert.execute(params![l.uri, serde_json::to_string(l)?])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// Şarkıların bağlarını Spotify'dan toplu istekle alır ve önbelleğe yazar.
pub async fn load_links(
    session: &Session,
    cache: &PageCache,
    uris: &[String],
) -> Result<Vec<TrackLinks>> {
    use librespot::{metadata::Track, protocol::metadata::Track as TrackMessage};
    let mut out = Vec::new();
    for batch in uris.chunks(BATCH) {
        let found =
            with_retry(|| fetch_extended::<TrackMessage>(session, ExtensionKind::TRACK_V4, batch))
                .await?;
        for (uri, message) in found {
            let Ok(parsed) = SpotifyUri::from_uri(&uri) else {
                continue;
            };
            let Ok(track) = Track::parse(&message, &parsed) else {
                continue;
            };
            out.push(TrackLinks {
                album: track
                    .album
                    .id
                    .to_uri()
                    .ok()
                    .map(|a| (a, track.album.name.clone())),
                artists: track
                    .artists
                    .iter()
                    .filter_map(|a| Some((a.id.to_uri().ok()?, a.name.clone())))
                    .collect(),
                uri,
            });
        }
    }
    cache.put_links(&out)?;
    Ok(out)
}

// ── Küçük yardımcılar ───────────────────────────────────────────────────

/// Biyografi metnindeki etiketleri ve varlıkları atar ("<a href=..>X</a>" → "X").
fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot::protocol::metadata::{Album as AlbumMsg, Biography, Date, Image, ImageGroup};

    fn row(uri: &str, ms: u32) -> TrackRow {
        TrackRow {
            uri: uri.into(),
            name: "Ad".into(),
            artists: "Sanatçı".into(),
            album: "Albüm".into(),
            duration_ms: ms,
            image: "https://i.scdn.co/image/aa".into(),
            plays: None,
        }
    }

    fn card(name: &str, year: u16, kind: ReleaseKind) -> AlbumCard {
        AlbumCard {
            uri: format!("spotify:album:{name}"),
            name: name.into(),
            artists: "A".into(),
            year,
            kind,
            image: String::new(),
        }
    }

    fn sample_page(tracks: usize) -> ArtistPage {
        ArtistPage {
            uri: "spotify:artist:a".into(),
            name: "Sanatçı".into(),
            image: "https://i.scdn.co/image/bb".into(),
            popularity: 70,
            bio: "Biyografi ".repeat(40),
            top_tracks: (0..10)
                .map(|i| row(&format!("spotify:track:{i}"), 200_000))
                .collect(),
            albums: (0..tracks)
                .map(|i| card(&format!("a{i}"), 2000 + (i % 24) as u16, ReleaseKind::Album))
                .collect(),
            singles: (0..tracks)
                .map(|i| card(&format!("s{i}"), 2010, ReleaseKind::Single))
                .collect(),
            compilations: vec![],
            appears_on: vec![],
            related: (0..20)
                .map(|i| ArtistCard {
                    uri: format!("spotify:artist:r{i}"),
                    name: format!("Benzer {i}"),
                    image: "https://i.scdn.co/image/cc".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn page_cache_roundtrip_and_change_detection() {
        let cache = PageCache::open_in_memory().unwrap();
        let page = Arc::new(sample_page(20));
        assert!(
            cache
                .put(&artist_key("spotify:artist:a"), &page, 1_000)
                .unwrap()
        );
        // Aynı içerik tekrar yazılınca "değişmedi".
        assert!(
            !cache
                .put(&artist_key("spotify:artist:a"), &page, 2_000)
                .unwrap()
        );
        let mut other = sample_page(20);
        other.name = "Yeni ad".into();
        assert!(
            cache
                .put(&artist_key("spotify:artist:a"), &Arc::new(other), 3_000)
                .unwrap()
        );
    }

    #[test]
    fn freshness_and_expiry() {
        let cache = PageCache::open_in_memory().unwrap();
        let key = artist_key("spotify:artist:a");
        cache.put(&key, &Arc::new(sample_page(3)), 0).unwrap();
        let get = |now| cache.get::<ArtistPage>(Kind::Artist, &key, now).unwrap();
        assert!(!get(ARTIST_FRESH_MS - 1).unwrap().stale);
        assert!(get(ARTIST_FRESH_MS).unwrap().stale);
        assert!(get(HARD_EXPIRE_MS - 1).is_some());
        assert!(get(HARD_EXPIRE_MS).is_none());
        // Albüm daha uzun taze.
        let akey = album_key("spotify:album:x");
        let album = Arc::new(AlbumPage {
            uri: "spotify:album:x".into(),
            name: "x".into(),
            artists: vec![],
            year: 1999,
            kind: ReleaseKind::Album,
            label: String::new(),
            image: String::new(),
            tracks: vec![],
            total_ms: 0,
        });
        cache.put(&akey, &album, 0).unwrap();
        assert!(
            !cache
                .get::<AlbumPage>(Kind::Album, &akey, ARTIST_FRESH_MS * 2)
                .unwrap()
                .unwrap()
                .stale
        );
        assert_eq!(cache.prune_at(HARD_EXPIRE_MS).unwrap(), 2);
        assert!(
            cache
                .get::<AlbumPage>(Kind::Album, &akey, 0)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn disk_hit_survives_memory_loss_and_old_format_is_dropped() {
        let dir = std::env::temp_dir().join(format!("openify-pages-{}", now_ms()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pages.db");
        let key = artist_key("spotify:artist:a");
        {
            let cache = PageCache::open(&path).unwrap();
            cache.put(&key, &Arc::new(sample_page(5)), 10).unwrap();
        }
        let cache = PageCache::open(&path).unwrap();
        let hit = cache
            .get::<ArtistPage>(Kind::Artist, &key, 20)
            .unwrap()
            .unwrap();
        assert_eq!(hit.page.albums.len(), 5);
        assert_eq!(hit.age_ms, 10);
        // Eski biçim satırı açılışta silinir.
        cache
            .w()
            .execute("UPDATE pages SET format = 0", [])
            .unwrap();
        drop(cache);
        let cache = PageCache::open(&path).unwrap();
        assert!(
            cache
                .get::<ArtistPage>(Kind::Artist, &key, 20)
                .unwrap()
                .is_none()
        );
        drop(cache);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_layer_is_capped_and_evicts_oldest() {
        let cache = PageCache::open_in_memory().unwrap();
        for i in 0..(MEMORY_CAP + 5) {
            let mut p = sample_page(1);
            p.uri = format!("spotify:artist:{i}");
            cache
                .put(&artist_key(&p.uri), &Arc::new(p.clone()), i as i64)
                .unwrap();
        }
        assert_eq!(cache.mem().len(), MEMORY_CAP);
        // En eski bellekten düştü ama diskten gelir.
        let first = artist_key("spotify:artist:0");
        assert!(!cache.mem().contains_key(&first));
        assert!(
            cache
                .get::<ArtistPage>(Kind::Artist, &first, 100)
                .unwrap()
                .is_some()
        );
    }

    /// Çıta: önbellekteki sayfa bir karede (16 ms'nin çok altında).
    /// Gerçekçi büyük sayfa: 100 sürüm + 20 benzer sanatçı + 10 şarkı.
    #[test]
    fn cached_page_opens_well_within_one_frame() {
        let cache = PageCache::open_in_memory().unwrap();
        let key = artist_key("spotify:artist:a");
        cache.put(&key, &Arc::new(sample_page(50)), 0).unwrap();
        // Bellek isabeti.
        let t = std::time::Instant::now();
        let hit = cache
            .get::<ArtistPage>(Kind::Artist, &key, 1)
            .unwrap()
            .unwrap();
        let mem_us = t.elapsed().as_micros();
        assert_eq!(hit.page.albums.len(), 50);
        // Disk isabeti (bellek boşken): JSON çözümlemesi dahil.
        cache.mem().clear();
        let t = std::time::Instant::now();
        let hit = cache
            .get::<ArtistPage>(Kind::Artist, &key, 1)
            .unwrap()
            .unwrap();
        let disk_us = t.elapsed().as_micros();
        assert_eq!(hit.page.singles.len(), 50);
        eprintln!("sayfa önbelleği: bellek {mem_us} µs, disk {disk_us} µs");
        assert!(mem_us < 1_000, "bellek isabeti {mem_us} µs");
        assert!(disk_us < 5_000, "disk isabeti {disk_us} µs");
    }

    #[test]
    fn corrupt_row_is_ignored() {
        let cache = PageCache::open_in_memory().unwrap();
        cache
            .w()
            .execute(
                "INSERT INTO pages (key, format, fetched_ms, payload) VALUES ('artist:x', ?1, 0, '{bozuk')",
                [PAGE_FORMAT],
            )
            .unwrap();
        assert!(
            cache
                .get::<ArtistPage>(Kind::Artist, "artist:x", 1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn strip_html_removes_tags_and_entities() {
        assert_eq!(
            strip_html("<p>Rock &amp; <a href=\"spotify:artist:1\">Roll</a>&nbsp;grubu</p>\n"),
            "Rock & Roll grubu"
        );
        assert_eq!(strip_html("düz metin"), "düz metin");
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn duration_labels() {
        assert_eq!(duration_label(47 * 60_000 + 10_000), "47 dk");
        assert_eq!(duration_label(72 * 60_000), "1 sa 12 dk");
        assert_eq!(duration_label(59 * 60_000 + 40_000), "1 sa 0 dk");
        assert_eq!(duration_label(0), "0 dk");
        assert_eq!(total_ms(&[row("a", 1000), row("b", 2500)]), 3500);
    }

    #[test]
    fn release_kind_mapping_and_sorting() {
        assert_eq!(ReleaseKind::from_album_type(AlbumType::EP), ReleaseKind::Ep);
        assert_eq!(
            ReleaseKind::from_album_type(AlbumType::SINGLE),
            ReleaseKind::Single
        );
        assert_eq!(
            ReleaseKind::from_album_type(AlbumType::COMPILATION),
            ReleaseKind::Compilation
        );
        assert_eq!(
            ReleaseKind::from_album_type(AlbumType::ALBUM),
            ReleaseKind::Album
        );
        let mut list = vec![
            card("eski", 2001, ReleaseKind::Album),
            card("yeni", 2020, ReleaseKind::Album),
            card("orta-a", 2010, ReleaseKind::Album),
            card("orta-b", 2010, ReleaseKind::Album),
        ];
        sort_releases(&mut list);
        let names: Vec<_> = list.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["yeni", "orta-a", "orta-b", "eski"]);
    }

    fn library_fixture() -> Library {
        let item = |id: CollectionId, name: &str, subtitle: &str| LibraryItem {
            id,
            name: name.into(),
            subtitle: subtitle.into(),
            image: String::new(),
        };
        Library {
            liked_count: 42,
            albums: vec![item(
                CollectionId::Album("spotify:album:1".into()),
                "Albüm 1",
                "Sanatçı",
            )],
            playlists: vec![
                item(
                    CollectionId::Playlist("spotify:playlist:mine".into()),
                    "Benim",
                    "12 şarkı",
                ),
                item(
                    CollectionId::Playlist("spotify:playlist:mix".into()),
                    "Daily Mix 1",
                    "Spotify · 50 şarkı",
                ),
                item(
                    CollectionId::Playlist("spotify:playlist:other".into()),
                    "Arkadaş",
                    "Takip edilen · 5 şarkı",
                ),
            ],
            artists: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn home_resolves_recent_in_order_and_finds_mixes() {
        let lib = library_fixture();
        let recent: Vec<String> = [
            "spotify:playlist:mix",
            "silinmis-anahtar",
            "liked",
            "spotify:playlist:yok",
            "spotify:album:1",
            "spotify:playlist:mine",
        ]
        .map(String::from)
        .into();
        let page = home(&lib, &recent, 10);
        let names: Vec<_> = page.recent.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Daily Mix 1", "Beğenilenler", "Albüm 1", "Benim"]);
        assert_eq!(page.recent[1].subtitle, "42 şarkı");
        assert_eq!(page.made_for_you.len(), 1);
        assert_eq!(page.made_for_you[0].name, "Daily Mix 1");
        assert_eq!(page.liked_count, 42);
        // Sınır uygulanır.
        assert_eq!(home(&lib, &recent, 2).recent.len(), 2);
        // Boş kütüphane/geçmiş çökmez.
        assert_eq!(home(&Library::default(), &[], 5), HomePage::default());
    }

    fn gid(n: u8) -> Vec<u8> {
        vec![n; 16]
    }

    fn artist_message(n: u8, name: &str) -> ArtistMessage {
        ArtistMessage {
            gid: Some(gid(n)),
            name: Some(name.into()),
            popularity: Some(55),
            ..Default::default()
        }
    }

    /// Sahte bir Spotify sanatçı mesajının sayfaya doğru dönüşmesi
    /// (ağ yok): gruplar, EP/tekli ayrımı, sıralama, biyografi temizliği.
    #[test]
    fn artist_message_assembles_into_page() {
        use librespot::protocol::metadata::AlbumGroup;
        let album_uri = |n: u8| AlbumMsg {
            gid: Some(gid(n)),
            ..Default::default()
        };
        let mut msg = artist_message(1, "Deneme Sanatçı");
        msg.album_group.push(AlbumGroup {
            album: vec![album_uri(10)],
            ..Default::default()
        });
        msg.single_group.push(AlbumGroup {
            album: vec![album_uri(11)],
            ..Default::default()
        });
        msg.appears_on_group.push(AlbumGroup {
            album: vec![album_uri(12)],
            ..Default::default()
        });
        msg.related.push(artist_message(2, "Benzer"));
        msg.biography.push(Biography {
            text: Some("<b>Kalın</b> yazı &amp; bilgi".into()),
            ..Default::default()
        });
        msg.portrait_group = Some(ImageGroup {
            image: vec![Image {
                file_id: Some(vec![0xab; 20]),
                size: Some(ImageSize::LARGE.into()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .into();
        let uri = SpotifyUri::from_uri("spotify:artist:0000000000000000000001").ok();
        let artist = Artist::parse(&msg, &uri.unwrap_or_else(|| unreachable!())).unwrap();

        let plan = ArtistPlan::new(&artist, "TR");
        assert_eq!(plan.release_uris.len(), 3);
        assert_eq!(plan.related_uris.len(), 1);
        let mut releases = HashMap::new();
        for (uri, year, kind) in [
            (&plan.release_uris[0], 2019, ReleaseKind::Album),
            (&plan.release_uris[1], 2021, ReleaseKind::Ep),
            (&plan.release_uris[2], 2022, ReleaseKind::Album),
        ] {
            releases.insert(
                uri.clone(),
                AlbumCard {
                    uri: uri.clone(),
                    name: uri.clone(),
                    artists: "A".into(),
                    year,
                    kind,
                    image: String::new(),
                },
            );
        }
        let page = assemble_artist(
            &artist,
            &plan,
            vec![row("spotify:track:1", 1000)],
            &releases,
            vec![],
        );
        assert_eq!(page.name, "Deneme Sanatçı");
        assert_eq!(page.popularity, 55);
        assert_eq!(page.bio, "Kalın yazı & bilgi");
        assert!(page.image.starts_with("https://i.scdn.co/image/abab"));
        assert_eq!(page.albums.len(), 1);
        assert_eq!(page.singles.len(), 1);
        assert_eq!(page.singles[0].kind, ReleaseKind::Ep);
        assert_eq!(page.appears_on.len(), 1);
        assert_eq!(page.appears_on[0].kind, ReleaseKind::AppearsOn);
        assert_eq!(page.top_tracks.len(), 1);

        // Albüm mesajı → albüm sayfası.
        let album_msg = AlbumMsg {
            gid: Some(gid(20)),
            name: Some("Albüm Adı".into()),
            label: Some("Plak Şirketi".into()),
            date: Some(Date {
                year: Some(2018),
                month: Some(5),
                day: Some(4),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        let album = Album::parse(
            &album_msg,
            &SpotifyUri::from_uri("spotify:album:0000000000000000000002").unwrap(),
        )
        .unwrap();
        let apage = assemble_album(
            "spotify:album:0000000000000000000002",
            &album,
            vec![
                row("spotify:track:1", 60_000),
                row("spotify:track:2", 120_000),
            ],
        );
        assert_eq!(apage.year, 2018);
        assert_eq!(apage.name, "Albüm Adı");
        assert_eq!(apage.label, "Plak Şirketi");
        assert_eq!(apage.total_ms, 180_000);
        assert_eq!(apage.kind, ReleaseKind::Album);
    }

    #[test]
    fn plan_dedups_and_caps_each_group() {
        use librespot::protocol::metadata::AlbumGroup;
        let mut msg = artist_message(1, "X");
        // Aynı albüm hem albüm hem tekli grubunda: bir kez istenir.
        let a = |n: u8| AlbumMsg {
            gid: Some(gid(n)),
            ..Default::default()
        };
        msg.album_group.push(AlbumGroup {
            album: vec![a(1)],
            ..Default::default()
        });
        msg.single_group.push(AlbumGroup {
            album: vec![a(1)],
            ..Default::default()
        });
        // "Şunlarda da var" 12 ile sınırlı.
        for n in 100..130 {
            msg.appears_on_group.push(AlbumGroup {
                album: vec![a(n)],
                ..Default::default()
            });
        }
        let artist = Artist::parse(
            &msg,
            &SpotifyUri::from_uri("spotify:artist:0000000000000000000001").unwrap(),
        )
        .unwrap();
        let plan = ArtistPlan::new(&artist, "TR");
        let appears = plan
            .groups
            .values()
            .filter(|g| **g == ReleaseGroup::AppearsOn)
            .count();
        assert_eq!(appears, MAX_APPEARS_ON);
        assert_eq!(plan.release_uris.len(), 1 + MAX_APPEARS_ON);
    }

    // ── Şema göçü ───────────────────────────────────────────────────────

    fn temp_db_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openify-pages-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Sürümlenmemiş (göçten önceki) dosya: tablolar var, `user_version` 0.
    fn legacy_pages_file(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE pages (key TEXT PRIMARY KEY, format INTEGER NOT NULL, fetched_ms INTEGER NOT NULL, payload TEXT NOT NULL);
             CREATE TABLE track_links (uri TEXT PRIMARY KEY, payload TEXT NOT NULL);
             INSERT INTO track_links VALUES ('spotify:track:a', '{\"uri\":\"spotify:track:a\",\"album\":null,\"artists\":[]}');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pages VALUES ('artist:old', 0, 1, '{}'), ('artist:keep', ?1, 1, '{bozuk')",
            [PAGE_FORMAT],
        )
        .unwrap();
    }

    #[test]
    fn unversioned_and_fresh_files_reach_the_latest_schema_and_keep_their_rows() {
        let dir = temp_db_dir("migrate");
        let fresh = PageCache::open(&dir.join("fresh.db")).unwrap();
        assert_eq!(migrate::user_version(&fresh.w()).unwrap(), 1);
        let want = migrate::schema_columns(&fresh.w());
        assert_eq!(want["pages"], ["key", "format", "fetched_ms", "payload"]);

        let path = dir.join("legacy.db");
        legacy_pages_file(&path);
        let cache = PageCache::open(&path).unwrap();
        assert_eq!(migrate::user_version(&cache.w()).unwrap(), 1);
        assert_eq!(migrate::schema_columns(&cache.w()), want);
        // Eski biçimli sayfa silindi (her açılışta olduğu gibi), geri kalan veri durur.
        assert_eq!(cache.keys_with_prefix("artist:"), ["artist:keep"]);
        assert_eq!(cache.links(&["spotify:track:a".to_string()]).len(), 1);
        drop(cache);
        // İkinci açılış zararsız.
        assert!(PageCache::open(&path).is_ok());

        // Daha yeni uygulamanın dosyası açılmaz, dokunulmaz.
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 7)
            .unwrap();
        let err = PageCache::open(&path)
            .err()
            .expect("daha yeni sürüm açılmamalı");
        assert!(format!("{err:#}").contains("version 7"), "{err:#}");
        assert_eq!(
            migrate::user_version(&Connection::open(&path).unwrap()).unwrap(),
            7
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_reads_go_through_the_read_connection_and_see_committed_writes() {
        let dir = temp_db_dir("readers");
        let cache = Arc::new(PageCache::open(&dir.join("pages.db")).unwrap());
        let key = artist_key("spotify:artist:r");
        cache.put(&key, &Arc::new(sample_page(2)), 10).unwrap();
        // Belleği atla: disk okumasını (okuma bağlantısından) zorla.
        cache.mem().clear();
        // Yazıcı açık bir yazma işleminde takılıyken disk okuması beklemez.
        let writer = cache.w();
        writer
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO track_links VALUES ('x', '{}');")
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = Arc::clone(&cache);
        let key2 = key.clone();
        std::thread::spawn(move || {
            let hit = reader
                .get::<ArtistPage>(Kind::Artist, &key2, 20)
                .unwrap()
                .map(|c| c.page.albums.len());
            let _ = tx.send(hit);
        });
        let hit = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("okuma yazıcıyı beklememeli");
        assert_eq!(hit, Some(2));
        writer.execute_batch("COMMIT").unwrap();
        drop(writer);
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Canlı okuma sınaması (elle: `cargo test --release live_ -- --ignored --nocapture`).
/// Yalnızca OKUR: bir sanatçı ve bir albüm sayfası (birkaç istek).
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn live_artist_and_album_pages() {
        let session = crate::auth::connect(None).await.expect("oturum");
        let db = Db::open_in_memory().unwrap();
        let cache = PageCache::open_in_memory().unwrap();

        let t = std::time::Instant::now();
        let artist = load_artist(
            &session,
            &db,
            &cache,
            "spotify:artist:4tZwfgrHOc3mvqYlEYSvVi",
        )
        .await
        .expect("sanatçı");
        eprintln!("SANATÇI soğuk: {} ms", t.elapsed().as_millis());
        let p = &artist.page;
        eprintln!(
            "{} | pop {} | bio {} harf | şarkı {} | albüm {} | tekli {} | derleme {} | şunlarda {} | benzer {} | tam {}",
            p.name,
            p.popularity,
            p.bio.len(),
            p.top_tracks.len(),
            p.albums.len(),
            p.singles.len(),
            p.compilations.len(),
            p.appears_on.len(),
            p.related.len(),
            artist.complete
        );
        eprintln!("ilk albüm: {:?}", p.albums.first());
        eprintln!("ilk şarkı: {:?}", p.top_tracks.first());
        eprintln!("ilk benzer: {:?}", p.related.first());
        eprintln!("görsel: {}", p.image);

        // Sıcak oturumda ikinci sanatçı (ilk çağrıdaki belirteç/bağlantı kurulumu hariç).
        let t = std::time::Instant::now();
        let second = load_artist(
            &session,
            &db,
            &cache,
            "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
        )
        .await
        .expect("ikinci sanatçı");
        eprintln!(
            "SANATÇI-2 sıcak: {} ms | {} | albüm {} | tekli {} | benzer {} | şarkı {}",
            t.elapsed().as_millis(),
            second.page.name,
            second.page.albums.len(),
            second.page.singles.len(),
            second.page.related.len(),
            second.page.top_tracks.len()
        );

        let t = std::time::Instant::now();
        let cached = cached_artist(&cache, "spotify:artist:4tZwfgrHOc3mvqYlEYSvVi")
            .unwrap()
            .unwrap();
        eprintln!(
            "SANATÇI önbellek: {} µs (bayat: {})",
            t.elapsed().as_micros(),
            cached.stale
        );

        let t = std::time::Instant::now();
        let album = load_album(
            &session,
            &db,
            &cache,
            "spotify:album:4m2880jivSbbyEGAKfITCa",
        )
        .await
        .expect("albüm");
        eprintln!("ALBÜM soğuk: {} ms", t.elapsed().as_millis());
        let a = &album.page;
        eprintln!(
            "{} | {} | {} | {} şarkı | {} | {:?} | tam {}",
            a.name,
            a.year,
            a.label,
            a.tracks.len(),
            duration_label(a.total_ms),
            a.artists.first(),
            album.complete
        );
    }
}
