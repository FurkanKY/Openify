//! Dinlenme sayıları: sanatçının aylık dinleyicisi, dünya sıralaması, en çok
//! dinlendiği şehirler ve şarkı başına dinlenme sayısı (albüm sayfası sütunu).
//!
//! Sayfa verisi başka bir uçtan geliyor (librespot `extended-metadata`); bu
//! sayılar yalnız Spotify'ın GraphQL ağ geçidinde var, bu yüzden ayrı ve
//! **isteğe bağlı** bir istekle alınır (`pathfinder.rs`):
//!
//!   - `queryArtistOverview` → `data.artistUnion.stats.monthlyListeners`,
//!     `followers`, `worldRank`, `topCities.items[]` ve
//!     `discography.topTracks.items[].track.playcount`;
//!   - `getAlbum` → `data.albumUnion.tracksV2.items[].track.playcount`
//!     (dinlenme sayısı burada **yazı** gelir).
//!
//! Sayfa açılışı başına en çok bir ek istek: sanatçıda tek sorgu hem sayıları
//! hem popüler şarkıların dinlenmesini getirir; albümde tek sorgu. Sayılar
//! gelmezse sayfa bugünkü gibi çalışır (iskelet yok, hata söylenmez; sayılar
//! gelince sessizce belirir).
//!
//! Önbellek `pages.db` (`stats:artist:<adres>`, `stats:album:<adres>`), 24 saat
//! taze; daha eskisi gösterilir ama arkadan tazelenir (`pages::Kind::Stats`).
//!
//! Ayrıştırma eksik alana ve şema değişikliğine toleranslıdır: alan adları
//! (`monthlyListeners` / `monthly_listeners`, `topCities` / `cities`,
//! `tracksV2` / `tracks`, `playcount` / `plays`) sırayla denenir, sayı hem sayı
//! hem yazı olarak okunur, tanınmayan öğe atlanır.
//!
//! Sınama kancası: `OPENIFY_TEST_STATS_JSON=<klasör ya da dosya>` (yalnız
//! sınama koşusunda) sayfaları ağa çıkmadan fikstürden doldurur. Klasörde
//! `artist_overview_stats.json` ve `album_playcounts.json` beklenir; tek dosya
//! verilirse ikisi için de o okunur.

use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use librespot::core::session::Session;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    cache::Db,
    pages::{Cached, Kind, PageCache, now_ms},
    pathfinder::{self, Operation, PathfinderError, Priority},
};

/// Sayılar bu kadar süre taze (yeniden istenmez).
pub const STATS_FRESH_MS: i64 = 24 * 3600 * 1000;
/// En çok kaç şehir gösterilir.
pub const MAX_CITIES: usize = 5;
/// Albüm sorgusunda istenen en çok şarkı.
const ALBUM_TRACK_LIMIT: u32 = 100;
/// Şehir adının en çok harfi.
const CITY_MAX: usize = 64;

/// Sanatçı sayfasının dinlenme sayıları.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistStats {
    pub monthly_listeners: u64,
    /// Takipçi sayısı (şimdilik gösterilmiyor; canlı sınamada alan doluluğu).
    pub followers: u64,
    /// Dünya sıralaması; yoksa `None`.
    pub world_rank: Option<u64>,
    /// En çok dinlendiği şehirler (Spotify'ın sırasıyla, en çok 5).
    pub top_cities: Vec<CityPlays>,
    /// Popüler şarkıların dinlenme sayısı: şarkı adresi → sayı.
    pub track_plays: BTreeMap<String, u64>,
}

impl ArtistStats {
    /// Hiçbir sayı okunamadı mı? (Önbelleğe yazılmaz, arayüz bir şey çizmez.)
    pub fn is_empty(&self) -> bool {
        self.monthly_listeners == 0
            && self.followers == 0
            && self.world_rank.is_none()
            && self.top_cities.is_empty()
            && self.track_plays.is_empty()
    }
}

/// Bir şehrin dinlenme sayısı.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CityPlays {
    pub city: String,
    pub plays: u64,
}

/// Albüm sayfasının dinlenme sayıları: şarkı adresi → sayı.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumPlaycounts {
    pub track_plays: BTreeMap<String, u64>,
}

