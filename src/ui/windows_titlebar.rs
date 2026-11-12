//! Windows başlık çubuğunu Openify'ın koyu arka planıyla birleştirir.
//!
//! gpui başlık çubuğunu sistem temasına göre açık ya da koyu çiziyor. Windows
//! açık temadayken Openify'ın koyu içeriğinin üstünde beyaz bir şerit kalıyor.
//! Burada DWM'e (Windows'un pencere yöneticisi) başlık çubuğunu koyu ve
//! uygulamanın arka plan rengiyle aynı çizmesini söylüyoruz.

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{COLORREF, HWND},
    Graphics::Dwm::{
        DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    },
};
use windows::core::BOOL;

use super::theme::{BG, TEXT};

/// gpui penceresinin Windows tutamacı. (`display_handle` gpui'de yazılmamış,
/// çağrılırsa çöker; yalnız pencere tutamacı okunur.)
pub fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(HWND(win32.hwnd.get() as *mut _))
}

pub fn apply(window: &Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };

    let dark = BOOL::from(true);
    let caption = colorref(BG);
    let text = colorref(TEXT);

    // Başarısız olursa (örneğin Windows 10'da renk ayarı yok) pencere yine
    // çalışır, sadece başlık çubuğu varsayılan kalır.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            size_of::<BOOL>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const _ as _,
            size_of::<COLORREF>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text as *const _ as _,
            size_of::<COLORREF>() as u32,
        );
    }
}

/// 0xRRGGBB → Windows'un 0x00BBGGRR biçimi.
fn colorref(rgb: u32) -> COLORREF {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    COLORREF(r | (g << 8) | (b << 16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_swaps_red_and_blue() {
        assert_eq!(colorref(0x112233).0, 0x00332211);
    }
}
