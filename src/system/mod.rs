//! Windows'la bütünleşme (Adım 10).
//!
//! - Windows medya kontrolü (SMTC) ve medya tuşları: `smtc.rs`
//! - Görev çubuğu önizlemesindeki önceki / çal / sonraki düğmeleri: `taskbar.rs`
//! - Tepsi simgesi: `tray.rs`
//! - Tek kopya çalışma ve ikinci açılıştan gelen "göster" isteği: `instance.rs`
//! - gpui'nin işlemediği pencere iletileri (kısayol, görev çubuğu düğmesi,
//!   tepsi tıklaması) için pencere alt sınıflaması: `hook.rs`
//!
//! İşletim sisteminden gelen her şey (ileti, SMTC olayı) yalnızca bir kanala
//! `SystemEvent` olarak bırakılır ve hemen dönülür: gpui o sırada kendi
//! durumunu ödünç almış olabilir, ileti işleyicisinden arayüze dokunmak
//! çakışırdı. Arayüz kanalı kendi döngüsünde okur (`ui::integration`).

pub mod app_theme;
mod dialog;
mod hook;
mod icons;
pub mod instance;
mod smtc;
mod taskbar;
mod tray;

use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};

use windows::Win32::Foundation::HWND;

use crate::{t, tf};
pub use dialog::{open_files_dialog, save_file_dialog};
pub use hook::hotkey_label;

/// İşletim sisteminden arayüze gelen olaylar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SystemEvent {
    /// Medya tuşu, Windows medya kontrolü ya da görev çubuğu düğmesi.
    Media(MediaButton),
    /// Windows medya kontrolündeki konum çubuğu sürüklendi (ms).
    SeekTo(u32),
    /// Pencereyi göster ve öne getir (tepsi simgesine tıklama, ikinci açılış).
    Show,
    /// Tepsi simgesine sağ tıklandı: menü bu noktada açılır (ekran pikseli).
    TrayMenu { x: i32, y: i32 },
    /// Her yerden palet kısayolu (Ctrl+Shift+Boşluk).
    Hotkey,
    /// Görev çubuğu düğmesi (yeniden) oluştu: önizleme düğmeleri eklenmeli.
    TaskbarButtonCreated,
    /// Explorer yeniden başladı: tepsi simgesi yeniden eklenmeli.
    TaskbarCreated,
    /// Windows renk modu değişti: simgeler ve Sistem tercihindeki UI yenilenir.
    ThemeChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaButton {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
}

/// Olay ve geldiği an (gecikme ölçümü için).
#[derive(Debug, Clone, Copy)]
pub struct Stamped {
    pub event: SystemEvent,
    pub at: Instant,
}

static EVENTS: OnceLock<async_channel::Sender<Stamped>> = OnceLock::new();

/// Olayları okuyacak kanal (bir kez çağrılır).
pub fn listen() -> async_channel::Receiver<Stamped> {
    let (tx, rx) = async_channel::unbounded();
    let _ = EVENTS.set(tx);
    rx
}

/// Olayı kanala bırak. Her iş parçacığından çağrılabilir, beklemez.
pub(crate) fn emit(event: SystemEvent) {
    if let Some(tx) = EVENTS.get() {
        let _ = tx.try_send(Stamped {
            event,
            at: Instant::now(),
        });
    }
}

/// Hangi parçalar açık.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    pub smtc: bool,
    pub taskbar: bool,
    pub tray: bool,
    pub hotkey: bool,
}

impl Features {
    /// Normal açılışta hepsi açık. Sınama koşusunda (`OPENIFY_TEST_DISPLAY`)
    /// hepsi kapalı: kullanıcının kendi Openify'ının medya tuşlarını,
    /// kısayolunu ve tepsisini çalmasın. `OPENIFY_TEST_SYSTEM` ile tek tek
    /// açılır: "smtc,taskbar,tray,hotkey" ya da "all".
    pub fn from_env() -> Self {
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_none() {
            return Self {
                smtc: true,
                taskbar: true,
                tray: true,
                hotkey: true,
            };
        }
        let list = std::env::var("OPENIFY_TEST_SYSTEM").unwrap_or_default();
        let on = |name: &str| {
            list.split(',')
                .any(|p| p.trim() == name || p.trim() == "all")
        };
        Self {
            smtc: on("smtc"),
            taskbar: on("taskbar"),
            tray: on("tray"),
            hotkey: on("hotkey"),
        }
    }
}

