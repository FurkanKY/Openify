//! "Windows açılınca başlat" (Ayarlar → Başlangıç).
//!
//! Windows'un kullanıcı başına açılış listesi: `HKCU\Software\Microsoft\
//! Windows\CurrentVersion\Run` altında bir değer. Değer yalnız kullanıcı
//! Ayarlar'daki anahtarı çevirince yazılır / silinir; Openify bunu kendiliğinden
//! hiç açmaz.
//!
//! Kayıt defteri `RunKey` ile soyutlandı: testler ve sınama kopyaları
//! (`OPENIFY_TEST_DISPLAY`) bellekteki sahte anahtarı kullanır, gerçek
//! `Run` anahtarına hiç dokunulmaz.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// `Run` anahtarındaki değerin adı.
pub const VALUE_NAME: &str = "Openify";
/// Oturum açılışında verilen bayrak: pencere açılmaz, tepsiye gizli başlar.
pub const TRAY_FLAG: &str = "--tray";

/// Açılış listesinin okunup yazıldığı yer.
pub trait RunKey: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, command: &str) -> Result<(), String>;
    fn remove(&self, name: &str) -> Result<(), String>;
}

/// Açılışta çalıştırılacak komut satırı: tırnaklı yol ve tepsi bayrağı.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" {TRAY_FLAG}", exe.display())
}

/// Komut satırı `--tray` ile mi başlıyor (açılış girdisinden gelen başlatma)?
pub fn started_hidden(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == TRAY_FLAG)
}

pub struct Autostart {
    key: Box<dyn RunKey>,
    exe: PathBuf,
}

impl Autostart {
    pub fn new(key: Box<dyn RunKey>, exe: PathBuf) -> Self {
        Self { key, exe }
    }

    /// Gerçek kayıt defteri; sınama kopyasında (`OPENIFY_TEST_DISPLAY`) ve
    /// Windows dışında bellekteki sahte anahtar.
    pub fn system() -> Self {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("openify.exe"));
        #[cfg(windows)]
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_none() {
            return Self::new(Box::new(WindowsRunKey(RUN_SUBKEY)), exe);
        }
        Self::new(Box::new(MemoryRunKey::default()), exe)
    }

    /// Açılış listesinde Openify var mı?
    pub fn is_enabled(&self) -> bool {
        self.key.get(VALUE_NAME).is_some_and(|c| !c.trim().is_empty())
    }

    /// Anahtarı çevir. Açarken bu exe'nin yolu yazılır (zaten varsa yenilenir).
    pub fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            self.key.set(VALUE_NAME, &command_line(&self.exe))
        } else if self.is_enabled() {
            self.key.remove(VALUE_NAME)
        } else {
            Ok(())
        }
    }
}

/// Bellekte duran sahte anahtar (testler, sınama kopyaları).
#[derive(Default)]
pub struct MemoryRunKey(Mutex<HashMap<String, String>>);

impl RunKey for MemoryRunKey {
    fn get(&self, name: &str) -> Option<String> {
        self.0.lock().ok()?.get(name).cloned()
    }

    fn set(&self, name: &str, command: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(name.into(), command.into());
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.remove(name);
        Ok(())
    }
}

