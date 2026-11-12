//! Spotify girişi ve kalıcı oturum.
//!
//! İlk girişte tarayıcıda Spotify onayı istenir. Ardından librespot'un verdiği
//! yeniden kullanılabilir oturum anahtarı Windows Credential Manager'a yazılır.
//! Sonraki açılışlarda tarayıcı açılmadan bu anahtarla bağlanılır.
//!
//! Tarayıcı YALNIZ kullanıcının isteğiyle açılır (`login`): kayıtlı anahtar
//! yok ya da reddedildiyse `connect_saved` hata döndürür, kendiliğinden
//! giriş açmaz (tepsiye gizliyken tarayıcı beliriyordu). Tarayıcı bekleyişi
//! iptal edilebilir ve zaman aşımı vardır (`LOGIN_TIMEOUT`).

use std::{
    fmt,
    io::Write,
    net::{SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use keyring::Entry;
use librespot::{
    core::{
        Error as SpotifyError, authentication::Credentials, cache::Cache, config::SessionConfig,
        error::ErrorKind, session::Session,
    },
    oauth::{OAuthClientBuilder, OAuthError},
    protocol::authentication::AuthenticationType,
};
use tokio::sync::{Notify, oneshot};

// Ses oturumu için Spotify'ın kendi masaüstü Client ID'si ve ona kayıtlı redirect URI.
//
// Neden kendi Client ID'miz değil: librespot Windows'ta Spotify'ın iç servislerine
// (şarkı bilgisi, ses dosyası) oturumun Client ID'siyle gidiyor. Bu servisler
// geliştirici uygulamalarının ID'lerini reddediyor (400 Bad Request).
// Bu yüzden uygulamada ayrı bir geliştirici Client ID'si, `.env` dosyası ya da
// ortam değişkeni yok; derlemek için hiçbir anahtar gerekmez.
const SPOTIFY_DESKTOP_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
const SPOTIFY_DESKTOP_REDIRECT_URI: &str = "http://127.0.0.1:8898/login";
/// Yukarıdaki yönlendirme adresinin yerel dinleyicisi (iptalde serbest bırakılır).
const LOGIN_ADDR: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8898);

/// Tarayıcıda girişin tamamlanması için beklenen en uzun süre.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
/// Tarayıcıdan jeton gelince oturumun açılması için en uzun süre.
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);
/// Bağlanınca hesap türünün (ürün bilgisi paketi) beklenen en uzun süresi;
/// gelmezse "bilinmiyor" sayılıp devam edilir, motor sonradan da bakar.
const ACCOUNT_WAIT: Duration = Duration::from_millis(2500);

/// Ses çalmak için gereken izin.
const SCOPES: &[&str] = &["streaming"];

/// Credential Manager'daki kaydın adı. Windows'ta "Kimlik Bilgileri Yöneticisi →
/// Windows Kimlik Bilgileri" altında görünür.
const KEYRING_SERVICE: &str = "Openify";
const KEYRING_USER: &str = "spotify-session";

/// Tarayıcıda giriş bitince gösterilen sayfa (o anki dilde; yazı `i18n` tablosunda).
fn login_done_page() -> String {
    format!(
        r#"<!doctype html>
<html lang="{}"><head><meta charset="utf-8"><title>{}</title></head><body style="font-family:system-ui;background:#111;color:#eee;display:grid;place-items:center;height:100vh;margin:0">
<h1>{}</h1>
</body></html>"#,
        if crate::i18n::lang() == crate::i18n::Lang::Tr { "tr" } else { "en" },
        crate::brand::APP_NAME,
        crate::t!(LoginDonePage),
    )
}

// ── Hesap türü ──────────────────────────────────────────────────────────

/// Spotify hesabının türü: librespot yalnız Premium'la ses akıtabilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    Premium,
    /// Ücretsiz (`free` / `open`).
    Free,
    /// Ürün bilgisi henüz gelmedi (ya da tanınmayan boş değer).
    Unknown,
}

