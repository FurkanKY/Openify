//! Arkadaş etkinliği: Spotify masaüstünün "Friend Activity" verisi, arkadaşların
//! en son dinlediği şarkı.
//!
//!   GET https://guc-spclient.spotify.com/presence-view/v1/buddylist
//!   { "friends": [ { "timestamp": ms, "user": { uri, name, imageUrl },
//!                    "track": { uri, name, imageUrl, album: { uri, name },
//!                               artist: { uri, name },
//!                               context: { uri, name, index } } } ] }
//!
//! Bu uç resmî değil (masaüstü istemcinin kullandığı gayriresmî uç), yanıtın
//! biçimi bilgiye dayanır: çözümleme eksik alana toleranslıdır (adı olmayan
//! arkadaş kimliğiyle, bağlamı olmayan şarkı tek başına gösterilir; şarkısı
//! olmayan satır atlanır). Canlı yanıtla doğrulanmadı: sahibinin elle
//! koşacağı `live_buddylist` sınaması var (`#[ignore]`).
//!
//! İstek, token, kapı (429, aralık, devre kesici), 401'de token yenileme ve
//! 5xx yeniden denemesi `pathfinder.rs`'in ortak altyapısından geçer; kapısı
//! ayrı bir örnek (`pathfinder::presence`): bu uçtaki 429 aramayı ya da
//! Keşfet'i durdurmaz. Sonuçlar yalnız bellekte tutulur (arayüzde); diske
//! yazılmaz. Ne zaman yenileneceği (panel ve pencere görünürken 60 sn'de
//! bir) arayüzün işidir (`ui/friends.rs`).

use std::collections::HashSet;

use librespot::core::session::Session;
use serde_json::Value;

use super::pathfinder::{self, LiveTransport, Pathfinder, PathfinderError, Priority, Transport};

/// Arkadaş listesinin adresi (tek yerde).
pub const BUDDYLIST_URL: &str = "https://guc-spclient.spotify.com/presence-view/v1/buddylist";
/// Listede en çok bu kadar arkadaş tutulur (en yeni dinleyenler).
pub const MAX_FRIENDS: usize = 100;
/// Bir yazının (ad, şarkı) en çok kaç harfi alınır.
const TEXT_MAX: usize = 200;

/// Bir arkadaşın son dinlediği şarkı.
#[derive(Debug, Clone, PartialEq)]
pub struct Friend {
    /// `spotify:user:<kimlik>`.
    pub user_uri: String,
    pub name: String,
    /// Profil resminin adresi (https); yoksa boş.
    pub image: String,
    /// Şarkının kapağının adresi (https); yanıtta yoksa boş.
    pub track_image: String,
    /// Dinlemenin zamanı (unix ms); bilinmiyorsa 0.
    pub timestamp_ms: i64,
    pub track_uri: String,
    pub track: String,
    /// Sanatçı adları, virgülle.
    pub artist: String,
    /// İlk sanatçının adresi; yoksa boş.
    pub artist_uri: String,
    pub album: String,
    pub album_uri: String,
    /// Çalınan bağlam (liste, albüm, sanatçı …) ve şarkının oradaki sırası.
    pub context_uri: Option<String>,
    pub context_name: Option<String>,
    pub context_index: Option<u32>,
}

/// Alınamamanın nedeni (kod; yazısı arayüzde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendsFailure {
    /// Ağ ya da sunucu hatası, bağlantı yok.
    Offline,
    /// 429 ya da devre kesici: bir süre sorulmayacak.
    RateLimited,
    /// 401 / 403: bu hesap ya da bu token için uç kapalı.
    Denied,
    /// Uç kalkmış (404) ya da yanıt anlaşılmıyor.
    Unavailable,
}

impl From<&PathfinderError> for FriendsFailure {
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
            // Hash'li işlemlere özgü hatalar bu uçta gelmez.
            _ => Self::Unavailable,
        }
    }
}

// ── Çözümleme ───────────────────────────────────────────────────────────

