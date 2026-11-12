//! Şarkı künyesi: Spotify'ın "Credits" verisi (kim seslendirdi, kim yazdı,
//! kim yapımcı, hangi plak şirketi).
//!
//!   GET https://spclient.wg.spotify.com/track-credits-view/v0/experimental/{trackId}/credits
//!   { "trackUri", "trackTitle",
//!     "roleCredits": [ { "roleTitle": "Performed By" | "Written By" | "Produced By" | …,
//!                        "artists": [ { "name", "uri"?, "imageUri"?, "subroles": [..], "weight" } ] } ],
//!     "extendedCredits": [ … ],
//!     "sourceNames": [ "plak şirketi", … ] }
//!
//! Bu uç resmî değil (web player'ın künye penceresinin kullandığı gayriresmî
//! uç) ve biçimi bilgiye dayanır: çözümleme eksik alana ve anahtar adı
//! değişikliğine toleranslıdır (rol listesi `roleCredits` / `credits` /
//! `roles`, kişi listesi `artists` / `people`, kişi düz yazı da olabilir; adı
//! olmayan kişi atılır, adresi bozuk olan kişi tıklanmaz ama görünür). 12 Kasım'da
//! gerçek hesapla bir kez koşturuldu (`live_credits` sınaması, `#[ignore]`,
//! yalnız hesap sahibi koşar): yanıt çözümlendi.
//!
//! İstek, token, kapı (429, aralık, devre kesici), 401'de token yenileme ve
//! 5xx yeniden denemesi `pathfinder.rs`'in ortak altyapısından geçer
//! (`Pathfinder::get_json`); kapısı ayrı bir örnek (`pathfinder::credits`):
//! buradaki 429 aramayı ya da Keşfet'i durdurmaz. Salt okuma. Sonuçlar yalnız
//! bellekte (oturum boyunca en çok `CACHE_MAX` şarkı) tutulur; diske yazılmaz.
//!
//! Rol adları Spotify'dan gelir (İngilizce); Seslendiren / Yazan / Yapımcı
//! rolleri tanınır ve arayüz dilinde adlandırılır (`RoleKind`), gerisi
//! Spotify'ın kendi başlığıyla gösterilir.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use librespot::core::session::Session;
use serde_json::Value;

use super::pathfinder::{self, LiveTransport, Pathfinder, PathfinderError, Priority, Transport};

/// Künye adresinin tabanı (tek yerde); `{base}/{trackId}/credits`.
pub const CREDITS_BASE: &str = "https://spclient.wg.spotify.com/track-credits-view/v0/experimental";
/// Bir yazının (ad, rol, kaynak) en çok kaç harfi alınır.
const TEXT_MAX: usize = 200;
/// Bir rolde en çok bu kadar kişi, en çok bu kadar rol, en çok bu kadar kaynak.
pub const MAX_PEOPLE: usize = 12;
pub const MAX_ROLES: usize = 12;
pub const MAX_SOURCES: usize = 4;
/// Oturum boyunca bellekte tutulan şarkı sayısı.
const CACHE_MAX: usize = 64;

/// Rolün türü: tanınan üçü arayüz dilinde adlandırılır, gerisi Spotify'ın
/// başlığıyla gösterilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    /// Seslendiren (Performed By, Main Artist, …).
    Performer,
    /// Yazan (Written By, Composer, Lyricist, Songwriter, …).
    Writer,
    /// Yapımcı (Produced By, Producer, …).
    Producer,
    Other,
}

impl RoleKind {
    /// Sıralamada yeri: önce üç tanınan rol, sonra gerisi.
    fn rank(self) -> u8 {
        match self {
            Self::Performer => 0,
            Self::Writer => 1,
            Self::Producer => 2,
            Self::Other => 3,
        }
    }
}

/// Künyedeki bir kişi. `uri`: sanatçı adresi (`spotify:artist:…`); varsa ad
/// tıklanınca sanatçı sayfası açılır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub kind: RoleKind,
    /// Spotify'ın rol başlığı ("Performed By"); `Other` için gösterilen odur.
    pub title: String,
    pub people: Vec<Person>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credits {
    /// Şarkının adı (yanıttaki; yoksa boş, arayüz kendi bildiğini gösterir).
    pub title: String,
    pub roles: Vec<Role>,
    /// Kaynak: plak şirketi / dağıtıcı.
    pub sources: Vec<String>,
}