/// Oturumun kullanıcı özniteliklerinden hesap türü. Önce `type`, yoksa
/// `product`: `free` ve `open` ücretsiz, başka dolu değer (premium, family,
/// duo, student…) Premium sayılır (bilmediğimiz bir plan adı yüzünden
/// ödeme yapan kullanıcıyı engellemeyelim), öznitelik yoksa bilinmiyor.
pub fn account_kind_with(get: impl Fn(&str) -> Option<String>) -> AccountKind {
    for key in ["type", "product"] {
        let Some(value) = get(key) else { continue };
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty() {
            continue;
        }
        return match value.as_str() {
            "free" | "open" => AccountKind::Free,
            _ => AccountKind::Premium,
        };
    }
    AccountKind::Unknown
}

#[cfg(test)]
pub fn account_kind(attributes: &std::collections::HashMap<String, String>) -> AccountKind {
    account_kind_with(|key| attributes.get(key).cloned())
}

/// Bağlı oturumun şu anki hesap türü.
pub fn session_account_kind(session: &Session) -> AccountKind {
    account_kind_with(|key| session.get_user_attribute(key))
}

/// Hesap türü gelene dek (en çok `total`) bekler; gelmezse `Unknown`.
pub async fn wait_account_kind_with(
    get: impl Fn(&str) -> Option<String>,
    total: Duration,
    step: Duration,
) -> AccountKind {
    let started = Instant::now();
    loop {
        let kind = account_kind_with(&get);
        if kind != AccountKind::Unknown || started.elapsed() >= total {
            return kind;
        }
        tokio::time::sleep(step).await;
    }
}

async fn wait_account_kind(session: &Session) -> AccountKind {
    wait_account_kind_with(|key| session.get_user_attribute(key), ACCOUNT_WAIT, Duration::from_millis(50)).await
}

// ── Hatalar ─────────────────────────────────────────────────────────────

