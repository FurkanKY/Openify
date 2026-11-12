//! Radyo: bir şarkıdan, sanatçıdan, albümden ya da listeden "benzer şarkılar"
//! akışı (Spotify'ın istasyon verisi; sonsuz radyo).
//!
//!   GET https://spclient.wg.spotify.com/radio-apollo/v3/stations/{tohum}
//!       ?autoplay=true&count={n}[&prev_tracks={id,id,…}]
//!   { "uri": "spotify:station:…", "title"?,
//!     "tracks": [ { "uri": "spotify:track:…", "original_gid"?, … }, … ],
//!     "next_page_url"? }
//!
//! `radio-apollo` Spotify'ın eski masaüstü istemcisinin radyo ucudur ve
//! librespot da aynı adresi taşır (`SpClient::get_apollo_station`); web
//! player'ın "Şarkı radyosu" menüsü ise `inspiredby-mix` ucuyla bir liste
//! kurdurur. Burada `radio-apollo` seçildi çünkü devamı (`prev_tracks`) ister:
//! sonsuz radyo için gerekli olan "şunları çaldım, yenilerini ver" bu uçta
//! var, `inspiredby-mix` ise sabit bir liste üretir. Resmî bir uç değildir ve
//! yanıtın biçimi bilgiye dayanır; bu yüzden çözümleme eksik / garip alana
//! toleranslıdır (şarkı listesi `tracks` / `mediaItems` / `items`, bir şarkı
//! düz adres ya da nesne olabilir; yalnız `spotify:track:` adresleri alınır,
//! tekrarlar atılır). Canlı yanıtla doğrulanmadı: sahibinin elle koşacağı
//! `live_radio` sınaması var (`#[ignore]`).
//!
//! Uç TEK sabit (`STATION_BASE`). İstek, token, kapı (aralık, 429, devre
//! kesici), 401'de token yenileme ve 5xx yeniden denemesi `pathfinder.rs`'in
//! ortak altyapısından geçer (`Pathfinder::get_json`) ve kapısı ayrıdır
//! (`pathfinder::radio`): radyodaki bir 429 aramayı ya da Keşfet'i durdurmaz.
//! SALT OKUMA: hesaba hiçbir şey yazılmaz.
//!
//! Gelen şarkıların adresleri ortak "şarkı bilgisi" toplu isteğinden geçer
//! (`library::fetch_tracks`, 100'lük tek istek) ve çalma bağlamı olarak
//! kurulur (`Engine::start_radio`); sıra sonuna yaklaşınca (`REFILL_AT`)
//! devamı istenir (`Feed::due`): tek seferde en çok `BATCH_MAX` şarkı, iki
//! istek arası en az `MIN_INTERVAL` (hata sonrası katlanarak uzar), bağlam en
//! çok `CTX_MAX` şarkı. Karıştırma kuralları (hiç çalma, açık sözlüler,
//! gizlenen şarkılar) radyoda da `Queue::pool` üzerinden geçerlidir.

use std::time::{Duration, Instant};

use librespot::core::session::Session;
use serde_json::Value;

use super::{
    credits::track_id,
    hidden::HiddenList,
    library::TrackRow,
    pathfinder::{self, LiveTransport, Pathfinder, PathfinderError, Priority, Transport},
    rules::{Rules, Traits},
};

/// İstasyon adresinin tabanı (tek yerde); `{base}/{tohum adresi}`.
pub const STATION_BASE: &str = "https://spclient.wg.spotify.com/radio-apollo/v3/stations";
/// Bir istekte istenen ve kabul edilen en çok şarkı.
pub const BATCH_MAX: usize = 50;
/// Kalan bağlam şarkısı bu sayıya inince devamı istenir.
pub const REFILL_AT: usize = 10;
/// İki devam isteği arasındaki en kısa süre (hata sonrası `2^hata` katı).
pub const MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Bağlamın büyüyebileceği en çok şarkı (sonsuz radyo belleği şişirmesin).
pub const CTX_MAX: usize = 1000;
/// `prev_tracks` ile bildirilen en çok şarkı (adres uzamasın).
pub const PREV_MAX: usize = 50;
/// Üst üste bu kadar boş / başarısız devamdan sonra radyo bırakılır (çalan
/// sıra bitene kadar çalar, yeni istek gitmez).
pub const MAX_FAILURES: u32 = 3;
/// Başlığın (arayüzde sıra başlığı) en çok harfi.
pub const TITLE_MAX: usize = 80;

