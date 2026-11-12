//! Paketleme ve otomatik güncelleme (Adım 16, Velopack).
//!
//! Arayüzden bağımsız: motor ya da gpui'yi bilmez. Arayüz yalnız `start`'a bir
//! geri çağrı verir; güncelleme indirilince geri çağrı `Event::Ready` ile
//! çağrılır ("Güncelleme hazır, yeniden açınca uygulanacak").
//!
//! Akış:
//! 1. `main()` en başta `hooks()` çalıştırır: Velopack'ın kurulum / güncelleme /
//!    kaldırma çağrıları (`--veloapp-*`) burada işlenir ve süreç çıkar. Bu,
//!    tek kopya kilidinden ve pencereden önce olur, kilitle çakışmaz.
//! 2. Tek kopya kilidi alındıktan sonra (`apply_pending`): önceki oturumda
//!    indirilmiş bir güncelleme varsa uygulanır, uygulama yeni sürümle yeniden
//!    açılır. Kilidi almamış bir ikinci açılış asla uygulamaz (çalışan ilk
//!    kopyanın dosyalarını bozmasın). Velopack'ın kendi "açılışta uygula"
//!    davranışı bu yüzden kapalı.
//! 3. Pencere açıldıktan bir süre sonra (`start`): arka plan iş parçacığında
//!    kaynağa bakılır, yeni sürüm varsa indirilir; uygulanmaz, çalma kesilmez.
//!
//! 4. Bozuk güncelleme geri alınır (`health.rs`): her açılış, sürüm "sağlıklı"
//!    sayılana dek (pencere açık + 20 sn çökmeden) bir deneme olarak
//!    `<önbellek>\update-health.json`'a yazılır. Sağlıklı olmadan 3. açılışa
//!    gelinirse (2 başarısız deneme) `startup` en son sağlıklı sürüme döner:
//!    önce güncellemeden önce yedeklenen paketten (`<önbellek>\rollback`,
//!    ağ gerekmez), yoksa kaynaktan (arka planda). Geri alınan sürüm kara
//!    listeye girer; daha yeni bir sürüm çıkana kadar yeniden indirilmez.
//!
//! Kurulu olmayan çalıştırmada (cargo run, sınama kopyaları) Velopack bir
//! kurulum bulamaz; bu modülün her parçası sessizce hiçbir şey yapmaz.
//!
//! Kaynak: `OPENIFY_UPDATE_URL` (düz bir HTTP klasörü: `releases.win.json` ve
//! paketler orada durur). `off` verilirse güncelleme denetimi kapalıdır.

mod health;

use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::t;
use velopack::{
    UpdateCheck, UpdateInfo, UpdateManager, VelopackApp, VelopackAsset, sources::HttpSource,
};

/// Varsayılan kaynak: kod deposunun kendi Releases sayfasındaki son sürüm
/// (ayrı bir sürüm deposu yok).
pub const DEFAULT_FEED: &str = "https://github.com/FurkanKY/Openify/releases/latest/download";

/// Açılıştan ilk denetime kadar beklenen süre (açılışı ve ilk kareyi hiç
/// etkilemesin, ağ çalışması çalmayla yarışmasın).
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
/// Açık kalan uygulama bu aralıkla yeniden bakar.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Arayüze bildirilen olaylar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Yeni sürüm indirildi, yeniden açılınca uygulanacak.
    Ready { version: String },
}

impl Event {
    /// Arayüzde gösterilecek kısa bildirim.
    pub fn notice(&self) -> String {
        match self {
            Event::Ready { .. } => t!(UpdateReadyNotice).into(),
        }
    }
}

/// Güncelleme kaynağı: `OPENIFY_UPDATE_URL`, yoksa varsayılan. `None` = kapalı.
pub fn feed_url() -> Option<String> {
    match std::env::var("OPENIFY_UPDATE_URL") {
        Ok(v) if v.trim().eq_ignore_ascii_case("off") => None,
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => Some(DEFAULT_FEED.to_string()),
    }
}

/// Kimlik: Windows'taki uygulama kimliği (AUMID). Velopack'a da bu verilir ki
/// süreç tek bir kimlikle çalışsın (bkz. `system::set_app_id`).
#[cfg(windows)]
fn aumid() -> &'static str {
    crate::system::app_id()
}

/// `main()`'in ilk işi. Velopack kancalarını işler (gerekirse çıkar).
pub fn hooks() {
    let mut app = VelopackApp::build().set_auto_apply_on_startup(false);
    #[cfg(windows)]
    {
        app = app
            .set_app_user_model_id(aumid())
            .on_before_uninstall_fast_callback(|_| remove_cache());
    }
    app.run();
}

