//! Benzer sanatçılar (Adım 12, 7 Kasım): sanatçı sayfasının son bölümü.
//!
//! ARTIST_V4'ün `related` alanı boş geliyor. Kaynak: resmi web player'ın
//! GraphQL sorgusu `queryArtistRelated`: istek, hash tablosu / keşfi, kapı ve
//! yeniden deneme `pathfinder.rs`'te (aramayla ve Keşfet'le ortak istemci).
//!
//! Spotify'ı boğmamak için:
//!   - yalnız açılan sanatçı için istenir (üzerine gelince önceden alma yok,
//!     benzerin benzeri hiç istenmez), aynı sanatçı için aynı anda tek istek
//!     (`wire.rs`);
//!   - önbellek (`pages.db`, `related:<uri>`): 7 gün taze, 30 güne kadar
//!     bayat gösterilir (arkadan tazelenir), boş sonuç 24 saat; sanatçı
//!     sayfasının 6 saatlik tazeliğinden bağımsız;
//!   - bu yolda en çok 2 sn'de bir istek (`pathfinder::Gate`, aramayla ortak);
//!     429'da Retry-After kadar hiç istek yok, yeniden deneme yok; üst üste
//!     iki 429 → bu oturumda Spotify yolu kapalı (devre kesici).
//! Hata ekranda söylenmez: bayat önbellek varsa o, yoksa bölüm yok.
//!
//! Yedek: Spotify yolu ölüyse (devre açık, sorgu artık yok ya da 403) Deezer'ın
//! herkese açık API'si: sanatçının popüler şarkısının ISRC'si (TRACK_V4) →
//! `track/isrc:<ISRC>` → Deezer sanatçısı → `artist/<id>/related`. Kartlar
//! Deezer'ın adı ve resmiyle (`deezer:artist:<id>` adresiyle); Spotify adresi
//! yalnız tıklanınca bir aramayla bulunur (`resolve_deezer`). Eşlemeler
//! `pages.db`'de kalıcı. Deezer'a en çok saniyede bir istek.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use librespot::{
    core::{SpotifyUri, session::Session},
    metadata::{Metadata, Track},
    protocol::{
        extension_kind::ExtensionKind,
        metadata::{Artist as ArtistMessage, Track as TrackMessage},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    cache::Db,
    pages::{ArtistCard, Kind, PageCache, fetch_extended, now_ms},
    pathfinder::{self, Admit, Gate, PathfinderError, Priority, StaleCause},
    search::{self, SearchHit, card_image, str_of},
};

/// Bölümde en çok kart.
pub const MAX_CARDS: usize = 20;

const DAY_MS: i64 = 24 * 3600 * 1000;
/// Bu kadar süre taze: yeniden istenmez.
pub const FRESH_MS: i64 = 7 * DAY_MS;
/// Bayat ama gösterilir (arkadan tazelenir); daha eskisi gösterilmez.
pub const STALE_MAX_MS: i64 = 30 * DAY_MS;
/// Boş sonuç bu kadar süre "yok" sayılır (yeniden sorulmaz).
pub const EMPTY_MS: i64 = DAY_MS;

const DEEZER_INTERVAL: Duration = Duration::from_secs(1);
const DEEZER_API: &str = "https://api.deezer.com/";
/// Deezer'ın "çok istek" hata kodu (HTTP 200 içinde gelir).
const DEEZER_QUOTA: i64 = 4;

// ── Türler ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelatedSource {
    Spotify,
    /// Kartların adresi `deezer:artist:<id>`: tıklanınca Spotify'da aranır.
    Deezer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelatedList {
    pub source: RelatedSource,
    pub artists: Vec<ArtistCard>,
}

/// Benzerleri istenen sanatçı; ad ve popüler şarkı yalnız Deezer yolu için.
#[derive(Debug, Clone)]
pub struct RelatedHint {
    pub uri: String,
    pub name: String,
    pub top_track: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Göster, isteme.
    Fresh,
    /// Göster, arkadan tazele.
    Stale,
    /// Gösterme, iste.
    Expired,
}

/// Önbellekteki listenin tazeliği (saf: testlenir).
pub fn freshness(age_ms: i64, empty: bool) -> Freshness {
    let fresh_for = if empty { EMPTY_MS } else { FRESH_MS };
    let show_for = if empty { EMPTY_MS } else { STALE_MAX_MS };
    if age_ms < fresh_for {
        Freshness::Fresh
    } else if age_ms < show_for {
        Freshness::Stale
    } else {
        Freshness::Expired
    }
}

fn key(uri: &str) -> String {
    format!("related:{uri}")
}

fn deezer_of_key(spotify_uri: &str) -> String {
    format!("deezer-of:{spotify_uri}")
}

fn spotify_of_key(deezer_uri: &str) -> String {
    format!("deezer-to:{deezer_uri}")
}

/// Kalıcı eşleme satırı (`pages.db`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Link {
    to: String,
}