/// Çalanın sisteme gösterilen hâli.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayStatus {
    /// Hiçbir şey yok (medya kontrolü gizlenir).
    #[default]
    Closed,
    Stopped,
    /// Yükleniyor.
    Changing,
    Playing,
    Paused,
}

/// Kapak resmi (JPEG / PNG baytları). Anahtar aynıysa yeniden yüklenmez.
#[derive(Clone)]
pub struct Cover {
    pub key: String,
    pub bytes: Arc<[u8]>,
}

impl std::fmt::Debug for Cover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cover({}, {} bayt)", self.key, self.bytes.len())
    }
}

/// Şu an çalanın sistemde gösterilecek bilgisi.
#[derive(Debug, Clone, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub status: PlayStatus,
    pub duration_ms: u32,
    pub position_ms: u32,
    pub can_play: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub cover: Option<Cover>,
}

impl NowPlaying {
    fn metadata_key(&self) -> (&str, &str, &str, Option<&str>) {
        (
            &self.title,
            &self.artist,
            &self.album,
            self.cover.as_ref().map(|c| c.key.as_str()),
        )
    }

    fn buttons(&self) -> (PlayStatus, bool, bool, bool) {
        (self.status, self.can_play, self.can_next, self.can_previous)
    }
}

/// Kurulan parçalar. Arayüz iş parçacığında yaşar (COM nesneleri).
pub struct Integration {
    hwnd: HWND,
    features: Features,
    smtc: Option<smtc::Smtc>,
    taskbar: Option<taskbar::Taskbar>,
    tray: Option<tray::Tray>,
    hotkey: hook::Hotkey,
    last: Option<NowPlaying>,
}

impl Integration {
    /// Ana pencereye bağlanır: yalnız ucuz parçalar (alt sınıflama,
    /// kısayol). Medya kontrolü, görev çubuğu ve tepsi ~35–45 ms sürüyor
    /// (simge çizimi, Explorer'la konuşma): ilk kareden sonra `install_rest`.
    pub fn install(hwnd: HWND, features: Features, hotkey_pref: bool) -> Self {
        let t = Instant::now();
        hook::install(hwnd);
        log::info!("sistem: pencere alt sınıflaması {:.2} ms", ms(t));

        let t = Instant::now();
        icons::set_window_icons(hwnd);
        log::info!("sistem: pencere simgeleri {:.2} ms", ms(t));

        let t = Instant::now();
        // Kısayol hem sistem parçası açıksa hem kullanıcı Ayarlar'dan kapatmadıysa.
        let hotkey = if features.hotkey && hotkey_pref {
            hook::Hotkey::register(hwnd)
        } else {
            hook::Hotkey::Off
        };
        if features.hotkey && hotkey_pref {
            log::info!("sistem: kısayol {:.2} ms ({hotkey:?})", ms(t));
        }

        Self {
            hwnd,
            features,
            smtc: None,
            taskbar: None,
            tray: None,
            hotkey,
            last: None,
        }
    }