/// Yanıtı arkadaş listesine çevirir: eksik alana toleranslı, en yeni önce,
/// her kullanıcı bir kez, en çok `MAX_FRIENDS`.
pub fn parse(value: &Value) -> Vec<Friend> {
    let Some(list) = value.get("friends").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut friends: Vec<Friend> = list.iter().filter_map(parse_friend).collect();
    // Kararlı sıralama: aynı anlıklar yanıttaki sırada kalır.
    friends.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
    let mut seen = HashSet::new();
    friends.retain(|f| seen.insert(f.user_uri.clone()));
    friends.truncate(MAX_FRIENDS);
    friends
}

fn parse_friend(v: &Value) -> Option<Friend> {
    let user = v.get("user")?;
    let user_uri = uri_of(user.get("uri"), "spotify:user:")?;
    let track = v.get("track")?;
    let track_uri = uri_of(track.get("uri"), "spotify:track:")?;
    let track_name = text(track.get("name"));
    if track_name.is_empty() {
        return None;
    }
    let name = match text(user.get("name")) {
        n if n.is_empty() => user_uri.rsplit(':').next().unwrap_or_default().to_string(),
        n => n,
    };

    // Sanatçı: tek nesne ya da liste (bilinmeyen biçime karşı ikisi de okunur).
    let mut artists: Vec<(String, String)> = Vec::new();
    if let Some(one) = track.get("artist").filter(|a| a.is_object()) {
        artists.push((
            text(one.get("name")),
            uri_of(one.get("uri"), "spotify:artist:").unwrap_or_default(),
        ));
    }
    if artists.is_empty()
        && let Some(many) = track.get("artists").and_then(Value::as_array)
    {
        artists.extend(many.iter().map(|a| {
            (
                text(a.get("name")),
                uri_of(a.get("uri"), "spotify:artist:").unwrap_or_default(),
            )
        }));
    }
    artists.retain(|(n, _)| !n.is_empty());
    let artist_uri = artists.first().map(|a| a.1.clone()).unwrap_or_default();
    let artist = artists
        .iter()
        .map(|a| a.0.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let album = track.get("album");
    let album_name = text(album.and_then(|a| a.get("name")));
    let album_uri = uri_of(album.and_then(|a| a.get("uri")), "spotify:album:").unwrap_or_default();

    let context = track.get("context");
    let context_uri = uri_of(context.and_then(|c| c.get("uri")), "spotify:");
    let context_name = Some(text(context.and_then(|c| c.get("name")))).filter(|n| !n.is_empty());
    let context_index = context
        .and_then(|c| c.get("index"))
        .and_then(|i| {
            i.as_u64()
                .or_else(|| i.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .and_then(|i| u32::try_from(i).ok());

    Some(Friend {
        user_uri,
        name,
        image: image_url(user.get("imageUrl")),
        track_image: image_url(track.get("imageUrl")),
        timestamp_ms: timestamp_ms(v.get("timestamp")),
        track_uri,
        track: track_name,
        artist,
        artist_uri,
        album: album_name,
        album_uri,
        context_name: context_uri.as_ref().and(context_name),
        context_index: context_uri.as_ref().and(context_index),
        context_uri,
    })
}

/// Düzgün biçimli bir Spotify adresi (`prefix` ile başlayan, kimliği olan).
fn uri_of(v: Option<&Value>, prefix: &str) -> Option<String> {
    let s = v?.as_str()?.trim();
    (s.starts_with(prefix) && s.len() > prefix.len() && !s.chars().any(char::is_control))
        .then(|| s.to_string())
}

/// Görünecek yazı: kırpılmış, denetim karakterleri atılmış, en çok `TEXT_MAX` harf.
fn text(v: Option<&Value>) -> String {
    let s = v.and_then(Value::as_str).unwrap_or_default();
    s.chars()
        .filter(|c| !c.is_control())
        .take(TEXT_MAX)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Profil resmi ve şarkı kapağı: yalnız https (http'ye https'e çevrilir);
/// `spotify:image:<kimlik>` Spotify'ın resim sunucusuna çevrilir; başka şema atılır.
fn image_url(v: Option<&Value>) -> String {
    let s = v.and_then(Value::as_str).unwrap_or_default().trim();
    if let Some(id) = s
        .strip_prefix("spotify:image:")
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        format!("https://i.scdn.co/image/{id}")
    } else if s.starts_with("https://") {
        s.to_string()
    } else if let Some(rest) = s.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        String::new()
    }
}

/// Unix ms (sayı ya da sayı yazısı); saniye gibi küçük bir değer ms'ye çevrilir.
fn timestamp_ms(v: Option<&Value>) -> i64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
    .unwrap_or(0);
    if n > 0 && n < 100_000_000_000 {
        n * 1000
    } else {
        n.max(0)
    }
}

// ── Alma ────────────────────────────────────────────────────────────────

/// Arkadaş listesini alır (kapı, token yenileme, yeniden deneme ortak
/// istemciden).
pub async fn load_with(
    pf: &Pathfinder,
    transport: &dyn Transport,
) -> Result<Vec<Friend>, FriendsFailure> {
    match pf
        .get_json(transport, "buddylist", BUDDYLIST_URL, Priority::Background)
        .await
    {
        Ok(value) => Ok(parse(&value)),
        Err(e) => {
            log::info!("arkadaş etkinliği alınamadı: {e}");
            Err(FriendsFailure::from(&e))
        }
    }
}

/// Canlı oturumla (`pathfinder::presence` kapısından).
pub async fn load(session: &Session) -> Result<Vec<Friend>, FriendsFailure> {
    load_with(pathfinder::presence(), &LiveTransport::new(session)).await
}

// ── Çalma ───────────────────────────────────────────────────────────────

/// Satıra tıklayınca ne çalınır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayPlan {
    /// Bağlamın şarkıları, bu şarkıdan başlayarak (`hint`: şarkının bağlamdaki
    /// sırası, birden çok yerde geçiyorsa en yakını seçmek için).
    Context {
        uri: String,
        track: String,
        hint: Option<usize>,
    },
    /// Yalnız şarkı (bağlam yok ya da çözülemeyen türde: koleksiyon, radyo …).
    Track(String),
}

pub fn play_plan(f: &Friend) -> PlayPlan {
    let playable = |uri: &str| {
        ["spotify:playlist:", "spotify:album:", "spotify:artist:"]
            .iter()
            .any(|p| uri.starts_with(p))
    };
    match &f.context_uri {
        Some(uri) if playable(uri) => PlayPlan::Context {
            uri: uri.clone(),
            track: f.track_uri.clone(),
            hint: f.context_index.map(|i| i as usize),
        },
        _ => PlayPlan::Track(f.track_uri.clone()),
    }
}

/// Bağlamın şarkı adresleri içinde başlanacak yer: şarkının kendisi; birden çok
/// yerde geçiyorsa ipucuna en yakını. Şarkı bağlamda yoksa `None` (yanlış
/// şarkıdan başlamak yerine şarkı tek başına çalınır).
pub fn start_position(uris: &[String], track: &str, hint: Option<usize>) -> Option<usize> {
    let mut found = uris
        .iter()
        .enumerate()
        .filter(|(_, u)| u.as_str() == track)
        .map(|(i, _)| i);
    let first = found.next()?;
    match hint {
        Some(h) => Some(
            std::iter::once(first)
                .chain(found)
                .min_by_key(|i| i.abs_diff(h))
                .unwrap_or(first),
        ),
        None => Some(first),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use bytes::Bytes;
    use futures::future::BoxFuture;

    use super::*;
    use crate::engine::pathfinder::{Config, DiscoverError, Request, Response, TransportError};

    const FIXTURE: &str = include_str!("fixtures/friend_buddylist.json");
    pub(crate) const DEMO: &str = include_str!("fixtures/friend_activity_demo.json");

    fn fixture() -> Vec<Friend> {
        parse(&serde_json::from_str(FIXTURE).unwrap())
    }

    #[test]
    fn parses_the_recorded_shape() {
        let friends = fixture();
        let names: Vec<&str> = friends.iter().map(|f| f.name.as_str()).collect();
        // Şarkısı / adresi bozuk olanlar, yanlış kullanıcı adresi ve ikinci
        // (eski) Aylin kaydı yok; en yeni önde, zamansız olan sonda.
        assert_eq!(names, ["Aylin", "Berk", "Cem", "12345678abc", "Gizem"]);

        let a = &friends[0];
        assert_eq!(a.user_uri, "spotify:user:aylin");
        assert_eq!(a.image, "https://i.scdn.co/image/ab6775700000ee85aylin");
        // Şarkı kapağı (http → https).
        assert_eq!(
            a.track_image,
            "https://i.scdn.co/image/ab67616d000048510001"
        );
        assert_eq!(a.timestamp_ms, 1762880000000);
        assert_eq!(
            (a.track_uri.as_str(), a.track.as_str()),
            ("spotify:track:t01", "Gece Yolcusu")
        );
        assert_eq!(
            (a.artist.as_str(), a.artist_uri.as_str()),
            ("Dalga Boyu", "spotify:artist:r01")
        );
        assert_eq!(
            (a.album.as_str(), a.album_uri.as_str()),
            ("Karanlik Oda", "spotify:album:a01")
        );
        assert_eq!(a.context_uri.as_deref(), Some("spotify:playlist:p01"));
        assert_eq!(a.context_name.as_deref(), Some("Gece Surusu"));
        assert_eq!(a.context_index, Some(4));
    }

    #[test]
    fn tolerates_missing_and_odd_fields() {
        let friends = fixture();
        // http profil resmi https'e çevrilir.
        assert_eq!(friends[1].image, "https://profile.example/berk.jpg");
        // Sayı yazısı olan zaman, sanatçı listesi (adlar birleşir, ilk adres).
        let cem = &friends[2];
        assert_eq!(cem.timestamp_ms, 1762878000000);
        assert_eq!(
            (cem.artist.as_str(), cem.artist_uri.as_str()),
            ("Deniz, Orman", "spotify:artist:r03")
        );
        assert_eq!(
            (cem.album.as_str(), cem.context_uri.clone()),
            ("Kisa Hikaye", None)
        );
        // Adı yok: kimliğin sonu; tehlikeli resim adresi atılır; sanatçı adresi yok.
        let anon = &friends[3];
        assert_eq!(anon.name, "12345678abc");
        assert_eq!(anon.image, "");
        assert_eq!(anon.track_image, "");
        // `spotify:image:<kimlik>` resim sunucusuna çevrilir; bozuk kimlik atılır.
        assert_eq!(
            image_url(Some(&serde_json::json!("spotify:image:ab67616d00001e02"))),
            "https://i.scdn.co/image/ab67616d00001e02"
        );
        assert_eq!(image_url(Some(&serde_json::json!("spotify:image:"))), "");
        assert_eq!(
            image_url(Some(&serde_json::json!("spotify:image:a/../b"))),
            ""
        );
        assert_eq!(
            (anon.artist.as_str(), anon.artist_uri.as_str()),
            ("Bilinmeyen", "")
        );
        assert_eq!(anon.album, "");
        // Zamanı olmayan arkadaş atılmaz, 0 olur.
        assert_eq!(friends[4].timestamp_ms, 0);
    }

    #[test]
    fn empty_or_unexpected_bodies_give_no_friends() {
        for body in [
            "{}",
            r#"{"friends":[]}"#,
            r#"{"friends":"x"}"#,
            "[]",
            "null",
            r#"{"friends":[1,null,{}]}"#,
        ] {
            assert!(
                parse(&serde_json::from_str(body).unwrap()).is_empty(),
                "{body}"
            );
        }
    }

    #[test]
    fn seconds_are_scaled_and_the_list_is_capped_newest_first() {
        let v: Value = serde_json::from_str(
            r#"{"friends":[{"timestamp":1762880000,"user":{"uri":"spotify:user:s"},"track":{"uri":"spotify:track:x","name":"N"}}]}"#,
        )
        .unwrap();
        assert_eq!(parse(&v)[0].timestamp_ms, 1762880000000);

        let many: Vec<Value> = (0..MAX_FRIENDS + 25)
            .map(|i| {
                serde_json::json!({
                    "timestamp": 1_762_000_000_000i64 + i as i64,
                    "user": { "uri": format!("spotify:user:u{i}"), "name": format!("U{i}") },
                    "track": { "uri": "spotify:track:x", "name": "N" }
                })
            })
            .collect();
        let friends = parse(&serde_json::json!({ "friends": many }));
        assert_eq!(friends.len(), MAX_FRIENDS);
        assert_eq!(
            friends[0].name,
            format!("U{}", MAX_FRIENDS + 24),
            "en yeni önde"
        );
    }

    #[test]
    fn demo_fixture_parses_every_friend() {
        // Ekran sınaması fikstürü: `ageSeconds` yalnız sınama kancasının alanı,
        // burada zaman 0 olur ama hiçbir satır düşmez.
        let friends = parse(&serde_json::from_str(DEMO).unwrap());
        assert_eq!(friends.len(), 9);
    }

    #[test]
    fn play_plan_uses_playable_contexts_only() {
        let friends = fixture();
        assert_eq!(
            play_plan(&friends[0]),
            PlayPlan::Context {
                uri: "spotify:playlist:p01".into(),
                track: "spotify:track:t01".into(),
                hint: Some(4)
            }
        );
        // Albüm bağlamı çalınabilir.
        assert!(
            matches!(play_plan(&friends[1]), PlayPlan::Context { ref uri, hint: Some(1), .. } if uri == "spotify:album:a02")
        );
        // Bağlamı olmayan ve koleksiyon bağlamı: yalnız şarkı.
        assert_eq!(
            play_plan(&friends[2]),
            PlayPlan::Track("spotify:track:t03".into())
        );
        assert_eq!(
            play_plan(&friends[3]),
            PlayPlan::Track("spotify:track:t04".into())
        );
    }

    #[test]
    fn start_position_prefers_the_track_and_the_closest_repeat() {
        let uris: Vec<String> = ["a", "b", "c", "b", "d"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(start_position(&uris, "c", None), Some(2));
        assert_eq!(
            start_position(&uris, "b", None),
            Some(1),
            "ipucu yoksa ilk geçtiği yer"
        );
        assert_eq!(
            start_position(&uris, "b", Some(3)),
            Some(3),
            "ipucuna yakın olan"
        );
        assert_eq!(start_position(&uris, "b", Some(0)), Some(1));
        assert_eq!(
            start_position(&uris, "z", Some(2)),
            None,
            "bağlamda yok: yanlış şarkıdan başlanmaz"
        );
        assert_eq!(start_position(&[], "a", None), None);
    }

    // ── Alma: sahte ağ ─────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeNet {
        responses: Mutex<VecDeque<Result<Response, TransportError>>>,
        urls: Mutex<Vec<String>>,
        refreshes: AtomicUsize,
    }

    fn reply(status: u16, body: &str) -> Result<Response, TransportError> {
        Ok(Response {
            status,
            retry_after: None,
            body: Bytes::from(body.to_string()),
        })
    }

    impl Transport for FakeNet {
        fn post<'a>(&'a self, _: &'a Request) -> BoxFuture<'a, Result<Response, TransportError>> {
            Box::pin(async { panic!("buddylist POST kullanmaz") })
        }
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Response, TransportError>> {
            Box::pin(async move {
                self.urls.lock().unwrap().push(url.to_string());
                self.responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("beklenmeyen istek")
            })
        }
        fn refresh_token<'a>(&'a self) -> BoxFuture<'a, Result<(), TransportError>> {
            Box::pin(async move {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
        fn discover<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, Result<String, DiscoverError>> {
            Box::pin(async { panic!("buddylist hash aramaz") })
        }
    }

    fn fast() -> Pathfinder {
        Pathfinder::new(Config {
            interval: Duration::ZERO,
            server_retries: 1,
            backoff_base: Duration::from_millis(1),
        })
    }

    #[tokio::test]
    async fn loads_through_the_shared_client_with_the_single_url() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().push_back(reply(200, FIXTURE));
        let friends = load_with(&fast(), &net).await.unwrap();
        assert_eq!(friends.len(), 5);
        assert_eq!(*net.urls.lock().unwrap(), [BUDDYLIST_URL]);
    }

    #[tokio::test]
    async fn a_401_refreshes_the_token_once_then_succeeds() {
        let net = FakeNet::default();
        net.responses
            .lock()
            .unwrap()
            .extend([reply(401, ""), reply(200, r#"{"friends":[]}"#)]);
        assert!(load_with(&fast(), &net).await.unwrap().is_empty());
        assert_eq!(net.refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(net.urls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn failures_become_typed_codes() {
        let code = |responses: Vec<Result<Response, TransportError>>| async move {
            let net = FakeNet::default();
            net.responses.lock().unwrap().extend(responses);
            load_with(&fast(), &net).await.unwrap_err()
        };
        assert_eq!(
            code(vec![reply(401, ""), reply(403, "")]).await,
            FriendsFailure::Denied
        );
        assert_eq!(code(vec![reply(403, "")]).await, FriendsFailure::Denied);
        assert_eq!(
            code(vec![reply(429, "")]).await,
            FriendsFailure::RateLimited
        );
        assert_eq!(
            code(vec![reply(404, "")]).await,
            FriendsFailure::Unavailable
        );
        assert_eq!(
            code(vec![reply(200, "not json")]).await,
            FriendsFailure::Unavailable
        );
        assert_eq!(
            code(vec![reply(500, ""), reply(502, "")]).await,
            FriendsFailure::Offline
        );
        assert_eq!(
            code(vec![Err(TransportError::Network("down".into()))]).await,
            FriendsFailure::Offline
        );
        assert_eq!(
            code(vec![Err(TransportError::Token("none".into()))]).await,
            FriendsFailure::Offline
        );
    }

    #[tokio::test]
    async fn a_429_blocks_the_next_request_without_sending_it() {
        let pf = fast();
        let net = FakeNet::default();
        net.responses.lock().unwrap().push_back(Ok(Response {
            status: 429,
            retry_after: Some(Duration::from_secs(120)),
            body: Bytes::new(),
        }));
        assert_eq!(
            load_with(&pf, &net).await.unwrap_err(),
            FriendsFailure::RateLimited
        );
        // Retry-After süresince ikinci istek ağa hiç çıkmaz.
        assert_eq!(
            load_with(&pf, &net).await.unwrap_err(),
            FriendsFailure::RateLimited
        );
        assert_eq!(net.urls.lock().unwrap().len(), 1);
    }

    // ── Canlı (sahibi onaylayınca elle) ────────────────────────────────

    /// CANLI okuma sınaması: tek bir `buddylist` isteği, salt okuma, hiçbir şey
    /// yazmaz. Gerçek Spotify'a gider; yalnız sahibinin onayıyla, elle koşulur:
    ///
    ///   cargo test --release live_buddylist -- --ignored --nocapture
    ///
    /// Arkadaşların adı / şarkısı yazdırılmaz (özel veri): yalnız sayı ve
    /// alanların dolu olup olmadığı. Uç oturum token'ını kabul etmezse
    /// `Denied` döner; bu da değerli bir sonuçtur (özellik o hesapta çalışmaz).
    #[tokio::test]
    #[ignore = "canli Spotify: yalniz sahibi onayiyla elle kosulur"]
    async fn live_buddylist() {
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

        match load(&session).await {
            Ok(friends) => {
                println!("buddylist: {} arkadas", friends.len());
                let with = |f: fn(&Friend) -> bool| friends.iter().filter(|x| f(x)).count();
                println!(
                    "  resim {} / baglam {} / baglam sirasi {} / sanatci adresi {} / album adresi {}",
                    with(|f| !f.image.is_empty()),
                    with(|f| f.context_uri.is_some()),
                    with(|f| f.context_index.is_some()),
                    with(|f| !f.artist_uri.is_empty()),
                    with(|f| !f.album_uri.is_empty()),
                );
                if let Some(newest) = friends.first() {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    println!(
                        "  en yeni dinleme {} sn once",
                        (now - newest.timestamp_ms) / 1000
                    );
                }
            }
            Err(e) => println!("buddylist alinamadi: {e:?}"),
        }
    }
}
