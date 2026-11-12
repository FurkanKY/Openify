//! Kendi SVG simgelerimizden Windows simgesi (HICON).
//!
//! Görev çubuğu önizleme düğmeleri ve tepsi simgesi uygulamanın simgeleriyle
//! aynı çizgide olsun diye `assets/icons`'taki SVG'ler istenen boyutta (ekranın
//! ölçeğinde) çizilir. Renk tek: simgenin kapsaması saydamlık olur, renk
//! uygulamanın ön plan rengi (açık temada gece odası rengi).

use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
            DeleteObject, GetDC, HGDIOBJ, ReleaseDC,
        },
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO},
    },
    core::w,
};

/// Uygulamanın ön plan rengi (DESIGN.md `foreground`).
const FOREGROUND: u32 = 0xF4F4F5;
/// Gece odası (DESIGN.md `night-room`): açık temada simge rengi.
const NIGHT_ROOM: u32 = 0x0E0E10;

/// Tepsi işareti (logo, karosuz "çentikte nokta"; bkz. DESIGN.md "Logo"):
/// ana çizim ve 16 px'e elle oturtulmuş hâli. Yalnız saydamlığı okunur, renk
/// temadan gelir (`ink`); `assets/logo/tray-*.ico` aynı SVG'lerden üretildi.
pub const TRAY: &[u8] = include_bytes!("../../assets/logo/tray-dark.svg");
pub const TRAY_16: &[u8] = include_bytes!("../../assets/logo/tray-dark-16.svg");
pub const PLAY: &[u8] = include_bytes!("../../assets/icons/play-fill.svg");
pub const PAUSE: &[u8] = include_bytes!("../../assets/icons/pause-fill.svg");
pub const PREVIOUS: &[u8] = include_bytes!("../../assets/icons/skip-back-fill.svg");
pub const NEXT: &[u8] = include_bytes!("../../assets/icons/skip-forward-fill.svg");

/// Görev çubuğu (ve önizlemesi) açık temada mı?
pub fn light_theme() -> bool {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    r.is_ok() && value == 1
}

/// Temaya uygun simge rengi.
pub fn ink(light: bool) -> u32 {
    if light { NIGHT_ROOM } else { FOREGROUND }
}

/// Simgenin kenar boyu (piksel): Windows'un küçük simge ölçüsü, görev
/// çubuğunun bulunduğu ana ekranın ölçeğinde.
pub fn small_icon_size() -> u32 {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
        UI::{
            HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::SM_CXSMICON,
        },
    };
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        GetSystemMetricsForDpi(SM_CXSMICON, dx).max(16) as u32
    }
}

/// Sahip olunan simge: düşünce silinir.
pub struct Icon(pub HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { DestroyIcon(self.0) };
        }
    }
}

/// Tepsi simgesi: 16 px'te elle oturtulmuş çizim (ana çizim küçültülünce
/// noktayla halka arasındaki çentik kayboluyordu), diğer boyutlarda ana çizim.
pub fn tray_svg(size: u32) -> &'static [u8] {
    if size == 16 { TRAY_16 } else { TRAY }
}

/// Pencerenin büyük ve küçük simgesini, exe'deki uygulama simgesinden
/// (kaynak kimliği 1, `assets/logo/openify.rc`) pencerenin ölçeğine uygun
/// boyutta verir. gpui sınıf simgesini tek boyutta (32 px) yüklüyor;
/// görev çubuğu ve Alt+Tab o zaman küçültülmüş bulanık simge gösteriyordu.
/// Simgeler süreç boyunca yaşar (pencere onları kullanır), bırakılmaz.
pub fn set_window_icons(hwnd: HWND) {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::{
                ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON,
                SM_CXSMICON, SendMessageW, WM_SETICON,
            },
        },
    };
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetricsForDpi(metric, dpi);
            match LoadImageW(
                Some(module.into()),
                windows::core::PCWSTR(1 as _),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            ) {
                Ok(icon) => {
                    SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(which as usize)),
                        Some(LPARAM(icon.0 as isize)),
                    );
                }
                Err(e) => log::warn!("uygulama simgesi yüklenemedi ({size} px): {e}"),
            }
        }
    }
}

/// SVG'yi `size`×`size` piksel, tek renk simgeye çizer.
pub fn render(svg: &[u8], size: u32, color: u32) -> Option<Icon> {
    let alpha = rasterize(svg, size)?;
    to_hicon(&alpha, size, color)
}