/// Önbellekteki liste ve tazeliği.
pub fn cached(cache: &PageCache, uri: &str) -> Option<(Arc<RelatedList>, Freshness)> {
    cached_at(cache, uri, now_ms())
}

fn cached_at(cache: &PageCache, uri: &str, now: i64) -> Option<(Arc<RelatedList>, Freshness)> {
    let hit = cache
        .get::<RelatedList>(Kind::Related, &key(uri), now)
        .ok()??;
    let state = freshness(hit.age_ms, hit.page.artists.is_empty());
    Some((hit.page, state))
}

fn store_at(cache: &PageCache, uri: &str, list: &Arc<RelatedList>, now: i64) -> Result<bool> {
    cache.put(&key(uri), list, now)
}

fn link(cache: &PageCache, key: &str) -> Option<String> {
    let hit = cache.get::<Link>(Kind::Related, key, now_ms()).ok()??;
    Some(hit.page.to.clone())
}

fn put_link(cache: &PageCache, key: &str, to: &str) {
    if let Err(e) = cache.put(key, &Arc::new(Link { to: to.to_string() }), now_ms()) {
        log::warn!("eşleme yazılamadı ({key}): {e:#}");
    }
}

/// Deezer kartının Spotify adresi önceden bulunduysa.
pub fn resolved(cache: &PageCache, deezer_uri: &str) -> Option<String> {
    link(cache, &spotify_of_key(deezer_uri))
}

// ── Deezer kapısı ───────────────────────────────────────────────────────
//
// Aynı kapı mantığı (`pathfinder::Gate`: aralık, 429, devre kesici); Spotify
// yolunun kapısı pathfinder istemcisinde.

static DEEZER_GATE: Mutex<Gate> = Mutex::new(Gate::new(DEEZER_INTERVAL));
/// Spotify yolu bu oturumda öldü (devre açık, sorgu yok ya da reddedildi).
static SPOTIFY_DEAD: AtomicBool = AtomicBool::new(false);

fn lock(gate: &Mutex<Gate>) -> MutexGuard<'_, Gate> {
    gate.lock().unwrap_or_else(|e| e.into_inner())
}

/// Kapıdan geç: sıra gelene kadar bekle, sonra yeniden bak.
async fn pass(gate: &Mutex<Gate>) -> Result<(), Admit> {
    let admit = lock(gate).admit(Instant::now());
    match admit {
        Admit::Go(wait) => {
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            match lock(gate).refused(Instant::now()) {
                Some(refused) => Err(refused),
                None => Ok(()),
            }
        }
        refused => Err(refused),
    }
}

// ── Spotify: queryArtistRelated ─────────────────────────────────────────

/// `data.artistUnion.relatedContent.relatedArtists.items[]` → kartlar
/// (eskiyen hash'i istemci yakalar, buraya gelmez).
/// Eksik alan → boş liste (çökmez); kendisi, yinelenen, adsız atlanır;
/// en çok `MAX_CARDS`. Resim yanıttakinden (kart başına ek istek yok).
pub fn parse_spotify(value: &Value, self_uri: &str) -> Result<Vec<ArtistCard>> {
    let union = &value["data"]["artistUnion"];
    if union.is_null() {
        // Hata var ve veri yok: önbelleğe "boş" diye yazılmasın.
        if let Some(message) = value["errors"][0]["message"].as_str() {
            bail!("GraphQL: {message}");
        }
        return Ok(Vec::new());
    }
    let Some(items) = union["relatedContent"]["relatedArtists"]["items"].as_array() else {
        return Ok(Vec::new());
    };
    let mut seen = HashSet::new();
    Ok(items
        .iter()
        .filter_map(|item| {
            // Bazı GraphQL listeleri öğeyi `data` içine sarıyor.
            let d = if item.get("uri").is_some() {
                item
            } else {
                &item["data"]
            };
            let uri = d["uri"].as_str()?;
            if !uri.starts_with("spotify:artist:")
                || uri == self_uri
                || !seen.insert(uri.to_string())
            {
                return None;
            }
            let name = str_of(&d["profile"]["name"]).trim().to_string();
            if name.is_empty() {
                return None;
            }
            Some(ArtistCard {
                uri: uri.to_string(),
                name,
                image: card_image(&d["visuals"]["avatarImage"]["sources"]),
            })
        })
        .take(MAX_CARDS)
        .collect())
}

/// Spotify yolu neden olmadı.
enum SpotifyFail {
    /// Bu oturumda bir daha denenmez: Deezer'a geç.
    Dead(String),
    /// Bu sefer olmadı (ağ, tek 429, ...): bayat önbellek ya da hiçbir şey.
    Failed(anyhow::Error),
}

