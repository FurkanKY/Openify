//! Varsayılan çıkış aygıtı bildirimleri (Windows: IMMNotificationClient;
//! diğer sistemlerde boş kalıplar).

#[cfg(windows)]
mod imp {
    //! Windows'un "varsayılan çıkış aygıtı değişti" bildirimini sayar. Karar
    //! (geç / duraklat) çalar iş parçacığında, `Device::follow_default`'ta;
    //! burası yalnız sayaç ve an tutar (geri çağrıda iş yapılmaz, COM
    //! belgesinin istediği gibi).

    use std::{
        sync::{
            OnceLock,
            atomic::{AtomicU64, Ordering::SeqCst},
        },
        time::{Duration, Instant},
    };

    use windows::{
        Win32::{
            Foundation::PROPERTYKEY,
            Media::Audio::{
                DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
                IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
            },
            System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree},
        },
        core::{PCWSTR, Result, implement},
    };

    static DEFAULT_GEN: AtomicU64 = AtomicU64::new(0);
    static DEFAULT_AT_MS: AtomicU64 = AtomicU64::new(0);
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    static STARTED: OnceLock<()> = OnceLock::new();

    fn now_ms() -> u64 {
        EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    /// Varsayılan çıkış aygıtı kaç kez değişti.
    pub fn default_gen() -> u64 {
        DEFAULT_GEN.load(SeqCst)
    }

    pub fn since_default_change() -> Duration {
        Duration::from_millis(now_ms().saturating_sub(DEFAULT_AT_MS.load(SeqCst)))
    }

    /// Bildirimleri bir kez, kendi iş parçacığında dinlemeye başlar.
    pub fn start() {
        STARTED.get_or_init(|| {
            EPOCH.get_or_init(Instant::now);
            let _ = std::thread::Builder::new().name("openify-audio-devices".into()).spawn(|| {
                if let Err(e) = listen() {
                    log::warn!("Ses aygıtı bildirimleri alınamıyor ({e}); varsayılan aygıt değişimi izlenmeyecek");
                }
            });
        });
    }

    fn listen() -> Result<()> {
        // SAFETY: bu iş parçacığına ait COM ilklemesi; kayıt ömür boyu sürer.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let client: IMMNotificationClient = Client.into();
            enumerator.RegisterEndpointNotificationCallback(&client)?;
            // Bildirimler COM'un iş parçacıklarından gelir; burası yalnız
            // kaydı (sayıcı ve istemci) canlı tutar.
            loop {
                std::thread::park();
            }
        }
    }

    #[implement(IMMNotificationClient)]
    struct Client;

    impl IMMNotificationClient_Impl for Client_Impl {
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
            Ok(())
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
            Ok(())
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
            Ok(())
        }
        fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
            // cpal'ın açtığı rol (eConsole); her rol ayrı bildirilir.
            if flow == eRender && role == eConsole {
                DEFAULT_AT_MS.store(now_ms(), SeqCst);
                DEFAULT_GEN.fetch_add(1, SeqCst);
            }
            Ok(())
        }
        fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
            Ok(())
        }
    }

    fn enumerator() -> Option<IMMDeviceEnumerator> {
        // SAFETY: cpal'ın bu iş parçacığında kullandığı kiple (STA) aynı;
        // zaten ilklenmişse sayaç artar, zararsız.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()
        }
    }

    /// Varsayılan çıkış aygıtının kimliği (cpal'ın açtığı: eRender / eConsole).
    pub fn default_render_id() -> Option<String> {
        let e = enumerator()?;
        // SAFETY: dönen dize COM belleğinde; kopyalanıp bırakılır.
        unsafe {
            let id = e.GetDefaultAudioEndpoint(eRender, eConsole).ok()?.GetId().ok()?;
            let s = id.to_string().ok();
            CoTaskMemFree(Some(id.0 as *const _));
            s
        }
    }

    /// Bu kimlikli aygıt hâlâ etkin mi (takılı, açık)?
    pub fn is_active(id: &str) -> bool {
        let Some(e) = enumerator() else { return false };
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` çağrı boyunca yaşar, sıfırla biter.
        unsafe {
            e.GetDevice(PCWSTR(wide.as_ptr()))
                .and_then(|d| d.GetState())
                .is_ok_and(|s| s == DEVICE_STATE_ACTIVE)
        }
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
mod imp {
    use std::time::Duration;

    pub fn start() {}
    pub fn default_gen() -> u64 {
        0
    }
    pub fn since_default_change() -> Duration {
        Duration::ZERO
    }
    pub fn default_render_id() -> Option<String> {
        None
    }
    pub fn is_active(_: &str) -> bool {
        true
    }
}

#[cfg(not(windows))]
pub use imp::*;