impl Credits {
    /// Gösterilecek hiçbir şey yok mu?
    pub fn is_empty(&self) -> bool {
        self.roles.iter().all(|r| r.people.is_empty()) && self.sources.is_empty()
    }

    /// Tıklanabilir (adresi olan) kişi sayısı.
    #[cfg(test)]
    pub fn linkable(&self) -> usize {
        self.roles.iter().flat_map(|r| &r.people).filter(|p| p.uri.is_some()).count()
    }
}

/// Alınamamanın nedeni (kod; yazısı arayüzde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditsFailure {
    /// Ağ ya da sunucu hatası, bağlantı yok.
    Offline,
    /// 429 ya da devre kesici.
    RateLimited,
    /// 401 / 403: bu hesap ya da bu token için uç kapalı.
    Denied,
    /// Uç kalkmış ya da yanıt anlaşılmıyor.
    Unavailable,
}

impl From<&PathfinderError> for CreditsFailure {
    fn from(e: &PathfinderError) -> Self {
        match e {
            PathfinderError::RateLimited { .. } | PathfinderError::Blocked { .. } | PathfinderError::CircuitOpen => {
                Self::RateLimited
            }
            PathfinderError::Denied { .. } => Self::Denied,
            PathfinderError::Http { status: 404 | 410 } | PathfinderError::InvalidResponse(_) => Self::Unavailable,
            PathfinderError::Token(_) | PathfinderError::Transport(_) | PathfinderError::Http { .. } => Self::Offline,
            // Hash'li işlemlere özgü hatalar bu uçta gelmez.
            _ => Self::Unavailable,
        }
    }
}

// ── Adres ───────────────────────────────────────────────────────────────

