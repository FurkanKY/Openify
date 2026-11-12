//! Spotify'ın GraphQL ağ geçidi "pathfinder" için tek istemci.
//!
//!   POST https://api-partner.spotify.com/pathfinder/v2/query
//!   { operationName, variables, extensions: { persistedQuery: { sha256Hash } } }
//!
//! Arama (`search.rs`), benzer sanatçılar (`related.rs`) ve Keşfet / Ana sayfa
//! (`explore.rs`) bu modülden geçer; HTTP çağrısı, işlem adı → sha256 tablosu,
//! hash keşfi, token yenileme, yeniden deneme ve istek kapısı burada tek yerde.
//!
//! - **Hash tablosu** (`Operation`): her işlemin adı, bulunduğu web player
//!   parçası, `meta` anahtarı ve (varsa) başlangıç hash'i. Bellekte tutulur,
//!   `HashStore` (kütüphane önbelleğinin `meta` tablosu) kalıcı kılar. Hash'i
//!   Spotify reddederse (400 / 404 / `PersistedQueryNotFound`) web player'ın
//!   JavaScript paketinden yenisi bulunur, istek bir kez yinelenir. Oturumda
//!   işlem başına bir yeniden arama hakkı var (web player'ı boğmayalım); yeni
//!   hash çalışırsa hak geri verilir.
//! - **Kapı** (`Gate`): bütün işlemler tek kapıdan geçer (aramalar da). 2 sn
//!   aralık, 429'da Retry-After kadar hiç istek yok, üst üste iki 429'da devre
//!   açılır. Arama (`Priority::Interactive`) sıraya girmez ve devre kesiciden
//!   etkilenmez (kullanıcı yazarken bekletilmesin; başka yedek yolu yok) ama
//!   429 beklemesine uyar ve kapının sırasını kendi payı kadar iter.
//! - **Yeniden deneme**: 5xx'te üstel bekleme ile en çok iki kez; 401'de token
//!   yenilenip bir kez; 429'da yinelenmez (kapıya bildirilir).
//! - **Hatalar** tipli (`PathfinderError`): motor kullanıcıya metin üretmez;
//!   çağıran hata türüne göre karar verir, `Display` yalnız günlük içindir.
//!
//! Ağ `Transport` arkasında: canlı hâli `LiveTransport` (librespot oturumu),
//! sınamalarda sahtesi.
//!
//! Hash keşfi: open.spotify.com → web-player.<x>.js → sorgunun parçası
//! (`xpui-routes-search`, ...) → `"<işlem>","query","<hash>"`.

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

use anyhow::Context;
use bytes::Bytes;
use futures::future::BoxFuture;
use librespot::core::session::Session;
use serde_json::{Value, json};

use super::cache::Db;

pub const URL: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const WEB_PLAYER_URL: &str = "https://open.spotify.com/";
const CDN_BUILD_URL: &str = "https://open.spotifycdn.com/cdn/build/web-player/";
/// Web player paketlerini indirirken kullanılan tarayıcı kimliği.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

// ── İşlemler (işlem adı → hash tablosu) ─────────────────────────────────

/// Bir persisted query.
#[derive(Debug)]
pub struct Operation {
    pub name: &'static str,
    /// Hash'in bulunduğu web player parçası.
    pub chunk: &'static str,
    /// Bulunan hash'in `meta` anahtarı.
    pub meta_key: &'static str,
    /// 7 Kasım 2026'da web player'da bulunan hash: yalnız başlangıç değeri
    /// (kayıtlı hash yoksa); eskirse yenisi bulunur. Yoksa ilk istekte aranır.
    pub seed: Option<&'static str>,
    /// `true`: yanıt dile bağlı (liste adları, bölüm başlıkları, açıklamalar);
    /// istek uygulamanın arayüz diliyle `accept-language` başlığı taşır
    /// (`language_header`). `false`: başlık gönderilmez.
    pub localized: bool,
    /// `Some(alan)`: katı yanıt denetimi. Yanıtta herhangi bir GraphQL hatası
    /// ya da `data.<alan>` yoksa hash eskimiş sayılır. `None`: yalnız
    /// `PersistedQueryNotFound` hatası.
    pub strict_root: Option<&'static str>,
}

impl Operation {
    /// Yanıt "bu hash'i tanımıyorum" diyor mu? Neden döner.
    fn stale_reason(&self, value: &Value) -> Option<String> {
        let Some(root) = self.strict_root else {
            return graphql_stale(value);
        };
        if let Some(errors) = value.get("errors").and_then(Value::as_array) {
            let message = errors
                .first()
                .and_then(|e| e["message"].as_str())
                .unwrap_or("unknown error");
            return Some(message.to_string());
        }
        value["data"][root]
            .is_null()
            .then(|| format!("response has no {root}"))
    }
}

/// Aramanın hash'i seed'siz: ilk istekte web player'dan bulunur. `meta`
/// anahtarı eski sürümlerle aynı (`search_hash`).
pub const SEARCH_DESKTOP: Operation = Operation {
    name: "searchDesktop",
    chunk: "xpui-routes-search",
    meta_key: "search_hash",
    seed: None,
    localized: false,
    strict_root: Some("searchV2"),
};
pub const ARTIST_RELATED: Operation = Operation {
    name: "queryArtistRelated",
    chunk: "xpui-routes-artist",
    meta_key: "hash:queryArtistRelated",
    seed: Some("3d031d6cb22a2aa7c8d203d49b49df731f58b1e2799cc38d9876d58771aa66f3"),
    localized: true,
    strict_root: None,
};
pub const HOME: Operation = Operation {
    name: "home",
    chunk: "xpui-main",
    meta_key: "hash:home",
    seed: Some("5fb7da7a03ee2776c0670856609cb85a189a3cecc36cc160b91cde79063b9cf1"),
    localized: true,
    strict_root: None,
};
pub const BROWSE_ALL: Operation = Operation {
    name: "browseAll",
    chunk: "xpui-routes-search",
    meta_key: "hash:browseAll",
    seed: Some("dbd8b55e09a58afc52eab438bc228ba28fd72ac2f2148c6c26354980e4579001"),
    localized: true,
    strict_root: None,
};
pub const BROWSE_PAGE: Operation = Operation {
    name: "browsePage",
    chunk: "browse-v2",
    meta_key: "hash:browsePage",
    seed: Some("f5c4e6d668f5716464a231c1cc8b22c1cbf6ad68b09929fd7de813a30581298b"),
    localized: true,
    strict_root: None,
};

/// GraphQL hatası "bu hash'i tanımıyorum" mu? Neden döner.
pub(super) fn graphql_stale(value: &Value) -> Option<String> {
    let errors = value.get("errors")?.as_array()?;
    errors
        .iter()
        .filter_map(|e| e["message"].as_str())
        .find(|m| m.contains("PersistedQueryNotFound"))
        .map(String::from)
}

// ── Hatalar ─────────────────────────────────────────────────────────────

/// Hash'in neden düzelmediği.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleCause {
    /// Tek istek: yeniden arama denenmedi (`send`).
    NotRecovered,
    /// Web player'dan yeni hash bulundu ama o da reddedildi.
    Rejected,
    /// Web player'daki hash reddedilen hash'in aynısı.
    Unchanged,
    /// Bu oturumda bu işlem için hash zaten yeniden arandı.
    AlreadyRediscovered,
}

/// Pathfinder hataları. `Display` yalnız günlük / hata ayıklama içindir
/// (arayüz metni değil); sayı ve anahtar sözcükleri (`429`, `403`,
/// `rate limited`, `request failed`) arayüzün hata sınıflandırması için kalır.
#[derive(Debug, thiserror::Error)]
pub enum PathfinderError {
    #[error("persisted query hash rejected: {operation} ({detail})")]
    StaleHash {
        operation: &'static str,
        detail: String,
        cause: StaleCause,
    },
    /// Web player paketleri indirildi ama sorgu yok (Spotify kaldırmış / adını değiştirmiş).
    #[error("operation {operation} is not in the web player bundle")]
    OperationGone { operation: &'static str },
    /// Web player paketleri indirilemedi (ağ).
    #[error("hash discovery failed: {0:#}")]
    Discovery(anyhow::Error),
    /// 429. `circuit_open`: bu 429 ile devre açıldı (oturum boyunca kapalı).
    #[error("rate limited (429, retry-after {retry_after:?})")]
    RateLimited {
        retry_after: Option<Duration>,
        circuit_open: bool,
    },
    /// Önceki 429'un bekleme süresi dolmadı (istek gönderilmedi).
    #[error("rate limited: waiting after 429 ({remaining:?} left)")]
    Blocked { remaining: Duration },
    /// Üst üste 429'lar devreyi açtı (istek gönderilmedi).
    #[error("rate limited: circuit open")]
    CircuitOpen,
    /// 401 / 403.
    #[error("denied ({status})")]
    Denied { status: u16 },
    /// 5xx (yeniden denemeler de bitti) ya da beklenmeyen durum kodu.
    #[error("HTTP {status}")]
    Http { status: u16 },
    #[error("token unavailable: {0}")]
    Token(String),
    #[error("request failed: {0}")]
    Transport(String),
    #[error("response is not valid JSON: {0}")]
    InvalidResponse(String),
}

/// Ağ katmanının hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    Token(String),
    Network(String),
}