    /// İlk kareden sonra: medya kontrolü, görev çubuğu düğmeleri, tepsi.
    /// O ana kadar bildirilen çalan da hemen verilir.
    pub fn install_rest(&mut self) {
        let (hwnd, features) = (self.hwnd, self.features);
        let light = icons::light_theme();
        let t = Instant::now();
        self.smtc = features.smtc.then(|| smtc::Smtc::new(hwnd)).and_then(|r| {
            r.inspect_err(|e| log::warn!("Windows medya kontrolü kurulamadı: {e}"))
                .ok()
        });
        if features.smtc {
            log::info!("sistem: medya kontrolü {:.2} ms", ms(t));
        }

        let t = Instant::now();
        self.taskbar = features
            .taskbar
            .then(|| taskbar::Taskbar::new(light))
            .and_then(|r| {
                r.inspect_err(|e| log::warn!("Görev çubuğu düğmeleri kurulamadı: {e}"))
                    .ok()
            });
        if features.taskbar {
            log::info!("sistem: görev çubuğu {:.2} ms", ms(t));
        }

        let t = Instant::now();
        self.tray = features
            .tray
            .then(|| tray::Tray::add(hwnd, light))
            .flatten();
        if features.tray {
            let added = if self.tray.is_some() {
                "eklendi"
            } else {
                "eklenemedi"
            };
            log::info!("sistem: tepsi simgesi {:.2} ms ({added})", ms(t));
        }

        // Görev çubuğu düğmesi kurulumdan önce oluşmuş olabilir
        // ("TaskbarButtonCreated" kaçtı): pencere görünürse düğmeleri şimdi ekle.
        let visible =
            unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd) }.as_bool();
        let last = self.last.take();
        if let (true, Some(taskbar)) = (visible, &mut self.taskbar) {
            taskbar.on_button_created(hwnd, last.as_ref().unwrap_or(&NowPlaying::default()));
        }
        if let Some(now) = last {
            self.update(now, true);
        }
    }

    /// Tepsi simgesi var mı? (Yoksa kapatınca pencere gizlenmez, uygulama
    /// kapanır: gizlenen pencereye dönmenin yolu kalmazdı.)
    pub fn tray_active(&self) -> bool {
        self.tray.as_ref().is_some_and(|t| t.added)
    }

    /// Ayarlar'dan "her yerden arama" kısayolunu aç / kapat. Sistem parçası
    /// kapalıysa (sınama kopyası) hiçbir şey yapmaz: gerçek kısayola
    /// dokunulmaz. Açarken kısayol alınamadıysa söylenecek metin döner.
    pub fn set_hotkey(&mut self, on: bool) -> Option<String> {
        if !self.features.hotkey {
            return None;
        }
        if on {
            if matches!(
                self.hotkey,
                hook::Hotkey::Off | hook::Hotkey::Failed | hook::Hotkey::Taken
            ) {
                self.hotkey = hook::Hotkey::register(self.hwnd);
            }
            self.hotkey_problem()
        } else {
            self.hotkey.unregister(self.hwnd);
            self.hotkey = hook::Hotkey::Off;
            None
        }
    }

    /// Her yerden palet kısayolu alınamadıysa (başka bir uygulama tutuyorsa)
    /// kullanıcıya söylenecek metin.
    pub fn hotkey_problem(&self) -> Option<String> {
        match self.hotkey {
            hook::Hotkey::Taken => Some(tf!(HotkeyTaken, hotkey_label())),
            _ => None,
        }
    }

    /// Çalanı sisteme bildirir. Yalnız değişen kısım gönderilir; zaman
    /// çizelgesi (konum) yalnız `timeline` istendiğinde ya da şarkı / durum
    /// değişince (her saniye değil).
    pub fn update(&mut self, now: NowPlaying, timeline: bool) {
        let last = self.last.as_ref();
        let metadata = last.is_none_or(|l| l.metadata_key() != now.metadata_key());
        let buttons = last.is_none_or(|l| l.buttons() != now.buttons());
        let duration = last.is_none_or(|l| l.duration_ms != now.duration_ms);
        let timeline = timeline || metadata || buttons || duration;
        if !(metadata || buttons || timeline) {
            return;
        }
        let t = Instant::now();
        if let Some(smtc) = &mut self.smtc {
            if metadata {
                smtc.set_metadata(&now);
            }
            if buttons {
                smtc.set_buttons(&now);
            }
            if timeline {
                smtc.set_timeline(&now);
            }
        }
        if buttons && let Some(taskbar) = &mut self.taskbar {
            taskbar.set_state(self.hwnd, &now);
        }
        if (metadata || buttons)
            && let Some(tray) = &mut self.tray
        {
            tray.set_tip(&tip(&now));
        }
        log::debug!(
            "sistem: güncellendi ({}{}{}) {:.2} ms",
            if metadata { "bilgi " } else { "" },
            if buttons { "düğmeler " } else { "" },
            if timeline { "konum" } else { "" },
            ms(t)
        );
        self.last = Some(now);
    }

    pub fn on_event(&mut self, event: SystemEvent) {
        match event {
            SystemEvent::TaskbarButtonCreated => {
                if let (Some(taskbar), Some(now)) = (&mut self.taskbar, &self.last) {
                    taskbar.on_button_created(self.hwnd, now);
                } else if let Some(taskbar) = &mut self.taskbar {
                    taskbar.on_button_created(self.hwnd, &NowPlaying::default());
                }
            }
            SystemEvent::TaskbarCreated => {
                if let Some(tray) = &mut self.tray {
                    tray.readd();
                }
            }
            SystemEvent::ThemeChanged => {
                let light = icons::light_theme();
                if let Some(tray) = &mut self.tray {
                    tray.set_theme(light);
                }
                if let Some(taskbar) = &mut self.taskbar {
                    let now = self.last.clone().unwrap_or_default();
                    taskbar.set_theme(self.hwnd, light, &now);
                }
            }
            _ => {}
        }
    }

    /// Çıkarken: tepsi simgesi kalmasın (yoksa fare üstüne gelene kadar
    /// hayalet simge kalıyor), kısayol bırakılsın.
    pub fn shutdown(&mut self) {
        if let Some(tray) = &mut self.tray {
            tray.remove();
        }
        self.hotkey.unregister(self.hwnd);
        if let Some(smtc) = &self.smtc {
            smtc.disable();
        }
    }
}

