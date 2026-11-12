//! Ana pencerenin gpui'nin işlemediği iletileri.
//!
//! gpui bilmediği iletileri (WM_HOTKEY, WM_COMMAND, tepsi geri çağrısı,
//! "TaskbarButtonCreated") Windows'un varsayılan işleyicisine bırakıyor.
//! Pencereyi alt sınıflayıp (`SetWindowSubclass`) bunları önce biz görüyoruz.
//! İşleyici yalnız olayı kanala bırakır ve döner: gpui o sırada kendi
//! durumunu ödünç almış olabilir, burada arayüze dokunulmaz.

use std::sync::OnceLock;

use windows::{
    Win32::{
        Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{
                HOT_KEY_MODIFIERS, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
                UnregisterHotKey, VK_SPACE,
            },
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                ChangeWindowMessageFilterEx, MSGFLT_ALLOW, RegisterWindowMessageW, WM_APP,
                WM_COMMAND, WM_CONTEXTMENU, WM_HOTKEY, WM_SETTINGCHANGE, WM_USER,
            },
        },
    },
    core::{PCWSTR, w},
};

use super::{MediaButton, SystemEvent, emit};
use crate::{t};

/// Tepsi simgesinin geri çağrı iletisi.
pub const WM_TRAY: u32 = WM_APP + 0x51;
/// Görev çubuğu önizleme düğmelerinin kimlikleri.
pub const THUMB_PREVIOUS: u32 = 0x4F01;
pub const THUMB_PLAY: u32 = 0x4F02;
pub const THUMB_NEXT: u32 = 0x4F03;

const SUBCLASS_ID: usize = 0x4F50_454E;
const HOTKEY_ID: i32 = 0x4F50;
/// THBN_CLICKED: önizleme düğmesine tıklandı (WM_COMMAND'ın üst sözcüğü).
const THBN_CLICKED: u32 = 0x1800;
/// Tepsi simgesi (NOTIFYICON_VERSION_4): sol tık ve klavyeyle seçim.
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;

static TASKBAR_BUTTON_CREATED: OnceLock<u32> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

/// Her yerden palet kısayolunun adı (kullanıcıya gösterilen).
pub fn hotkey_label() -> &'static str {
    t!(HotkeyLabel)
}

pub fn install(hwnd: HWND) {
    let button = *TASKBAR_BUTTON_CREATED
        .get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) });
    let created =
        *TASKBAR_CREATED.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
    unsafe {
        // Openify yönetici olarak çalışırsa Explorer'ın iletileri süzülür.
        for msg in [button, created, WM_COMMAND] {
            let _ = ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, None);
        }
        if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0).as_bool() {
            log::warn!("Pencere alt sınıflanamadı: medya düğmeleri ve tepsi çalışmayacak");
        }
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => {
            emit(SystemEvent::Hotkey);
            return LRESULT(0);
        }
        WM_COMMAND if ((wparam.0 >> 16) & 0xFFFF) as u32 == THBN_CLICKED => {
            let button = match (wparam.0 & 0xFFFF) as u32 {
                THUMB_PREVIOUS => Some(MediaButton::Previous),
                THUMB_PLAY => Some(MediaButton::Toggle),
                THUMB_NEXT => Some(MediaButton::Next),
                _ => None,
            };
            if let Some(button) = button {
                emit(SystemEvent::Media(button));
                return LRESULT(0);
            }
        }
        WM_TRAY => {
            match (lparam.0 & 0xFFFF) as u32 {
                NIN_SELECT | NIN_KEYSELECT => emit(SystemEvent::Show),
                WM_CONTEXTMENU => {
                    // Sürüm 4: imlecin yeri wParam'da (ekran pikseli).
                    let x = (wparam.0 & 0xFFFF) as u16 as i16 as i32;
                    let y = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                    emit(SystemEvent::TrayMenu { x, y });
                }
                _ => {}
            }
            return LRESULT(0);
        }
        WM_SETTINGCHANGE if is_color_set_change(lparam) => emit(SystemEvent::ThemeChanged),
        _ if Some(&msg) == TASKBAR_BUTTON_CREATED.get() => emit(SystemEvent::TaskbarButtonCreated),
        _ if Some(&msg) == TASKBAR_CREATED.get() => emit(SystemEvent::TaskbarCreated),
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Açık / koyu tema değişimi "ImmersiveColorSet" ayar değişimiyle gelir.
fn is_color_set_change(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let name = unsafe { PCWSTR(lparam.0 as *const u16).to_string() };
    name.is_ok_and(|n| n == "ImmersiveColorSet")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    Off,
    Registered,
    /// Başka bir uygulama tutuyor.
    Taken,
    Failed,
}

impl Hotkey {
    pub fn register(hwnd: HWND) -> Self {
        // Ctrl+Alt+Boşluk bu makinede başka bir uygulamaya kayıtlıydı (hata 1409);
        // kullanıcı Ctrl+Shift+Boşluk'u seçti (7 Kasım).
        let mods: HOT_KEY_MODIFIERS = MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
        match unsafe { RegisterHotKey(Some(hwnd), HOTKEY_ID, mods, VK_SPACE.0 as u32) } {
            Ok(()) => Hotkey::Registered,
            Err(e) => {
                // ERROR_HOTKEY_ALREADY_REGISTERED (1409)
                let taken = unsafe { GetLastError() }.0 == 1409 || e.code().0 as u32 == 0x8007_0581;
                log::warn!(
                    "{} kaydedilemedi ({}): {e}",
                    hotkey_label(),
                    if taken { "başka bir uygulama kullanıyor" } else { "bilinmeyen hata" }
                );
                if taken { Hotkey::Taken } else { Hotkey::Failed }
            }
        }
    }

    pub fn unregister(&mut self, hwnd: HWND) {
        if *self == Hotkey::Registered {
            let _ = unsafe { UnregisterHotKey(Some(hwnd), HOTKEY_ID) };
            *self = Hotkey::Off;
        }
    }
}