/// Kaldırırken önbelleği sil: `%LOCALAPPDATA%\Openify` (şarkı önbelleği,
/// kapaklar, SQLite, kayıt). Kullanıcı verisine (`%APPDATA%\Openify\settings.json`,
/// Kimlik Bilgisi Yöneticisi'ndeki giriş) dokunulmaz.
///
/// Paket kimliği "Openify" ise kurulum klasörü de bu klasördür; Velopack'ın
/// kendi dosyaları (`current`, `packages`, `Update.exe`…) kaldırmayı yapan
/// Update.exe tarafından silinir, burada atlanır. Kimlik başkaysa (sınama
/// paketi "Openify.Test") hiçbir şey silinmez: kullanıcının gerçek önbelleği
/// sınama kaldırmasından etkilenmesin.
#[cfg(windows)]
fn remove_cache() {
    let Some(cache) = dirs::cache_dir().map(|d| d.join("Openify")) else {
        return;
    };
    let Some(root) = install_root() else { return };
    if root != cache {
        return;
    }
    const VELOPACK: [&str; 5] = ["current", "packages", "Update.exe", "sq.version", ".betaId"];
    let Ok(entries) = std::fs::read_dir(&cache) else {
        return;
    };
    for entry in entries.flatten() {
        if VELOPACK
            .iter()
            .any(|n| entry.file_name().eq_ignore_ascii_case(n))
        {
            continue;
        }
        let path = entry.path();
        let _ = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}

/// Kurulum klasörü: `<kök>\current\openify.exe` düzeninde exe'nin iki üstü.
fn install_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let current = exe.parent()?;
    if !current.file_name()?.eq_ignore_ascii_case("current") {
        return None;
    }
    let root = current.parent()?;
    root.join("Update.exe").exists().then(|| root.to_path_buf())
}

/// Kurulu sürümün (Velopack bildirimindeki) sürümü; yalnız kurulu çalışmada dolar.
static CURRENT: OnceLock<String> = OnceLock::new();
/// Geri dönüş sürüyor: güncelleme denetimi bu sırada bozuk sürümü yeniden indirmesin.
static ROLLING_BACK: AtomicBool = AtomicBool::new(false);
/// Geri alınmış bozuk sürüm (yalnız Ayarlar → Hakkında'daki metin için).
static ROLLED_BACK: Mutex<Option<String>> = Mutex::new(None);

/// Tek kopya kilidini almış (ya da sınama koşusu) süreçte, açılışta çağrılır:
/// 1. açılışı sağlık kaydına yazar (bkz. `health`);
/// 2. indirilmiş bir güncelleme varsa uygular ve yeniden açar (süreç çıkar);
/// 3. sürüm art arda sağlıklı olamadıysa en son sağlıklı sürüme döner.
///    Kurulu değilse hemen döner, hiçbir dosyaya dokunmaz.
pub fn startup() {
    let Ok(manager) = UpdateManager::new(HttpSource::new(""), None, None) else {
        return;
    };
    let current = manager.get_current_version().to_string();
    let _ = CURRENT.set(current.clone());
    let action = register_start(&current);
    // Bekleyen daha yeni (ve kara listede olmayan) bir sürüm varsa onu uygulamak,
    // geri dönmekten iyidir: düzeltme zaten inmiş olabilir.
    apply_pending(&manager);
    if let health::Action::Rollback { to, bad } = action {
        rollback(&manager, to, bad);
    }
}

/// Bekleyen güncellemeyi uygular (kara listedeyse uygulamaz).
fn apply_pending(manager: &UpdateManager) {
    let Some(pending) = manager.get_update_pending_restart() else {
        return;
    };
    if is_blocked(&pending.Version) {
        log::info!("güncelleme: {} kara listede, uygulanmıyor", pending.Version);
        return;
    }
    log::info!("güncelleme: {} uygulanıyor", pending.Version);
    if let Err(e) = manager.apply_updates_and_restart(&pending) {
        log::warn!("güncelleme: uygulanamadı: {e}");
    }
}