/// Tepsi simgesinin ipucu: çalan şarkı.
fn tip(now: &NowPlaying) -> String {
    match now.status {
        PlayStatus::Closed => crate::brand::APP_NAME.into(),
        _ if now.title.is_empty() => crate::brand::APP_NAME.into(),
        PlayStatus::Paused => format!("{} · {} ({})", now.title, now.artist, t!(Paused)),
        _ => format!("{} · {}", now.title, now.artist),
    }
}

/// Uygulama kimliği (AUMID): "Openify", sınama koşusunda "Openify.Test".
/// Paketleme (`scripts/surum-hazirla.ps1 --aumid`) kısayola da aynısını yazar.
pub fn app_id() -> &'static str {
    if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
        "Openify.Test"
    } else {
        "Openify"
    }
}

/// Uygulamanın Windows'taki kimliği (görev çubuğu gruplaması, medya
/// kontrolündeki ad). Pencereden önce verilmeli. Sınama koşusunda ayrı bir
/// kimlik: sınama penceresi kullanıcının Openify'ıyla aynı gruba düşmesin.
pub fn set_app_id() {
    use windows::{Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID, core::HSTRING};
    let t = Instant::now();
    let test = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some();
    if test && let Some(hwnd) = foreground_window() {
        TEST_FOREGROUND.store(hwnd.0 as isize, std::sync::atomic::Ordering::Relaxed);
    }
    let id = app_id();
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id)) } {
        log::warn!("Uygulama kimliği verilemedi: {e}");
    }
    log::info!("sistem: uygulama kimliği {:.2} ms", ms(t));
}

/// Sınama koşusu başlarken öndeki pencere (kullanıcınınki).
static TEST_FOREGROUND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Sınama koşusunda pencere açılınca odak kullanıcıya geri verilir: gpui
/// pencereyi gösterirken öne alıyor, betiğin dışarıdan geri vermesi her
/// zaman tutmuyordu. Bir kez döner.
pub fn take_test_foreground() -> Option<HWND> {
    let raw = TEST_FOREGROUND.swap(0, std::sync::atomic::Ordering::Relaxed);
    (raw != 0).then_some(HWND(raw as *mut _))
}

/// Pencereyi gizle (tepsiye).
pub fn hide_window(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Gizlenince: sürecin bütün NT yığınlarını sıkıştır (`HeapCompact`): serbest
/// kalmış ama işletim sistemine geri verilmemiş sayfa aralıkları bırakılır.
/// Dönen değer en büyük serbest bloğun boyu toplamı (bayt, bilgi için).
pub fn compact_heaps() -> usize {
    use windows::Win32::{
        Foundation::HANDLE,
        System::Memory::{GetProcessHeaps, HEAP_FLAGS, HeapCompact},
    };
    let mut heaps = [HANDLE::default(); 96];
    let n = unsafe { GetProcessHeaps(&mut heaps) } as usize;
    let mut largest = 0;
    for heap in &heaps[..n.min(heaps.len())] {
        largest += unsafe { HeapCompact(*heap, HEAP_FLAGS(0)) };
    }
    largest
}

/// Pencereyi göster (küçültülmüşse ya da gizliyse geri getir) ve isteniyorsa
/// öne al. Öne alma yalnız işlem o an izinliyse olur (tepsi tıklaması,
/// kısayol, ikinci açılışın `AllowSetForegroundWindow`'u); olduysa `true`.
pub fn show_window(hwnd: HWND, front: bool) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindowVisible, SW_RESTORE, SW_SHOWNA, SW_SHOWNOACTIVATE, SetForegroundWindow,
        ShowWindow,
    };
    unsafe {
        // Öne alınmayacaksa (sınama) gösterirken de etkinleştirme.
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, if front { SW_RESTORE } else { SW_SHOWNOACTIVATE });
        } else if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
        front && SetForegroundWindow(hwnd).as_bool()
    }
}

/// Pencere ekranda görünür mü (gizli / kapalı değil)?
pub fn is_visible(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// Pencere öndeki pencere mi?
pub fn is_foreground(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, IsWindowVisible};
    unsafe {
        GetForegroundWindow() == hwnd
            && IsWindowVisible(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
    }
}

/// Şu an öndeki pencere (palet kapanınca odak ona geri verilir).
pub fn foreground_window() -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then_some(hwnd)
}