/// Çalan radyonun kimliği: tohum adresi ve gösterilecek ad (sıra başlığı).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioInfo {
    pub seed: String,
    pub title: String,
}

// ── Tohum ve adres ──────────────────────────────────────────────────────

/// Radyosu başlatılabilen adres: şarkı, sanatçı, albüm ya da liste.
pub fn seed_ok(uri: &str) -> bool {
    ["spotify:track:", "spotify:artist:", "spotify:album:", "spotify:playlist:"].iter().any(|p| {
        uri.strip_prefix(p)
            .is_some_and(|id| !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
    })
}

/// Şarkı tohumu mu (radyo o şarkıyla başlar)?
pub fn is_track_seed(uri: &str) -> bool {
    uri.starts_with("spotify:track:")
}

/// İstasyon isteğinin adresi. `prev`: şimdiye dek verilen şarkılar (en son
/// `PREV_MAX` tanesinin kimliği gider; geçersiz adres atlanır).
pub fn station_url(seed: &str, count: usize, prev: &[String]) -> String {
    let count = count.clamp(1, BATCH_MAX);
    let mut url = format!("{STATION_BASE}/{seed}?autoplay=true&count={count}");
    let ids: Vec<&str> = prev.iter().filter_map(|u| track_id(u)).collect();
    let tail = &ids[ids.len().saturating_sub(PREV_MAX)..];
    if !tail.is_empty() {
        url.push_str("&prev_tracks=");
        url.push_str(&tail.join(","));
    }
    url
}

// ── Çözümleme ───────────────────────────────────────────────────────────

/// Yanıttaki bir şarkı: adres + (yanıt taşıyorsa) künye.
#[derive(Debug, Clone, PartialEq)]
pub struct RadioTrack {
    pub uri: String,
    pub row: Option<TrackRow>,
}

/// Çözümlenmiş yanıt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Station {
    /// Yanıttaki başlık (yoksa boş).
    pub title: String,
    pub tracks: Vec<RadioTrack>,
}

