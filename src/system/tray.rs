//! Tepsi simgesi (`Shell_NotifyIconW`).
//!
//! Simge ana pencereye bağlı: tıklamalar `hook::WM_TRAY` iletisiyle o
//! pencerenin alt sınıf işleyicisine gelir. Explorer yeniden başlarsa
//! ("TaskbarCreated") simge yeniden eklenir. Menü Windows'un menüsü değil,
//! uygulamanın kendi açılır penceresi (`ui::popups`).

use windows::Win32::{
    Foundation::HWND,
    UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
    },
};

use super::{hook::WM_TRAY, icons};

const TRAY_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    icon: icons::Icon,
    tip: String,
    pub added: bool,
}

impl Tray {
    pub fn add(hwnd: HWND, light: bool) -> Option<Self> {
        let size = icons::small_icon_size();
        let icon = icons::render(icons::tray_svg(size), size, icons::ink(light))?;
        let mut tray = Tray { hwnd, icon, tip: crate::brand::APP_NAME.into(), added: false };
        tray.readd();
        tray.added.then_some(tray)
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon.0,
            ..Default::default()
        };
        copy_wide(&mut data.szTip, &self.tip);
        data
    }

    /// Ekle (Explorer yeniden başladıysa yeniden).
    pub fn readd(&mut self) {
        let mut data = self.data();
        unsafe {
            // Önceki hâli kalmışsa önce kaldır (yeniden eklemede hata vermesin).
            if self.added {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
            if self.added {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            } else {
                log::warn!("Tepsi simgesi eklenemedi");
            }
        }
    }

    pub fn set_tip(&mut self, tip: &str) {
        if self.tip == tip {
            return;
        }
        self.tip = tip.to_string();
        if self.added {
            let data = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }

    pub fn set_theme(&mut self, light: bool) {
        let size = icons::small_icon_size();
        if let Some(icon) = icons::render(icons::tray_svg(size), size, icons::ink(light)) {
            self.icon = icon;
            if self.added {
                let data = self.data();
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
                }
            }
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let data = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

/// Metni sabit uzunluklu UTF-16 alana yazar (sığmazsa "…" ile kısaltır).
pub fn copy_wide(dst: &mut [u16], text: &str) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let max = dst.len() - 1;
    let n = if wide.len() > max {
        dst[..max - 1].copy_from_slice(&wide[..max - 1]);
        dst[max - 1] = '…' as u16;
        max
    } else {
        dst[..wide.len()].copy_from_slice(&wide);
        wide.len()
    };
    dst[n] = 0;
}

#[cfg(test)]
mod tests {
    use super::copy_wide;

    #[test]
    fn long_tip_is_cut_with_ellipsis() {
        let mut buf = [0u16; 8];
        copy_wide(&mut buf, "Openify çalıyor");
        let s = String::from_utf16(&buf[..buf.iter().position(|c| *c == 0).unwrap()]).unwrap();
        assert_eq!(s, "Openif…");
        let mut buf = [1u16; 8];
        copy_wide(&mut buf, "abc");
        assert_eq!(&buf[..4], &[b'a' as u16, b'b' as u16, b'c' as u16, 0]);
    }
}