impl AlbumPlaycounts {
    pub fn is_empty(&self) -> bool {
        self.track_plays.is_empty()
    }
}

/// Sayılar alınamadı (ayrıntı günlükte; arayüz sessiz kalır).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsFailure {
    Offline,
    RateLimited,
    Denied,
    Unavailable,
}

impl From<&PathfinderError> for StatsFailure {
    fn from(e: &PathfinderError) -> Self {
        match e {
            PathfinderError::RateLimited { .. }
            | PathfinderError::Blocked { .. }
            | PathfinderError::CircuitOpen => Self::RateLimited,
            PathfinderError::Denied { .. } => Self::Denied,
            PathfinderError::Http { status: 404 | 410 } | PathfinderError::InvalidResponse(_) => {
                Self::Unavailable
            }
            PathfinderError::Token(_)
            | PathfinderError::Transport(_)
            | PathfinderError::Http { .. } => Self::Offline,
            _ => Self::Unavailable,
        }
    }
}

// ── Sorgular ────────────────────────────────────────────────────────────

/// Sanatçı sayfasının özeti: aylık dinleyici, takipçi, dünya sıralaması,
/// şehirler ve popüler şarkıların dinlenmesi (tek istek).
pub const ARTIST_OVERVIEW: Operation = Operation {
    name: "queryArtistOverview",
    chunk: "xpui-routes-artist",
    meta_key: "hash:queryArtistOverview",
    // Hash ilk istekte web player'dan bulunur ve kalıcılaşır.
    seed: None,
    localized: false,
    strict_root: None,
};

/// Albümün şarkıları ve dinlenme sayıları (tek istek).
pub const ALBUM_TRACKS: Operation = Operation {
    name: "getAlbum",
    chunk: "xpui-routes-album",
    meta_key: "hash:getAlbum",
    seed: None,
    localized: false,
    strict_root: None,
};

fn artist_variables(uri: &str) -> Value {
    json!({ "uri": uri, "locale": "", "includePrerelease": true })
}

fn album_variables(uri: &str) -> Value {
    json!({ "uri": uri, "locale": "", "offset": 0, "limit": ALBUM_TRACK_LIMIT })
}

// ── Önbellek ────────────────────────────────────────────────────────────

fn artist_key(uri: &str) -> String {
    format!("stats:artist:{uri}")
}

fn album_key(uri: &str) -> String {
    format!("stats:album:{uri}")
}

/// Önbellekteki sanatçı sayıları (bayat olsa da) ve yaşı.
pub fn cached_artist(cache: &PageCache, uri: &str) -> Option<Cached<ArtistStats>> {
    cache.get(Kind::Stats, &artist_key(uri), now_ms()).ok()?
}

/// Önbellekteki albüm sayıları (bayat olsa da) ve yaşı.
pub fn cached_album(cache: &PageCache, uri: &str) -> Option<Cached<AlbumPlaycounts>> {
    cache.get(Kind::Stats, &album_key(uri), now_ms()).ok()?
}

// ── Ayrıştırma ──────────────────────────────────────────────────────────

/// `data` varsa içine iner; yoksa kök.
fn data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

/// GraphQL kökü (`artistUnion` / `albumUnion`); yoksa kökün kendisi.
fn union<'a>(value: &'a Value, name: &str) -> &'a Value {
    let root = data(value);
    root.get(name).unwrap_or(root)
}

/// Sayı: JSON sayısı, ondalık ya da gruplanmış yazı ("1.234.567") olabilir.
fn count(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    if let Some(f) = value.as_f64() {
        return (f >= 0.).then_some(f.round() as u64);
    }
    let text = value.as_str()?;
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty())
        .then(|| digits.parse::<u64>().ok())
        .flatten()
}

/// Kısa yazı: denetim karakterleri atılır, kırpılır, en çok `max` harf.
fn text(value: Option<&Value>, max: usize) -> Option<String> {
    let s = value?.as_str()?;
    let s: String = s.chars().filter(|c| !c.is_control()).take(max).collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Sayıların ilk bulunduğu alan (şema değişikliğine tolerans).
fn count_field(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| count(value.get(*k)))
}