/// `spotify:track:<kimlik>` adresinin kimliği (yalnız harf ve rakam); başka
/// adres (bölüm, yerel dosya) için `None`.
pub fn track_id(uri: &str) -> Option<&str> {
    let id = uri.strip_prefix("spotify:track:")?;
    (!id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(id)
}

/// Şarkının künye adresi.
pub fn credits_url(track_id: &str) -> String {
    format!("{CREDITS_BASE}/{track_id}/credits")
}

// ── Çözümleme ───────────────────────────────────────────────────────────

/// Yanıtı künyeye çevirir. Eksik alana toleranslı: boş yanıt boş künye.
/// Roller sıralı gelir: Seslendiren, Yazan, Yapımcı, gerisi (aynı türdeki
/// roller birleşir, kişiler tekrarsız).
pub fn parse(value: &Value) -> Credits {
    let title = text(value.get("trackTitle").or_else(|| value.get("title")));

    let mut roles: Vec<Role> = Vec::new();
    for key in ["roleCredits", "credits", "roles", "extendedCredits"] {
        let Some(list) = value.get(key).and_then(Value::as_array) else { continue };
        for item in list {
            if let Some(role) = parse_role(item) {
                merge(&mut roles, role);
            }
        }
    }
    // Kararlı sıralama: aynı sırada gelenler yanıttaki sırada kalır.
    roles.sort_by_key(|r| r.kind.rank());
    roles.truncate(MAX_ROLES);

    let mut sources: Vec<String> = Vec::new();
    let raw_sources = value
        .get("sourceNames")
        .or_else(|| value.get("sources"))
        .or_else(|| value.get("source"))
        .or_else(|| value.get("label"));
    match raw_sources {
        Some(Value::Array(list)) => {
            for v in list {
                // Dizi düz yazı ya da `{ "name": … }` olabilir.
                let s = text(Some(if v.is_object() { v.get("name").unwrap_or(v) } else { v }));
                if !s.is_empty() && !sources.contains(&s) {
                    sources.push(s);
                }
            }
        }
        Some(v) => {
            let s = text(Some(v));
            if !s.is_empty() {
                sources.push(s);
            }
        }
        None => {}
    }
    sources.truncate(MAX_SOURCES);

    Credits { title, roles, sources }
}

fn parse_role(v: &Value) -> Option<Role> {
    let title = ["roleTitle", "title", "role", "name"].iter().find_map(|k| Some(text(v.get(k))).filter(|t| !t.is_empty()))?;
    let list = ["artists", "people", "credits", "names"].iter().find_map(|k| v.get(k).and_then(Value::as_array))?;
    let mut people: Vec<Person> = Vec::new();
    for p in list {
        if let Some(person) = parse_person(p)
            && !people.iter().any(|q| q.name == person.name)
        {
            people.push(person);
        }
    }
    people.truncate(MAX_PEOPLE);
    (!people.is_empty()).then(|| Role { kind: kind_of(&title), title, people })
}

fn parse_person(v: &Value) -> Option<Person> {
    // Düz yazı ("Ad Soyad") ya da nesne.
    let (name, uri) = match v {
        Value::String(_) => (text(Some(v)), None),
        Value::Object(_) => (text(v.get("name")), artist_uri(v.get("uri").or_else(|| v.get("artistUri")))),
        _ => return None,
    };
    (!name.is_empty()).then_some(Person { name, uri })
}

/// Rolün türü, başlığa bakarak (küçük harfle; "Co-producer", "Executive
/// Producer", "Songwriter" da tanınır).
pub fn kind_of(title: &str) -> RoleKind {
    let t = title.to_lowercase();
    // "Writer" (writ…) ve "Producer" (produc…) "performer"dan önce: "Written
    // By" ve "Produced By" kendi başına; "Performed By" / "Main Artist" /
    // "Vocals" seslendiren.
    if ["writ", "compos", "lyric", "songwrit", "author", "arrang"].iter().any(|k| t.contains(k)) {
        RoleKind::Writer
    } else if t.contains("produc") {
        RoleKind::Producer
    } else if ["perform", "main artist", "featured", "vocal", "singer"].iter().any(|k| t.contains(k)) {
        RoleKind::Performer
    } else {
        RoleKind::Other
    }
}

/// Aynı türdeki rolleri birleştirir (Spotify "Writer" ve "Composer"ı ayrı
/// verebilir: ikisi de "Yazan"); `Other` roller başlığıyla eşleşirse birleşir.
fn merge(roles: &mut Vec<Role>, role: Role) {
    let same = |r: &Role| r.kind == role.kind && (role.kind != RoleKind::Other || r.title == role.title);
    match roles.iter_mut().find(|r| same(r)) {
        Some(existing) => {
            for p in role.people {
                if !existing.people.iter().any(|q| q.name == p.name) {
                    existing.people.push(p);
                }
            }
            existing.people.truncate(MAX_PEOPLE);
        }
        None => roles.push(role),
    }
}

/// Düzgün biçimli bir sanatçı adresi; yoksa ya da başka türde `None`.
fn artist_uri(v: Option<&Value>) -> Option<String> {
    const PREFIX: &str = "spotify:artist:";
    let s = v?.as_str()?.trim();
    (s.starts_with(PREFIX) && s.len() > PREFIX.len() && !s.chars().any(char::is_control)).then(|| s.to_string())
}

/// Görünecek yazı: denetim karakterleri atılmış, kırpılmış, en çok `TEXT_MAX` harf.
fn text(v: Option<&Value>) -> String {
    let s = v.and_then(Value::as_str).unwrap_or_default();
    s.chars().filter(|c| !c.is_control()).take(TEXT_MAX).collect::<String>().trim().to_string()
}

// ── Alma ────────────────────────────────────────────────────────────────

fn cache() -> &'static Mutex<HashMap<String, Arc<Credits>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Credits>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn cached(uri: &str) -> Option<Arc<Credits>> {
    cache().lock().unwrap_or_else(|e| e.into_inner()).get(uri).cloned()
}

fn remember(uri: &str, credits: &Arc<Credits>) {
    let mut map = cache().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= CACHE_MAX {
        map.clear(); // basit: dolunca baştan (künye nadiren yeniden istenir)
    }
    map.insert(uri.to_string(), credits.clone());
}