impl From<TransportError> for PathfinderError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Token(m) => Self::Token(m),
            TransportError::Network(m) => Self::Transport(m),
        }
    }
}

/// Hash aranırken: paketler indirildi ama sorgu yok (Spotify kaldırmış ya da
/// adını değiştirmiş) mı, yoksa indirme mi olmadı?
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("{0} is not in the web player bundle")]
    Missing(String),
    #[error(transparent)]
    Fetch(#[from] anyhow::Error),
}

// ── Ağ seti ─────────────────────────────────────────────────────────────

/// Gönderilecek istek (gövde hash'le birlikte hazır).
#[derive(Debug)]
pub struct Request {
    pub body: String,
    pub accept_language: Option<&'static str>,
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: Bytes,
}

/// Ağ ucu: tek istek, token yenileme, hash keşfi. Canlısı `LiveTransport`,
/// sınamalarda sahtesi.
pub trait Transport: Send + Sync {
    /// Tek istek (token'ı kendisi alır; yeniden deneme yok).
    fn post<'a>(&'a self, request: &'a Request) -> BoxFuture<'a, Result<Response, TransportError>>;
    /// Düz GET (arkadaş etkinliği gibi hash'siz uçlar): aynı token ve
    /// client-token başlıkları, gövde yok, yeniden deneme yok. Varsayılan:
    /// desteklenmiyor (yalnız `post` isteyen sahteler derlensin diye).
    fn get<'a>(&'a self, _url: &'a str) -> BoxFuture<'a, Result<Response, TransportError>> {
        Box::pin(async {
            Err(TransportError::Network(
                "GET is not supported by this transport".into(),
            ))
        })
    }
    /// 401 sonrası token'ı yenile.
    fn refresh_token<'a>(&'a self) -> BoxFuture<'a, Result<(), TransportError>>;
    /// Web player paketlerinden işlemin güncel hash'ini bul.
    fn discover<'a>(
        &'a self,
        chunk: &'a str,
        operation: &'a str,
    ) -> BoxFuture<'a, Result<String, DiscoverError>>;
}

/// Bulunan hash'lerin kalıcı yeri.
pub trait HashStore: Send + Sync {
    fn load(&self, key: &str) -> Option<String>;
    fn save(&self, key: &str, hash: &str);
}

impl HashStore for Db {
    fn load(&self, key: &str) -> Option<String> {
        self.meta(key).ok().flatten()
    }

    fn save(&self, key: &str, hash: &str) {
        let _ = self.set_meta(key, hash);
    }
}

/// Canlı ağ: librespot oturumunun login5 token'ı + client-token başlığı (Web
/// API token'ı burada reddediliyor). `request_fut` kullanılır: `request`,
/// 429'da Retry-After kadar uyuyup kendiliğinden yeniden deniyor; burada 429
/// yinelenmez, kapıya bildirilir.
pub struct LiveTransport<'a> {
    session: &'a Session,
}

impl<'a> LiveTransport<'a> {
    pub fn new(session: &'a Session) -> Self {
        Self { session }
    }

    /// Oturumun login5 token'ı ve client-token'ı (`post` ve `get` ikisini de ister).
    async fn credentials(&self) -> Result<(String, String), TransportError> {
        let token = self
            .session
            .login5()
            .auth_token()
            .await
            .map_err(|e| TransportError::Token(e.to_string()))?;
        let client_token = self
            .session
            .spclient()
            .client_token()
            .await
            .map_err(|e| TransportError::Token(format!("client-token: {e}")))?;
        Ok((token.access_token, client_token))
    }

    /// İsteği oturumun HTTP istemcisiyle gönderir; gövde yalnız başarıda okunur.
    async fn exchange(
        &self,
        http_request: http::Request<Bytes>,
    ) -> Result<Response, TransportError> {
        let response = self
            .session
            .http_client()
            .request_fut(http_request)
            .map_err(|e| TransportError::Network(format!("request not built: {e}")))?
            .await
            .map_err(|e| TransportError::Network(e.to_string()))?;
        let status = response.status();
        let retry_after = retry_after(response.headers());
        let body = if status.is_success() {
            http_body_util::BodyExt::collect(response.into_body())
                .await
                .map_err(|e| TransportError::Network(format!("response not read: {e}")))?
                .to_bytes()
        } else {
            Bytes::new()
        };
        Ok(Response {
            status: status.as_u16(),
            retry_after,
            body,
        })
    }
}

impl Transport for LiveTransport<'_> {
    fn post<'a>(&'a self, request: &'a Request) -> BoxFuture<'a, Result<Response, TransportError>> {
        Box::pin(async move {
            let (token, client_token) = self.credentials().await?;
            let mut builder = http::Request::post(URL)
                .header("Authorization", format!("Bearer {token}"))
                .header("client-token", client_token)
                .header("content-type", "application/json;charset=UTF-8")
                .header("app-platform", "WebPlayer");
            if let Some(language) = request.accept_language {
                builder = builder.header("accept-language", language);
            }
            let http_request = builder
                .body(Bytes::from(request.body.clone()))
                .map_err(|e| TransportError::Network(e.to_string()))?;
            self.exchange(http_request).await
        })
    }

    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Response, TransportError>> {
        Box::pin(async move {
            let (token, client_token) = self.credentials().await?;
            let http_request = http::Request::get(url)
                .header("Authorization", format!("Bearer {token}"))
                .header("client-token", client_token)
                .header("accept", "application/json")
                .header("app-platform", "WebPlayer")
                .body(Bytes::new())
                .map_err(|e| TransportError::Network(e.to_string()))?;
            self.exchange(http_request).await
        })
    }

    fn refresh_token<'a>(&'a self) -> BoxFuture<'a, Result<(), TransportError>> {
        // login5 süresi dolan token'ı kendisi yeniler; çağrı bunu zorlar.
        Box::pin(async move {
            self.session
                .login5()
                .auth_token()
                .await
                .map(|_| ())
                .map_err(|e| TransportError::Token(e.to_string()))
        })
    }

    fn discover<'a>(
        &'a self,
        chunk: &'a str,
        operation: &'a str,
    ) -> BoxFuture<'a, Result<String, DiscoverError>> {
        Box::pin(discover_operation_hash(chunk, operation))
    }
}

/// `Retry-After: <saniye>` (tarih biçimi kullanılmıyor; yoksa `None`).
fn retry_after(headers: &http::HeaderMap) -> Option<Duration> {
    let value = headers.get(http::header::RETRY_AFTER)?.to_str().ok()?;
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

// ── İstek kapısı: aralık, 429 ve devre kesici ──────────────────────────

/// Kapının bir yolun (Spotify ya da Deezer) isteğine cevabı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Gidebilir; önce bu kadar beklenir (sıradaki boş an).
    Go(Duration),
    /// 429 sonrası bekleme: bu kadar daha istek yok.
    Blocked(Duration),
    /// Devre açık: bu oturumda hiç istek yok.
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Arka plan (benzer sanatçılar, Keşfet): sıraya girer, devre kesiciye uyar.
    Background,
    /// Kullanıcı bekliyor (arama): beklemeden gider, 429 beklemesine uyar,
    /// devre açık olsa da denenir; sıradaki arka plan isteklerini kendi payı
    /// kadar öteler.
    Interactive,
}