/// SVG'nin kapsaması (0–255), satır satır.
pub fn rasterize(svg: &[u8], size: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default())
        .inspect_err(|e| log::warn!("simge okunamadı: {e}"))
        .ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.pixels().iter().map(|p| p.alpha()).collect())
}

fn to_hicon(alpha: &[u8], size: u32, color: u32) -> Option<Icon> {
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            // Eksi: satırlar yukarıdan aşağı.
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let screen = GetDC(None::<HWND>);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color_bitmap =
            CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None::<HWND>, screen);
        let color_bitmap = color_bitmap.ok()?;
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(color_bitmap.0));
            return None;
        }
        // BGRA, düz (önceden çarpılmamış) saydamlık: tek renk simgede renk
        // her pikselde aynı, kapsama saydamlığa gider.
        let out = std::slice::from_raw_parts_mut(bits as *mut u8, alpha.len() * 4);
        for (i, a) in alpha.iter().enumerate() {
            out[i * 4] = b;
            out[i * 4 + 1] = g;
            out[i * 4 + 2] = r;
            out[i * 4 + 3] = *a;
        }
        let mask_bits = vec![0u8; (size.div_ceil(16) * 2 * size) as usize];
        let mask = CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            Some(mask_bits.as_ptr() as *const _),
        );
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color_bitmap,
        });
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(color_bitmap.0));
        icon.ok().map(Icon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_rasterize_with_visible_ink() {
        for svg in [TRAY, PLAY, PAUSE, PREVIOUS, NEXT] {
            for size in [16, 20, 24, 32] {
                let svg = if svg == TRAY { tray_svg(size) } else { svg };
                let alpha = rasterize(svg, size).expect("simge çizilmeli");
                assert_eq!(alpha.len(), (size * size) as usize);
                let inked = alpha.iter().filter(|a| **a > 128).count();
                // Simge boş değil ama kareyi de doldurmuyor.
                assert!(inked > (size * size / 20) as usize, "{size}px: {inked}");
                assert!(inked < (size * size * 3 / 4) as usize, "{size}px: {inked}");
            }
        }
    }

    /// Görsel denetim: simgeleri koyu ve açık görev çubuğu zemininde, 4 kat
    /// büyütülmüş olarak `%TEMP%\openify-ekran\simgeler.png`'ye yazar.
    /// `cargo test --release -- --ignored dump_icons`
    #[test]
    #[ignore]
    fn dump_icons() {
        let size = small_icon_size();
        let icons = [tray_svg(size), PREVIOUS, PLAY, PAUSE, NEXT];
        let zoom = 4u32;
        let cell = size * zoom + 8;
        let mut out = image::RgbaImage::new(cell * icons.len() as u32, cell * 2);
        for (row, (light, bg)) in [(false, 0x202020u32), (true, 0xF3F3F3u32)]
            .iter()
            .enumerate()
        {
            let fg = ink(*light);
            for (col, svg) in icons.iter().enumerate() {
                let alpha = rasterize(svg, size).unwrap();
                for y in 0..cell {
                    for x in 0..cell {
                        let (ix, iy) = (x.wrapping_sub(4) / zoom, y.wrapping_sub(4) / zoom);
                        let a = if x >= 4 && y >= 4 && ix < size && iy < size {
                            alpha[(iy * size + ix) as usize] as u32
                        } else {
                            0
                        };
                        let mixc = |s: u32| -> u8 {
                            let (f, b) = ((fg >> s) & 0xFF, (bg >> s) & 0xFF);
                            ((f * a + b * (255 - a)) / 255) as u8
                        };
                        out.put_pixel(
                            col as u32 * cell + x,
                            row as u32 * cell + y,
                            image::Rgba([mixc(16), mixc(8), mixc(0), 255]),
                        );
                    }
                }
            }
        }
        let dir = std::env::temp_dir().join("openify-ekran");
        std::fs::create_dir_all(&dir).unwrap();
        out.save(dir.join("simgeler.png")).unwrap();
        println!("{size} px simgeler: {}", dir.join("simgeler.png").display());
    }

    #[test]
    fn ink_follows_theme() {
        assert_eq!(ink(false), FOREGROUND);
        assert_eq!(ink(true), NIGHT_ROOM);
    }
}