/// Künyeyi alır (kapı, token yenileme, yeniden deneme ortak istemciden).
/// Şarkı adresi künyesi olmayan türdeyse (bölüm, yerel dosya) boş künye döner.
/// 404: "bu şarkının künyesi yok", boş künye (hata değil).
pub async fn load_with(pf: &Pathfinder, transport: &dyn Transport, uri: &str) -> Result<Arc<Credits>, CreditsFailure> {
    let Some(id) = track_id(uri) else { return Ok(Arc::new(Credits::default())) };
    if let Some(hit) = cached(uri) {
        return Ok(hit);
    }
    // Kullanıcı bekliyor (düğmeye bastı): sıraya girmez, 429 beklemesine uyar.
    match pf.get_json(transport, "track-credits", &credits_url(id), Priority::Interactive).await {
        Ok(value) => {
            let credits = Arc::new(parse(&value));
            if !credits.is_empty() {
                remember(uri, &credits);
            }
            Ok(credits)
        }
        Err(PathfinderError::Http { status: 404 | 410 }) => Ok(Arc::new(Credits::default())),
        Err(e) => {
            log::info!("şarkı künyesi alınamadı: {e}");
            Err(CreditsFailure::from(&e))
        }
    }
}

/// Canlı oturumla (`pathfinder::credits` kapısından).
pub async fn load(session: &Session, uri: &str) -> Result<Arc<Credits>, CreditsFailure> {
    load_with(pathfinder::credits(), &LiveTransport::new(session), uri).await
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use bytes::Bytes;
    use futures::future::BoxFuture;

    use super::*;
    use crate::engine::pathfinder::{Config, DiscoverError, Request, Response, TransportError};

    pub(crate) const FIXTURE: &str = include_str!("fixtures/track_credits.json");
    const SPARSE: &str = include_str!("fixtures/track_credits_sparse.json");

    fn fixture() -> Credits {
        parse(&serde_json::from_str(FIXTURE).unwrap())
    }

    fn names(role: &Role) -> Vec<&str> {
        role.people.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn parses_the_recorded_shape_in_a_fixed_role_order() {
        let c = fixture();
        assert_eq!(c.title, "Gece Yolcusu");
        // Yanıtta sıra Yapımcı, Seslendiren, Yazan, Mix, Besteci: çıktıda
        // Seslendiren, Yazan (Besteci birleşti), Yapımcı, sonra Mix.
        let kinds: Vec<RoleKind> = c.roles.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RoleKind::Performer, RoleKind::Writer, RoleKind::Producer, RoleKind::Other]);

        let performer = &c.roles[0];
        assert_eq!(performer.title, "Performed By");
        assert_eq!(names(performer), ["Dalga Boyu", "Kuzey Isigi"]);
        assert_eq!(performer.people[0].uri.as_deref(), Some("spotify:artist:r01"));
        assert_eq!(performer.people[1].uri.as_deref(), Some("spotify:artist:r02"));

        // Yazan: iki rol birleşti, aynı ad (Deniz Aksu) tek; adresi olmayan kişi tıklanmaz.
        let writer = &c.roles[1];
        assert_eq!(names(writer), ["Deniz Aksu", "Mert Kaya", "Ece Tan"]);
        assert_eq!(writer.people[0].uri.as_deref(), Some("spotify:artist:w01"));
        assert_eq!(writer.people[1].uri, None);

        assert_eq!(names(&c.roles[2]), ["Mert Kaya"]);
        assert_eq!(c.roles[3].title, "Mixing Engineer");
        assert_eq!(c.sources, ["Karanlik Oda Plaklari", "Dagitim A.S."]);
        assert!(!c.is_empty());
        assert_eq!(c.linkable(), 4, "r01, r02, w01 ve p01 adresi olanlar");
    }

    #[test]
    fn tolerates_missing_and_odd_fields() {
        let c = parse(&serde_json::from_str(SPARSE).unwrap());
        // Başlıksız rol, kişisiz rol, adsız kişi, bozuk adres atlanır / tıklanmaz;
        // düz yazı kişi okunur; kaynak tek yazı olarak da gelebilir.
        assert_eq!(c.title, "");
        assert_eq!(c.roles.len(), 2);
        assert_eq!(c.roles[0].kind, RoleKind::Performer);
        assert_eq!(names(&c.roles[0]), ["Yalniz Ad", "Duz Yazi"]);
        assert_eq!(c.roles[0].people[0].uri, None, "bozuk adres tıklanmaz ama ad görünür");
        assert_eq!(c.roles[1].kind, RoleKind::Producer);
        assert_eq!(c.sources, ["Tek Kaynak"]);
    }

    #[test]
    fn empty_and_unrelated_bodies_make_an_empty_credits() {
        for body in ["{}", "[]", r#"{"roleCredits":[]}"#, r#"{"roleCredits":[{"roleTitle":"X","artists":[]}]}"#, "null", "3"] {
            let c = parse(&serde_json::from_str(body).unwrap());
            assert!(c.is_empty(), "{body}");
        }
    }

    #[test]
    fn role_kinds_are_recognised_from_the_title() {
        for (title, kind) in [
            ("Performed By", RoleKind::Performer),
            ("Main Artist", RoleKind::Performer),
            ("Featured Artist", RoleKind::Performer),
            ("Vocals", RoleKind::Performer),
            ("Written By", RoleKind::Writer),
            ("Composer", RoleKind::Writer),
            ("Songwriter", RoleKind::Writer),
            ("Lyricist", RoleKind::Writer),
            ("Produced By", RoleKind::Producer),
            ("Co-Producer", RoleKind::Producer),
            ("Executive Producer", RoleKind::Producer),
            ("Mixing Engineer", RoleKind::Other),
            ("Mastering Engineer", RoleKind::Other),
            ("", RoleKind::Other),
        ] {
            assert_eq!(kind_of(title), kind, "{title}");
        }
    }

    #[test]
    fn limits_hold_for_huge_responses() {
        let people: Vec<Value> = (0..40).map(|i| serde_json::json!({ "name": format!("Kisi {i}") })).collect();
        let roles: Vec<Value> =
            (0..30).map(|i| serde_json::json!({ "roleTitle": format!("Rol {i}"), "artists": people })).collect();
        let sources: Vec<String> = (0..10).map(|i| format!("Kaynak {i}")).collect();
        let c = parse(&serde_json::json!({ "roleCredits": roles, "sourceNames": sources }));
        assert_eq!(c.roles.len(), MAX_ROLES);
        assert!(c.roles.iter().all(|r| r.people.len() == MAX_PEOPLE));
        assert_eq!(c.sources.len(), MAX_SOURCES);
        let long = "a".repeat(1000);
        let c = parse(&serde_json::json!({ "roleCredits": [{ "roleTitle": "Performed By", "artists": [{ "name": long }] }] }));
        assert_eq!(c.roles[0].people[0].name.chars().count(), TEXT_MAX);
    }

    #[test]
    fn track_id_accepts_only_track_uris() {
        assert_eq!(track_id("spotify:track:4uLU6hMCjMI75M1A2tKUQC"), Some("4uLU6hMCjMI75M1A2tKUQC"));
        assert_eq!(track_id("spotify:episode:abc"), None);
        assert_eq!(track_id("spotify:track:"), None);
        assert_eq!(track_id("spotify:track:a/b"), None, "yol ayracı içeren kimlik adrese karışmaz");
        assert_eq!(track_id("local:::x:1"), None);
        assert_eq!(
            credits_url("4uLU6hMCjMI75M1A2tKUQC"),
            "https://spclient.wg.spotify.com/track-credits-view/v0/experimental/4uLU6hMCjMI75M1A2tKUQC/credits"
        );
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
            Box::pin(async { panic!("künye POST kullanmaz") })
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
            Box::pin(async { panic!("künye hash aramaz") })
        }
    }

    fn fast() -> Pathfinder {
        Pathfinder::new(Config { interval: Duration::ZERO, server_retries: 1, backoff_base: Duration::from_millis(1) })
    }

    #[tokio::test]
    async fn loads_through_the_shared_client_with_the_single_url() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().push_back(reply(200, FIXTURE));
        let c = load_with(&fast(), &net, "spotify:track:LoadsOk1").await.unwrap();
        assert_eq!(c.roles.len(), 4);
        assert_eq!(*net.urls.lock().unwrap(), [credits_url("LoadsOk1")]);
        // İkinci açılış bellekten: ağa çıkmaz.
        let again = load_with(&fast(), &net, "spotify:track:LoadsOk1").await.unwrap();
        assert_eq!(again.roles.len(), 4);
        assert_eq!(net.urls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_404_means_no_credits_and_non_tracks_never_hit_the_network() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().push_back(reply(404, ""));
        let c = load_with(&fast(), &net, "spotify:track:NoneHere1").await.unwrap();
        assert!(c.is_empty());
        let c = load_with(&fast(), &net, "spotify:episode:abc").await.unwrap();
        assert!(c.is_empty());
        assert_eq!(net.urls.lock().unwrap().len(), 1, "bölüm için istek yok");
    }

    #[tokio::test]
    async fn failures_become_typed_codes() {
        let code = |responses: Vec<Result<Response, TransportError>>| async move {
            let net = FakeNet::default();
            net.responses.lock().unwrap().extend(responses);
            load_with(&fast(), &net, "spotify:track:Fails1").await.unwrap_err()
        };
        assert_eq!(code(vec![reply(401, ""), reply(403, "")]).await, CreditsFailure::Denied);
        assert_eq!(code(vec![reply(403, "")]).await, CreditsFailure::Denied);
        assert_eq!(code(vec![reply(429, "")]).await, CreditsFailure::RateLimited);
        assert_eq!(code(vec![reply(200, "not json")]).await, CreditsFailure::Unavailable);
        assert_eq!(code(vec![reply(500, ""), reply(502, "")]).await, CreditsFailure::Offline);
        assert_eq!(code(vec![Err(TransportError::Network("down".into()))]).await, CreditsFailure::Offline);
        assert_eq!(code(vec![Err(TransportError::Token("none".into()))]).await, CreditsFailure::Offline);
    }

    #[tokio::test]
    async fn a_401_refreshes_the_token_once_then_succeeds() {
        let net = FakeNet::default();
        net.responses.lock().unwrap().extend([reply(401, ""), reply(200, FIXTURE)]);
        assert!(!load_with(&fast(), &net, "spotify:track:Refresh1").await.unwrap().is_empty());
        assert_eq!(net.refreshes.load(Ordering::SeqCst), 1);
    }

    // ── Canlı (sahibi onaylayınca elle) ────────────────────────────────

    /// CANLI okuma sınaması: tek bir künye isteği, salt okuma, hiçbir şey
    /// yazmaz. Gerçek Spotify'a gider; yalnız sahibinin onayıyla, elle koşulur:
    ///
    ///   cargo test --release live_credits -- --ignored --nocapture
    ///
    /// Şarkı adı / kişi adları yazdırılmaz: yalnız sayılar ve alanların dolu
    /// olup olmadığı. `OPENIFY_LIVE_TRACK=spotify:track:…` ile başka şarkı.
    /// Uç oturum token'ını kabul etmezse `Denied` döner; bu da değerli bir
    /// sonuçtur (özellik o hesapta çalışmaz).
    #[tokio::test]
    #[ignore = "canli Spotify: yalniz sahibi onayiyla elle kosulur"]
    async fn live_credits() {
        use librespot::core::{authentication::Credentials, config::SessionConfig};
        let json = keyring::Entry::new("Openify", "spotify-session")
            .expect("Credential Manager")
            .get_password()
            .expect("Kayitli Spotify girisi yok");
        let credentials: Credentials = serde_json::from_str(&json).expect("kayitli giris bozuk");
        let session = Session::new(SessionConfig::default(), None);
        session.connect(credentials, false).await.expect("Spotify'a baglanilamadi");

        // Varsayılan: Rick Astley, "Never Gonna Give You Up".
        let uri = std::env::var("OPENIFY_LIVE_TRACK").unwrap_or_else(|_| "spotify:track:4cOdK2wGLETKBW3PvgPWqT".into());
        match load(&session, &uri).await {
            Ok(credits) => {
                println!("kunye: {} rol, {} kaynak, bos: {}", credits.roles.len(), credits.sources.len(), credits.is_empty());
                for role in &credits.roles {
                    let with_uri = role.people.iter().filter(|p| p.uri.is_some()).count();
                    println!("  rol {:?}: {} kisi, adresi olan {}", role.kind, role.people.len(), with_uri);
                }
                println!("  baslik dolu {} / tiklanabilir kisi {}", !credits.title.is_empty(), credits.linkable());
            }
            Err(e) => println!("kunye alinamadi: {e:?}"),
        }
    }
}