/// Bu açılışı kaydeder ve alınacak kararı döner.
fn register_start(current: &str) -> health::Action {
    let Some(path) = health::state_path() else {
        return health::Action::None;
    };
    let before = health::load_from(&path);
    let (after, action) = health::on_start(&before, current);
    if after != before
        && let Err(e) = health::save_to(&path, &after)
    {
        log::warn!("güncelleme: sağlık kaydı yazılamadı: {e}");
    }
    if let Some(bad) = after.blocked.as_deref().filter(|b| is_newer(b, current))
        && let Ok(mut r) = ROLLED_BACK.lock()
    {
        *r = Some(bad.to_string());
    }
    action
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

/// Bu sürüm kara listede mi (geri alınmış bozuk sürüm ya da altı)?
fn is_blocked(version: &str) -> bool {
    health::state_path().is_some_and(|p| health::is_blocked(&health::load_from(&p), version))
}

/// Durum dosyasını okuyup değiştirir; değiştiyse yazar.
fn update_state(change: impl FnOnce(&health::State, &str) -> health::State) {
    let (Some(current), Some(path)) = (CURRENT.get(), health::state_path()) else {
        return;
    };
    let before = health::load_from(&path);
    let after = change(&before, current);
    if after != before
        && let Err(e) = health::save_to(&path, &after)
    {
        log::warn!("güncelleme: sağlık kaydı yazılamadı: {e}");
    }
}

/// Sürüm sağlıklı sayıldı: pencere açık ve süreç `HEALTHY_AFTER` boyunca yaşadı.
fn mark_healthy() {
    update_state(health::on_healthy);
}

/// Uygulama düzgünce kapanıyor: sağlıklı sayılmadan kapanan bir açılış (hemen
/// kapatmak gibi) başarısız deneme sayılmasın. Kurulu değilse hiçbir şey yapmaz.
pub fn clean_exit() {
    update_state(health::on_clean_exit);
}

/// Pencere açıldı: `HEALTHY_AFTER` sonra sürümü sağlıklı işaretler.
fn arm_health() {
    if CURRENT.get().is_none() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("openify-update-health".into())
        .spawn(|| {
            std::thread::sleep(health::HEALTHY_AFTER);
            mark_healthy();
        });
}

/// Güncellemeden önce şimdiki (sağlıklı kanıtlı) sürümün tam paketini yedekler:
/// `download_updates` eski paketleri siler, geri dönüş için bir kopya kalsın.
/// Başarısızlık güncellemeyi engellemez.
fn backup_current_package() {
    let (Some(current), Some(path), Some(root), Some(dir)) = (
        CURRENT.get(),
        health::state_path(),
        install_root(),
        health::backup_dir(),
    ) else {
        return;
    };
    if !health::should_backup(&health::load_from(&path), current) {
        return;
    }
    match health::backup_package(&root.join("packages"), &dir, current) {
        Ok(true) => log::debug!("güncelleme: {current} paketi geri dönüş için yedeklendi"),
        Ok(false) => {
            log::info!("güncelleme: {current} paketi packages klasöründe yok, yedeklenemedi")
        }
        Err(e) => log::warn!("güncelleme: yedek alınamadı: {e}"),
    }
}

/// Sürüm sağlıklı olamadı: `to` sürümüne dön (`None` = kaynaktaki bir önceki).
/// Önce yerel yedek (ağsız, hemen); yoksa kaynaktan, arka planda. Hiçbiri
/// olmazsa sessizce vazgeçer; sonraki açılışta yeniden denenir.
fn rollback(manager: &UpdateManager, to: Option<String>, bad: String) {
    log::warn!("güncelleme: {bad} sağlıklı açılamadı, önceki sürüme dönülüyor");
    ROLLING_BACK.store(true, Ordering::SeqCst);
    if let (Some(good), Some(root), Some(dir)) =
        (to.as_deref(), install_root(), health::backup_dir())
        && let Some(backup) = health::find_package(&dir, good)
    {
        match health::stage_backup(&backup, &root.join("packages")) {
            Ok(file) => apply_rollback(manager, &file, good, &bad),
            Err(e) => log::warn!("güncelleme: yedek hazırlanamadı: {e}"),
        }
    }
    // Buraya gelindiyse yerel yoldan uygulanamadı (yedek yok / uygulama hata verdi).
    let Some(url) = feed_url() else {
        ROLLING_BACK.store(false, Ordering::SeqCst);
        return;
    };
    let current = manager.get_current_version().to_string();
    let spawned = std::thread::Builder::new()
        .name("openify-update-rollback".into())
        .spawn(move || {
            if let Err(e) = rollback_from_feed(&url, to.as_deref(), &bad, &current) {
                log::info!("güncelleme: kaynaktan geri dönüş olmadı: {e}");
            }
            ROLLING_BACK.store(false, Ordering::SeqCst);
        });
    if spawned.is_err() {
        ROLLING_BACK.store(false, Ordering::SeqCst);
    }
}

/// Kaynaktan, ağla: bilinen sağlıklı sürümün (ya da bir öncekinin) tam paketini
/// indirir ve uygular. Kaynakta o sürümün paketi durmuyorsa hata döner.
fn rollback_from_feed(url: &str, to: Option<&str>, bad: &str, current: &str) -> Result<(), String> {
    let manager =
        UpdateManager::new(HttpSource::new(url), None, None).map_err(|e| e.to_string())?;
    let feed = manager.get_release_feed().map_err(|e| e.to_string())?;
    let fulls: Vec<String> = feed
        .Assets
        .iter()
        .filter(|a| a.Type.eq_ignore_ascii_case("Full"))
        .map(|a| a.Version.clone())
        .collect();
    let target = health::pick_rollback_target(&fulls, to, current)
        .ok_or("no rollback target in the feed")?;
    let asset = feed
        .Assets
        .iter()
        .find(|a| a.Type.eq_ignore_ascii_case("Full") && a.Version == target)
        .ok_or("package missing from the feed")?;
    // Velopack'ın "eski sürüme iniş" bilgisi: yalnız tam paket, delta yok.
    let info = UpdateInfo {
        TargetFullRelease: asset.clone(),
        BaseRelease: None,
        DeltasToTarget: Vec::new(),
        IsDowngrade: true,
    };
    log::info!("güncelleme: {target} kaynaktan indiriliyor (geri dönüş)");
    // Eski paketleri (bozuk sürüm dahil) siler, yalnız bu kalır.
    manager
        .download_updates(&info, None)
        .map_err(|e| e.to_string())?;
    apply_rollback(&manager, &asset.FileName, &target, bad);
    Err("rollback was not applied".into())
}

/// Hazırlanmış (packages klasöründe duran) paketi uygular ve yeniden açar:
/// süreç çıkar. Dönerse uygulama başarısız olmuştur. Kara liste ve sayaç
/// yalnız uygulamadan hemen önce yazılır; başarısızsa eski hâline döner.
fn apply_rollback(manager: &UpdateManager, file: &str, good: &str, bad: &str) {
    let path = health::state_path();
    let before = path.as_deref().map(health::load_from);
    if let (Some(path), Some(before)) = (&path, &before) {
        let mut next = before.clone();
        next.blocked = Some(bad.to_string());
        next.trial_version = None;
        next.trial_starts = 0;
        let _ = health::save_to(path, &next);
    }
    let asset = VelopackAsset {
        FileName: file.to_string(),
        Version: good.to_string(),
        Type: "Full".into(),
        ..Default::default()
    };
    // Eski sürüme iniş de normal güncellemeyle aynı çağrıdır (bkz. Velopack
    // `check_for_updates` + `apply_updates_and_restart`).
    if let Err(e) = manager.apply_updates_and_restart(&asset) {
        log::warn!("güncelleme: geri dönüş uygulanamadı: {e}");
        if let (Some(path), Some(before)) = (&path, &before) {
            let _ = health::save_to(path, before);
        }
    }
}

/// Bu sürüm (Ayarlar → Hakkında).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Güncellemenin şimdiki durumu (Ayarlar → Hakkında'da gösterilir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Bu kopya kurulu değil (geliştirme, sınama): güncelleme denetimi yok.
    NotInstalled,
    /// Denetim `OPENIFY_UPDATE_URL=off` ile kapalı.
    Off,
    /// Henüz denetlenmedi (açılıştan sonraki ilk denetim bekleniyor).
    Waiting,
    Checking,
    UpToDate,
    /// Yeni sürüm indirildi, yeniden açınca uygulanacak.
    Ready(String),
    /// Son denetim başarısız (ağ yok gibi).
    Failed,
    /// Yeni sürüm açılamadığı için önceki sürüme dönüldü (bozuk sürümün adı);
    /// daha yeni bir sürüm çıkana kadar kalır.
    RolledBack(String),
}