fn text(v: Option<&Value>) -> String {
    let s = v.and_then(Value::as_str).unwrap_or_default();
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

/// Sanatçılar: düz yazı, yazı dizisi ya da `{ "name" }` dizisi.
fn artists(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(_)) => text(v),
        Some(Value::Array(list)) => list
            .iter()
            .map(|a| if a.is_object() { text(a.get("name")) } else { text(Some(a)) })
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

fn parse_track(v: &Value) -> Option<RadioTrack> {
    let uri = match v {
        Value::String(_) => text(Some(v)),
        Value::Object(_) => {
            ["uri", "trackUri", "track_uri"].iter().find_map(|k| Some(text(v.get(k))).filter(|t| !t.is_empty()))?
        }
        _ => return None,
    };
    track_id(&uri)?;
    // Yanıt künye de taşıyorsa (ad şart) kullanılır; yoksa motor toplu istekle alır.
    let row = v.is_object().then(|| text(v.get("name"))).filter(|n| !n.is_empty()).map(|name| TrackRow {
        uri: uri.clone(),
        name,
        artists: artists(v.get("artists").or_else(|| v.get("artistNames"))),
        album: match v.get("album") {
            Some(a) if a.is_object() => text(a.get("name")),
            other => text(other),
        },
        duration_ms: ["duration_ms", "durationMs", "duration"]
            .iter()
            .find_map(|k| v.get(k).and_then(Value::as_u64))
            .map_or(0, |d| d.min(u64::from(u32::MAX)) as u32),
        image: String::new(),
    });
    Some(RadioTrack { uri, row })
}

/// Yanıtı çözümler. Boş / ilgisiz yanıt boş istasyondur. En çok `BATCH_MAX`
/// şarkı, tekrarsız, yanıttaki sırayla.
pub fn parse(value: &Value) -> Station {
    let mut tracks: Vec<RadioTrack> = Vec::new();
    for key in ["tracks", "mediaItems", "items"] {
        let Some(list) = value.get(key).and_then(Value::as_array) else { continue };
        for item in list {
            if let Some(t) = parse_track(item)
                && !tracks.iter().any(|x| x.uri == t.uri)
            {
                tracks.push(t);
            }
        }
    }
    tracks.truncate(BATCH_MAX);
    let title: String = text(value.get("title").or_else(|| value.get("name"))).chars().take(TITLE_MAX).collect();
    Station { title, tracks }
}

// ── Başlangıç şarkısı ───────────────────────────────────────────────────

/// Radyo yeni başlarken ilk çalacak şarkı: şarkı tohumunda tohumun kendisi
/// (kullanıcı onu seçti: kurallara bakılmaz); başka tohumda kuralların
/// (hiç çalma, açık söz, gizlenen) dışlamadığı ilk şarkı. Hepsi dışlanmışsa
/// 0 (kullanıcı radyoyu kendisi başlattı).
pub fn first_playable(
    rows: &[TrackRow],
    traits: &dyn Fn(&str) -> Option<Traits>,
    rules: &Rules,
    hidden: &HiddenList,
    seed_is_track: bool,
) -> usize {
    if seed_is_track {
        return 0;
    }
    rows.iter()
        .position(|r| !hidden.hides(None, &r.uri) && !rules.excludes(r, traits(&r.uri).as_ref()))
        .unwrap_or(0)
}

// ── Devam zamanlaması ───────────────────────────────────────────────────

/// Devam isteğinin zamanlaması (motor durumu; saf: zaman dışarıdan verilir).
#[derive(Debug, Default)]
pub struct Feed {
    in_flight: bool,
    last: Option<Instant>,
    failures: u32,
    /// Radyo nesli: yeni radyo başlayınca artar; eski radyonun gecikmiş devam
    /// yanıtı bununla ayıklanır.
    generation: u64,
    /// Başlatılması istenen (yanıtı beklenen) en yeni radyonun tohumu.
    wanted: Option<String>,
}

impl Feed {
    /// Yeni bir radyo isteniyor (yanıtı gelince başlar; daha yeni bir istek
    /// öncekini geçersiz kılar).
    pub fn want(&mut self, seed: &str) {
        self.wanted = Some(seed.to_string());
    }

    /// Başlangıç yanıtı beklenen radyonunsa `true` (ve beklenen düşer).
    pub fn take_wanted(&mut self, seed: &str) -> bool {
        if self.wanted.as_deref() == Some(seed) {
            self.wanted = None;
            true
        } else {
            false
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Yeni radyo başladı: sayaçlar sıfır, nesil artar, ilk isteğin zamanı
    /// kaydedilir (devam hemen arkasından istenmesin).
    pub fn started(&mut self, now: Instant) {
        self.in_flight = false;
        self.last = Some(now);
        self.failures = 0;
        self.generation += 1;
    }

    #[cfg(test)]
    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// Şimdi devam istenmeli mi? `remaining`: bağlamda çalınmayı bekleyen
    /// şarkı, `ctx_len`: bağlamın uzunluğu.
    pub fn due(&self, now: Instant, remaining: usize, ctx_len: usize) -> bool {
        if self.in_flight || remaining > REFILL_AT || ctx_len >= CTX_MAX || self.failures >= MAX_FAILURES {
            return false;
        }
        self.last.is_none_or(|t| now.saturating_duration_since(t) >= MIN_INTERVAL * (1 << self.failures.min(4)))
    }

    pub fn begin(&mut self, now: Instant) {
        self.in_flight = true;
        self.last = Some(now);
    }

    /// İstek bitti. `added`: bağlama giren yeni şarkı sayısı (0: boş ya da hata).
    pub fn end(&mut self, added: usize) {
        self.in_flight = false;
        if added > 0 {
            self.failures = 0;
        } else {
            self.failures += 1;
        }
    }

    #[cfg(test)]
    pub fn failures(&self) -> u32 {
        self.failures
    }
}

// ── Alma ────────────────────────────────────────────────────────────────

/// Alınamamanın nedeni (kod; yazısı arayüzde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioFailure {
    /// Ağ ya da sunucu hatası.
    Offline,
    /// 429 ya da devre kesici.
    RateLimited,
    /// 401 / 403: bu hesap ya da bu token için uç kapalı.
    Denied,
    /// Uç kalkmış, tohum tanınmıyor ya da yanıt anlaşılmıyor.
    Unavailable,
    /// Yanıt geldi ama içinde çalınacak şarkı yok.
    Empty,
}

impl From<&PathfinderError> for RadioFailure {
    fn from(e: &PathfinderError) -> Self {
        match e {
            PathfinderError::RateLimited { .. } | PathfinderError::Blocked { .. } | PathfinderError::CircuitOpen => {
                Self::RateLimited
            }
            PathfinderError::Denied { .. } => Self::Denied,
            PathfinderError::Http { status: 400 | 404 | 410 } | PathfinderError::InvalidResponse(_) => {
                Self::Unavailable
            }
            PathfinderError::Token(_) | PathfinderError::Transport(_) | PathfinderError::Http { .. } => Self::Offline,
            _ => Self::Unavailable,
        }
    }
}

/// Sınama kancası `OPENIFY_TEST_RADIO_JSON=<dosya>`: yanıt dosyadan okunur,
/// ağa çıkılmaz. Yalnız sınama koşusunda (`OPENIFY_TEST_DISPLAY` ya da
/// `OPENIFY_SETTINGS_PATH` varken) geçerli; dosya okunamazsa / çözülemezse yok.
pub fn test_fixture() -> Option<Value> {
    let path = std::env::var_os("OPENIFY_TEST_RADIO_JSON")?;
    if std::env::var_os("OPENIFY_TEST_DISPLAY").is_none() && std::env::var_os("OPENIFY_SETTINGS_PATH").is_none() {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// İstasyonu alır (kapı, token yenileme, yeniden deneme ortak istemciden).
/// `first`: kullanıcı bekliyor (kapıda sıraya girmez); değilse devam isteği
/// (arka plan: sıraya girer, devre kesiciye uyar).
pub async fn load_with(
    pf: &Pathfinder,
    transport: &dyn Transport,
    seed: &str,
    prev: &[String],
    first: bool,
) -> Result<Station, RadioFailure> {
    if !seed_ok(seed) {
        return Err(RadioFailure::Unavailable);
    }
    let priority = if first { Priority::Interactive } else { Priority::Background };
    let url = station_url(seed, BATCH_MAX, prev);
    match pf.get_json(transport, "radio", &url, priority).await {
        Ok(value) => {
            let station = parse(&value);
            if station.tracks.is_empty() {
                log::info!("radyo: yanıtta çalınacak şarkı yok");
                Err(RadioFailure::Empty)
            } else {
                Ok(station)
            }
        }
        Err(e) => {
            log::info!("radyo alınamadı: {e}");
            Err(RadioFailure::from(&e))
        }
    }
}

/// Canlı oturumla (`pathfinder::radio` kapısından). Sınama kancası varsa
/// ağa çıkmadan dosyadan.
pub async fn load(session: &Session, seed: &str, prev: &[String], first: bool) -> Result<Station, RadioFailure> {
    if let Some(value) = test_fixture() {
        let station = parse(&value);
        return if station.tracks.is_empty() { Err(RadioFailure::Empty) } else { Ok(station) };
    }
    load_with(pathfinder::radio(), &LiveTransport::new(session), seed, prev, first).await
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use bytes::Bytes;
    use futures::future::BoxFuture;

    use super::*;
    use crate::engine::pathfinder::{Config, DiscoverError, Request, Response, TransportError};

    pub(crate) const FIXTURE: &str = include_str!("fixtures/radio_apollo.json");

    fn station() -> Station {
        parse(&serde_json::from_str(FIXTURE).unwrap())
    }

    // ── Çözümleme ──────────────────────────────────────────────────────

    #[test]
    fn parses_the_recorded_shape_keeping_order_and_dropping_junk() {
        let s = station();
        assert_eq!(s.title, "Gece Yolcusu Radyosu");
        let uris: Vec<&str> = s.tracks.iter().map(|t| t.uri.as_str()).collect();
        // Tekrar (radio7 iki kez), bölüm, bozuk adres atılır; sıra yanıttaki gibi.
        assert_eq!(uris.len(), 11);
        assert_eq!(uris[0], "spotify:track:0000000000000000radio1");
        assert_eq!(uris[6], "spotify:track:0000000000000000radio7");
        assert_eq!(uris[7], "spotify:track:0000000000000000radio9");
        assert!(uris.iter().all(|u| u.starts_with("spotify:track:")));
        // Künye varsa okunur: sanatçı dizisi, düz yazı, yazı dizisi, süre adları.
        let row = |i: usize| s.tracks[i].row.clone().unwrap();
        assert_eq!(
            (row(0).name.as_str(), row(0).artists.as_str(), row(0).album.as_str(), row(0).duration_ms),
            ("Sessiz Harf", "Kuzey Isigi", "Aralik", 212_000)
        );
        assert_eq!(row(1).artists, "Dalga Boyu, Ece Tan");
        assert_eq!((row(2).artists.as_str(), row(2).duration_ms), ("Mert Kaya", 243_000), "düz yazı sanatçı, durationMs");
        assert_eq!(row(4).artists, "Kuzey Isigi", "yazı dizisi");
        // Yalnız adres veren şarkı künyesiz (motor toplu istekle alır).
        assert!(s.tracks[6].row.is_none());
    }

    #[test]
    fn tolerates_other_key_names_plain_uris_and_garbage() {
        let v = serde_json::json!({
            "mediaItems": [ "spotify:track:aaa111", { "trackUri": "spotify:track:bbb222" }, 7, null, { "uri": "" }, "spotify:album:zzz" ],
            "items": [ { "uri": "spotify:track:aaa111" }, { "uri": "spotify:track:ccc333", "name": "  ", "artists": 5 } ],
        });
        let s = parse(&v);
        let uris: Vec<&str> = s.tracks.iter().map(|t| t.uri.as_str()).collect();
        assert_eq!(uris, ["spotify:track:aaa111", "spotify:track:bbb222", "spotify:track:ccc333"]);
        assert!(s.tracks.iter().all(|t| t.row.is_none()), "adsız künye kurulmaz");
        assert_eq!(s.title, "");
    }

    #[test]
    fn empty_and_unrelated_bodies_make_an_empty_station() {
        for body in
            ["{}", "[]", "null", "3", r#"{"tracks":[]}"#, r#"{"tracks":"x"}"#, r#"{"tracks":[{"uri":"spotify:episode:e"}]}"#]
        {
            assert!(parse(&serde_json::from_str(body).unwrap()).tracks.is_empty(), "{body}");
        }
    }

    #[test]
    fn a_huge_response_is_capped_and_texts_are_clean() {
        let list: Vec<Value> = (0..400).map(|i| serde_json::json!({ "uri": format!("spotify:track:t{i}") })).collect();
        assert_eq!(parse(&serde_json::json!({ "tracks": list })).tracks.len(), BATCH_MAX);
        let long = "a".repeat(500);
        let s = parse(&serde_json::json!({
            "title": format!("  {long}\n"),
            "tracks": [{ "uri": "spotify:track:x1", "name": "Ad\u{0007}\u{0000}ı" }]
        }));
        assert_eq!(s.title.chars().count(), TITLE_MAX);
        assert_eq!(s.tracks[0].row.as_ref().unwrap().name, "Adı", "denetim karakterleri atılır");
    }

    // ── Adres ──────────────────────────────────────────────────────────

    #[test]
    fn seeds_are_tracks_artists_albums_and_playlists_only() {
        for ok in [
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "spotify:artist:0TnOYISbd1XYRBk9myaseg",
            "spotify:album:1uyf3l2d4XYwiEqAb7t7fX",
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
        ] {
            assert!(seed_ok(ok), "{ok}");
        }
        for bad in [
            "",
            "liked",
            "spotify:track:",
            "spotify:episode:abc",
            "spotify:track:a/b",
            "spotify:track:a?b=1",
            "spotify:user:x",
            "local:::x:1",
            "spotify:track:a b",
        ] {
            assert!(!seed_ok(bad), "{bad}");
        }
        assert!(is_track_seed("spotify:track:x") && !is_track_seed("spotify:album:x"));
    }

    #[test]
    fn station_url_is_the_single_constant_with_count_and_the_last_ids() {
        let base = "https://spclient.wg.spotify.com/radio-apollo/v3/stations";
        assert_eq!(station_url("spotify:track:abc", 50, &[]), format!("{base}/spotify:track:abc?autoplay=true&count=50"));
        // Sayı sınırlanır; geçersiz adresler (bölüm, yerel) prev'e girmez.
        let prev = vec![
            "spotify:track:p1".to_string(),
            "spotify:episode:e".into(),
            "local:::x:1".into(),
            "spotify:track:p2".into(),
        ];
        assert_eq!(
            station_url("spotify:artist:art", 9_999, &prev),
            format!("{base}/spotify:artist:art?autoplay=true&count=50&prev_tracks=p1,p2")
        );
        assert!(station_url("spotify:artist:art", 0, &[]).ends_with("count=1"));
        // En çok PREV_MAX, en sondakiler.
        let many: Vec<String> = (0..200).map(|i| format!("spotify:track:t{i}")).collect();
        let url = station_url("spotify:track:abc", 30, &many);
        let ids = url.split("prev_tracks=").nth(1).unwrap().split(',').collect::<Vec<_>>();
        assert_eq!(ids.len(), PREV_MAX);
        assert_eq!((ids[0], ids[PREV_MAX - 1]), ("t150", "t199"));
    }

    // ── Başlangıç şarkısı ──────────────────────────────────────────────

    fn row(name: &str, artists: &str) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{name}"),
            name: name.into(),
            artists: artists.into(),
            album: String::new(),
            duration_ms: 1000,
            image: String::new(),
        }
    }

    #[test]
    fn the_first_song_skips_blocked_hidden_and_explicit_unless_the_seed_is_a_track() {
        let rows = vec![row("a", "Drake"), row("b", "Kaya"), row("c", "Tan"), row("d", "Aksu")];
        let rules = Rules::new(false, true, ["drake"]);
        let explicit = |u: &str| Some(Traits { explicit: u.ends_with(":c"), popularity: 0 });
        let hidden = HiddenList::new(vec![crate::engine::hidden::HiddenSong {
            uri: "spotify:track:b".into(),
            context: None,
            name: String::new(),
            artists: String::new(),
            hidden_at: 1,
        }]);
        // a: hiç çalma, b: gizli, c: açık söz → d.
        assert_eq!(first_playable(&rows, &explicit, &rules, &hidden, false), 3);
        // Şarkı tohumu: ilk şarkı (tohum) kurallara bakılmadan.
        assert_eq!(first_playable(&rows, &explicit, &rules, &hidden, true), 0);
        // Hiç kural yoksa ilk şarkı.
        assert_eq!(first_playable(&rows, &|_| None, &Rules::default(), &HiddenList::default(), false), 0);
        // Hepsi dışlanmışsa 0.
        let all = Rules::new(false, false, ["drake", "kaya", "tan", "aksu"]);
        assert_eq!(first_playable(&rows, &|_| None, &all, &HiddenList::default(), false), 0);
    }

    // ── Devam zamanlaması ──────────────────────────────────────────────

    #[test]
    fn the_follow_up_waits_for_the_end_of_the_queue_and_for_the_interval() {
        let t0 = Instant::now();
        let mut feed = Feed::default();
        feed.started(t0);
        let later = |s: u64| t0 + Duration::from_secs(s);
        // Sıra dolu: istek yok.
        assert!(!feed.due(later(600), REFILL_AT + 1, 50));
        // Sona yaklaştı ama aralık dolmadı (ilk istekten 10 sn).
        assert!(!feed.due(later(10), REFILL_AT, 50));
        assert!(!feed.due(later(29), 3, 50));
        assert!(feed.due(later(30), REFILL_AT, 50), "aralık dolunca");
        assert!(feed.due(later(30), 0, 50), "sıra bitmiş de olsa");
    }

    #[test]
    fn only_one_request_in_flight_and_failures_back_off_then_stop() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut feed = Feed::default();
        assert!(feed.due(at(0), 3, 50), "hiç istek yapılmadıysa hemen");
        feed.begin(at(0));
        assert!(feed.in_flight() && !feed.due(at(1000), 3, 50), "yoldayken ikincisi yok");
        // Boş / hatalı dönüş: aralık ikiye katlanır (30 → 60 → 120), 3'te bırakılır.
        feed.end(0);
        assert_eq!(feed.failures(), 1);
        assert!(!feed.due(at(59), 3, 50) && feed.due(at(60), 3, 50));
        feed.begin(at(60));
        feed.end(0);
        assert!(!feed.due(at(60 + 119), 3, 50) && feed.due(at(60 + 120), 3, 50));
        feed.begin(at(180));
        feed.end(0);
        assert_eq!(feed.failures(), MAX_FAILURES);
        assert!(!feed.due(at(100_000), 0, 50), "artık istek gitmez");
        // Yeni radyo her şeyi sıfırlar.
        feed.started(at(200));
        assert_eq!(feed.failures(), 0);
        assert!(feed.due(at(230), 3, 50));
        // Başarı hata sayacını sıfırlar.
        feed.begin(at(230));
        feed.end(0);
        feed.begin(at(300));
        feed.end(12);
        assert_eq!(feed.failures(), 0);
    }

    #[test]
    fn only_the_newest_wanted_radio_starts_and_old_follow_ups_are_told_apart() {
        let mut feed = Feed::default();
        feed.want("spotify:track:a");
        feed.want("spotify:track:b");
        assert!(!feed.take_wanted("spotify:track:a"), "daha yeni istek öncekini geçersiz kılar");
        assert!(feed.take_wanted("spotify:track:b"));
        assert!(!feed.take_wanted("spotify:track:b"), "bir kez");
        // Nesil: yeni radyo başlayınca artar; eski radyonun yanıtı bununla ayıklanır.
        let old = feed.generation();
        feed.started(Instant::now());
        assert_ne!(feed.generation(), old);
    }

    #[test]
    fn the_context_never_grows_past_the_cap() {
        let feed = Feed::default();
        let now = Instant::now();
        assert!(feed.due(now, 0, CTX_MAX - 1));
        assert!(!feed.due(now, 0, CTX_MAX));
    }

    // ── Alma: sahte ağ ─────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeNet {
        responses: Mutex<VecDeque<Result<Response, TransportError>>>,
        urls: Mutex<Vec<String>>,
        refreshes: AtomicUsize,
    }

    fn reply(status: u16, body: &str) -> Result<Response, TransportError> {
        Ok(Response { status, retry_after: None, body: Bytes::from(body.to_string()) })
    }

    impl Transport for FakeNet {
        fn post<'a>(&'a self, _: &'a Request) -> BoxFuture<'a, Result<Response, TransportError>> {
            Box::pin(async { panic!("radyo POST kullanmaz") })
        }
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Response, TransportError>> {
            Box::pin(async move {
                self.urls.lock().unwrap().push(url.to_string());
                self.responses.lock().unwrap().pop_front().expect("beklenmeyen istek")
            })
        }
        fn refresh_token<'a>(&'a self) -> BoxFuture<'a, Result<(), TransportError>> {
            Box::pin(async move {
                self.refreshes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
        fn discover<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<String, DiscoverError>> {
            Box::pin(async { panic!("radyo hash aramaz") })
        }
    }

    fn fast() -> Pathfinder {
        Pathfinder::new(Config { interval: Duration::ZERO, server_retries: 1, backoff_base: Duration::from_millis(1) })
    }

    #[tokio::test]
    async fn loads_through_the_shared_client_with_one_request_per_call() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().extend([reply(200, FIXTURE), reply(200, FIXTURE)]);
        let first = load_with(&fast(), &net, "spotify:track:0000000000000000seed01", &[], true).await.unwrap();
        assert_eq!(first.tracks.len(), 11);
        let prev: Vec<String> = first.tracks.iter().map(|t| t.uri.clone()).collect();
        load_with(&fast(), &net, "spotify:track:0000000000000000seed01", &prev, false).await.unwrap();
        let urls = net.urls.lock().unwrap();
        assert_eq!(urls.len(), 2, "çağrı başına tek istek");
        assert!(!urls[0].contains("prev_tracks"));
        assert!(urls[1].contains("prev_tracks=0000000000000000radio1,0000000000000000radio2"), "{}", urls[1]);
    }

    #[tokio::test]
    async fn a_bad_seed_never_hits_the_network_and_an_empty_answer_is_a_code() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().push_back(reply(200, r#"{"tracks":[]}"#));
        assert_eq!(load_with(&fast(), &net, "liked", &[], true).await.unwrap_err(), RadioFailure::Unavailable);
        assert_eq!(net.urls.lock().unwrap().len(), 0);
        assert_eq!(load_with(&fast(), &net, "spotify:album:abc", &[], true).await.unwrap_err(), RadioFailure::Empty);
    }

    #[tokio::test]
    async fn failures_become_typed_codes() {
        let code = |responses: Vec<Result<Response, TransportError>>| async move {
            let net = FakeNet::default();
            net.responses.lock().unwrap().extend(responses);
            load_with(&fast(), &net, "spotify:track:abc", &[], true).await.unwrap_err()
        };
        assert_eq!(code(vec![reply(401, ""), reply(403, "")]).await, RadioFailure::Denied);
        assert_eq!(code(vec![reply(403, "")]).await, RadioFailure::Denied);
        assert_eq!(code(vec![reply(429, "")]).await, RadioFailure::RateLimited);
        assert_eq!(code(vec![reply(404, "")]).await, RadioFailure::Unavailable);
        assert_eq!(code(vec![reply(400, "")]).await, RadioFailure::Unavailable);
        assert_eq!(code(vec![reply(200, "not json")]).await, RadioFailure::Unavailable);
        assert_eq!(code(vec![reply(500, ""), reply(502, "")]).await, RadioFailure::Offline);
        assert_eq!(code(vec![Err(TransportError::Network("down".into()))]).await, RadioFailure::Offline);
    }

    #[tokio::test]
    async fn a_401_refreshes_the_token_once_then_succeeds() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().extend([reply(401, ""), reply(200, FIXTURE)]);
        assert!(load_with(&fast(), &net, "spotify:track:abc", &[], true).await.is_ok());
        assert_eq!(net.refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_429_stops_the_follow_ups_but_never_touches_another_gate() {
        // Radyonun kapısı kendi örneği: orada alınan 429 sonraki isteği (aynı kapıdan)
        // engeller, ama başka bir Pathfinder (arama / Keşfet) etkilenmez.
        let radio_gate =
            Pathfinder::new(Config { interval: Duration::ZERO, server_retries: 0, backoff_base: Duration::from_millis(1) });
        let other_gate = fast();
        let net = FakeNet::default();
        net.responses.lock().unwrap().extend([reply(429, ""), reply(200, FIXTURE)]);
        assert_eq!(
            load_with(&radio_gate, &net, "spotify:track:abc", &[], true).await.unwrap_err(),
            RadioFailure::RateLimited
        );
        // Devamı (arka plan) artık kapıdan geçmez (Retry-After süresi): istek gitmez.
        assert_eq!(
            load_with(&radio_gate, &net, "spotify:track:abc", &[], false).await.unwrap_err(),
            RadioFailure::RateLimited
        );
        assert_eq!(net.urls.lock().unwrap().len(), 1);
        assert!(load_with(&other_gate, &net, "spotify:track:abc", &[], true).await.is_ok());
    }

    // ── Canlı (sahibi onaylayınca elle) ────────────────────────────────

    /// CANLI okuma sınaması: tek bir radyo isteği (+ bir devam isteği), salt
    /// okuma, hiçbir şey yazmaz. Gerçek Spotify'a gider; yalnız sahibinin
    /// onayıyla, elle koşulur:
    ///
    ///   cargo test --release live_radio -- --ignored --nocapture
    ///
    /// Şarkı / sanatçı adları yazdırılmaz: yalnız sayılar ve alanların dolu
    /// olup olmadığı. `OPENIFY_LIVE_SEED=spotify:artist:…` ile başka tohum.
    /// Uç oturum token'ını kabul etmezse `Denied`, tohum türünü tanımazsa
    /// `Unavailable` döner; bunlar da değerli sonuçlardır.
    #[tokio::test]
    #[ignore = "canli Spotify: yalniz sahibi onayiyla elle kosulur"]
    async fn live_radio() {
        use librespot::core::{authentication::Credentials, config::SessionConfig};
        let json = keyring::Entry::new("Openify", "spotify-session")
            .expect("Credential Manager")
            .get_password()
            .expect("Kayitli Spotify girisi yok");
        let credentials: Credentials = serde_json::from_str(&json).expect("kayitli giris bozuk");
        let session = Session::new(SessionConfig::default(), None);
        session.connect(credentials, false).await.expect("Spotify'a baglanilamadi");

        // Varsayılan: Rick Astley, "Never Gonna Give You Up".
        let seed = std::env::var("OPENIFY_LIVE_SEED").unwrap_or_else(|_| "spotify:track:4cOdK2wGLETKBW3PvgPWqT".into());
        match load(&session, &seed, &[], true).await {
            Ok(station) => {
                let with_row = station.tracks.iter().filter(|t| t.row.is_some()).count();
                println!(
                    "radyo: {} sarki, kunyeli {}, baslik dolu {}, ilk sarki tohumun kendisi {}",
                    station.tracks.len(),
                    with_row,
                    !station.title.is_empty(),
                    station.tracks.first().is_some_and(|t| t.uri == seed)
                );
                let prev: Vec<String> = station.tracks.iter().map(|t| t.uri.clone()).collect();
                match load(&session, &seed, &prev, false).await {
                    Ok(more) => {
                        let fresh = more.tracks.iter().filter(|t| !prev.contains(&t.uri)).count();
                        println!("devam: {} sarki, oncekilerden farkli {}", more.tracks.len(), fresh);
                    }
                    Err(e) => println!("devam alinamadi: {e:?}"),
                }
            }
            Err(e) => println!("radyo alinamadi: {e:?}"),
        }
    }
}