/// Bir yolun istek kapısı. Saf: zaman dışarıdan verilir.
#[derive(Debug)]
pub struct Gate {
    interval: Duration,
    /// Sıradaki isteğin en erken anı (bekleyen istekler de yer ayırır).
    next: Option<Instant>,
    blocked_until: Option<Instant>,
    /// Üst üste 429 sayısı (yalnız başarı sıfırlar).
    strikes: u32,
    open: bool,
}

/// 429'da Retry-After başlığı yoksa beklenen süre.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Retry-After bundan uzunsa bu kadar beklenir (saçma değere karşı).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);
/// Üst üste bu kadar 429 → devre açılır (oturum boyunca kapalı).
const STRIKES_TO_OPEN: u32 = 2;

impl Gate {
    pub const fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: None,
            blocked_until: None,
            strikes: 0,
            open: false,
        }
    }

    /// Arka plan isteği için yer ayırır.
    pub fn admit(&mut self, now: Instant) -> Admit {
        self.admit_as(now, Priority::Background)
    }

    pub fn admit_as(&mut self, now: Instant, priority: Priority) -> Admit {
        if let Some(refused) = self.refused_for(now, priority) {
            return refused;
        }
        let base = match self.next {
            Some(next) if next > now => next,
            _ => now,
        };
        self.next = Some(base + self.interval);
        match priority {
            Priority::Background => Admit::Go(base - now),
            Priority::Interactive => Admit::Go(Duration::ZERO),
        }
    }

    /// Beklemeden sonra: arada 429 geldiyse ya da devre açıldıysa gitme.
    pub fn refused(&self, now: Instant) -> Option<Admit> {
        self.refused_for(now, Priority::Background)
    }

    fn refused_for(&self, now: Instant, priority: Priority) -> Option<Admit> {
        if self.open && priority == Priority::Background {
            return Some(Admit::Open);
        }
        match self.blocked_until {
            Some(until) if now < until => Some(Admit::Blocked(until - now)),
            _ => None,
        }
    }

    pub fn on_success(&mut self) {
        self.strikes = 0;
    }

    /// 429 geldi: Retry-After kadar istek yok. Devre açıldıysa `true`.
    pub fn on_rate_limited(&mut self, now: Instant, retry_after: Option<Duration>) -> bool {
        self.strikes += 1;
        let wait = retry_after
            .unwrap_or(DEFAULT_RETRY_AFTER)
            .min(MAX_RETRY_AFTER);
        self.blocked_until = Some(now + wait);
        if self.strikes >= STRIKES_TO_OPEN {
            self.open = true;
        }
        self.open
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ── İstemci ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Config {
    /// Kapının istekler arası aralığı.
    pub interval: Duration,
    /// 5xx'te en çok bu kadar yeniden deneme.
    pub server_retries: u32,
    /// İlk yeniden denemeden önceki bekleme; sonrakiler ikiye katlanır.
    pub backoff_base: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(2),
            server_retries: 2,
            backoff_base: Duration::from_millis(500),
        }
    }
}

/// Oturum boyunca paylaşılan durum: kapı, hash tablosu, yeniden arama hakları.
pub struct Pathfinder {
    config: Config,
    gate: Mutex<Gate>,
    hashes: Mutex<HashMap<&'static str, String>>,
    /// Hash'i bu oturumda yeniden aranmış (ve henüz düzelmemiş) işlemler.
    rediscovered: Mutex<HashSet<&'static str>>,
}

/// Bütün motorun paylaştığı istemci (tek kapı).
pub fn shared() -> &'static Pathfinder {
    static SHARED: OnceLock<Pathfinder> = OnceLock::new();
    SHARED.get_or_init(|| Pathfinder::new(Config::default()))
}

/// Arkadaş etkinliğinin kendi istemcisi (kapısı ayrı): `buddylist` ucu GraphQL
/// ağ geçidinden ayrı bir adreste; orada alınan 429 aramayı, Keşfet'i ya da
/// benzer sanatçıları durdurmasın, onların 429'u da buradakini. Kapı, token
/// yenileme ve yeniden deneme kuralları aynı (`get_json`).
pub fn presence() -> &'static Pathfinder {
    static PRESENCE: OnceLock<Pathfinder> = OnceLock::new();
    PRESENCE.get_or_init(|| {
        Pathfinder::new(Config {
            interval: Duration::from_secs(5),
            server_retries: 1,
            backoff_base: Duration::from_secs(1),
        })
    })
}

/// Şarkı künyesinin kendi istemcisi (kapısı ayrı; `credits.rs`): künye ucundaki
/// 429 aramayı ya da Keşfet'i durdurmasın. Kullanıcı düğmeye basınca tek istek;
/// aralık ve yeniden deneme kuralları `get_json` ile aynı.
pub fn credits() -> &'static Pathfinder {
    static CREDITS: OnceLock<Pathfinder> = OnceLock::new();
    CREDITS.get_or_init(|| {
        Pathfinder::new(Config {
            interval: Duration::from_secs(2),
            server_retries: 1,
            backoff_base: Duration::from_secs(1),
        })
    })
}

/// Radyonun kendi istemcisi (kapısı ayrı; `radio.rs`): istasyon ucundaki 429
/// aramayı ya da Keşfet'i durdurmasın. Devam istekleri seyrek (en az 30 sn,
/// `radio::MIN_INTERVAL`), kapının 5 sn aralığı yalnız sıkışık istekleri açar.
pub fn radio() -> &'static Pathfinder {
    static RADIO: OnceLock<Pathfinder> = OnceLock::new();
    RADIO.get_or_init(|| {
        Pathfinder::new(Config {
            interval: Duration::from_secs(5),
            server_retries: 1,
            backoff_base: Duration::from_secs(1),
        })
    })
}

/// Paylaşılan istemciyle canlı bir işlem (kısayol).
pub async fn query(
    session: &Session,
    db: Option<&Db>,
    op: &Operation,
    variables: Value,
    priority: Priority,
) -> Result<Value, PathfinderError> {
    let store = db.map(|d| d as &dyn HashStore);
    shared()
        .call(&LiveTransport::new(session), store, op, variables, priority)
        .await
}

fn request_for(op: &Operation, hash: &str, variables: &Value) -> Request {
    let body = json!({
        "operationName": op.name,
        "variables": variables,
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
    });
    Request {
        body: body.to_string(),
        accept_language: language_header(op, crate::i18n::lang()),
    }
}

/// İşlem dile bağlıysa `accept-language` değeri: arayüz dilinin kodu (`en`, `tr`;
/// yeni bir arayüz dili `Lang::code` ile otomatik gelir). Dile bağlı olmayan
/// işlemde başlık yok.
fn language_header(op: &Operation, lang: crate::i18n::Lang) -> Option<&'static str> {
    op.localized.then(|| lang.code())
}

impl Pathfinder {
    pub fn new(config: Config) -> Self {
        Self {
            gate: Mutex::new(Gate::new(config.interval)),
            config,
            hashes: Mutex::new(HashMap::new()),
            rediscovered: Mutex::new(HashSet::new()),
        }
    }

    /// İşlemin bilinen hash'i: bellek, sonra kalıcı kayıt, sonra seed.
    fn hash_for(&self, op: &Operation, store: Option<&dyn HashStore>) -> Option<String> {
        if let Some(h) = lock(&self.hashes).get(op.name) {
            return Some(h.clone());
        }
        let found = store
            .and_then(|s| s.load(op.meta_key))
            .or_else(|| op.seed.map(String::from))?;
        Some(lock(&self.hashes).entry(op.name).or_insert(found).clone())
    }

    fn current_hash(&self, op: &Operation) -> Option<String> {
        lock(&self.hashes).get(op.name).cloned()
    }

    /// Hash'i belleğe yaz (kalıcı kayıt ayrı).
    pub fn remember(&self, op: &Operation, hash: &str) {
        lock(&self.hashes).insert(op.name, hash.to_string());
    }

    fn claim_rediscovery(&self, op: &Operation) -> bool {
        lock(&self.rediscovered).insert(op.name)
    }

    fn release_rediscovery(&self, op: &Operation) {
        lock(&self.rediscovered).remove(op.name);
    }

    fn adopt(&self, op: &Operation, hash: &str, store: Option<&dyn HashStore>) {
        self.remember(op, hash);
        if let Some(store) = store {
            store.save(op.meta_key, hash);
        }
    }