/// Kayıtlı anahtarla bağlanma sonucu (bkz. `connect_saved`).
#[derive(Debug)]
pub enum ConnectError {
    /// Kayıtlı giriş yok (ya da Credential Manager okunamadı / kayıt bozuk).
    NoCredentials,
    /// Spotify kayıtlı anahtarı reddetti (kayıt silindi): yeniden giriş gerekir.
    Rejected,
    /// Hesap ücretsiz: librespot ses akıtamaz.
    NeedsPremium,
    /// Ağ ya da sunucu sorunu: anahtar silinmedi, yeniden denenebilir.
    Offline(anyhow::Error),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCredentials => write!(f, "Kayıtlı Spotify girişi yok"),
            Self::Rejected => write!(f, "Kayıtlı Spotify girişi geçersiz"),
            Self::NeedsPremium => write!(f, "Hesap Spotify Premium değil"),
            Self::Offline(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// Tarayıcı girişinin sonucu (hata hâli).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    /// Kullanıcı vazgeçti.
    Cancelled,
    /// `LOGIN_TIMEOUT` içinde tamamlanmadı.
    TimedOut,
    /// Yerel yönlendirme portu (8898) başka bir uygulamada açık.
    PortBusy,
    /// Jeton alındı ama Spotify'a ulaşılamadı.
    Network,
    /// Hesap ücretsiz.
    NeedsPremium,
    /// Başka bir hata (teknik ayrıntı yalnız günlüğe).
    Failed(String),
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "giriş iptal edildi"),
            Self::TimedOut => write!(f, "giriş zaman aşımına uğradı"),
            Self::PortBusy => write!(f, "yerel giriş portu ({}) kullanımda", LOGIN_ADDR.port()),
            Self::Network => write!(f, "Spotify'a ulaşılamadı"),
            Self::NeedsPremium => write!(f, "hesap Spotify Premium değil"),
            Self::Failed(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoginError {}

/// Spotify'ın bağlanma hatası neyi söylüyor?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// "Premium account required".
    PremiumRequired,
    /// Anahtar / giriş reddedildi.
    Rejected,
    /// Reddedilmedi (ağ vb.).
    Other,
}

fn refusal(e: &SpotifyError) -> Refusal {
    if e.kind != ErrorKind::PermissionDenied {
        return Refusal::Other;
    }
    if e.to_string().to_ascii_lowercase().contains("premium account required") {
        Refusal::PremiumRequired
    } else {
        Refusal::Rejected
    }
}

// ── Bağlanma ────────────────────────────────────────────────────────────

/// Kayıtlı anahtarla bağlanır. ASLA tarayıcı açmaz: anahtar yoksa
/// `NoCredentials`, Spotify reddederse (hesaptan çıkış yapılmış, şifre
/// değişmiş) kaydı silip `Rejected` döner; arayüz "Giriş yap" gösterir.
/// Ağ hatası gibi başka bir sorunda anahtar silinmez (`Offline`).
/// `cache` verilirse çalınan şarkıların ses dosyaları oraya kaydedilir.
pub async fn connect_saved(cache: Option<Cache>) -> Result<Session, ConnectError> {
    connect_saved_with(cache, None, true).await
}

/// `connect_saved`in ayrıntılı hâli. `device_id`: verilirse oturum o KALICI
/// aygıt kimliğiyle açılır (Connect: telefonun listesinde hep aynı aygıt;
/// verilmezse librespot her oturumda rastgele üretir). `forget_rejected`:
/// Spotify kayıtlı anahtarı reddedince kaydı sil (uygulama böyle yapar; denemeler
/// kullanıcının kaydına dokunmaz).
pub async fn connect_saved_with(
    cache: Option<Cache>,
    device_id: Option<&str>,
    forget_rejected: bool,
) -> Result<Session, ConnectError> {
    let credentials = match load() {
        Ok(Some(credentials)) => credentials,
        Ok(None) => return Err(ConnectError::NoCredentials),
        Err(e) => {
            log::warn!("Kayıtlı giriş okunamadı, giriş istenecek: {e:#}");
            return Err(ConnectError::NoCredentials);
        }
    };
    let session = new_session(cache, device_id);
    match session.connect(credentials, false).await {
        Ok(()) => {}
        Err(e) => {
            return Err(match refusal(&e) {
                Refusal::PremiumRequired => ConnectError::NeedsPremium,
                Refusal::Rejected => {
                    log::info!("Kayıtlı giriş artık geçerli değil, yeniden giriş gerekiyor ({e})");
                    if forget_rejected && let Err(e) = clear() {
                        log::warn!("Geçersiz kayıtlı giriş silinemedi: {e:#}");
                    }
                    ConnectError::Rejected
                }
                Refusal::Other => ConnectError::Offline(anyhow::Error::new(e).context("Spotify'a bağlanılamadı")),
            });
        }
    }
    match wait_account_kind(&session).await {
        AccountKind::Free => {
            session.shutdown();
            Err(ConnectError::NeedsPremium)
        }
        _ => Ok(session),
    }
}

/// Tarayıcıda Spotify girişi (yalnız kullanıcı isteğiyle): tarayıcıyı açar,
/// onayı bekler, oturumu açar ve anahtarı saklar. `cancel.notify_one()`
/// bekleyişi bitirir (`LoginError::Cancelled`); `LOGIN_TIMEOUT` dolarsa
/// `TimedOut`. İkisinde de yerel dinleyici serbest bırakılır.
pub async fn login(cache: Option<Cache>, cancel: Arc<Notify>) -> Result<Session, LoginError> {
    let token = browser_token(&cancel, LOGIN_TIMEOUT).await?;
    let session = new_session(cache, None);
    match tokio::time::timeout(SESSION_TIMEOUT, session.connect(Credentials::with_access_token(&token), false)).await {
        Err(_) => {
            log::warn!("Spotify oturumu {} sn'de açılamadı", SESSION_TIMEOUT.as_secs());
            return Err(LoginError::Network);
        }
        Ok(Err(e)) => {
            log::warn!("Spotify oturumu açılamadı: {e}");
            return Err(match refusal(&e) {
                Refusal::PremiumRequired => LoginError::NeedsPremium,
                Refusal::Rejected => LoginError::Failed(e.to_string()),
                Refusal::Other => LoginError::Network,
            });
        }
        Ok(Ok(())) => {}
    }

    // Bağlantı kurulunca librespot yeniden kullanılabilir bir anahtar verir;
    // bir sonraki açılış için sakla (ücretsiz hesapta da: bir sonraki açılış
    // yine bağlanıp "Premium gerekli"yi gösterir). Saklanamazsa çalmaya engel değil.
    let reusable = Credentials {
        username: Some(session.username()),
        auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
        auth_data: session.auth_data(),
    };
    if let Err(e) = save(&reusable) {
        log::warn!("Giriş kaydedilemedi, bir dahaki açılışta yeniden sorulacak ({e:#})");
    }
    match wait_account_kind(&session).await {
        AccountKind::Free => {
            session.shutdown();
            Err(LoginError::NeedsPremium)
        }
        _ => Ok(session),
    }
}

/// Terminal komutu (`openify <bağlantı>`) için: kayıtlı giriş yoksa ya da
/// reddedildiyse tarayıcıda girer (kullanıcı komutu kendisi çalıştırdı).
pub async fn connect(cache: Option<Cache>) -> Result<Session> {
    match connect_saved(cache.clone()).await {
        Ok(session) => Ok(session),
        Err(ConnectError::NoCredentials | ConnectError::Rejected) => {
            println!("Tarayıcıda Spotify girişi açılıyor...");
            login(cache, Arc::new(Notify::new())).await.map_err(anyhow::Error::new)
        }
        Err(ConnectError::NeedsPremium) => anyhow::bail!("Bu hesap Spotify Premium değil; Openify yalnız Premium'la çalar"),
        Err(ConnectError::Offline(e)) => Err(e),
    }
}

/// Kayıtlı giriş var mı?
#[allow(dead_code)]
pub fn has_saved() -> bool {
    matches!(load(), Ok(Some(_)))
}

/// Kayıtlı girişi siler. Kayıt yoksa sessizce başarılı olur.
pub fn clear() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("Kayıtlı giriş silinemedi"),
    }
}