/// Bir kaptaki `items` dizisi (dizi doğrudan da gelebilir).
fn items(value: &Value) -> &[Value] {
    if let Some(list) = value.as_array() {
        return list;
    }
    value
        .get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Şarkı öğelerinden dinlenme sayıları: `items[].track.uri` + `playcount`.
fn parse_track_plays(containers: &[Option<&Value>]) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for container in containers.iter().flatten() {
        for item in items(container) {
            // Öğe ya `{ track: {...} }` ya da doğrudan şarkı nesnesi.
            let track = item.get("track").unwrap_or(item);
            let Some(uri) = track.get("uri").and_then(Value::as_str) else {
                continue;
            };
            if !uri.starts_with("spotify:track:") {
                continue;
            }
            if let Some(n) = count_field(track, &["playcount", "plays", "playCount"]) {
                out.entry(uri.to_string()).or_insert(n);
            }
        }
    }
    out
}

fn parse_cities(container: Option<&Value>) -> Vec<CityPlays> {
    let Some(container) = container else {
        return Vec::new();
    };
    let mut out: Vec<CityPlays> = Vec::new();
    for item in items(container) {
        let city = text(item.get("city"), CITY_MAX)
            .or_else(|| text(item.get("cityName"), CITY_MAX))
            .or_else(|| text(item.get("name"), CITY_MAX))
            .or_else(|| item.get("city").and_then(|c| text(c.get("name"), CITY_MAX)));
        let Some(city) = city else { continue };
        if out.iter().any(|c| c.city == city) {
            continue;
        }
        let plays = count_field(
            item,
            &["numberOfListeners", "listeners", "plays", "playcount"],
        )
        .unwrap_or(0);
        out.push(CityPlays { city, plays });
        if out.len() >= MAX_CITIES {
            break;
        }
    }
    out
}

/// `queryArtistOverview` yanıtı → sanatçı sayıları.
pub fn parse_artist(value: &Value) -> ArtistStats {
    let artist = union(value, "artistUnion");
    let stats = artist.get("stats").unwrap_or(artist);
    let top_tracks = artist.get("discography").and_then(|d| d.get("topTracks"));
    ArtistStats {
        monthly_listeners: count_field(stats, &["monthlyListeners", "monthly_listeners"])
            .or_else(|| count_field(artist, &["monthlyListeners", "monthly_listeners"]))
            .unwrap_or(0),
        followers: count_field(stats, &["followers", "followerCount"])
            .or_else(|| count_field(artist, &["followers", "followerCount"]))
            .unwrap_or(0),
        world_rank: count_field(stats, &["worldRank", "world_rank"])
            .or_else(|| count_field(artist, &["worldRank", "world_rank"]))
            .filter(|r| *r > 0),
        top_cities: parse_cities(
            stats
                .get("topCities")
                .or_else(|| stats.get("cities"))
                .or_else(|| artist.get("topCities")),
        ),
        track_plays: parse_track_plays(&[
            top_tracks,
            artist.get("topTracks"),
            stats.get("topTracks"),
        ]),
    }
}

/// `getAlbum` yanıtı → albümün şarkı dinlenme sayıları.
pub fn parse_album(value: &Value) -> AlbumPlaycounts {
    let album = union(value, "albumUnion");
    AlbumPlaycounts {
        track_plays: parse_track_plays(&[
            album.get("tracksV2"),
            album.get("tracks"),
            album.get("trackList"),
        ]),
    }
}

// ── Sınama fikstürü ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fixture {
    Artist,
    Album,
}

/// Sınama fikstürü açık mı? (`OPENIFY_TEST_STATS_JSON`, yalnız sınama koşusu.)
/// Açıksa önbellek okunmaz: sayfalar her koşuda fikstürden dolar.
pub fn fixture_active() -> bool {
    crate::crash::is_test_run() && std::env::var_os("OPENIFY_TEST_STATS_JSON").is_some()
}

/// Sınama fikstüründen sanatçı sayıları (ağa çıkmaz, oturum istemez).
pub fn fixture_artist() -> Option<Arc<ArtistStats>> {
    fixture(Fixture::Artist).map(|v| Arc::new(v.map(|v| parse_artist(&v)).unwrap_or_default()))
}

