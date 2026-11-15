//! Çökme kaydı: sessiz panikleri günlüğe ve `crash-<zaman>.txt` dosyasına yazar.
//!
//! Release derlemesi `windows_subsystem = "windows"`: konsol yok, stderr'e
//! düşen panik mesajını kimse görmez ve bir iş parçacığı (motor, ses) sessizce
//! ölebilir. `install_panic_hook` her panikte mesajı, yeri, iş parçacığının
//! adını ve geri izlemeyi günlük dosyasına yazar, yazıyı diske boşaltır
//! (flush) ve günlük klasörüne bir çökme raporu koyar (son 5 rapor kalır).
//!
//! Günlük klasörü: `%LOCALAPPDATA%\Openify` (`OPENIFY_LOG_DIR` ile başka klasör:
//! sınamalar gerçek klasöre yazmasın).

use std::{
    any::Any,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::ffi::c_void;

/// Saklanan en çok çökme raporu.
pub const KEEP_REPORTS: usize = 5;

/// Günlük ve çökme raporlarının klasörü.
pub fn log_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OPENIFY_LOG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = test_sandbox() {
        return Some(dir.join("log"));
    }
    Some(dirs::cache_dir()?.join("Openify"))
}

/// Bu çalışma bir sınama koşusu mu? (`OPENIFY_TEST_DISPLAY` ya da
/// `OPENIFY_SETTINGS_PATH` verilmiş.) Gerçek kullanıcı ikisini de vermez.
pub fn is_test_run() -> bool {
    std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
        || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some()
}

/// Sınama kopyalarında Windows'un erişim ihlali ileti kutusunu kapat.
/// `main` bunu güncelleme kancalarından ve pencere oluşturulmasından önce çağırır.
pub fn initialize_early() {
    #[cfg(windows)]
    if is_test_run() {
        // SEM_NOGPFAULTERRORBOX: 0x0002. Bilerek normal açılış davranışını değiştirmez.
        unsafe { SetErrorMode(0x0002) };
    }
}

/// Erişim ihlallerini yalnız izole sınama çalışmasında modül + göreli adresle kaydet.
pub fn install_test_exception_handler() {
    #[cfg(windows)]
    if is_test_run() {
        // Handler süreç sonuna kadar Windows tarafından tutulur.
        let handler = unsafe { AddVectoredExceptionHandler(1, Some(log_native_exception)) };
        if handler.is_null() {
            log::error!(target: "openify", "VEH kurulamadı: AddVectoredExceptionHandler başarısız");
        } else {
            log::info!(target: "openify", "sınama VEH hata tanılaması kuruldu");
        }
    }
}

#[cfg(windows)]
#[repr(C)]
struct NativeExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut NativeExceptionRecord,
    address: *mut c_void,
    parameter_count: u32,
    information: [usize; 15],
}

#[cfg(windows)]
#[repr(C)]
struct NativeExceptionPointers {
    record: *mut NativeExceptionRecord,
    context: *mut c_void,
}