/// Windows'un oturum açılış listesi. Alt anahtar parametre: gerçek kullanımda
/// `Run`, elle çalıştırılan test başka bir alt anahtarda.
#[cfg(windows)]
pub struct WindowsRunKey(pub &'static str);

#[cfg(windows)]
pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
mod registry {
    use windows::{
        Win32::{
            Foundation::ERROR_SUCCESS,
            System::Registry::{
                HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
            },
        },
        core::HSTRING,
    };

    use super::{RunKey, WindowsRunKey};

    impl RunKey for WindowsRunKey {
        fn get(&self, name: &str) -> Option<String> {
            let name = HSTRING::from(name);
            let subkey = HSTRING::from(self.0);
            let mut size = 0u32;
            let probe = unsafe {
                RegGetValueW(HKEY_CURRENT_USER, &subkey, &name, RRF_RT_REG_SZ, None, None, Some(&mut size))
            };
            if probe != ERROR_SUCCESS || size < 2 {
                return None;
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2)];
            let mut size = (buf.len() * 2) as u32;
            let read = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    &subkey,
                    &name,
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr() as *mut _),
                    Some(&mut size),
                )
            };
            if read != ERROR_SUCCESS {
                return None;
            }
            let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..len]))
        }

        fn set(&self, name: &str, command: &str) -> Result<(), String> {
            let name = HSTRING::from(name);
            let subkey = HSTRING::from(self.0);
            let data: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            let r = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &subkey,
                    &name,
                    REG_SZ.0,
                    Some(data.as_ptr() as *const _),
                    (data.len() * 2) as u32,
                )
            };
            if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("kayıt defteri hatası {}", r.0)) }
        }

        fn remove(&self, name: &str) -> Result<(), String> {
            let name = HSTRING::from(name);
            let subkey = HSTRING::from(self.0);
            let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &subkey, &name) };
            // Değer zaten yoksa sorun değil.
            if r == ERROR_SUCCESS || r.0 == 2 { Ok(()) } else { Err(format!("kayıt defteri hatası {}", r.0)) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> Autostart {
        Autostart::new(Box::new(MemoryRunKey::default()), PathBuf::from(r"C:\Openify\current\openify.exe"))
    }

    #[test]
    fn command_is_quoted_and_starts_in_the_tray() {
        assert_eq!(
            command_line(Path::new(r"C:\Program Files\Openify\current\openify.exe")),
            r#""C:\Program Files\Openify\current\openify.exe" --tray"#
        );
    }

    #[test]
    fn toggling_writes_and_removes_the_entry() {
        let a = fake();
        assert!(!a.is_enabled(), "kendiliğinden açık gelmez");
        a.set_enabled(true).unwrap();
        assert!(a.is_enabled());
        assert_eq!(a.key.get(VALUE_NAME).unwrap(), r#""C:\Openify\current\openify.exe" --tray"#);
        // Tekrar açmak zararsız, değeri yeniler.
        a.set_enabled(true).unwrap();
        a.set_enabled(false).unwrap();
        assert!(!a.is_enabled());
        // Kapalıyken kapatmak da zararsız.
        a.set_enabled(false).unwrap();
    }

    #[test]
    fn tray_flag_is_recognised_only_as_an_argument() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(started_hidden(&args(&["openify.exe", "--tray"])));
        assert!(!started_hidden(&args(&["openify.exe"])));
        assert!(!started_hidden(&args(&["--tray"])), "ilk eleman exe yolu");
        assert!(!started_hidden(&args(&["openify.exe", "logout"])));
    }

    /// Sahte anahtarlar birbirinden bağımsız (hiçbiri gerçek kayıt defterine bağlı değil).
    #[test]
    fn fake_keys_are_independent() {
        let (a, b) = (fake(), fake());
        a.set_enabled(true).unwrap();
        assert!(!b.is_enabled());
    }

    /// Windows uygulamasının elle sınaması: gerçek `Run` anahtarına DEĞİL,
    /// bu test için ayrılmış `Software\Openify-sinama` alt anahtarına yazar,
    /// sonra siler. Varsayılan `cargo test`te çalışmaz:
    /// `cargo test windows_run_key -- --ignored`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn windows_run_key_round_trip_in_a_private_subkey() {
        let key = WindowsRunKey(r"Software\Openify-sinama\Run");
        assert_eq!(key.get("Openify"), None);
        key.set("Openify", r#""C:\Test Klasör\openify.exe" --tray"#).unwrap();
        assert_eq!(key.get("Openify").as_deref(), Some(r#""C:\Test Klasör\openify.exe" --tray"#));
        key.remove("Openify").unwrap();
        assert_eq!(key.get("Openify"), None);
        key.remove("Openify").unwrap();
    }
}