/// Pathfinder hatası → Spotify yolu bu oturumda kapanır mı, bu seferlik mi.
fn classify(error: PathfinderError) -> SpotifyFail {
    match error {
        PathfinderError::CircuitOpen => SpotifyFail::Dead("devre açık".into()),
        PathfinderError::Blocked { remaining } => SpotifyFail::Failed(anyhow!(
            "429 sonrası bekleniyor ({} sn kaldı)",
            remaining.as_secs()
        )),
        PathfinderError::RateLimited {
            circuit_open: true, ..
        } => kill_spotify("üst üste iki 429".into()),
        PathfinderError::StaleHash {
            detail,
            cause: StaleCause::Rejected,
            ..
        } => kill_spotify(format!("yeni hash de reddedildi: {detail}")),
        PathfinderError::StaleHash {
            cause: StaleCause::Unchanged,
            ..
        } => kill_spotify("güncel hash reddediliyor".into()),
        PathfinderError::StaleHash { detail, .. } => {
            // Bu oturumda zaten arandı (başka bir sanatçıda): bir daha değil.
            SpotifyFail::Failed(anyhow!(
                "hash eskimiş ({detail}), bu oturumda yeniden aranmıştı"
            ))
        }
        PathfinderError::OperationGone { operation } => {
            kill_spotify(format!("{operation} artık yok"))
        }
        PathfinderError::Denied { status: 403 } => kill_spotify("izin yok: 403".into()),
        other => SpotifyFail::Failed(other.into()),
    }
}

fn kill_spotify(why: String) -> SpotifyFail {
    SPOTIFY_DEAD.store(true, Ordering::Relaxed);
    log::warn!("benzer sanatçılar: Spotify yolu bu oturumda kapandı ({why}); yedek Deezer");
    SpotifyFail::Dead(why)
}

async fn from_spotify(
    session: &Session,
    db: Option<&Db>,
    uri: &str,
) -> Result<Vec<ArtistCard>, SpotifyFail> {
    if SPOTIFY_DEAD.load(Ordering::Relaxed) {
        return Err(SpotifyFail::Dead("önceden kapandı".into()));
    }
    let value = pathfinder::query(
        session,
        db,
        &pathfinder::ARTIST_RELATED,
        json!({ "uri": uri }),
        Priority::Background,
    )
    .await
    .map_err(classify)?;
    parse_spotify(&value, uri).map_err(SpotifyFail::Failed)
}

// ── Deezer yedeği ───────────────────────────────────────────────────────

/// Ad karşılaştırması: büyük/küçük harf ve boşluk farkı yok sayılır.
pub fn same_name(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b
}

/// `track/isrc:<ISRC>` yanıtında aranan sanatçının Deezer kimliği: adı
/// tutan katkıcı ya da ana sanatçı. Ad tutmazsa `None` (yanlış sanatçının
/// benzerlerini göstermektense hiç göstermemek).
pub fn parse_deezer_track(value: &Value, name: &str) -> Option<u64> {
    if value.get("error").is_some() {
        return None;
    }
    let contributors = value["contributors"].as_array().into_iter().flatten();
    for c in contributors.chain(std::iter::once(&value["artist"])) {
        if same_name(c["name"].as_str().unwrap_or_default(), name) {
            return c["id"].as_u64();
        }
    }
    None
}

/// `artist/<id>/related` → kartlar (`deezer:artist:<id>`).
pub fn parse_deezer_related(value: &Value) -> Result<Vec<ArtistCard>> {
    if let Some(e) = value.get("error") {
        bail!(
            "Deezer: {}",
            e["message"].as_str().unwrap_or("bilinmeyen hata")
        );
    }
    let Some(items) = value["data"].as_array() else {
        return Ok(Vec::new());
    };
    let mut seen = HashSet::new();
    Ok(items
        .iter()
        .filter_map(|a| {
            let id = a["id"].as_u64()?;
            let name = str_of(&a["name"]).trim().to_string();
            if name.is_empty() || !seen.insert(id) {
                return None;
            }
            // Resmi olmayan sanatçıda Deezer boş md5'li yer tutucu veriyor
            // (".../images/artist//250x250-..."): bizim yer tutucu çizilsin.
            let image = [&a["picture_big"], &a["picture_medium"]]
                .into_iter()
                .filter_map(Value::as_str)
                .find(|u| !u.is_empty() && !u.contains("/artist//"))
                .unwrap_or_default()
                .to_string();
            Some(ArtistCard {
                uri: format!("deezer:artist:{id}"),
                name,
                image,
            })
        })
        .take(MAX_CARDS)
        .collect())
}

/// Deezer'a GET (kapıdan geçerek). 200 içindeki kota hatası 429 sayılır.
async fn deezer_get(client: &reqwest::Client, path: &str) -> Result<Value> {
    pass(&DEEZER_GATE)
        .await
        .map_err(|a| anyhow!("Deezer kapısı kapalı: {a:?}"))?;
    let response = client
        .get(format!("{DEEZER_API}{path}"))
        .send()
        .await
        .context("Deezer'a ulaşılamadı")?;
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        lock(&DEEZER_GATE).on_rate_limited(Instant::now(), None);
        bail!("Deezer 429");
    }
    let bytes = response
        .error_for_status()?
        .bytes()
        .await
        .context("Deezer yanıtı okunamadı")?;
    let value: Value = serde_json::from_slice(&bytes).context("Deezer yanıtı çözülemedi")?;
    if value["error"]["code"].as_i64() == Some(DEEZER_QUOTA) {
        lock(&DEEZER_GATE).on_rate_limited(Instant::now(), Some(Duration::from_secs(5)));
        bail!("Deezer kota");
    }
    lock(&DEEZER_GATE).on_success();
    Ok(value)
}