#[cfg(windows)]
unsafe extern "system" fn log_native_exception(pointers: *mut NativeExceptionPointers) -> i32 {
    const ACCESS_VIOLATION: u32 = 0xC000_0005;
    if pointers.is_null() || unsafe { (*pointers).record.is_null() } {
        return 0; // EXCEPTION_CONTINUE_SEARCH
    }
    let record = unsafe { &*(*pointers).record };
    if record.code != ACCESS_VIOLATION {
        return 0;
    }

    let address = record.address as usize;
    let (module_name, base) = native_address_module(address);
    let access = match record.information[0] {
        0 => "read",
        1 => "write",
        8 => "execute",
        _ => "unknown",
    };
    let bad_address = record.information[1];
    let context = unsafe { (*pointers).context.cast::<u8>() };
    // x64 CONTEXT offsets: Rsp=152, Rip=248. This target is a 64-bit Windows build.
    let rsp = unsafe { std::ptr::read_unaligned(context.add(152).cast::<u64>()) } as usize;
    let rip = unsafe { std::ptr::read_unaligned(context.add(248).cast::<u64>()) } as usize;
    let mut stack = [0usize; 32];
    let mut bytes_read = 0usize;
    let stack_read = unsafe {
        ReadProcessMemory(
            usize::MAX as *mut c_void,
            rsp as *const c_void,
            stack.as_mut_ptr().cast(),
            std::mem::size_of_val(&stack),
            &mut bytes_read,
        )
    } != 0;
    let stack_modules = if stack_read {
        stack
            .iter()
            .take(bytes_read / std::mem::size_of::<usize>())
            .filter_map(|&frame| {
                (frame > 0x1_0000)
                    .then(|| native_address_module(frame))
                    .filter(|(_, frame_base)| *frame_base != 0)
                    .map(|(name, frame_base)| {
                        format!("{}+0x{:X}", module_leaf(&name), frame - frame_base)
                    })
            })
            .collect::<Vec<_>>()
            .join(",")
    } else {
        format!("stack unreadable ({bytes_read} bytes)")
    };
    let report = format_native_exception(
        record.code,
        &module_name,
        address,
        base,
        access,
        bad_address,
    );
    log::error!(target: "openify", "{report}");
    log::error!(target: "openify", "NATIVE EXCEPTION stack rip=0x{rip:X} [{stack_modules}]");
    log::logger().flush();
    0 // EXCEPTION_CONTINUE_SEARCH: uygulamanın normal exception akışı sürsün.
}

#[cfg(windows)]
fn native_address_module(address: usize) -> (String, usize) {
    let mut module = std::ptr::null_mut();
    let found = unsafe {
        GetModuleHandleExW(
            0x0000_0004 | 0x0000_0002, // FROM_ADDRESS | UNCHANGED_REFCOUNT
            address as *const u16,
            &mut module,
        )
    } != 0;
    if !found {
        return ("unknown module".to_string(), 0);
    }
    let mut buffer = [0u16; 260];
    let length = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };
    (
        String::from_utf16_lossy(&buffer[..length.min(buffer.len() as u32) as usize]),
        module as usize,
    )
}

#[cfg(windows)]
fn module_leaf(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetErrorMode(mode: u32) -> u32;
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: Option<unsafe extern "system" fn(*mut NativeExceptionPointers) -> i32>,
    ) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, file_name: *mut u16, size: u32) -> u32;
    fn ReadProcessMemory(
        process: *mut c_void,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> i32;
}

/// Erişim ihlali için modül yolu, ham adres ve modül içi ofset.
fn format_native_exception(
    code: u32,
    module: &str,
    address: usize,
    base: usize,
    access: &str,
    target: usize,
) -> String {
    format!(
        "NATIVE EXCEPTION code=0x{code:08X} module={module}+0x{:X} address=0x{address:X} access={access} target=0x{target:X}",
        address.saturating_sub(base),
    )
}

/// Sınama koşusunda klasör açıkça verilmemişse kullanılan geçici kum havuzu
/// (`%TEMP%\openify-sandbox`). Böylece unutulan bir ortam değişkeni
/// kullanıcının gerçek `%LOCALAPPDATA%\Openify` önbelleğini ve günlüğünü
/// değiştiremez (11 Kasım: bir sınama koşusu gerçek `library.db`'yi göçürdü).
pub fn test_sandbox() -> Option<PathBuf> {
    is_test_run().then(|| std::env::temp_dir().join("openify-sandbox"))
}