/// Sınama fikstüründen albüm sayıları (ağa çıkmaz, oturum istemez).
pub fn fixture_album() -> Option<Arc<AlbumPlaycounts>> {
    fixture(Fixture::Album).map(|v| Arc::new(v.map(|v| parse_album(&v)).unwrap_or_default()))
}

/// `OPENIFY_TEST_STATS_JSON`: klasörse fikstür adları, dosyaysa kendisi.
/// Değişken verilmiş ama dosya okunamıyorsa `Some(None)`: ağa çıkılmaz.
fn fixture(kind: Fixture) -> Option<Option<Value>> {
    if !crate::crash::is_test_run() {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("OPENIFY_TEST_STATS_JSON")?);
    let path = if path.is_dir() {
        path.join(match kind {
            Fixture::Artist => "artist_overview_stats.json",
            Fixture::Album => "album_playcounts.json",
        })
    } else {
        path
    };
    Some(
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok()),
    )
}

// ── Alma ────────────────────────────────────────────────────────────────

/// Sanatçı sayılarını alır (kapı, token yenileme, yeniden deneme ortak
/// istemciden). Tek istek; sayı yoksa boş döner.
pub async fn load_artist(
    session: &Session,
    db: Option<&Db>,
    cache: &PageCache,
    uri: &str,
) -> Result<Arc<ArtistStats>, StatsFailure> {
    if let Some(fixture) = fixture(Fixture::Artist) {
        let stats = fixture.map(|v| parse_artist(&v)).unwrap_or_default();
        return Ok(Arc::new(stats));
    }
    let value = pathfinder::query(
        session,
        db,
        &ARTIST_OVERVIEW,
        artist_variables(uri),
        Priority::Background,
    )
    .await
    .map_err(|e| {
        log::info!("sanatçı dinlenme sayıları alınamadı ({uri}): {e}");
        StatsFailure::from(&e)
    })?;
    let stats = Arc::new(parse_artist(&value));
    if !stats.is_empty() {
        cache
            .put(&artist_key(uri), &stats, now_ms())
            .map_err(|e| log::warn!("sanatçı sayıları önbelleğe yazılamadı ({uri}): {e:#}"))
            .ok();
    }
    Ok(stats)
}

