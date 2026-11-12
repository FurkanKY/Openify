//! Openify: Spotify hesabını kullanan hızlı, sade masaüstü istemci.
//!
//! Kullanım:
//!   openify                       pencereyi açar
//!   openify --tray                pencere açmadan tepsiye gizli başlar (Windows açılışı)
//!   openify <şarkı bağlantısı>    pencere açmadan terminalde çalar
//!   openify logout                kayıtlı Spotify girişini siler
//!   openify connect-spike         Spotify Connect canlı denemesi (yalnız `--features connect-spike`
//!                                 derlemesinde)

// Release derlemede arka planda boş bir konsol penceresi açılmasın.
// Terminalden çalıştırıldığında çıktılar yine görünür (attach_console).
// `connect-spike` derlemesi (sahibinin Connect denemesi) bir konsol uygulamasıdır: etkileşimli
// onay listesi standart girdi / çıktı ister.
#![cfg_attr(all(not(debug_assertions), not(feature = "connect-spike")), windows_subsystem = "windows")]

mod auth;
mod autostart;
mod brand;
mod crash;
mod engine;
mod i18n;
mod cli;
#[cfg(feature = "connect-spike")]
mod connect_spike;
mod link;
#[cfg(feature = "mem-stats")]
mod mem_stats;
mod msg;
mod settings;
#[cfg(windows)]
mod system;
mod ui;
mod update;

use anyhow::Result;
use std::{sync::OnceLock, time::Instant};

/// Sürecin başladığı an: açılış aşamalarını ölçmek için.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Süreç başlayalı geçen süre (milisaniye).
pub fn since_start() -> u128 {
    STARTED.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}

/// Sınama kopyası (`OPENIFY_TEST_DISPLAY`) kendini kapatır: varsayılan 15 dk
/// sonra, `OPENIFY_TEST_EXIT_AFTER=sn` ile başka süre, `0` ile hiç. Sınama
/// betikleri kopyayı kapatamadığında (süreç sonlandırma izni
/// yok, betik yarıda kaldı) görev çubuğunda pencere birikmesin diye.
fn test_watchdog() {
    if std::env::var_os("OPENIFY_TEST_DISPLAY").is_none() {
        return;
    }
    let secs = test_exit_after(std::env::var("OPENIFY_TEST_EXIT_AFTER").ok().as_deref());
    if secs == 0 {
        return;
    }
    let _ = std::thread::Builder::new().name("test-watchdog".into()).spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        log::info!("sınama kopyası {secs} sn doldu, kapanıyor (OPENIFY_TEST_EXIT_AFTER)");
        std::process::exit(0);
    });
}

/// `OPENIFY_TEST_EXIT_AFTER` değeri; yoksa ya da bozuksa 15 dk.
fn test_exit_after(value: Option<&str>) -> u64 {
    value.and_then(|v| v.trim().parse().ok()).unwrap_or(15 * 60)
}

#[cfg(test)]
mod watchdog_tests {
    use super::test_exit_after;

    #[test]
    fn exit_after_defaults_and_overrides() {
        assert_eq!(test_exit_after(None), 900);
        assert_eq!(test_exit_after(Some("bozuk")), 900);
        assert_eq!(test_exit_after(Some(" 120 ")), 120);
        assert_eq!(test_exit_after(Some("0")), 0);
    }
}