/// Panik yükünün metni (`&str` / `String`; başka türde sabit bir ifade).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Panik kancasını kurar. Günlük kurulduktan (`init_logging`) sonra çağrılır.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed").to_string();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".into());
        let message = panic_message(info.payload());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let report = format_report(&thread_name, &location, &message, &backtrace, unix_now());

        // Önce günlük (en önemlisi), hemen diske boşalt.
        log::error!("PANİK: iş parçacığı '{thread_name}', {location}: {message}\n{backtrace}");
        log::logger().flush();
        if let Some(dir) = log_dir() {
            match write_report(&dir, &report, unix_now()) {
                Ok(path) => log::error!("çökme raporu yazıldı: {}", path.display()),
                Err(e) => log::error!("çökme raporu yazılamadı: {e}"),
            }
            log::logger().flush();
        }
        // Terminalden çalıştırılırsa varsayılan çıktı da görünsün.
        default(info);
    }));
}

/// Çökme raporunun metni.
pub fn format_report(
    thread: &str,
    location: &str,
    message: &str,
    backtrace: &str,
    unix_secs: u64,
) -> String {
    // Geliştirici raporu (kullanıcıya gösterilmez): İngilizce etiketler.
    format!(
        "Openify crash report\n\
         version: {}\n\
         time: {} (UTC)\n\
         thread: {thread}\n\
         location: {location}\n\
         message: {message}\n\
         \n\
         backtrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        timestamp(unix_secs),
    )
}

/// `dir` içine `crash-<YYYYMMDD-HHMMSS>.txt` yazar (aynı saniyede ikinci
/// rapor `-2`, `-3`… alır), sonra en yeni `KEEP_REPORTS` rapordan fazlasını siler.
pub fn write_report(dir: &Path, report: &str, unix_secs: u64) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = timestamp(unix_secs);
    let mut n = 1;
    let (path, mut file) = loop {
        let name = if n == 1 {
            format!("crash-{stamp}.txt")
        } else {
            format!("crash-{stamp}-{n}.txt")
        };
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break (path, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(e) => return Err(e),
        }
    };
    file.write_all(report.as_bytes())?;
    file.sync_all()?;
    prune_reports(dir, KEEP_REPORTS);
    Ok(path)
}

/// En yeni `keep` çökme raporu dışındakileri siler (ad zamana göre sıralanır).
pub fn prune_reports(dir: &Path, keep: usize) {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    // "-2" ekli ikinci rapor aynı saniyede ilkinden SONRA yazıldı: ada göre
    // sıralamak bunu bozar ("…-2.txt" < "….txt"); önce damga, sonra ek sayısı.
    reports.sort_by_key(|p| report_order(p));
    let excess = reports.len().saturating_sub(keep);
    for old in reports.into_iter().take(excess) {
        let _ = fs::remove_file(old);
    }
}