fn deezer_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("Openify/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()?)
}

/// Şarkının ISRC'si (TRACK_V4, tek istek).
async fn track_isrc(session: &Session, track_uri: &str) -> Result<String> {
    let one = [track_uri.to_string()];
    let (_, message) = fetch_extended::<TrackMessage>(session, ExtensionKind::TRACK_V4, &one)
        .await?
        .pop()
        .context("şarkı bilgisi gelmedi")?;
    let track = Track::parse(&message, &SpotifyUri::from_uri(track_uri)?)?;
    track
        .external_ids
        .iter()
        .find(|e| e.external_type.eq_ignore_ascii_case("isrc"))
        .map(|e| e.id.clone())
        .context("şarkının ISRC'si yok")
}

async fn from_deezer(
    session: &Session,
    cache: &PageCache,
    hint: &RelatedHint,
) -> Result<Vec<ArtistCard>> {
    let client = deezer_client()?;
    let deezer_id = match link(cache, &deezer_of_key(&hint.uri)) {
        Some(id) => id,
        None => {
            let top = hint.top_track.as_deref().context("popüler şarkı yok")?;
            let isrc = track_isrc(session, top).await?;
            let track = deezer_get(&client, &format!("track/isrc:{isrc}")).await?;
            let id = parse_deezer_track(&track, &hint.name)
                .with_context(|| format!("Deezer'da {} bulunamadı (ISRC {isrc})", hint.name))?
                .to_string();
            put_link(cache, &deezer_of_key(&hint.uri), &id);
            id
        }
    };
    let related = deezer_get(
        &client,
        &format!("artist/{deezer_id}/related?limit={MAX_CARDS}"),
    )
    .await?;
    parse_deezer_related(&related)
}

// ── Dışarıya açık işler ─────────────────────────────────────────────────

/// Benzer sanatçıları indirir ve önbelleğe yazar (boş sonuç da yazılır:
/// 24 saat yeniden sorulmaz). Hata önbelleğe yazılmaz.
pub async fn load(
    session: &Session,
    db: Option<&Db>,
    cache: &PageCache,
    hint: &RelatedHint,
) -> Result<Arc<RelatedList>> {
    let started = Instant::now();
    let list = match from_spotify(session, db, &hint.uri).await {
        Ok(artists) => RelatedList {
            source: RelatedSource::Spotify,
            artists,
        },
        Err(SpotifyFail::Dead(why)) => {
            log::debug!(
                "benzer sanatçılar {}: Spotify yolu kapalı ({why}), Deezer'dan",
                hint.uri
            );
            RelatedList {
                source: RelatedSource::Deezer,
                artists: from_deezer(session, cache, hint).await?,
            }
        }
        Err(SpotifyFail::Failed(e)) => return Err(e),
    };
    let list = Arc::new(list);
    store_at(cache, &hint.uri, &list, now_ms())?;
    log::debug!(
        "benzer sanatçılar {}: {} kart ({:?}), {} ms",
        hint.uri,
        list.artists.len(),
        list.source,
        started.elapsed().as_millis()
    );
    Ok(list)
}

/// Keşfet'in "benzer sanatçılardan öneriler" bölümü: yalnız önbellekteki
/// benzer sanatçı listelerinden (ağsız; hiçbir istek gitmez). Sayfası açılmış
/// sanatçıların benzerleri, kaç listede geçtiğine göre (eşitlikte listede
/// üstte olana göre); açılmış sanatçıların kendileri ve `known` adlardaki
/// (küçük harf) sanatçılar (kütüphanedeki albümlerin sanatçıları) çıkar.
/// Deezer kartları (Spotify adresi yok) alınmaz.
pub fn suggestions(cache: &PageCache, known: &HashSet<String>, limit: usize) -> Vec<ArtistCard> {
    let now = now_ms();
    let prefix = key("spotify:artist:");
    let seeds: Vec<String> = cache
        .keys_with_prefix(&prefix)
        .into_iter()
        .filter_map(|k| k.strip_prefix("related:").map(String::from))
        .collect();
    let seed_set: HashSet<&str> = seeds.iter().map(String::as_str).collect();
    // adres → (kaç listede, listelerdeki en iyi sıra, kart)
    let mut found: HashMap<String, (usize, usize, ArtistCard)> = HashMap::new();
    let mut order = Vec::new();
    for seed in &seeds {
        let Some((list, state)) = cached_at(cache, seed, now) else {
            continue;
        };
        if state == Freshness::Expired || list.source != RelatedSource::Spotify {
            continue;
        }
        for (rank, card) in list.artists.iter().enumerate() {
            if !card.uri.starts_with("spotify:artist:")
                || seed_set.contains(card.uri.as_str())
                || known.contains(&card.name.trim().to_lowercase())
            {
                continue;
            }
            let entry = found.entry(card.uri.clone()).or_insert_with(|| {
                order.push(card.uri.clone());
                (0, usize::MAX, card.clone())
            });
            entry.0 += 1;
            entry.1 = entry.1.min(rank);
        }
    }
    let mut ranked: Vec<(usize, usize, usize, ArtistCard)> = order
        .iter()
        .enumerate()
        .filter_map(|(i, u)| found.remove(u).map(|(n, r, c)| (n, r, i, c)))
        .collect();
    ranked.sort_by_key(|(n, r, i, _)| (std::cmp::Reverse(*n), *r, *i));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, _, c)| c)
        .collect()
}