    /// Kapıdan geç: sıra gelene kadar bekle, sonra yeniden bak.
    async fn pass(&self, priority: Priority) -> Result<(), PathfinderError> {
        let admit = lock(&self.gate).admit_as(Instant::now(), priority);
        match admit {
            Admit::Go(wait) => {
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
                match lock(&self.gate).refused_for(Instant::now(), priority) {
                    Some(refused) => Err(refused.into()),
                    None => Ok(()),
                }
            }
            refused => Err(refused.into()),
        }
    }

    /// Tek istek, verilen hash'le: kapı, 5xx / 401 yeniden denemesi, 429'un
    /// kapıya bildirilmesi. Hash'i düzeltmez (`call` düzeltir); eski hash
    /// `StaleHash { cause: NotRecovered }` döner.
    pub async fn send(
        &self,
        transport: &dyn Transport,
        op: &Operation,
        hash: &str,
        variables: &Value,
        priority: Priority,
    ) -> Result<Value, PathfinderError> {
        let request = request_for(op, hash, variables);
        let mut server_attempts = 0u32;
        let mut refreshed = false;
        loop {
            self.pass(priority).await?;
            let response = transport.post(&request).await?;
            match response.status {
                200..=299 => {
                    let value: Value = serde_json::from_slice(&response.body)
                        .map_err(|e| PathfinderError::InvalidResponse(e.to_string()))?;
                    if let Some(detail) = op.stale_reason(&value) {
                        return Err(PathfinderError::StaleHash {
                            operation: op.name,
                            detail,
                            cause: StaleCause::NotRecovered,
                        });
                    }
                    lock(&self.gate).on_success();
                    return Ok(value);
                }
                429 => {
                    let circuit_open =
                        lock(&self.gate).on_rate_limited(Instant::now(), response.retry_after);
                    log::warn!(
                        "{}: Spotify 429 (Retry-After {:?}); yeniden denenmiyor",
                        op.name,
                        response.retry_after
                    );
                    return Err(PathfinderError::RateLimited {
                        retry_after: response.retry_after,
                        circuit_open,
                    });
                }
                // Eski hash'le Spotify genelde 400 / 404 döndürüyor.
                status @ (400 | 404) => {
                    return Err(PathfinderError::StaleHash {
                        operation: op.name,
                        detail: format!("HTTP {status}"),
                        cause: StaleCause::NotRecovered,
                    });
                }
                401 if !refreshed => {
                    refreshed = true;
                    transport.refresh_token().await?;
                }
                status @ (401 | 403) => return Err(PathfinderError::Denied { status }),
                status @ 500..=599 if server_attempts < self.config.server_retries => {
                    let wait = self.config.backoff_base * 2u32.saturating_pow(server_attempts);
                    server_attempts += 1;
                    log::debug!(
                        "{}: HTTP {status}, {wait:?} sonra yeniden deneniyor ({server_attempts})",
                        op.name
                    );
                    tokio::time::sleep(wait).await;
                }
                status => return Err(PathfinderError::Http { status }),
            }
        }
    }

    /// Hash'siz GET (`Transport::get`): kapı, 5xx / 401 yeniden denemesi ve
    /// 429'un kapıya bildirilmesi `send` ile aynı; gövde JSON olarak döner.
    /// `name` yalnız günlük içindir.
    pub async fn get_json(
        &self,
        transport: &dyn Transport,
        name: &'static str,
        url: &str,
        priority: Priority,
    ) -> Result<Value, PathfinderError> {
        let mut server_attempts = 0u32;
        let mut refreshed = false;
        loop {
            self.pass(priority).await?;
            let response = transport.get(url).await?;
            match response.status {
                200..=299 => {
                    let value: Value = serde_json::from_slice(&response.body)
                        .map_err(|e| PathfinderError::InvalidResponse(e.to_string()))?;
                    lock(&self.gate).on_success();
                    return Ok(value);
                }
                429 => {
                    let circuit_open =
                        lock(&self.gate).on_rate_limited(Instant::now(), response.retry_after);
                    log::warn!(
                        "{name}: Spotify 429 (Retry-After {:?}); yeniden denenmiyor",
                        response.retry_after
                    );
                    return Err(PathfinderError::RateLimited {
                        retry_after: response.retry_after,
                        circuit_open,
                    });
                }
                401 if !refreshed => {
                    refreshed = true;
                    transport.refresh_token().await?;
                }
                status @ (401 | 403) => return Err(PathfinderError::Denied { status }),
                status @ 500..=599 if server_attempts < self.config.server_retries => {
                    let wait = self.config.backoff_base * 2u32.saturating_pow(server_attempts);
                    server_attempts += 1;
                    log::debug!(
                        "{name}: HTTP {status}, {wait:?} sonra yeniden deneniyor ({server_attempts})"
                    );
                    tokio::time::sleep(wait).await;
                }
                status => return Err(PathfinderError::Http { status }),
            }
        }
    }