/// Sıralama anahtarı: (damga, aynı saniyedeki sıra).
fn report_order(path: &Path) -> (String, u32) {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let rest = stem.strip_prefix("crash-").unwrap_or(stem);
    // YYYYMMDD-HHMMSS[-n]
    let mut parts = rest.splitn(3, '-');
    let date = parts.next().unwrap_or_default();
    let time = parts.next().unwrap_or_default();
    let n = parts.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    (format!("{date}-{time}"), n)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `YYYYMMDD-HHMMSS` (UTC).
pub fn timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// 1970-01-01'den gün sayısı → (yıl, ay, gün); Howard Hinnant'ın algoritması.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openify-crash-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn timestamps_are_civil_utc() {
        assert_eq!(timestamp(0), "19700101-000000");
        assert_eq!(timestamp(1_700_000_000), "20231114-221320");
        // Artık gün ve yıl sonu.
        assert_eq!(timestamp(951_782_400), "20000229-000000");
        assert_eq!(timestamp(1_767_225_599), "20251231-235959");
    }

    #[test]
    fn panic_payloads_become_text() {
        let s: Box<dyn Any + Send> = Box::new("sabit yazı");
        assert_eq!(panic_message(s.as_ref()), "sabit yazı");
        let s: Box<dyn Any + Send> = Box::new(String::from("biçimli 42"));
        assert_eq!(panic_message(s.as_ref()), "biçimli 42");
        let s: Box<dyn Any + Send> = Box::new(42u8);
        assert_eq!(panic_message(s.as_ref()), "unknown panic payload");
        // Gerçek bir panikten.
        let caught = std::panic::catch_unwind(|| panic!("gerçek panik {}", 7)).unwrap_err();
        assert_eq!(panic_message(caught.as_ref()), "gerçek panik 7");
    }

    #[test]
    fn report_has_message_location_thread_and_backtrace() {
        let r = format_report(
            "openify-engine",
            "src/engine/mod.rs:1:2",
            "bir şey bozuldu",
            "  0: bir::işlev",
            1_700_000_000,
        );
        for needle in [
            "openify-engine",
            "src/engine/mod.rs:1:2",
            "bir şey bozuldu",
            "0: bir::işlev",
            "20231114-221320 (UTC)",
            env!("CARGO_PKG_VERSION"),
        ] {
            assert!(r.contains(needle), "{needle} yok:\n{r}");
        }
    }

    #[test]
    fn native_exception_report_identifies_module_and_offset() {
        let report = format_native_exception(
            0xC000_0005,
            "C:\\Windows\\System32\\dxgi.dll",
            0x7FFB_33BD_27A9,
            0x7FFB_33BD_0000,
            "write",
            0,
        );
        assert!(report.contains("0xC0000005"), "{report}");
        assert!(report.contains("dxgi.dll"), "{report}");
        assert!(report.contains("dxgi.dll+0x27A9"), "{report}");
        assert!(report.contains("access=write target=0x0"), "{report}");
    }

    #[test]
    fn only_the_last_five_reports_are_kept() {
        let dir = temp_dir("keep");
        for i in 0..8u64 {
            write_report(&dir, &format!("rapor {i}"), 1_700_000_000 + i * 10).unwrap();
        }
        // Başka dosyalara dokunulmaz.
        fs::write(dir.join("openify.log"), "günlük").unwrap();
        prune_reports(&dir, KEEP_REPORTS);
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 6, "{names:?}");
        assert!(names.contains(&"openify.log".to_string()));
        let crashes: Vec<&String> = names.iter().filter(|n| n.starts_with("crash-")).collect();
        assert_eq!(crashes.len(), KEEP_REPORTS);
        // En eskiler (0, 1, 2) gitti (3..7 kaldı); en yeniler kaldı.
        assert_eq!(crashes[0], "crash-20231114-221350.txt");
        assert_eq!(crashes[4], "crash-20231114-221430.txt");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_reports_in_the_same_second_both_survive_and_order_by_time() {
        let dir = temp_dir("same-second");
        let a = write_report(&dir, "ilk", 1_700_000_000).unwrap();
        let b = write_report(&dir, "ikinci", 1_700_000_000).unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read_to_string(&a).unwrap(), "ilk");
        assert_eq!(fs::read_to_string(&b).unwrap(), "ikinci");
        // Beş rapor sınırında yeni olan (ikinci) silinmez, en eski silinir.
        for i in 1..=4u64 {
            write_report(&dir, "daha yeni", 1_700_000_000 + i).unwrap();
        }
        assert!(!a.exists(), "en eski rapor silinmeli");
        assert!(
            b.exists(),
            "aynı saniyenin ikinci raporu ondan eski olanlardan sonra silinir"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_creates_the_directory_and_returns_the_path() {
        let dir = temp_dir("create").join("yeni").join("klasör");
        let path = write_report(&dir, "içerik", 0).unwrap();
        assert_eq!(path, dir.join("crash-19700101-000000.txt"));
        assert_eq!(fs::read_to_string(path).unwrap(), "içerik");
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn log_dir_can_be_redirected_for_tests() {
        // Ortam değişkenini değiştirmeden: varsayılan Openify klasörüne çıkar.
        if std::env::var_os("OPENIFY_LOG_DIR").is_none() {
            assert!(log_dir().is_some_and(|d| d.ends_with("Openify")));
        }
    }
}