/// Aynı adı taşıyanlar arasında en popüler (eşitlikte arama sırası).
pub fn pick_by_popularity(
    exact: &[&SearchHit],
    popularity: &HashMap<String, i32>,
) -> Option<String> {
    let mut best: Option<(&SearchHit, i32)> = None;
    for hit in exact {
        let p = popularity.get(&hit.uri).copied().unwrap_or(0);
        if best.is_none_or(|(_, b)| p > b) {
            best = Some((*hit, p));
        }
    }
    best.map(|(h, _)| h.uri.clone())
}

/// Deezer kartına tıklandı: Spotify adresini bul (tek `searchDesktop`;
/// aynı adlı birden çok sanatçı varsa popülerliğe bir ARTIST_V4 isteği).
/// Arama hash'ini pathfinder istemcisi yönetir (`db`: kalıcı kayıt).
pub async fn resolve_deezer(
    session: &Session,
    cache: &PageCache,
    db: Option<&Db>,
    deezer_uri: &str,
    name: &str,
) -> Result<String> {
    if let Some(uri) = resolved(cache, deezer_uri) {
        return Ok(uri);
    }
    let results = search::run(session, db, name, 10).await?;
    let exact: Vec<&SearchHit> = results
        .artists
        .iter()
        .filter(|h| same_name(&h.name, name))
        .collect();
    let uri = match exact.len() {
        0 => bail!("Spotify'da \"{name}\" adlı sanatçı yok"),
        1 => exact[0].uri.clone(),
        _ => {
            let uris: Vec<String> = exact.iter().map(|h| h.uri.clone()).collect();
            let popularity: HashMap<String, i32> =
                fetch_extended::<ArtistMessage>(session, ExtensionKind::ARTIST_V4, &uris)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(u, m)| (u, m.popularity.unwrap_or(0)))
                    .collect();
            pick_by_popularity(&exact, &popularity).context("seçilemedi")?
        }
    };
    put_link(cache, &spotify_of_key(deezer_uri), &uri);
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gerçek yanıttan kırpılmış örnek (Adamlar, 7 Kasım 2026; kişisel veri yok).
    const SPOTIFY_FIXTURE: &str = include_str!("fixtures/related_adamlar.json");
    const ADAMLAR: &str = "spotify:artist:1g4J8P1JWwanNyyXckRX5W";

    #[test]
    fn suggestions_come_from_cached_lists_only() {
        let cache = PageCache::open_in_memory().unwrap();
        let card = |n: u32, name: &str| ArtistCard {
            uri: format!("spotify:artist:{n}"),
            name: name.into(),
            image: String::new(),
        };
        let put = |seed: &str, artists: Vec<ArtistCard>| {
            let list = Arc::new(RelatedList {
                source: RelatedSource::Spotify,
                artists,
            });
            store_at(&cache, seed, &list, now_ms()).unwrap();
        };
        // Seed 1 ve 2 açılmış; her ikisinin benzerlerinde 10 geçiyor, 11 yalnız birinde,
        // 12 kütüphanedeki bir albümün sanatçısı, 2 numara seed'in kendisi.
        put(
            "spotify:artist:1",
            vec![
                card(10, "On"),
                card(2, "Seed İki"),
                card(11, "Onbir"),
                card(12, "Kütüphanede"),
            ],
        );
        put(
            "spotify:artist:2",
            vec![card(11, "Onbir"), card(10, "On"), card(13, "Onüç")],
        );
        let known: HashSet<String> = ["kütüphanede".to_string()].into();
        let got = suggestions(&cache, &known, 10);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        // 10 ve 11 ikişer listede; 10 daha üstte (en iyi sıra 0), sonra 13.
        assert_eq!(names, ["On", "Onbir", "Onüç"], "{names:?}");
        assert_eq!(suggestions(&cache, &known, 2).len(), 2);
        // Hiç liste yoksa boş.
        assert!(suggestions(&PageCache::open_in_memory().unwrap(), &known, 5).is_empty());
    }

    #[test]
    fn parses_real_spotify_response() {
        let value: Value = serde_json::from_str(SPOTIFY_FIXTURE).unwrap();
        let cards = parse_spotify(&value, ADAMLAR).unwrap();
        // Örnekte 22 öğe (Spotify 40 döndürdü): en çok 20 kart, Spotify'ın sırası.
        assert_eq!(cards.len(), MAX_CARDS);
        assert_eq!(cards[0].name, "Duman");
        assert_eq!(cards[0].uri, "spotify:artist:6RTC1abMgBC7Krg6qJQHJh");
        // 160 / 320 / 640 arasından kart için 320.
        assert_eq!(
            cards[0].image,
            "https://i.scdn.co/image/ab676161000051745eedfda3da66eb4591be4fa7"
        );
        for c in &cards {
            assert!(c.uri.starts_with("spotify:artist:"), "{}", c.uri);
            assert_ne!(c.uri, ADAMLAR);
            assert!(!c.name.is_empty());
            assert!(c.image.starts_with("https://"), "{}: {}", c.name, c.image);
        }
        let unique: HashSet<_> = cards.iter().map(|c| &c.uri).collect();
        assert_eq!(unique.len(), cards.len());
    }

    #[test]
    fn spotify_parsing_is_defensive() {
        // Eksik alanlar → boş liste, çökme yok.
        assert!(parse_spotify(&json!({}), ADAMLAR).unwrap().is_empty());
        assert!(
            parse_spotify(&json!({ "data": {} }), ADAMLAR)
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_spotify(
                &json!({ "data": { "artistUnion": { "__typename": "NotFound" } } }),
                ADAMLAR
            )
            .unwrap()
            .is_empty()
        );
        assert!(parse_spotify(&json!({ "data": { "artistUnion": { "relatedContent": { "relatedArtists": { "items": "x" } } } } }), ADAMLAR).unwrap().is_empty());
        // Bozuk öğeler atlanır, yinelenen ve kendisi çıkar, `data` sarmalı da okunur.
        let value = json!({ "data": { "artistUnion": { "relatedContent": { "relatedArtists": { "items": [
            { "uri": "spotify:artist:a", "profile": { "name": "A" } },
            { "uri": "spotify:artist:a", "profile": { "name": "A tekrar" } },
            { "uri": ADAMLAR, "profile": { "name": "Kendisi" } },
            { "uri": "spotify:track:x", "profile": { "name": "Şarkı" } },
            { "uri": "spotify:artist:b" },
            { "profile": { "name": "Adresiz" } },
            { "data": { "uri": "spotify:artist:c", "profile": { "name": " C " },
                "visuals": { "avatarImage": { "sources": [
                    { "url": "https://i.scdn.co/image/small", "width": 160 },
                    { "url": "https://i.scdn.co/image/mid", "width": 320 },
                    { "url": "https://i.scdn.co/image/big", "width": 640 } ] } } } },
            42
        ] } } } } });
        let cards = parse_spotify(&value, ADAMLAR).unwrap();
        let names: Vec<_> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["A", "C"]);
        assert_eq!(cards[0].image, "");
        assert_eq!(cards[1].image, "https://i.scdn.co/image/mid");
        // En çok 20.
        let many: Vec<Value> = (0..40)
            .map(|i| json!({ "uri": format!("spotify:artist:{i}"), "profile": { "name": format!("S{i}") } }))
            .collect();
        let value = json!({ "data": { "artistUnion": { "relatedContent": { "relatedArtists": { "items": many } } } } });
        assert_eq!(parse_spotify(&value, ADAMLAR).unwrap().len(), MAX_CARDS);
    }

    #[test]
    fn spotify_errors_are_surfaced() {
        // Veri yok, başka hata: önbelleğe "boş" yazılmasın diye hata.
        let other = json!({ "errors": [ { "message": "Something broke" } ], "data": null });
        assert!(parse_spotify(&other, ADAMLAR).is_err());
        // Kısmi hata ama veri var: veri okunur.
        let partial = json!({ "errors": [ { "message": "partial" } ], "data": { "artistUnion": { "relatedContent": { "relatedArtists": { "items": [
            { "uri": "spotify:artist:a", "profile": { "name": "A" } } ] } } } } });
        assert_eq!(parse_spotify(&partial, ADAMLAR).unwrap().len(), 1);
    }

    #[test]
    fn pathfinder_errors_decide_whether_spotify_is_dead() {
        let stale = |cause| PathfinderError::StaleHash {
            operation: "queryArtistRelated",
            detail: "x".into(),
            cause,
        };
        let dead = |e: PathfinderError| matches!(classify(e), SpotifyFail::Dead(_));
        // Bu oturumda kapanır.
        assert!(dead(PathfinderError::CircuitOpen));
        assert!(dead(PathfinderError::RateLimited {
            retry_after: None,
            circuit_open: true
        }));
        assert!(dead(stale(StaleCause::Rejected)));
        assert!(dead(stale(StaleCause::Unchanged)));
        assert!(dead(PathfinderError::OperationGone {
            operation: "queryArtistRelated"
        }));
        assert!(dead(PathfinderError::Denied { status: 403 }));
        // Bu seferlik.
        assert!(!dead(PathfinderError::Blocked {
            remaining: Duration::from_secs(5)
        }));
        assert!(!dead(PathfinderError::RateLimited {
            retry_after: None,
            circuit_open: false
        }));
        assert!(!dead(stale(StaleCause::AlreadyRediscovered)));
        assert!(!dead(PathfinderError::Denied { status: 401 }));
        assert!(!dead(PathfinderError::Http { status: 503 }));
        assert!(!dead(PathfinderError::Transport("offline".into())));
        assert!(!dead(PathfinderError::Discovery(anyhow!("offline"))));
        // classify kapıyı kapatmış olabilir: sınamaları birbirine bulaştırma.
        SPOTIFY_DEAD.store(false, Ordering::Relaxed);
    }

    #[test]
    fn freshness_rules() {
        let h = 3600 * 1000;
        // Dolu liste: 7 gün taze, 30 güne kadar bayat (gösterilir), sonra yok.
        assert_eq!(freshness(0, false), Freshness::Fresh);
        assert_eq!(freshness(FRESH_MS - 1, false), Freshness::Fresh);
        assert_eq!(freshness(FRESH_MS, false), Freshness::Stale);
        assert_eq!(freshness(STALE_MAX_MS - 1, false), Freshness::Stale);
        assert_eq!(freshness(STALE_MAX_MS, false), Freshness::Expired);
        // Boş sonuç: 24 saat "yok", sonra yeniden sorulur (bayat hâli yok).
        assert_eq!(freshness(23 * h, true), Freshness::Fresh);
        assert_eq!(freshness(24 * h, true), Freshness::Expired);
        // Sanatçı sayfasının 6 saatinden bağımsız.
        assert_eq!(freshness(7 * h, false), Freshness::Fresh);
    }

    #[test]
    fn cache_roundtrip_with_ttls() {
        let cache = PageCache::open_in_memory().unwrap();
        let list = Arc::new(RelatedList {
            source: RelatedSource::Spotify,
            artists: vec![ArtistCard {
                uri: "spotify:artist:a".into(),
                name: "A".into(),
                image: String::new(),
            }],
        });
        assert!(cached_at(&cache, ADAMLAR, 0).is_none());
        assert!(store_at(&cache, ADAMLAR, &list, 1_000).unwrap());
        let (got, state) = cached_at(&cache, ADAMLAR, 2_000).unwrap();
        assert_eq!(*got, *list);
        assert_eq!(state, Freshness::Fresh);
        assert_eq!(
            cached_at(&cache, ADAMLAR, 1_000 + FRESH_MS).unwrap().1,
            Freshness::Stale
        );
        assert_eq!(
            cached_at(&cache, ADAMLAR, 1_000 + STALE_MAX_MS).unwrap().1,
            Freshness::Expired
        );
        // Boş sonuç (olumsuz önbellek).
        let other = "spotify:artist:b";
        let empty = Arc::new(RelatedList {
            source: RelatedSource::Spotify,
            artists: vec![],
        });
        store_at(&cache, other, &empty, 0).unwrap();
        assert_eq!(
            cached_at(&cache, other, EMPTY_MS - 1).unwrap().1,
            Freshness::Fresh
        );
        assert_eq!(
            cached_at(&cache, other, EMPTY_MS).unwrap().1,
            Freshness::Expired
        );
        // Sanatçı sayfasıyla aynı tabloda ama kendi anahtarında.
        assert!(
            cache
                .get::<RelatedList>(Kind::Related, &format!("artist:{ADAMLAR}"), 0)
                .unwrap()
                .is_none()
        );
    }

    /// Gerçek Deezer yanıtları (7 Kasım 2026, kırpılmış): Adamlar'ın popüler
    /// şarkısının ISRC araması ve `artist/6696895/related` (ilk 6).
    const DEEZER_TRACK: &str = include_str!("fixtures/deezer_track_isrc.json");
    const DEEZER_RELATED: &str = include_str!("fixtures/deezer_related_adamlar.json");

    #[test]
    fn deezer_parsing() {
        let track: Value = serde_json::from_str(DEEZER_TRACK).unwrap();
        assert_eq!(parse_deezer_track(&track, "Adamlar"), Some(6696895));
        assert_eq!(parse_deezer_track(&track, " adamlar "), Some(6696895));
        // Ad tutmuyorsa yanlış sanatçının benzerleri gösterilmez.
        assert_eq!(parse_deezer_track(&track, "Başkası"), None);
        assert_eq!(
            parse_deezer_track(&json!({ "error": { "code": 800 } }), "Adamlar"),
            None
        );
        assert_eq!(parse_deezer_track(&json!({}), "Adamlar"), None);
        // Konuk sanatçı da katkıcılardan bulunur.
        let featured = json!({ "artist": { "id": 1, "name": "Ana" },
            "contributors": [ { "id": 1, "name": "Ana" }, { "id": 99, "name": "Konuk" } ] });
        assert_eq!(parse_deezer_track(&featured, "konuk"), Some(99));

        let related: Value = serde_json::from_str(DEEZER_RELATED).unwrap();
        let cards = parse_deezer_related(&related).unwrap();
        assert_eq!(cards.len(), 6);
        assert!(
            cards
                .iter()
                .any(|c| c.name == "Duman" && c.uri == "deezer:artist:14826")
        );
        for c in &cards {
            assert!(c.uri.starts_with("deezer:artist:"));
            assert!(
                c.image.starts_with("https://") && c.image.contains("500x500"),
                "{}",
                c.image
            );
        }

        // Yer tutucu resim, yinelenen, kimliksiz, adsız.
        let edge = json!({ "data": [
            { "id": 2, "name": "Yer Tutucu",
              "picture_medium": "https://cdn-images.dzcdn.net/images/artist//250x250-000000-80-0-0.jpg",
              "picture_big": "https://cdn-images.dzcdn.net/images/artist//500x500-000000-80-0-0.jpg" },
            { "id": 2, "name": "Yer Tutucu" },
            { "name": "Kimliksiz" },
            { "id": 3, "name": "  " },
            { "id": 4, "name": "Orta", "picture_medium": "https://x/250x250.jpg" }
        ] });
        let cards = parse_deezer_related(&edge).unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].image, "", "boş md5'li yer tutucu resim atılır");
        assert_eq!(cards[1].image, "https://x/250x250.jpg");
        assert!(
            parse_deezer_related(
                &json!({ "error": { "message": "Quota limit exceeded", "code": 4 } })
            )
            .is_err()
        );
        assert!(parse_deezer_related(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn deezer_click_resolution_picks_exact_and_popular() {
        let hit = |uri: &str, name: &str| SearchHit {
            uri: uri.into(),
            name: name.into(),
            subtitle: "Sanatçı".into(),
            image: String::new(),
        };
        let hits = [
            hit("spotify:artist:1", "Kalben"),
            hit("spotify:artist:2", "KALBEN "),
            hit("spotify:artist:3", "Kalbenler"),
        ];
        let exact: Vec<&SearchHit> = hits
            .iter()
            .filter(|h| same_name(&h.name, "kalben"))
            .collect();
        assert_eq!(exact.len(), 2);
        let popularity = HashMap::from([
            ("spotify:artist:1".to_string(), 40),
            ("spotify:artist:2".to_string(), 61),
        ]);
        assert_eq!(
            pick_by_popularity(&exact, &popularity).unwrap(),
            "spotify:artist:2"
        );
        // Eşitlikte arama sırası.
        assert_eq!(
            pick_by_popularity(&exact, &HashMap::new()).unwrap(),
            "spotify:artist:1"
        );
        assert!(pick_by_popularity(&[], &popularity).is_none());
        assert!(!same_name("", ""));

        // Eşleme kalıcı.
        let cache = PageCache::open_in_memory().unwrap();
        assert!(resolved(&cache, "deezer:artist:1").is_none());
        put_link(
            &cache,
            &spotify_of_key("deezer:artist:1"),
            "spotify:artist:2",
        );
        assert_eq!(
            resolved(&cache, "deezer:artist:1").unwrap(),
            "spotify:artist:2"
        );
    }
}

/// Canlı sınama (elle, tek sefer: `cargo test --release live_related -- --ignored --nocapture`).
/// Spotify'a TEK `queryArtistRelated` isteği (hash eskiyse istemci en çok bir
/// tane daha yapar); sonucu gerçek sayfa önbelleğine yazar (arayüz sınaması yeniden
/// istemesin). `OPENIFY_RELATED_DUMP=<dosya>` verilirse ham yanıt oraya.
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn live_related_adamlar() {
        let uri = "spotify:artist:1g4J8P1JWwanNyyXckRX5W";
        let session = crate::auth::connect(None).await.expect("oturum");
        let t = Instant::now();
        let value = pathfinder::query(
            &session,
            None,
            &pathfinder::ARTIST_RELATED,
            json!({ "uri": uri }),
            Priority::Background,
        )
        .await
        .expect("istek");
        eprintln!("istek: {} ms", t.elapsed().as_millis());
        if let Some(path) = std::env::var_os("OPENIFY_RELATED_DUMP") {
            std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        }
        let cards = parse_spotify(&value, uri).expect("çözümleme");
        eprintln!("{} benzer sanatçı:", cards.len());
        for c in &cards {
            eprintln!("  {} | {} | {}", c.name, c.uri, c.image);
        }
        let cache = PageCache::open_default().expect("sayfa önbelleği");
        let list = Arc::new(RelatedList {
            source: RelatedSource::Spotify,
            artists: cards,
        });
        store_at(&cache, uri, &list, now_ms()).unwrap();
        eprintln!("önbelleğe yazıldı: related:{uri}");
    }
}