    /// İşlemi çağırır: hash'i tablodan alır (yoksa web player'dan bulur), eski
    /// hash'te yenisini bir kez arayıp isteği yineler, bulunan hash'i kaydeder.
    pub async fn call(
        &self,
        transport: &dyn Transport,
        store: Option<&dyn HashStore>,
        op: &Operation,
        variables: Value,
        priority: Priority,
    ) -> Result<Value, PathfinderError> {
        let mut hash = match self.hash_for(op, store) {
            Some(hash) => hash,
            // Seed'siz işlem, ilk istek: yeniden arama hakkını harcamaz.
            None => {
                let found = self.discover(transport, op).await?;
                self.adopt(op, &found, store);
                found
            }
        };
        let mut rediscovered_here = false;
        let mut adopted_elsewhere = false;
        loop {
            match self.send(transport, op, &hash, &variables, priority).await {
                Ok(value) => {
                    if rediscovered_here {
                        // Yeni hash çalıştı: ileride yine eskirse yeniden aranabilsin.
                        self.release_rediscovery(op);
                    }
                    return Ok(value);
                }
                Err(PathfinderError::StaleHash { detail, .. }) => {
                    let stale = |cause: StaleCause| PathfinderError::StaleHash {
                        operation: op.name,
                        detail: detail.clone(),
                        cause,
                    };
                    if rediscovered_here {
                        return Err(stale(StaleCause::Rejected));
                    }
                    // Eşzamanlı başka bir istek hash'i çoktan düzeltmiş olabilir.
                    if !adopted_elsewhere
                        && let Some(current) = self.current_hash(op)
                        && current != hash
                    {
                        adopted_elsewhere = true;
                        hash = current;
                        continue;
                    }
                    if !self.claim_rediscovery(op) {
                        // Bu oturumda zaten arandı: bir daha değil.
                        return Err(stale(StaleCause::AlreadyRediscovered));
                    }
                    rediscovered_here = true;
                    log::info!(
                        "{}: hash eskimiş ({detail}), web player'dan yenisi aranıyor",
                        op.name
                    );
                    match self.discover(transport, op).await {
                        Ok(new) if new != hash => {
                            self.adopt(op, &new, store);
                            hash = new;
                        }
                        Ok(_) => return Err(stale(StaleCause::Unchanged)),
                        Err(e) => {
                            if matches!(e, PathfinderError::Discovery(_)) {
                                // İndirilemedi: sonra yeniden aranabilsin.
                                self.release_rediscovery(op);
                            }
                            return Err(e);
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn discover(
        &self,
        transport: &dyn Transport,
        op: &Operation,
    ) -> Result<String, PathfinderError> {
        transport
            .discover(op.chunk, op.name)
            .await
            .map_err(|e| match e {
                DiscoverError::Missing(_) => PathfinderError::OperationGone { operation: op.name },
                DiscoverError::Fetch(e) => PathfinderError::Discovery(e),
            })
    }
}

impl From<Admit> for PathfinderError {
    fn from(refused: Admit) -> Self {
        match refused {
            Admit::Open => Self::CircuitOpen,
            Admit::Blocked(remaining) => Self::Blocked { remaining },
            // `Go` reddedilmiş olamaz; yine de güvenli tarafta.
            Admit::Go(_) => Self::Blocked {
                remaining: Duration::ZERO,
            },
        }
    }
}

// ── Hash'in otomatik bulunması (web player paketleri) ───────────────────

/// Web player'ın paketlerinden bir GraphQL sorgusunun (`operation`) güncel
/// hash'ini bulur: önce ana paket, sonra sorgunun bulunduğu parça (`chunk`,
/// ör. `xpui-routes-search`, `xpui-routes-artist`).
///
/// Paketler librespot'un HTTP istemcisiyle indirilmiyor: o, `User-Agent`
/// başlığını kendi değeriyle eziyor ve Spotify tanımadığı istemciye web
/// player yerine "desteklenmeyen tarayıcı" sayfası gönderiyor.
pub async fn discover_operation_hash(
    chunk: &str,
    operation: &str,
) -> Result<String, DiscoverError> {
    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .build()
        .map_err(anyhow::Error::from)?;
    let html = fetch_text(&client, WEB_PLAYER_URL).await?;
    let main_url = find_main_bundle(&html).context("web-player bundle not found")?;
    let main_js = fetch_text(&client, &main_url).await?;
    if let Some(hash) = find_query_hash(&main_js, operation) {
        return Ok(hash);
    }

    // Parça adı → kimlik, kimlik → dosya sürümü (birden çok eşleşme olabilir:
    // JS ve CSS haritaları; sırayla denenir).
    let Some(chunk_id) = find_chunk_id(&main_js, chunk) else {
        return Err(DiscoverError::Missing(operation.to_string()));
    };
    let mut fetched = false;
    for version in find_chunk_versions(&main_js, &chunk_id) {
        let url = format!("{CDN_BUILD_URL}{chunk}.{version}.js");
        let Ok(js) = fetch_text(&client, &url).await else {
            continue;
        };
        fetched = true;
        if let Some(hash) = find_query_hash(&js, operation) {
            return Ok(hash);
        }
    }
    if fetched {
        Err(DiscoverError::Missing(operation.to_string()))
    } else {
        Err(DiscoverError::Fetch(anyhow::anyhow!(
            "chunk {chunk} could not be downloaded"
        )))
    }
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .with_context(|| format!("{url} could not be downloaded"))?;
    Ok(response.text().await?)
}

/// `<script src=".../web-player.<sürüm>.js">`
fn find_main_bundle(html: &str) -> Option<String> {
    html.split("src=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .find(|s| s.contains("/web-player.") && s.ends_with(".js"))
        .map(String::from)
}

/// `4406:"xpui-routes-search"` → `4406`
fn find_chunk_id(js: &str, name: &str) -> Option<String> {
    let needle = format!(":\"{name}\"");
    let at = js.find(&needle)?;
    let id: String = js[..at]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (!id.is_empty()).then_some(id)
}

/// `4406:"a89e24fe"` biçimindeki bütün eşleşmeler (8 haneli onaltılık).
fn find_chunk_versions(js: &str, id: &str) -> Vec<String> {
    let needle = format!("{id}:\"");
    let mut versions = Vec::new();
    let mut rest = js;
    while let Some(at) = rest.find(&needle) {
        // Başka bir sayının sonu olmasın (ör. 14406:).
        let before = rest[..at].chars().last();
        rest = &rest[at + needle.len()..];
        if before.is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let value: String = rest.chars().take_while(|c| *c != '"').collect();
        if value.len() == 8
            && value.chars().all(|c| c.is_ascii_hexdigit())
            && !versions.contains(&value)
        {
            versions.push(value);
        }
    }
    versions
}

/// `"searchDesktop","query","<64 haneli hash>"`
fn find_query_hash(js: &str, operation: &str) -> Option<String> {
    let needle = format!("\"{operation}\",\"query\",\"");
    let at = js.find(&needle)? + needle.len();
    let hash: String = js[at..].chars().take_while(|c| *c != '"').collect();
    (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;

    // ── Paket ayrıştırma ───────────────────────────────────────────────

    #[test]
    fn finds_bundle_chunk_and_hash() {
        let html = r#"<script src="https://x/gtm.js"></script><script src="https://open.spotifycdn.com/cdn/build/web-player/web-player.421304b0.js"></script>"#;
        assert_eq!(
            find_main_bundle(html).unwrap(),
            "https://open.spotifycdn.com/cdn/build/web-player/web-player.421304b0.js"
        );

        let js = r#"{4068:"xpui-routes-country-hub",4406:"xpui-routes-search",4563:"x"}...{14406:"deadbeef",4406:"a0a5ee13"}...{4406:"a89e24fe"}"#;
        let id = find_chunk_id(js, "xpui-routes-search").unwrap();
        assert_eq!(id, "4406");
        assert_eq!(find_chunk_versions(js, &id), ["a0a5ee13", "a89e24fe"]);

        let hash = "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03";
        let chunk = format!(
            r#"new x("searchAlbums","query","{}",null),new x("searchDesktop","query","{hash}",null)"#,
            "0".repeat(64)
        );
        assert_eq!(find_query_hash(&chunk, "searchDesktop").unwrap(), hash);
        assert!(find_query_hash(&chunk, "searchNope").is_none());
    }

    #[test]
    fn stale_detection_per_operation() {
        let not_found = json!({ "errors": [ { "message": "PersistedQueryNotFound" } ] });
        let other = json!({ "errors": [ { "message": "Something broke" } ], "data": null });
        let ok = json!({ "data": { "searchV2": { "x": 1 }, "artistUnion": {} } });
        // Yalnız PersistedQueryNotFound (benzer sanatçılar, Keşfet).
        assert!(ARTIST_RELATED.stale_reason(&not_found).is_some());
        assert!(ARTIST_RELATED.stale_reason(&other).is_none());
        assert!(ARTIST_RELATED.stale_reason(&ok).is_none());
        // Arama katı: her GraphQL hatası ve eksik `searchV2` eskime sayılır.
        assert_eq!(
            SEARCH_DESKTOP.stale_reason(&not_found).as_deref(),
            Some("PersistedQueryNotFound")
        );
        assert_eq!(
            SEARCH_DESKTOP.stale_reason(&other).as_deref(),
            Some("Something broke")
        );
        assert!(
            SEARCH_DESKTOP
                .stale_reason(&json!({ "data": {} }))
                .is_some()
        );
        assert!(SEARCH_DESKTOP.stale_reason(&ok).is_none());
    }

    #[test]
    fn retry_after_header() {
        let mut headers = http::HeaderMap::new();
        assert_eq!(retry_after(&headers), None);
        headers.insert(http::header::RETRY_AFTER, "17".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(17)));
        headers.insert(
            http::header::RETRY_AFTER,
            "Wed, 21 Oct 2015 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), None);
    }

    // ── Kapı ───────────────────────────────────────────────────────────

    const INTERVAL: Duration = Duration::from_secs(2);

    #[test]
    fn gate_spaces_requests() {
        let t0 = Instant::now();
        let mut g = Gate::new(INTERVAL);
        assert_eq!(g.admit(t0), Admit::Go(Duration::ZERO));
        // Hemen ardından gelen ikinci ve üçüncü istek sıraya girer.
        assert_eq!(g.admit(t0), Admit::Go(INTERVAL));
        assert_eq!(
            g.admit(t0 + Duration::from_millis(500)),
            Admit::Go(Duration::from_millis(3500))
        );
        // Uzun aradan sonra beklemeden.
        assert_eq!(
            g.admit(t0 + Duration::from_secs(60)),
            Admit::Go(Duration::ZERO)
        );
    }

    #[test]
    fn gate_honors_retry_after_and_opens_after_two_429s() {
        let t0 = Instant::now();
        let mut g = Gate::new(INTERVAL);
        assert_eq!(g.admit(t0), Admit::Go(Duration::ZERO));
        assert!(!g.on_rate_limited(t0, Some(Duration::from_secs(30))));
        // Retry-After boyunca istek yok.
        assert_eq!(
            g.admit(t0 + Duration::from_secs(10)),
            Admit::Blocked(Duration::from_secs(20))
        );
        assert!(g.refused(t0 + Duration::from_secs(29)).is_some());
        // Süre dolunca geçer; başarı sayacı sıfırlar.
        assert!(matches!(
            g.admit(t0 + Duration::from_secs(30)),
            Admit::Go(_)
        ));
        g.on_success();
        assert!(!g.on_rate_limited(t0 + Duration::from_secs(31), None));
        // Başlık yoksa varsayılan bekleme.
        assert_eq!(
            g.admit(t0 + Duration::from_secs(31)),
            Admit::Blocked(DEFAULT_RETRY_AFTER)
        );
        // Üst üste ikinci 429: devre açılır, oturum boyunca kapalı.
        assert!(g.on_rate_limited(t0 + Duration::from_secs(200), Some(Duration::from_secs(1))));
        assert!(g.is_open());
        assert_eq!(g.admit(t0 + Duration::from_secs(10_000)), Admit::Open);
        // Saçma uzun Retry-After sınırlanır.
        let mut g = Gate::new(INTERVAL);
        g.on_rate_limited(t0, Some(Duration::from_secs(10 * 24 * 3600)));
        assert_eq!(g.admit(t0), Admit::Blocked(MAX_RETRY_AFTER));
    }

    #[test]
    fn interactive_requests_skip_the_queue_but_respect_429() {
        let t0 = Instant::now();
        let mut g = Gate::new(INTERVAL);
        assert_eq!(g.admit(t0), Admit::Go(Duration::ZERO));
        // Arama sıraya girmez ama sıradaki arka plan isteğini bir aralık öteler.
        assert_eq!(
            g.admit_as(t0, Priority::Interactive),
            Admit::Go(Duration::ZERO)
        );
        assert_eq!(g.admit(t0), Admit::Go(2 * INTERVAL));
        // 429 beklemesine arama da uyar.
        g.on_rate_limited(t0, Some(Duration::from_secs(30)));
        assert_eq!(
            g.admit_as(t0 + Duration::from_secs(10), Priority::Interactive),
            Admit::Blocked(Duration::from_secs(20))
        );
        // Devre açılınca arka plan kapanır, arama bekleme bitince yine denenir.
        assert!(g.on_rate_limited(t0 + Duration::from_secs(40), Some(Duration::from_secs(5))));
        assert_eq!(g.admit(t0 + Duration::from_secs(100)), Admit::Open);
        assert!(matches!(
            g.admit_as(t0 + Duration::from_secs(100), Priority::Interactive),
            Admit::Go(_)
        ));
        assert!(matches!(
            g.admit_as(t0 + Duration::from_secs(41), Priority::Interactive),
            Admit::Blocked(_)
        ));
    }

    // ── Sahte ağ ───────────────────────────────────────────────────────

    const OLD: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const NEWER: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    fn ok_body() -> Response {
        Response {
            status: 200,
            retry_after: None,
            body: Bytes::from(r#"{"data":{"searchV2":{"ok":true},"artistUnion":{}}}"#),
        }
    }

    fn status(status: u16) -> Response {
        Response {
            status,
            retry_after: None,
            body: Bytes::new(),
        }
    }

    fn too_many(retry_after: Duration) -> Response {
        Response {
            status: 429,
            retry_after: Some(retry_after),
            body: Bytes::new(),
        }
    }

    #[derive(Default)]
    struct Fake {
        /// Sıradaki yanıtlar; boşsa 200.
        responses: Mutex<VecDeque<Result<Response, TransportError>>>,
        /// Gönderilen istekler: (işlem, hash, an).
        posts: Mutex<Vec<(String, String, Instant)>>,
        discoveries: Mutex<VecDeque<Result<String, DiscoverFailure>>>,
        discover_calls: AtomicUsize,
        refresh_calls: AtomicUsize,
        /// İlk istek gidince çalışır (eşzamanlı düzeltme taklidi).
        on_first_post: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    enum DiscoverFailure {
        Missing,
        Fetch,
    }

    impl Fake {
        fn respond(self, r: Vec<Result<Response, TransportError>>) -> Self {
            *self.responses.lock().unwrap() = r.into();
            self
        }
        fn discover_with(self, d: Vec<Result<String, DiscoverFailure>>) -> Self {
            *self.discoveries.lock().unwrap() = d.into();
            self
        }
        fn hashes(&self) -> Vec<String> {
            self.posts
                .lock()
                .unwrap()
                .iter()
                .map(|p| p.1.clone())
                .collect()
        }
        fn post_count(&self) -> usize {
            self.posts.lock().unwrap().len()
        }
    }

    impl Transport for Fake {
        fn post<'a>(
            &'a self,
            request: &'a Request,
        ) -> BoxFuture<'a, Result<Response, TransportError>> {
            Box::pin(async move {
                let value: Value = serde_json::from_str(&request.body).unwrap();
                let hash = value["extensions"]["persistedQuery"]["sha256Hash"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let operation = value["operationName"].as_str().unwrap().to_string();
                self.posts
                    .lock()
                    .unwrap()
                    .push((operation, hash, Instant::now()));
                if let Some(hook) = self.on_first_post.lock().unwrap().take() {
                    hook();
                }
                self.responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Ok(ok_body()))
            })
        }

        fn refresh_token<'a>(&'a self) -> BoxFuture<'a, Result<(), TransportError>> {
            Box::pin(async move {
                self.refresh_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }

        fn discover<'a>(
            &'a self,
            _chunk: &'a str,
            _operation: &'a str,
        ) -> BoxFuture<'a, Result<String, DiscoverError>> {
            Box::pin(async move {
                self.discover_calls.fetch_add(1, Ordering::SeqCst);
                match self
                    .discoveries
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("beklenmeyen keşif")
                {
                    Ok(h) => Ok(h),
                    Err(DiscoverFailure::Missing) => Err(DiscoverError::Missing("op".into())),
                    Err(DiscoverFailure::Fetch) => {
                        Err(DiscoverError::Fetch(anyhow::anyhow!("offline")))
                    }
                }
            })
        }
    }

    #[derive(Default)]
    struct MemStore(Mutex<HashMap<String, String>>);

    impl MemStore {
        fn with(key: &str, hash: &str) -> Self {
            let s = Self::default();
            s.save(key, hash);
            s
        }
        fn get(&self, key: &str) -> Option<String> {
            self.0.lock().unwrap().get(key).cloned()
        }
    }

    impl HashStore for MemStore {
        fn load(&self, key: &str) -> Option<String> {
            self.get(key)
        }
        fn save(&self, key: &str, hash: &str) {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), hash.to_string());
        }
    }

    fn fast() -> Config {
        Config {
            interval: Duration::ZERO,
            server_retries: 2,
            backoff_base: Duration::from_millis(1),
        }
    }

    const BG: Priority = Priority::Background;
    const UI: Priority = Priority::Interactive;

    // ── Hash kaçırma → yeniden keşif ───────────────────────────────────

    #[tokio::test]
    async fn hash_miss_rediscovers_retries_and_persists() {
        let pf = Pathfinder::new(fast());
        let store = MemStore::with("hash:queryArtistRelated", OLD);
        let fake = Fake::default()
            .respond(vec![Ok(status(400))])
            .discover_with(vec![Ok(NEW.into())]);
        let v = pf
            .call(
                &fake,
                Some(&store),
                &ARTIST_RELATED,
                json!({ "uri": "x" }),
                BG,
            )
            .await
            .unwrap();
        assert_eq!(v["data"]["searchV2"]["ok"], true);
        assert_eq!(fake.hashes(), [OLD, NEW]);
        assert_eq!(fake.discover_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.get("hash:queryArtistRelated").as_deref(),
            Some(NEW),
            "bulunan hash kalıcı"
        );
        // Sonraki çağrı yeni hash'le doğrudan gider.
        pf.call(&fake, Some(&store), &ARTIST_RELATED, json!({}), BG)
            .await
            .unwrap();
        assert_eq!(fake.hashes(), [OLD, NEW, NEW]);
        assert_eq!(fake.discover_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn persisted_query_not_found_in_a_200_also_rediscovers() {
        let pf = Pathfinder::new(fast());
        let body = Bytes::from(r#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#);
        let fake = Fake::default()
            .respond(vec![Ok(Response {
                status: 200,
                retry_after: None,
                body,
            })])
            .discover_with(vec![Ok(NEW.into())]);
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        assert_eq!(fake.hashes(), [HOME.seed.unwrap(), NEW]);
    }

    #[tokio::test]
    async fn seedless_operation_discovers_before_first_request() {
        let pf = Pathfinder::new(fast());
        let store = MemStore::default();
        let fake = Fake::default().discover_with(vec![Ok(NEW.into())]);
        pf.call(
            &fake,
            Some(&store),
            &SEARCH_DESKTOP,
            json!({ "searchTerm": "a" }),
            UI,
        )
        .await
        .unwrap();
        assert_eq!(fake.hashes(), [NEW]);
        assert_eq!(store.get("search_hash").as_deref(), Some(NEW));
        // İlk keşif yeniden arama hakkını harcamadı: sonra eskirse aranır.
        *fake.responses.lock().unwrap() = VecDeque::from([Ok(status(404))]);
        fake.discoveries.lock().unwrap().push_back(Ok(NEWER.into()));
        pf.call(&fake, Some(&store), &SEARCH_DESKTOP, json!({}), UI)
            .await
            .unwrap();
        assert_eq!(fake.hashes(), [NEW, NEW, NEWER]);
    }

    #[tokio::test]
    async fn stored_hash_is_used_before_seed_and_memory_before_store() {
        let pf = Pathfinder::new(fast());
        let store = MemStore::with("hash:home", OLD);
        let fake = Fake::default();
        pf.call(&fake, Some(&store), &HOME, json!({}), BG)
            .await
            .unwrap();
        // Kayıt başka bir şeye çevrilse de bellekteki geçerli.
        store.save("hash:home", NEWER);
        pf.call(&fake, Some(&store), &HOME, json!({}), BG)
            .await
            .unwrap();
        assert_eq!(fake.hashes(), [OLD, OLD]);
    }

    #[tokio::test]
    async fn rediscovery_is_once_per_operation_and_failures_are_typed() {
        let pf = Pathfinder::new(fast());
        // Yeni hash da reddedilir.
        let fake = Fake::default()
            .respond(vec![Ok(status(400)), Ok(status(400))])
            .discover_with(vec![Ok(NEW.into())]);
        let e = pf
            .call(&fake, None, &BROWSE_ALL, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(
                e,
                PathfinderError::StaleHash {
                    cause: StaleCause::Rejected,
                    ..
                }
            ),
            "{e:?}"
        );
        // Aynı oturumda ikinci kez aranmaz.
        *fake.responses.lock().unwrap() = VecDeque::from([Ok(status(400))]);
        let e = pf
            .call(&fake, None, &BROWSE_ALL, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(
                e,
                PathfinderError::StaleHash {
                    cause: StaleCause::AlreadyRediscovered,
                    ..
                }
            ),
            "{e:?}"
        );
        assert_eq!(fake.discover_calls.load(Ordering::SeqCst), 1);
        // Başka işlemin hakkı ayrı.
        let fake2 = Fake::default()
            .respond(vec![Ok(status(404))])
            .discover_with(vec![Ok(NEW.into())]);
        pf.call(&fake2, None, &BROWSE_PAGE, json!({}), BG)
            .await
            .unwrap();
        assert_eq!(fake2.discover_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn discovery_outcomes_map_to_typed_errors() {
        // Aynı hash: güncel hash reddediliyor.
        let pf = Pathfinder::new(fast());
        let fake = Fake::default()
            .respond(vec![Ok(status(400))])
            .discover_with(vec![Ok(HOME.seed.unwrap().into())]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(
                e,
                PathfinderError::StaleHash {
                    cause: StaleCause::Unchanged,
                    ..
                }
            ),
            "{e:?}"
        );

        // Sorgu web player'da yok.
        let pf = Pathfinder::new(fast());
        let fake = Fake::default()
            .respond(vec![Ok(status(400))])
            .discover_with(vec![Err(DiscoverFailure::Missing)]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(e, PathfinderError::OperationGone { operation: "home" }),
            "{e:?}"
        );

        // İndirme olmadı: hak geri verilir, sonra yeniden aranabilir.
        let pf = Pathfinder::new(fast());
        let fake = Fake::default()
            .respond(vec![Ok(status(400)), Ok(status(400))])
            .discover_with(vec![Err(DiscoverFailure::Fetch), Ok(NEW.into())]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::Discovery(_)), "{e:?}");
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        assert_eq!(fake.discover_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rediscovery_right_returns_after_the_new_hash_works() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default()
            .respond(vec![Ok(status(400))])
            .discover_with(vec![Ok(NEW.into()), Ok(NEWER.into())]);
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        // Saatler sonra yine eskiyor: yeniden aranır.
        *fake.responses.lock().unwrap() = VecDeque::from([Ok(status(400))]);
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        assert_eq!(fake.hashes(), [HOME.seed.unwrap(), NEW, NEW, NEWER]);
    }

    #[tokio::test]
    async fn concurrent_fix_is_adopted_without_a_second_discovery() {
        let pf = Arc::new(Pathfinder::new(fast()));
        let fixer = pf.clone();
        let fake = Fake::default().respond(vec![Ok(status(400))]);
        // İlk istek giderken başka bir istek hash'i çoktan düzeltmiş.
        *fake.on_first_post.lock().unwrap() = Some(Box::new(move || fixer.remember(&HOME, NEW)));
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        assert_eq!(fake.hashes(), [HOME.seed.unwrap(), NEW]);
        assert_eq!(fake.discover_calls.load(Ordering::SeqCst), 0);
    }

    // ── 5xx / 401 / 403 ────────────────────────────────────────────────

    #[tokio::test]
    async fn server_errors_back_off_exponentially_then_succeed() {
        let cfg = Config {
            backoff_base: Duration::from_millis(40),
            ..fast()
        };
        let pf = Pathfinder::new(cfg);
        let fake = Fake::default().respond(vec![Ok(status(503)), Ok(status(502))]);
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        let posts = fake.posts.lock().unwrap();
        assert_eq!(posts.len(), 3);
        let gap1 = posts[1].2 - posts[0].2;
        let gap2 = posts[2].2 - posts[1].2;
        assert!(gap1 >= Duration::from_millis(38), "{gap1:?}");
        assert!(
            gap2 >= Duration::from_millis(78),
            "ikiye katlanır: {gap2:?}"
        );
    }

    #[tokio::test]
    async fn server_errors_give_up_after_the_retries() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default().respond(vec![
            Ok(status(500)),
            Ok(status(500)),
            Ok(status(503)),
            Ok(status(500)),
        ]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::Http { status: 503 }), "{e:?}");
        assert_eq!(fake.post_count(), 3, "ilk + 2 yeniden deneme");
        assert_eq!(
            fake.discover_calls.load(Ordering::SeqCst),
            0,
            "5xx hash'i suçlamaz"
        );
        // Beklenmeyen kod yinelenmez.
        let fake = Fake::default().respond(vec![Ok(status(418))]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::Http { status: 418 }));
        assert_eq!(fake.post_count(), 1);
    }

    #[tokio::test]
    async fn unauthorized_refreshes_the_token_once() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default().respond(vec![Ok(status(401))]);
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        assert_eq!(fake.refresh_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fake.post_count(), 2);
        // İkinci 401: reddedildi.
        let fake = Fake::default().respond(vec![Ok(status(401)), Ok(status(401))]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(e, PathfinderError::Denied { status: 401 }),
            "{e:?}"
        );
        assert_eq!(fake.refresh_calls.load(Ordering::SeqCst), 1);
        // 403 yinelenmez.
        let fake = Fake::default().respond(vec![Ok(status(403))]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::Denied { status: 403 }));
        assert_eq!(fake.post_count(), 1);
    }

    #[tokio::test]
    async fn transport_and_json_failures_are_typed_and_not_retried() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default().respond(vec![Err(TransportError::Network(
            "connection reset".into(),
        ))]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(&e, PathfinderError::Transport(m) if m.contains("reset")),
            "{e:?}"
        );
        assert_eq!(fake.post_count(), 1);
        let fake =
            Fake::default().respond(vec![Err(TransportError::Token("no credentials".into()))]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::Token(_)), "{e:?}");
        let fake = Fake::default().respond(vec![Ok(Response {
            status: 200,
            retry_after: None,
            body: Bytes::from("<html>"),
        })]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::InvalidResponse(_)), "{e:?}");
    }

    // ── 429 ve ortak kapı ──────────────────────────────────────────────

    #[tokio::test]
    async fn a_429_blocks_every_operation_including_search() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default().respond(vec![Ok(too_many(Duration::from_secs(30)))]);
        let e = pf
            .call(&fake, None, &ARTIST_RELATED, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(e, PathfinderError::RateLimited { retry_after: Some(d), circuit_open: false } if d == Duration::from_secs(30)),
            "{e:?}"
        );
        assert_eq!(fake.post_count(), 1, "429 yinelenmez");
        // Başka bir arka plan işlemi de, arama da Retry-After boyunca gönderilmez.
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::Blocked { .. }), "{e:?}");
        let store = MemStore::with("search_hash", OLD);
        let e = pf
            .call(&fake, Some(&store), &SEARCH_DESKTOP, json!({}), UI)
            .await
            .unwrap_err();
        assert!(
            matches!(e, PathfinderError::Blocked { remaining } if remaining > Duration::from_secs(20)),
            "{e:?}"
        );
        assert_eq!(fake.post_count(), 1, "kapı aramayı da tuttu");
        // Hata metni arayüzün sınıflandırması için "429" / "rate" içerir.
        assert!(
            PathfinderError::Blocked {
                remaining: Duration::ZERO
            }
            .to_string()
            .contains("rate")
        );
        assert!(e.to_string().to_lowercase().contains("rate"));
    }

    #[tokio::test]
    async fn two_429s_open_the_circuit_but_search_keeps_trying() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default().respond(vec![
            Ok(too_many(Duration::ZERO)),
            Ok(too_many(Duration::ZERO)),
        ]);
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(
            e,
            PathfinderError::RateLimited {
                circuit_open: false,
                ..
            }
        ));
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(
                e,
                PathfinderError::RateLimited {
                    circuit_open: true,
                    ..
                }
            ),
            "{e:?}"
        );
        // Arka plan: devre açık, istek yok.
        let e = pf
            .call(&fake, None, &BROWSE_ALL, json!({}), BG)
            .await
            .unwrap_err();
        assert!(matches!(e, PathfinderError::CircuitOpen), "{e:?}");
        assert_eq!(fake.post_count(), 2);
        // Arama (başka yedeği yok) bekleme bitince yine denenir.
        let store = MemStore::with("search_hash", OLD);
        pf.call(&fake, Some(&store), &SEARCH_DESKTOP, json!({}), UI)
            .await
            .unwrap();
        assert_eq!(fake.post_count(), 3);
    }

    #[tokio::test]
    async fn search_shares_the_gate_with_background_requests() {
        let cfg = Config {
            interval: Duration::from_millis(120),
            ..fast()
        };
        let pf = Pathfinder::new(cfg);
        let store = MemStore::with("search_hash", OLD);
        let fake = Fake::default();
        // Arama beklemeden gider...
        pf.call(&fake, Some(&store), &SEARCH_DESKTOP, json!({}), UI)
            .await
            .unwrap();
        // ...ama hemen ardından gelen arka plan isteği onun payını bekler.
        pf.call(&fake, None, &ARTIST_RELATED, json!({}), BG)
            .await
            .unwrap();
        // Arka plan istekleri kendi aralarında da sıralı.
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        let posts = fake.posts.lock().unwrap();
        assert_eq!(posts.len(), 3);
        let search_to_related = posts[1].2 - posts[0].2;
        // Kapı yuvaları ilk istekten sayılır: yük altında bir isteğin kaydı
        // geç düşse de yuva zamanı kaymaz, ardışık fark kısa görünebilir.
        let search_to_home = posts[2].2 - posts[0].2;
        assert!(
            search_to_related >= Duration::from_millis(110),
            "arama kapının sırasını itti: {search_to_related:?}"
        );
        assert!(
            search_to_home >= Duration::from_millis(230),
            "{search_to_home:?}"
        );
    }

    #[tokio::test]
    async fn concurrent_background_requests_are_spaced() {
        let cfg = Config {
            interval: Duration::from_millis(80),
            ..fast()
        };
        let pf = Arc::new(Pathfinder::new(cfg));
        let fake = Arc::new(Fake::default());
        let mut tasks = Vec::new();
        for _ in 0..3 {
            let (pf, fake) = (pf.clone(), fake.clone());
            tasks.push(tokio::spawn(async move {
                pf.call(&*fake, None, &HOME, json!({}), BG)
                    .await
                    .map(|_| ())
            }));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        let mut times: Vec<Instant> = fake.posts.lock().unwrap().iter().map(|p| p.2).collect();
        times.sort();
        assert_eq!(times.len(), 3);
        assert!(
            times[1] - times[0] >= Duration::from_millis(70),
            "{:?}",
            times[1] - times[0]
        );
        assert!(
            times[2] - times[0] >= Duration::from_millis(150),
            "{:?}",
            times[2] - times[0]
        );
    }

    #[tokio::test]
    async fn success_resets_the_strike_count() {
        let pf = Pathfinder::new(fast());
        let fake = Fake::default().respond(vec![
            Ok(too_many(Duration::ZERO)),
            Ok(ok_body()),
            Ok(too_many(Duration::ZERO)),
        ]);
        assert!(pf.call(&fake, None, &HOME, json!({}), BG).await.is_err());
        pf.call(&fake, None, &HOME, json!({}), BG).await.unwrap();
        let e = pf
            .call(&fake, None, &HOME, json!({}), BG)
            .await
            .unwrap_err();
        assert!(
            matches!(
                e,
                PathfinderError::RateLimited {
                    circuit_open: false,
                    ..
                }
            ),
            "ardışık değil: {e:?}"
        );
    }

    #[tokio::test]
    async fn request_body_and_headers_match_the_operation() {
        let pf = Pathfinder::new(fast());
        let seen = Arc::new(Mutex::new(Vec::<(String, Option<&'static str>)>::new()));
        struct Spy(Arc<Mutex<Vec<(String, Option<&'static str>)>>>);
        impl Transport for Spy {
            fn post<'a>(
                &'a self,
                r: &'a Request,
            ) -> BoxFuture<'a, Result<Response, TransportError>> {
                Box::pin(async move {
                    self.0
                        .lock()
                        .unwrap()
                        .push((r.body.clone(), r.accept_language));
                    Ok(ok_body())
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
                Box::pin(async { Err(DiscoverError::Missing("x".into())) })
            }
        }
        let spy = Spy(seen.clone());
        pf.call(
            &spy,
            None,
            &ARTIST_RELATED,
            json!({ "uri": "spotify:artist:a" }),
            BG,
        )
        .await
        .unwrap();
        let store = MemStore::with("search_hash", OLD);
        pf.call(
            &spy,
            Some(&store),
            &SEARCH_DESKTOP,
            json!({ "searchTerm": "q" }),
            UI,
        )
        .await
        .unwrap();
        let seen = seen.lock().unwrap();
        let body: Value = serde_json::from_str(&seen[0].0).unwrap();
        assert_eq!(body["operationName"], "queryArtistRelated");
        assert_eq!(body["variables"]["uri"], "spotify:artist:a");
        assert_eq!(body["extensions"]["persistedQuery"]["version"], 1);
        assert_eq!(
            body["extensions"]["persistedQuery"]["sha256Hash"],
            ARTIST_RELATED.seed.unwrap()
        );
        // Birim testlerde arayüz dili varsayılan Türkçedir (`i18n::lang`).
        assert_eq!(seen[0].1, Some("tr"));
        assert_eq!(seen[1].1, None, "arama accept-language göndermez");
        let body: Value = serde_json::from_str(&seen[1].0).unwrap();
        assert_eq!(body["operationName"], "searchDesktop");
        assert_eq!(body["extensions"]["persistedQuery"]["sha256Hash"], OLD);
    }

    #[test]
    fn accept_language_follows_the_interface_language() {
        use crate::i18n::{Lang, with_lang};
        // Dile bağlı işlemler arayüz dilini taşır, arama hiçbir şey göndermez.
        for op in [&HOME, &BROWSE_ALL, &BROWSE_PAGE, &ARTIST_RELATED] {
            assert!(op.localized, "{} dile bağlı olmalı", op.name);
            assert_eq!(language_header(op, Lang::En), Some("en"));
            assert_eq!(language_header(op, Lang::Tr), Some("tr"));
        }
        assert_eq!(language_header(&SEARCH_DESKTOP, Lang::En), None);
        // İstek kurulurken etkin dil okunur: dil değişince sonraki istek değişir.
        let en = with_lang(Lang::En, || {
            request_for(&HOME, "h", &json!({})).accept_language
        });
        let tr = with_lang(Lang::Tr, || {
            request_for(&HOME, "h", &json!({})).accept_language
        });
        assert_eq!((en, tr), (Some("en"), Some("tr")));
        assert_eq!(
            with_lang(Lang::En, || request_for(&SEARCH_DESKTOP, "h", &json!({}))
                .accept_language),
            None
        );
    }
}