/// Albüm şarkılarının dinlenme sayılarını alır. Tek istek.
pub async fn load_album(
    session: &Session,
    db: Option<&Db>,
    cache: &PageCache,
    uri: &str,
) -> Result<Arc<AlbumPlaycounts>, StatsFailure> {
    if let Some(fixture) = fixture(Fixture::Album) {
        let stats = fixture.map(|v| parse_album(&v)).unwrap_or_default();
        return Ok(Arc::new(stats));
    }
    let value = pathfinder::query(
        session,
        db,
        &ALBUM_TRACKS,
        album_variables(uri),
        Priority::Background,
    )
    .await
    .map_err(|e| {
        log::info!("albüm dinlenme sayıları alınamadı ({uri}): {e}");
        StatsFailure::from(&e)
    })?;
    let stats = Arc::new(parse_album(&value));
    if !stats.is_empty() {
        cache
            .put(&album_key(uri), &stats, now_ms())
            .map_err(|e| log::warn!("albüm sayıları önbelleğe yazılamadı ({uri}): {e:#}"))
            .ok();
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pathfinder::{
        Config, DiscoverError, Request, Response, Transport, TransportError,
    };
    use bytes::Bytes;
    use futures::future::BoxFuture;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };

    const ARTIST: &str = include_str!("fixtures/artist_overview_stats.json");
    const ALBUM: &str = include_str!("fixtures/album_playcounts.json");

    fn artist() -> ArtistStats {
        parse_artist(&serde_json::from_str(ARTIST).unwrap())
    }

    #[test]
    fn reads_the_recorded_artist_shape() {
        let s = artist();
        assert_eq!(s.monthly_listeners, 12_345_678);
        assert_eq!(s.followers, 2_345_678);
        assert_eq!(s.world_rank, Some(42));
        let cities: Vec<&str> = s.top_cities.iter().map(|c| c.city.as_str()).collect();
        assert_eq!(cities, ["Istanbul", "Ankara", "Izmir", "Bursa", "Antalya"]);
        assert_eq!(s.top_cities[0].plays, 987_654);
        assert_eq!(
            s.track_plays.get("spotify:track:fixture01"),
            Some(&4_567_890)
        );
        assert_eq!(s.track_plays.len(), 3);
        assert!(!s.is_empty());
    }

    #[test]
    fn reads_the_recorded_album_shape_with_string_playcounts() {
        let a = parse_album(&serde_json::from_str(ALBUM).unwrap());
        assert_eq!(a.track_plays.len(), 3);
        assert_eq!(a.track_plays.get("spotify:track:fixture11"), Some(&123_456));
        assert_eq!(a.track_plays.get("spotify:track:fixture13"), Some(&7));
        assert!(!a.is_empty());
    }

    #[test]
    fn tolerates_missing_fields_and_other_names() {
        // Alan adları değişmiş, sayılar yazı, şehir nesnesi ve düz yazı karışık;
        // 5'ten çok şehir ve tekrar eden şehir var.
        let value = serde_json::json!({
            "data": { "artistUnion": {
                "stats": {
                    "monthly_listeners": "1.234.567",
                    "followerCount": "7654321",
                    "world_rank": "17",
                    "cities": { "items": [
                        { "city": { "name": "Lizbon" }, "listeners": "900" },
                        { "city": "Lizbon", "numberOfListeners": 800 },
                        { "city": "Porto", "plays": "700" },
                        { "city": "Braga" }, { "name": "Faro" },
                        { "city": "Coimbra" }, { "city": "Setubal" }
                    ] }
                },
                "topTracks": { "items": [
                    { "uri": "spotify:track:zz1", "plays": "55" },
                    { "uri": "spotify:track:zz2" },
                    { "uri": "spotify:episode:no", "playcount": 9 }
                ] }
            } }
        });
        let s = parse_artist(&value);
        assert_eq!(s.monthly_listeners, 1_234_567);
        assert_eq!(s.followers, 7_654_321);
        assert_eq!(s.world_rank, Some(17));
        let cities: Vec<&str> = s.top_cities.iter().map(|c| c.city.as_str()).collect();
        assert_eq!(cities, ["Lizbon", "Porto", "Braga", "Faro", "Coimbra"]);
        assert_eq!(s.top_cities[0].plays, 900);
        assert_eq!(s.track_plays.get("spotify:track:zz1"), Some(&55));
        assert!(!s.track_plays.contains_key("spotify:track:zz2"));
        assert!(
            s.track_plays
                .keys()
                .all(|u| u.starts_with("spotify:track:"))
        );
    }

    #[test]
    fn empty_and_unrelated_bodies_make_empty_stats() {
        for body in [
            "{}",
            "[]",
            r#"{"data":{}}"#,
            r#"{"data":{"artistUnion":{}}}"#,
            r#"{"data":{"albumUnion":{"tracksV2":{"items":[]}}}}"#,
            "null",
            "3",
        ] {
            let v: Value = serde_json::from_str(body).unwrap();
            assert!(parse_artist(&v).is_empty(), "{body}");
            assert!(parse_album(&v).is_empty(), "{body}");
        }
    }

    #[test]
    fn album_tolerates_old_container_names() {
        let value = serde_json::json!({
            "albumUnion": { "tracks": { "items": [
                { "track": { "uri": "spotify:track:old1", "playCount": 12 } },
                { "track": { "playcount": "34" } }
            ] } }
        });
        let a = parse_album(&value);
        assert_eq!(a.track_plays.get("spotify:track:old1"), Some(&12));
        assert_eq!(a.track_plays.len(), 1);
    }

    #[test]
    fn variables_carry_the_address_and_album_window() {
        let a = artist_variables("spotify:artist:x");
        assert_eq!(a["uri"], "spotify:artist:x");
        let b = album_variables("spotify:album:y");
        assert_eq!(b["uri"], "spotify:album:y");
        assert_eq!(b["offset"], 0);
        assert_eq!(b["limit"], ALBUM_TRACK_LIMIT);
    }

    #[test]
    fn failure_codes_follow_the_shared_client() {
        let code = |e: PathfinderError| StatsFailure::from(&e);
        assert_eq!(
            code(PathfinderError::Denied { status: 403 }),
            StatsFailure::Denied
        );
        assert_eq!(
            code(PathfinderError::RateLimited {
                retry_after: None,
                circuit_open: false
            }),
            StatsFailure::RateLimited
        );
        assert_eq!(
            code(PathfinderError::CircuitOpen),
            StatsFailure::RateLimited
        );
        assert_eq!(
            code(PathfinderError::Http { status: 404 }),
            StatsFailure::Unavailable
        );
        assert_eq!(
            code(PathfinderError::InvalidResponse("x".into())),
            StatsFailure::Unavailable
        );
        assert_eq!(
            code(PathfinderError::Transport("down".into())),
            StatsFailure::Offline
        );
        assert_eq!(
            code(PathfinderError::Token("none".into())),
            StatsFailure::Offline
        );
    }

    /// Çıta: 24 saat taze, sonrası bayat (arkadan tazelenir), 60 günde silinir.
    #[test]
    fn cache_lives_a_day_and_hard_expires_like_pages() {
        let cache = PageCache::open_in_memory().unwrap();
        let uri = "spotify:artist:cachetest";
        let stats = Arc::new(artist());
        cache.put(&artist_key(uri), &stats, 0).unwrap();
        let at = |now| {
            cache
                .get::<ArtistStats>(Kind::Stats, &artist_key(uri), now)
                .unwrap()
        };
        assert!(!at(STATS_FRESH_MS - 1).unwrap().stale);
        assert!(at(STATS_FRESH_MS).unwrap().stale);
        assert!(at(60 * 24 * 3600 * 1000).is_none());
        // Albüm sayıları aynı tabloda, ayrı anahtarla.
        let album = Arc::new(AlbumPlaycounts {
            track_plays: [("spotify:track:a".to_string(), 5)].into(),
        });
        cache
            .put(&album_key("spotify:album:x"), &album, 10)
            .unwrap();
        let hit = cache
            .get::<AlbumPlaycounts>(Kind::Stats, &album_key("spotify:album:x"), 20)
            .unwrap()
            .unwrap();
        assert_eq!(hit.page.track_plays.get("spotify:track:a"), Some(&5));
    }

    #[test]
    fn cache_keys_keep_artist_and_album_apart() {
        assert_eq!(
            artist_key("spotify:artist:a"),
            "stats:artist:spotify:artist:a"
        );
        assert_eq!(album_key("spotify:album:a"), "stats:album:spotify:album:a");
    }

    // ── Alma: sahte ağ ─────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeNet {
        posts: Mutex<VecDeque<Result<Response, TransportError>>>,
        bodies: Mutex<Vec<String>>,
    }

    impl FakeNet {
        fn reply(self, body: &str) -> Self {
            self.posts.lock().unwrap().push_back(Ok(Response {
                status: 200,
                retry_after: None,
                body: Bytes::from(body.to_string()),
            }));
            self
        }
    }

    impl Transport for FakeNet {
        fn post<'a>(
            &'a self,
            request: &'a Request,
        ) -> BoxFuture<'a, Result<Response, TransportError>> {
            Box::pin(async move {
                self.bodies.lock().unwrap().push(request.body.clone());
                self.posts
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("beklenmeyen istek")
            })
        }
        fn refresh_token<'a>(&'a self) -> BoxFuture<'a, Result<(), TransportError>> {
            Box::pin(async { Ok(()) })
        }
        fn discover<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<String, DiscoverError>> {
            Box::pin(async { Ok("0".repeat(64)) })
        }
    }

    fn fast() -> pathfinder::Pathfinder {
        pathfinder::Pathfinder::new(Config {
            interval: Duration::ZERO,
            server_retries: 0,
            backoff_base: Duration::from_millis(1),
        })
    }

    #[tokio::test]
    async fn a_single_request_fills_the_artist_stats_and_is_cached() {
        let net = FakeNet::default().reply(ARTIST);
        let cache = PageCache::open_in_memory().unwrap();
        let pf = fast();
        // Gerçek yol `pathfinder::query` üzerinden gider; sahte ağla aynı
        // işlem tanımını doğruluyoruz (tek POST, doğru işlem adı).
        let value = pf
            .call(
                &net,
                None,
                &ARTIST_OVERVIEW,
                artist_variables("spotify:artist:fixture01"),
                Priority::Background,
            )
            .await
            .unwrap();
        let stats = Arc::new(parse_artist(&value));
        assert_eq!(stats.monthly_listeners, 12_345_678);
        cache
            .put(&artist_key("spotify:artist:fixture01"), &stats, 0)
            .unwrap();
        let bodies = net.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("queryArtistOverview"));
        assert!(bodies[0].contains("spotify:artist:fixture01"));
    }

    // ── Canlı (sahibi onaylayınca elle) ────────────────────────────────

    /// CANLI okuma sınaması: bir sanatçı + bir albüm isteği, salt okuma;
    /// hiçbir şey yazmaz. Yalnız sahibinin onayıyla, elle koşulur:
    ///
    ///   cargo test --release live_stats -- --ignored --nocapture
    ///
    /// Ad yazdırılmaz: yalnız alanların dolu olup olmadığı ve sayılar.
    /// `OPENIFY_LIVE_ARTIST` / `OPENIFY_LIVE_ALBUM` ile başka adresler.
    /// Albüm adresi verilmezse sanatçı yanıtındaki ilk albüm kullanılır.
    #[tokio::test]
    #[ignore = "canli Spotify: yalniz sahibi onayiyla elle kosulur"]
    async fn live_stats() {
        use librespot::core::{authentication::Credentials, config::SessionConfig};
        let json = keyring::Entry::new("Openify", "spotify-session")
            .expect("Credential Manager")
            .get_password()
            .expect("Kayitli Spotify girisi yok");
        let credentials: Credentials = serde_json::from_str(&json).expect("kayitli giris bozuk");
        let session = Session::new(SessionConfig::default(), None);
        session
            .connect(credentials, false)
            .await
            .expect("Spotify'a baglanilamadi");
        let cache = PageCache::open_in_memory().unwrap();

        let artist_uri = std::env::var("OPENIFY_LIVE_ARTIST")
            .unwrap_or_else(|_| "spotify:artist:0gxyHStUsqpMadRV0Di1Qt".into());
        let value = match pathfinder::query(
            &session,
            None,
            &ARTIST_OVERVIEW,
            artist_variables(&artist_uri),
            Priority::Interactive,
        )
        .await
        {
            Ok(value) => value,
            Err(e) => {
                println!("sanatci sayilari alinamadi: {e}");
                return;
            }
        };
        let stats = parse_artist(&value);
        println!(
            "sanatci: aylik {} (dolu {}), takipci {} (dolu {}), dunya sirasi {:?}, sehir {}, sarki {}",
            stats.monthly_listeners,
            stats.monthly_listeners > 0,
            stats.followers,
            stats.followers > 0,
            stats.world_rank,
            stats.top_cities.len(),
            stats.track_plays.len()
        );
        println!(
            "  sehir adlari dolu {}, sayilari dolu {}, sarki sayilari dolu {}",
            stats.top_cities.iter().all(|c| !c.city.is_empty()),
            stats.top_cities.iter().all(|c| c.plays > 0),
            stats.track_plays.values().all(|n| *n > 0)
        );

        let album_uri = std::env::var("OPENIFY_LIVE_ALBUM").ok().or_else(|| {
            value
                .pointer("/data/artistUnion/discography/albums/items/0/releases/items/0/uri")
                .or_else(|| {
                    value.pointer(
                        "/data/artistUnion/discography/topTracks/items/0/track/albumOfTrack/uri",
                    )
                })
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|u| u.starts_with("spotify:album:"))
        });
        let Some(album_uri) = album_uri else {
            println!("album: sanatci yanitinda albüm adresi yok (OPENIFY_LIVE_ALBUM verilebilir)");
            return;
        };
        match load_album(&session, None, &cache, &album_uri).await {
            Ok(album) => {
                let filled = album.track_plays.values().filter(|n| **n > 0).count();
                println!(
                    "album: sarki {} (sayisi dolu {})",
                    album.track_plays.len(),
                    filled
                );
            }
            Err(e) => println!("album sayilari alinamadi: {e:?}"),
        }
    }
}