static STATUS: std::sync::Mutex<Status> = std::sync::Mutex::new(Status::Waiting);

fn set_status(status: Status) {
    if let Ok(mut s) = STATUS.lock() {
        *s = status;
    }
}

/// Şimdiki durum. Kurulu değilse ya da denetim kapalıysa ona göre.
pub fn status() -> Status {
    if feed_url().is_none() {
        return Status::Off;
    }
    if !installed() {
        return Status::NotInstalled;
    }
    let status = STATUS.lock().map(|s| s.clone()).unwrap_or(Status::Waiting);
    if matches!(status, Status::Waiting | Status::UpToDate)
        && let Some(bad) = ROLLED_BACK.lock().ok().and_then(|r| r.clone())
    {
        return Status::RolledBack(bad);
    }
    status
}

/// Kurulu bir sürüm mü çalışıyor (Velopack kurulumu bulundu mu)? Bir kez bakılır.
fn installed() -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| UpdateManager::new(HttpSource::new(""), None, None).is_ok())
}

/// "Şimdi denetle": arka planda bir denetim yapar; sonucu `status()` verir,
/// yeni sürüm indirildiyse `on_event` çağrılır. Kurulu değilse ya da bir
/// denetim zaten sürüyorsa hiçbir şey yapmaz.
pub fn check_now(on_event: impl Fn(Event) + Send + 'static) {
    let Some(url) = feed_url() else { return };
    if !installed() || status() == Status::Checking {
        return;
    }
    set_status(Status::Checking);
    let _ = std::thread::Builder::new()
        .name("openify-update-now".into())
        .spawn(move || {
            let Ok(manager) = UpdateManager::new(HttpSource::new(&url), None, None) else {
                set_status(Status::Failed);
                return;
            };
            match check_and_download(&manager) {
                Ok(Some(version)) => {
                    set_status(Status::Ready(version.clone()));
                    on_event(Event::Ready { version });
                }
                Ok(None) => set_status(Status::UpToDate),
                Err(e) => {
                    log::info!("güncelleme: denetim başarısız: {e}");
                    set_status(Status::Failed);
                }
            }
        });
}