fn new_session(cache: Option<Cache>, device_id: Option<&str>) -> Session {
    // Varsayılan ayar zaten SPOTIFY_DESKTOP_CLIENT_ID'yi kullanıyor.
    let mut config = SessionConfig::default();
    if let Some(id) = device_id {
        config.device_id = id.to_string();
    }
    Session::new(config, cache)
}

// ── Tarayıcı bekleyişi (iptal edilebilir, zaman aşımlı) ─────────────────

/// librespot'un OAuth dinleyicisi engelleyen bir `accept` ile bekler ve
/// durdurulamaz; bu yüzden ayrı iş parçacığında çalışır, motor tarafı
/// `wait_login` ile iptal / zaman aşımını izler ve dinleyiciyi kendi
/// bağlantısıyla uyandırıp (`release_listener`) iş parçacığının bitmesini sağlar.
async fn browser_token(cancel: &Notify, timeout: Duration) -> Result<String, LoginError> {
    log::info!("Tarayıcıda Spotify girişi açılıyor...");
    let oauth = OAuthClientBuilder::new(SPOTIFY_DESKTOP_CLIENT_ID, SPOTIFY_DESKTOP_REDIRECT_URI, SCOPES.to_vec())
        .open_in_browser()
        .with_custom_message(&login_done_page())
        .build()
        .map_err(|e| LoginError::Failed(e.to_string()))?;
    let (tx, rx) = oneshot::channel();
    let done = Arc::new(AtomicBool::new(false));
    let thread_done = done.clone();
    std::thread::Builder::new()
        .name("openify-login".into())
        .spawn(move || {
            let result = oauth.get_access_token().map(|t| t.access_token).map_err(|e| match e {
                OAuthError::AuthCodeListenerBind { .. } => LoginError::PortBusy,
                other => LoginError::Failed(other.to_string()),
            });
            thread_done.store(true, Ordering::SeqCst);
            let _ = tx.send(result);
        })
        .map_err(|e| LoginError::Failed(e.to_string()))?;
    wait_login(rx, cancel, timeout, move || release_listener(LOGIN_ADDR, done)).await
}