/// Odağı bu pencereye ver (izin varsa).
pub fn set_foreground(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    unsafe { SetForegroundWindow(hwnd).as_bool() }
}

/// Sınama koşusu mu (`OPENIFY_TEST_DISPLAY`)? Sınamada hiçbir pencere öne
/// alınmaz (kullanıcının odağı çalınmaz).
pub fn test_mode() -> bool {
    std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
}

/// Gizli açılmış açılır pencereyi (gpui `show: false`) ekranın bu dikdörtgenine
/// (ekran pikseli: sol, üst, en, boy) yerleştirip etkinleştirmeden gösterir:
/// ilk kare `ShowWindow` içinde çizilir. Sonra `activate` ise öne alır.
///
/// Sıra önemli: öne almak, önceki uygulamanın odağı bırakmasını bekliyor
/// (ölçülen: 50–250 ms); önce gösterip sonra öne alınca palet bu beklemeden
/// önce ekranda. Öne alındıysa `true`.
pub fn show_popup(hwnd: HWND, rect: (i32, i32, i32, i32), activate: bool) -> bool {
    use windows::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{
            GetWindowRect, HWND_TOPMOST, SW_SHOWNA, SWP_NOACTIVATE, SetWindowPos, ShowWindow,
        },
    };
    let (x, y, w, h) = rect;
    prepare_popup(hwnd);
    unsafe {
        // Başka ölçekli bir ekrana taşınınca gpui boyu ölçekle değiştiriyor
        // (WM_DPICHANGED): ikinci yerleştirme aynı ekranda, boy tutar.
        for _ in 0..2 {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
            let mut r = RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);
            if (r.left, r.top, r.right - r.left, r.bottom - r.top) == (x, y, w, h) {
                break;
            }
        }
        let _ = ShowWindow(hwnd, SW_SHOWNA);
    }
    activate && set_foreground(hwnd)
}

/// Açılır pencereyi (tepsi menüsü, her yerden palet) diğer pencerelerin
/// üstüne al ve Windows'un kendi çerçevesini (köşe, kenar, gölge) kapat:
/// panel ve gölgesi bizim çizimimiz, pencerenin geri kalanı saydam.
pub fn prepare_popup(hwnd: HWND) {
    use windows::Win32::{
        Graphics::Dwm::{
            DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
        },
        UI::WindowsAndMessaging::{
            HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        },
    };
    unsafe {
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as _,
            size_of_val(&policy) as u32,
        );
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as _,
            size_of_val(&corner) as u32,
        );
        // DWMWA_COLOR_NONE: kenar çizgisi yok.
        let none: u32 = 0xFFFF_FFFE;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Ekranın bir noktasının bulunduğu ekranın çalışma alanı (görev çubuğu
/// hariç, ekran pikseli), ölçeği (1.0 = 96 dpi) ve gpui'deki sırası.
#[derive(Debug, Clone, Copy)]
pub struct Monitor {
    pub work: (i32, i32, i32, i32),
    pub scale: f32,
    pub index: usize,
}

/// `point` hangi ekrandaysa onun bilgisi; `None` ise fare imlecinin ekranı.
pub fn monitor_at(point: Option<(i32, i32)>) -> Option<Monitor> {
    monitor_from(|| unsafe {
        use windows::Win32::{
            Foundation::POINT,
            Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
            UI::WindowsAndMessaging::GetCursorPos,
        };
        let mut p = POINT::default();
        match point {
            Some((x, y)) => p = POINT { x, y },
            None => {
                let _ = GetCursorPos(&mut p);
            }
        }
        MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
    })
}

/// Pencerenin bulunduğu ekran.
pub fn monitor_of(hwnd: HWND) -> Option<Monitor> {
    monitor_from(|| unsafe {
        use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
    })
}

fn monitor_from(find: impl FnOnce() -> windows::Win32::Graphics::Gdi::HMONITOR) -> Option<Monitor> {
    use windows::{
        Win32::{
            Foundation::{LPARAM, RECT},
            Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
            UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        },
        core::BOOL,
    };
    let monitor = find();
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.ok().ok()?;
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
    // gpui ekranları `EnumDisplayMonitors` sırasıyla numaralıyor.
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(m);
        BOOL::from(true)
    }
    let mut all: Vec<HMONITOR> = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut all as *mut _ as isize),
        )
    };
    let index = all.iter().position(|m| *m == monitor).unwrap_or(0);
    let w = info.rcWork;
    Some(Monitor {
        work: (w.left, w.top, w.right, w.bottom),
        scale: dx as f32 / 96.,
        index,
    })
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.
}