/// Arka planda güncelleme denetimini başlatır. `on_event` bir arka plan
/// iş parçacığından çağrılır. Kurulu değilse ya da kaynak kapalıysa hiçbir şey yapmaz.
pub fn start(on_event: impl Fn(Event) + Send + 'static) {
    // Pencere açıldı: sürüm bir süre sonra sağlıklı sayılır (güncelleme
    // denetiminden ve kaynaktan bağımsız, bkz. `health`).
    arm_health();
    let Some(url) = feed_url() else {
        log::info!("güncelleme: kapalı (OPENIFY_UPDATE_URL=off)");
        return;
    };
    let Ok(manager) = UpdateManager::new(HttpSource::new(&url), None, None) else {
        log::debug!("güncelleme: kurulu değil, denetim yok");
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("openify-update".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            // Yedek ancak sürüm sağlıklı kanıtlandıktan sonra alınır; ilk denetim
            // sağlık zamanlayıcısıyla yarışmasın.
            mark_healthy();
            let mut announced = None;
            loop {
                set_status(Status::Checking);
                match check_and_download(&manager) {
                    Ok(Some(version)) => {
                        set_status(Status::Ready(version.clone()));
                        if announced.as_ref() != Some(&version) {
                            announced = Some(version.clone());
                            on_event(Event::Ready { version });
                        }
                    }
                    Ok(None) => set_status(Status::UpToDate),
                    Err(e) => {
                        log::info!("güncelleme: denetim başarısız: {e}");
                        set_status(Status::Failed);
                    }
                }
                std::thread::sleep(CHECK_EVERY);
            }
        });
    if let Err(e) = spawned {
        log::warn!("güncelleme: iş parçacığı açılamadı: {e}");
    }
}

/// Yeni sürüm varsa indirir ve sürümünü döner; zaten indirilmişse (yeniden
/// açılmayı bekliyor) sürümü yine döner, indirme tekrarlanmaz.
fn check_and_download(manager: &UpdateManager) -> Result<Option<String>, velopack::Error> {
    // Geri dönüş sürerken bozuk sürümü yeniden indirme.
    if ROLLING_BACK.load(Ordering::SeqCst) {
        return Ok(None);
    }
    if let Some(pending) = manager.get_update_pending_restart() {
        if is_blocked(&pending.Version) {
            return Ok(None);
        }
        return Ok(Some(pending.Version));
    }
    match manager.check_for_updates()? {
        UpdateCheck::UpdateAvailable(info) => {
            let version = info.TargetFullRelease.Version.clone();
            // Geri alınan bozuk sürüm (ve altı) daha yeni bir sürüm çıkana kadar inmez.
            if is_blocked(&version) {
                log::info!("güncelleme: {version} kara listede, indirilmiyor");
                return Ok(None);
            }
            // `download_updates` eski paketleri siler: önce geri dönüş yedeği.
            backup_current_package();
            log::info!("güncelleme: {version} indiriliyor");
            manager.download_updates(&info, None)?;
            log::info!("güncelleme: {version} indirildi");
            Ok(Some(version))
        }
        _ => Ok(None),
    }
}