/// Giriş sonucunu, iptali ve zaman aşımını bekler. İptal ya da zaman
/// aşımında `on_abort` çağrılır (dinleyiciyi serbest bırakmak için).
pub async fn wait_login<T>(
    result: oneshot::Receiver<Result<T, LoginError>>,
    cancel: &Notify,
    timeout: Duration,
    on_abort: impl FnOnce(),
) -> Result<T, LoginError> {
    tokio::select! {
        r = result => match r {
            Ok(r) => r,
            Err(_) => Err(LoginError::Failed("giriş iş parçacığı beklenmedik biçimde bitti".into())),
        },
        _ = cancel.notified() => {
            on_abort();
            Err(LoginError::Cancelled)
        }
        _ = tokio::time::sleep(timeout) => {
            on_abort();
            Err(LoginError::TimedOut)
        }
    }
}

/// Engelleyen dinleyiciyi uyandırır: `addr`'e boş bir istek gönderir, dinleyici
/// "adres kodu yok" hatasıyla biter ve port serbest kalır. Dinleyici henüz
/// bağlanmamış olabilir (iptal çok erken geldi): `done` (iş parçacığı bitti)
/// oluncaya ya da ~5 sn dolana dek 200 ms'de bir yeniden dener.
pub fn release_listener(addr: SocketAddr, done: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new().name("openify-login-release".into()).spawn(move || {
        for _ in 0..25 {
            if done.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
                let _ = stream.write_all(b"GET /iptal HTTP/1.1\r\n\r\n");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

// ── Credential Manager ──────────────────────────────────────────────────

fn entry() -> Result<Entry> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER).context("Windows Credential Manager'a erişilemedi")
}

fn load() -> Result<Option<Credentials>> {
    let json = match entry()?.get_password() {
        Ok(json) => json,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(e).context("Kayıtlı giriş okunamadı"),
    };
    match serde_json::from_str(&json) {
        Ok(credentials) => Ok(Some(credentials)),
        // Bozuk kayıt: sil ve yeniden giriş iste.
        Err(_) => {
            clear()?;
            Ok(None)
        }
    }
}

fn save(credentials: &Credentials) -> Result<()> {
    let json = serde_json::to_string(credentials)?;
    entry()?.set_password(&json).context("Giriş Credential Manager'a yazılamadı")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, net::TcpListener};

    /// Gerçek Credential Manager'a geçici bir kayıt yazar, okur ve siler.
    /// Çalıştırmak için: cargo test -- --ignored
    #[test]
    #[ignore]
    fn credential_manager_roundtrip() {
        let entry = Entry::new("Openify-test", "roundtrip").unwrap();
        let creds = Credentials {
            username: Some("test".into()),
            auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
            auth_data: vec![1, 2, 3, 250],
        };
        entry.set_password(&serde_json::to_string(&creds).unwrap()).unwrap();
        let back: Credentials = serde_json::from_str(&entry.get_password().unwrap()).unwrap();
        entry.delete_credential().unwrap();
        assert_eq!(back, creds);
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }

    fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn account_kind_maps_the_type_attribute() {
        assert_eq!(account_kind(&attrs(&[("type", "premium")])), AccountKind::Premium);
        assert_eq!(account_kind(&attrs(&[("type", "free")])), AccountKind::Free);
        assert_eq!(account_kind(&attrs(&[("type", "open")])), AccountKind::Free);
        assert_eq!(account_kind(&attrs(&[("type", " FREE ")])), AccountKind::Free, "boşluk ve büyük harf");
        // Bilmediğimiz plan adı Premium sayılır: ödeme yapanı engelleme.
        assert_eq!(account_kind(&attrs(&[("type", "family")])), AccountKind::Premium);
    }

    #[test]
    fn account_kind_falls_back_to_product_and_unknown() {
        assert_eq!(account_kind(&attrs(&[("product", "free")])), AccountKind::Free);
        assert_eq!(account_kind(&attrs(&[("product", "premium")])), AccountKind::Premium);
        // `type` varsa `product`'a bakılmaz.
        assert_eq!(account_kind(&attrs(&[("type", "premium"), ("product", "free")])), AccountKind::Premium);
        assert_eq!(account_kind(&attrs(&[("type", ""), ("product", "open")])), AccountKind::Free, "boş değer atlanır");
        assert_eq!(account_kind(&attrs(&[])), AccountKind::Unknown);
        assert_eq!(account_kind(&attrs(&[("country", "TR"), ("autoplay", "1")])), AccountKind::Unknown);
        assert_eq!(account_kind(&attrs(&[("type", "  ")])), AccountKind::Unknown);
    }

    #[tokio::test]
    async fn waiting_for_the_account_kind_returns_when_it_arrives_or_times_out() {
        let seen = std::sync::Mutex::new(0u32);
        // 3. okumada ürün bilgisi "gelir".
        let kind = wait_account_kind_with(
            |key| {
                let mut n = seen.lock().unwrap();
                *n += 1;
                (key == "type" && *n >= 3).then(|| "free".to_string())
            },
            Duration::from_secs(2),
            Duration::from_millis(5),
        )
        .await;
        assert_eq!(kind, AccountKind::Free);
        // Hiç gelmezse süre dolunca "bilinmiyor".
        let started = Instant::now();
        let kind = wait_account_kind_with(|_| None, Duration::from_millis(60), Duration::from_millis(10)).await;
        assert_eq!(kind, AccountKind::Unknown);
        assert!(started.elapsed() >= Duration::from_millis(60) && started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn refusal_tells_premium_required_from_bad_credentials() {
        let premium = SpotifyError::permission_denied(anyhow::anyhow!("Login failed with reason: Premium account required"));
        let bad = SpotifyError::permission_denied(anyhow::anyhow!("Login failed with reason: Bad credentials"));
        let network = SpotifyError::unavailable(anyhow::anyhow!("transport returned no data"));
        assert_eq!(refusal(&premium), Refusal::PremiumRequired);
        assert_eq!(refusal(&bad), Refusal::Rejected);
        assert_eq!(refusal(&network), Refusal::Other);
    }

    #[test]
    fn login_listener_matches_the_redirect_uri() {
        assert!(SPOTIFY_DESKTOP_REDIRECT_URI.contains(&format!("127.0.0.1:{}", LOGIN_ADDR.port())));
    }

    #[tokio::test]
    async fn wait_login_passes_the_result_through() {
        let (tx, rx) = oneshot::channel();
        tx.send(Ok::<_, LoginError>("jeton".to_string())).unwrap();
        let aborted = AtomicBool::new(false);
        let got = wait_login(rx, &Notify::new(), Duration::from_secs(5), || aborted.store(true, Ordering::SeqCst)).await;
        assert_eq!(got, Ok("jeton".to_string()));
        assert!(!aborted.load(Ordering::SeqCst), "başarıda dinleyiciye dokunulmaz");

        let (tx, rx) = oneshot::channel();
        tx.send(Err::<String, _>(LoginError::PortBusy)).unwrap();
        assert_eq!(
            wait_login(rx, &Notify::new(), Duration::from_secs(5), || {}).await,
            Err(LoginError::PortBusy)
        );
        // İş parçacığı sonuç vermeden öldü.
        let (tx, rx) = oneshot::channel::<Result<String, LoginError>>();
        drop(tx);
        assert!(matches!(wait_login(rx, &Notify::new(), Duration::from_secs(5), || {}).await, Err(LoginError::Failed(_))));
    }

    #[tokio::test]
    async fn wait_login_can_be_cancelled_and_releases_the_listener() {
        let (_tx, rx) = oneshot::channel::<Result<String, LoginError>>();
        let cancel = Arc::new(Notify::new());
        let aborted = Arc::new(AtomicBool::new(false));
        let (c, a) = (cancel.clone(), aborted.clone());
        let waiter = tokio::spawn(async move {
            wait_login(rx, &c, Duration::from_secs(30), move || a.store(true, Ordering::SeqCst)).await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.notify_one();
        assert_eq!(waiter.await.unwrap(), Err(LoginError::Cancelled));
        assert!(aborted.load(Ordering::SeqCst));
        // İptal, bekleyen bir dinleyici yokken de (izin saklanır) hemen geçer.
        let (_tx2, rx2) = oneshot::channel::<Result<String, LoginError>>();
        let cancel = Notify::new();
        cancel.notify_one();
        assert_eq!(wait_login(rx2, &cancel, Duration::from_secs(30), || {}).await, Err(LoginError::Cancelled));
    }

    #[tokio::test]
    async fn wait_login_times_out() {
        let (_tx, rx) = oneshot::channel::<Result<String, LoginError>>();
        let aborted = AtomicBool::new(false);
        let started = Instant::now();
        let got = wait_login(rx, &Notify::new(), Duration::from_millis(60), || aborted.store(true, Ordering::SeqCst)).await;
        assert_eq!(got, Err(LoginError::TimedOut));
        assert!(aborted.load(Ordering::SeqCst));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Engelleyen bir dinleyici (librespot'unkinin aynısı: `incoming().next()`),
    /// `release_listener` ile bitirilir; port yerel ve rastgele, Spotify yok.
    #[test]
    fn release_listener_unblocks_a_waiting_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let thread_done = done.clone();
        let worker = std::thread::spawn(move || {
            let accepted = listener.incoming().flatten().next().is_some();
            thread_done.store(true, Ordering::SeqCst);
            accepted
        });
        release_listener(addr, done.clone());
        let started = Instant::now();
        while !done.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done.load(Ordering::SeqCst), "dinleyici 5 sn içinde uyanmalı");
        assert!(worker.join().unwrap());
    }

    /// İptal, dinleyici henüz bağlanmadan gelirse: yeniden denemeler sonradan
    /// bağlanan dinleyiciyi de uyandırır.
    #[test]
    fn release_listener_retries_until_the_listener_appears() {
        // Boş bir port bul, bırak; dinleyici biraz sonra aynı portta açılır.
        let addr = {
            let probe = TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap()
        };
        let done = Arc::new(AtomicBool::new(false));
        release_listener(addr, done.clone());
        std::thread::sleep(Duration::from_millis(450));
        let thread_done = done.clone();
        let worker = std::thread::spawn(move || {
            let Ok(listener) = TcpListener::bind(addr) else {
                // Port bu arada başkasına geçtiyse sınama anlamsız: geç.
                thread_done.store(true, Ordering::SeqCst);
                return false;
            };
            let accepted = listener.incoming().flatten().next().is_some();
            thread_done.store(true, Ordering::SeqCst);
            accepted
        });
        let started = Instant::now();
        while !done.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(done.load(Ordering::SeqCst));
        let _ = worker.join();
    }
}