fn main() -> Result<()> {
    STARTED.get_or_init(Instant::now);
    // Velopack kurulum / güncelleme / kaldırma kancaları: her şeyden önce
    // (tek kopya kilidinden ve pencereden önce; gerekirse süreç burada çıkar).
    update::hooks();
    #[cfg(feature = "mem-stats")]
    mem_stats::start();
    let args: Vec<String> = std::env::args().collect();
    // Konsola yalnız terminal komutları bağlanır (bkz. `attach_console`).
    let gui = matches!(args.get(1).map(String::as_str), None | Some(autostart::TRAY_FLAG));
    if !gui {
        attach_console();
    }

    // Connect denemesi kendi günlüğünü kurar (Spotify istek sayacı dahil) ve kendi çalışma ortamını
    // açar; uygulama günlüğü, çökme kancası ve tek kopya kilidi bu komuta karışmaz.
    #[cfg(feature = "connect-spike")]
    if args.get(1).map(String::as_str) == Some("connect-spike") {
        return connect_spike::run(&args[2..]);
    }

    // librespot'un uyarı ve hataları terminalde görünsün.
    // Daha ayrıntılı çıktı için: $env:RUST_LOG="librespot=debug"
    init_logging();
    // Release derlemede konsol yok: paniği (motor ve ses iş parçacıkları
    // dahil) günlüğe ve çökme raporuna yaz, sessizce ölmesin.
    crash::install_panic_hook();
    log::debug!("aşama {} ms: kayıt başladı", since_start());
    test_watchdog();

    // Windows oturum açılışı (`--tray`, bkz. `autostart`): pencere tepsiye
    // gizli başlar.
    let start_hidden = autostart::started_hidden(&args);
    match args.get(1).map(String::as_str) {
        None | Some(autostart::TRAY_FLAG) => {
            // Tek kopya: Openify zaten açıksa onun penceresi öne gelir, bu
            // açılış kapanır (bkz. `system::instance`). Terminal komutları
            // (şarkı bağlantısı, logout) kilide bakmaz.
            #[cfg(windows)]
            let _instance = match system::instance::acquire(start_hidden) {
                system::instance::Launch::Forwarded => return Ok(()),
                system::instance::Launch::Primary(guard) => Some(guard),
                system::instance::Launch::Bypass => None,
            };
            // Kilidi almış süreç: açılışı sağlık kaydına yaz, önceki oturumda
            // indirilmiş güncellemeyi uygula, art arda açılamayan sürümü geri al.
            update::startup();
            #[cfg(windows)]
            system::set_app_id();
            ui::run(start_hidden);
            Ok(())
        }
        Some("logout") => {
            auth::clear()?;
            println!("Çıkış yapıldı. Bir sonraki açılışta Spotify girişi yeniden istenecek.");
            Ok(())
        }
        Some(track) => tokio::runtime::Runtime::new()?.block_on(cli::play(track)),
    }
}

/// Kayıtlar terminale ve `%LOCALAPPDATA%\Openify\openify.log` dosyasına yazılır.
/// Dosya 5 MB'ı geçince bir öncekine (`openify.old.log`) taşınır.
/// Varsayılan: Openify'ın bilgileri, librespot'un uyarıları ve hangi Spotify
/// sunucusuna bağlanıldığı. Daha ayrıntılı: `$env:RUST_LOG="librespot=debug"`.
fn init_logging() {
    use std::io::Write;

    struct Tee(Option<std::fs::File>);
    impl Write for Tee {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            // Önce dosya: stderr bir yerde takılsa da kayıt eksiksiz kalır.
            if let Some(file) = &mut self.0 {
                let _ = file.write_all(buf);
            }
            let _ = std::io::stderr().write_all(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if let Some(file) = &mut self.0 {
                file.flush()?;
            }
            std::io::stderr().flush()
        }
    }

    let file = crash::log_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join("openify.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
            let _ = std::fs::rename(&path, dir.join("openify.old.log"));
        }
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
    });

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "openify=info,librespot=warn,librespot_core::session=info",
    ))
    .format_timestamp_millis()
    .target(env_logger::Target::Pipe(Box::new(Tee(file))))
    .init();
}

/// Release derlemede uygulama "pencere uygulaması" olarak işaretli olduğu için
/// kendi konsolu yok. Terminal komutlarında (`logout`, şarkı bağlantısı) o
/// terminale bağlan ki `println!` çıktıları görünsün. Pencere modunda
/// BAĞLANMAZ: kimsenin okumadığı (kapanmış, ya da metin seçilip QuickEdit ile
/// durdurulmuş) bir konsola yazan kayıt satırı sonsuza dek bekler ve kayıt
/// yazan her iş parçacığını (motor dahil) kilitler. Kayıtlar zaten
/// `openify.log`'a gider.
fn attach_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    unsafe {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
