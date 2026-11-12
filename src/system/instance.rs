//! Tek kopya çalışma.
//!
//! İlk açılış adlı bir mutex alır ve ileti alan görünmez bir pencere açar
//! (yalnız ileti penceresi, sınıfı "Openify.Ipc"). İkinci açılış mutex'i
//! dolu bulunca o pencereyi bulur, ilk kopyanın öne gelmesine izin verir
//! (`AllowSetForegroundWindow`), `WM_COPYDATA` ile "göster" der ve kapanır.
//! İlk kopya henüz açılıyorsa pencereyi en fazla 2 sn bekler.
//!
//! Sınama koşuları (`OPENIFY_TEST_DISPLAY`) kilidi hiç almaz: kullanıcının
//! kendi Openify'ı açıkken de çalışırlar. `OPENIFY_INSTANCE_ID=ad` verilirse
//! kilit ve pencere o ada özel olur (tek kopya davranışını sınamak için).

use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
        },
        System::{
            DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW, Threading::CreateMutexW,
        },
        UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW,
            FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE, MSGFLT_ALLOW, RegisterClassW,
            SMTO_ABORTIFHUNG, SendMessageTimeoutW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COPYDATA,
            WNDCLASSW,
        },
    },
    core::{HSTRING, w},
};

use super::{SystemEvent, emit};

/// Kilidin ve ileti penceresinin ortak ön eki (başka bir uygulamanın
/// "Openify" adlı bir şeyiyle karışmasın).
const BASE: &str = "Openify-5d0c9b2e";
/// "Göster" isteğinin kimliği (`COPYDATASTRUCT::dwData`).
const COPYDATA_SHOW: usize = 0x4F50_454E;
/// İkinci açılış ilk kopyanın penceresini en fazla bu kadar bekler.
const WAIT_FOR_FIRST: Duration = Duration::from_secs(2);

pub enum Launch {
    /// İlk kopya: pencereyi aç. Kilit süreç boyunca tutulur.
    Primary(Guard),
    /// Başka bir kopya zaten açıktı, ona "göster" dendi: çık.
    Forwarded,
    /// Sınama koşusu: kilit yok.
    Bypass,
}

/// Mutex'in tutamacı (süreç bitince kendiliğinden bırakılır).
pub struct Guard(#[allow(dead_code)] HANDLE);

fn instance_id() -> Option<String> {
    std::env::var("OPENIFY_INSTANCE_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Bu kopyanın ileti penceresinin başlığı: ikinci açılış pencereyi bununla
/// bulur. Sınama koşusunda sürece özel (kimse bulmasın).
fn ipc_title() -> String {
    match instance_id() {
        Some(id) => format!("{BASE}-{id}"),
        None if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() => {
            format!("{BASE}-sinama-{}", std::process::id())
        }
        None => format!("{BASE}-main"),
    }
}

/// `main()`'in en başında, pencereli açılışta çağrılır. `quiet`: oturum
/// açılışı (`--tray`), ikinci kopya ilkinin penceresini göstermeden çıkar.
pub fn acquire(quiet: bool) -> Launch {
    let started = Instant::now();
    let id = instance_id();
    if id.is_none() && std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
        log::info!("sistem: sınama koşusu, tek kopya kilidi atlandı");
        return Launch::Bypass;
    }
    let name = HSTRING::from(format!("Local\\{BASE}-{}", id.as_deref().unwrap_or("main")));
    loop {
        let handle = unsafe { CreateMutexW(None, false, &name) };
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        match handle {
            Ok(h) if !exists => {
                log::info!(
                    "sistem: tek kopya kilidi {:.2} ms",
                    started.elapsed().as_secs_f64() * 1000.
                );
                return Launch::Primary(Guard(h));
            }
            Ok(h) => unsafe {
                let _ = CloseHandle(h);
            },
            Err(e) => {
                // Kilit alınamadıysa (olağan değil) yine de aç.
                log::warn!("Tek kopya kilidi alınamadı: {e}");
                return Launch::Bypass;
            }
        }
        // Oturum açılışından gelen `--tray` açılışı: zaten açıksa hiçbir şey yapma.
        if quiet {
            return Launch::Forwarded;
        }
        if forward_show() {
            log::info!(
                "Openify zaten açık: var olan pencere öne getirildi ({:.1} ms)",
                started.elapsed().as_secs_f64() * 1000.
            );
            return Launch::Forwarded;
        }
        if started.elapsed() >= WAIT_FOR_FIRST {
            log::warn!("Openify zaten açık ama penceresine ulaşılamadı; bu açılış kapanıyor");
            return Launch::Forwarded;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// İlk kopyanın ileti penceresine "göster" der. Pencere yoksa `false`.
fn forward_show() -> bool {
    let title = HSTRING::from(ipc_title());
    let Ok(target) =
        (unsafe { FindWindowExW(Some(HWND_MESSAGE), None, w!("Openify.Ipc"), &title) })
    else {
        return false;
    };
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(target, Some(&mut pid));
        // Kullanıcı bizi başlattı, öne gelme hakkı bizde: ilk kopyaya devret.
        if pid != 0 {
            let _ = AllowSetForegroundWindow(pid);
        }
        let payload = b"show";
        let data = COPYDATASTRUCT {
            dwData: COPYDATA_SHOW,
            cbData: payload.len() as u32,
            lpData: payload.as_ptr() as *mut _,
        };
        let mut result = 0usize;
        let sent = SendMessageTimeoutW(
            target,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&data as *const _ as isize),
            SMTO_ABORTIFHUNG,
            1000,
            Some(&mut result),
        );
        sent.0 != 0 && result == 1
    }
}

/// İlk kopyanın ileti penceresi (arayüz iş parçacığında, gpui'nin ileti
/// döngüsü onu da işler).
pub fn create_ipc_window() -> Option<HWND> {
    let t = Instant::now();
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(ipc_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Openify.Ipc"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("Openify.Ipc"),
            &HSTRING::from(ipc_title()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .inspect_err(|e| log::warn!("Tek kopya ileti penceresi açılamadı: {e}"))
        .ok()?;
        let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
        log::info!(
            "sistem: ileti penceresi {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.
        );
        Some(hwnd)
    }
}

unsafe extern "system" fn ipc_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_COPYDATA && lparam.0 != 0 {
        let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        if data.dwData == COPYDATA_SHOW {
            log::info!("sistem: ikinci açılış \"göster\" dedi");
            emit(SystemEvent::Show);
            return LRESULT(1);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
