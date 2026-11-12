//! Ses aygıtı: akışın sahibi `Device` (açma, yeniden açma, varsayılan aygıtı
//! izleme) ve Ayarlar'daki aygıt seçimi. Bkz. `mod.rs` başlığı.

use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering::*},
    },
    thread,
    time::{Duration, Instant},
};

use super::{CHANNELS, DEVICE_NONE, OutputCtl, RATE, RELEASE_WAIT, source::RingSource, watch};

/// Varsayılan ses aygıtı var mı (bağlanınca "ses çıkışı yok" demek için).
pub fn has_device() -> bool {
    use cpal::traits::HostTrait;
    cpal::default_host().default_output_device().is_some()
}

/// Akış hatası: aygıt gittiyse (ve bu akış hâlâ etkin kuşaksa) çıkışa söyle.
fn stream_error(
    ctl: Weak<OutputCtl>,
    epoch: u64,
) -> impl FnMut(cpal::StreamError) + Clone + Send + 'static {
    move |e| {
        log::error!("Ses çıkışı hatası: {e}");
        if matches!(e, cpal::StreamError::DeviceNotAvailable)
            && let Some(ctl) = ctl.upgrade()
            && ctl.source_gen.load(SeqCst) == epoch
        {
            ctl.signal_device_lost("akış hatası");
        }
    }
}

/// Ses kartı akışının sahibi; çalar iş parçacığında yaşar (`tap::LevelSink`).
pub(in crate::engine) struct Device {
    ctl: Arc<OutputCtl>,
    stream: Option<rodio::OutputStream>,
    /// Açık akışın Windows aygıt kimliği: varsayılan değişince eskisi hâlâ
    /// etkin mi (kullanıcı değiştirdi) yoksa gitti mi (kulaklık çıktı)?
    id: Option<String>,
    /// Açık akış kullanıcının ayarlardan seçtiği aygıtta mı? O zaman Windows'un
    /// varsayılan aygıtı değişse de akış yerinde kalır.
    pinned: bool,
    /// Gördüğümüz son aygıt seçimi (`PREF_GEN`): değiştiyse yeni aygıta geçilir.
    pref_seen: u64,
    /// Testler: gerçek aygıt yok; açmak yalnız kaynak kuşağını ilerletir.
    headless: bool,
}

impl Device {
    /// O anki varsayılan aygıtı açar (açamazsa `start` yeniden dener).
    pub fn open(ctl: Arc<OutputCtl>) -> Self {
        watch::start();
        let mut device = Self {
            ctl,
            stream: None,
            id: None,
            pinned: false,
            pref_seen: PREF_GEN.load(SeqCst),
            headless: false,
        };
        if let Err(e) = device.reopen() {
            log::error!("Ses çıkışı açılamadı: {e}");
        }
        device
    }

    #[cfg(test)]
    pub fn headless(ctl: Arc<OutputCtl>) -> Self {
        Self {
            ctl,
            stream: None,
            id: None,
            pinned: false,
            pref_seen: PREF_GEN.load(SeqCst),
            headless: true,
        }
    }

    fn is_open(&self) -> bool {
        self.headless || self.stream.is_some()
    }

    /// Çalar `start` (çıkış bekletilirken): akış yoksa, aygıt gittiyse ya da
    /// varsayılan aygıt değiştiyse o anki varsayılan açılır; kuyruk ve konum
    /// olduğu gibi, kaldığı kareden sürer.
    pub fn ensure_for_start(&mut self) -> Result<(), String> {
        let stale = !self.is_open()
            || self.ctl.is_device_lost()
            || PREF_GEN.load(SeqCst) != self.pref_seen
            || (!self.pinned && watch::default_gen() != self.ctl.route_gen.load(SeqCst));
        if !stale {
            return Ok(());
        }
        self.reopen()
            .inspect_err(|_| self.ctl.device_notice.store(DEVICE_NONE, SeqCst))
    }

    /// Çalarken (her yazmada): varsayılan aygıt değişeli `REROUTE_GRACE`
    /// geçtiyse ve eski aygıt hâlâ etkinse (kullanıcı değiştirdi) yeni aygıta
    /// kesintisiz geçilir; eski aygıt gittiyse aygıt kaybı sayılır (yazar
    /// hata döner, çalar duraklar: kulaklık çıkınca hoparlörden çalmasın).
    pub fn follow_default(&mut self) {
        if self.ctl.is_device_lost() {
            return;
        }
        // Kullanıcı ayarlardan başka bir aygıt seçti (ya da "sistem
        // varsayılanı"na döndü): çalarken kesintisiz geçilir.
        if PREF_GEN.load(SeqCst) != self.pref_seen {
            match self.switch() {
                Ok(()) => log::info!("Ses aygıtı ayarlardan değişti: yeni aygıta geçildi"),
                Err(e) => {
                    log::warn!("Seçilen ses aygıtı açılamadı ({e}); eskisinde sürülüyor");
                    self.pref_seen = PREF_GEN.load(SeqCst);
                }
            }
            return;
        }
        // Seçili aygıtta Windows'un varsayılanı önemsenmez.
        if self.pinned || !self.ctl.reroute_due() {
            return;
        }
        let gen_now = watch::default_gen();
        if let Some(id) = &self.id
            && !watch::is_active(id)
        {
            self.ctl.route_gen.store(gen_now, SeqCst);
            self.ctl
                .signal_device_lost("varsayılan aygıt değişti, eskisi artık yok");
            return;
        }
        match self.switch() {
            Ok(()) => log::info!("Varsayılan ses aygıtı değişti: yeni aygıta geçildi"),
            Err(e) => {
                // Eskisi hâlâ çalıyor: orada sür, bir dahaki değişimde yeniden dene.
                log::warn!("Yeni varsayılan aygıt açılamadı ({e}); eskisinde sürülüyor");
                self.ctl.route_gen.store(gen_now, SeqCst);
            }
        }
    }

    /// Eski akışı kapatıp o anki varsayılanı açar. Çıkış bekletilirken ya da
    /// aygıt gitmişken: inişi beklenecek bir şey yok, kaynak kaldığı kareden.
    fn reopen(&mut self) -> Result<(), String> {
        let ctl = Arc::clone(&self.ctl);
        let old = ctl.source_gen.load(SeqCst);
        let epoch = old + 1;
        // Önce eskisi kapanır (geri çağrıları durur), sonra yenisi okur.
        self.stream = None;
        ctl.released_gen.fetch_max(old, SeqCst);
        ctl.held.store(true, SeqCst);
        let route = watch::default_gen();
        let pref = PREF_GEN.load(SeqCst);
        if !self.headless {
            let (stream, id, pinned) = open_stream(&ctl, epoch)?;
            self.stream = Some(stream);
            self.id = id;
            self.pinned = pinned;
        }
        self.pref_seen = pref;
        ctl.route_gen.store(route, SeqCst);
        ctl.source_gen.store(epoch, SeqCst);
        if ctl.device_lost.swap(false, SeqCst) {
            log::info!("Ses çıkışı o anki varsayılan aygıtta yeniden açıldı");
        }
        ctl.trace("reopen");
        Ok(())
    }

    /// Çalarken yeni varsayılan aygıta geçiş: yeni akış bekleme kipinde
    /// açılır (eskisi çalmayı sürdürür), sonra eski kaynak 8 ms'lik inişle
    /// bırakır ve yenisi aynı kareden 8 ms'lik girişle sürer.
    fn switch(&mut self) -> Result<(), String> {
        let ctl = Arc::clone(&self.ctl);
        let old = ctl.source_gen.load(SeqCst);
        let epoch = old + 1;
        let route = watch::default_gen();
        let pref = PREF_GEN.load(SeqCst);
        let (stream, id, pinned) = if self.headless {
            (None, None, false)
        } else {
            let (stream, id, pinned) = open_stream(&ctl, epoch)?;
            (Some(stream), id, pinned)
        };
        ctl.source_gen.store(epoch, SeqCst);
        let t = Instant::now();
        while ctl.released_gen.load(SeqCst) < old && t.elapsed() < RELEASE_WAIT {
            thread::sleep(Duration::from_millis(1));
        }
        // Bırakmadıysa (eski aygıtın geri çağrısı gelmiyor) kapanınca bırakmış sayılır.
        drop(std::mem::replace(&mut self.stream, stream));
        ctl.released_gen.fetch_max(old, SeqCst);
        self.id = id;
        self.pinned = pinned;
        self.pref_seen = pref;
        ctl.route_gen.store(route, SeqCst);
        ctl.trace_arg("reroute", t.elapsed().as_secs_f64() * 1000.0);
        Ok(())
    }
}

/// Varsayılan aygıtı açar, kaynağı `epoch` kuşağıyla karıştırıcıya ekler
/// (etkin kuşaktan büyükse bekleme kipinde). Aygıt / biçim seçimi
/// librespot'un rodio çıkışındakiyle aynı (`create_sink`): önce stereo
/// 44,1 kHz, olmazsa aygıtın varsayılan hızı (rodio doğrusal dönüştürür),
/// o da olmazsa aygıtın varsayılanı; örnek biçimi S16. Çalar iş
/// parçacığında çağrılır (akış o iş parçacığında yaşar). Dönen ikinci
/// değer aygıtın Windows kimliği.
fn open_stream(
    ctl: &Arc<OutputCtl>,
    epoch: u64,
) -> Result<(rodio::OutputStream, Option<String>, bool), String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    // cpal da varsayılanı (eRender / eConsole) açar; arada değişirse
    // `route_gen` farkı bir sonraki yazmada yeniden denetletir.
    let mut id = watch::default_render_id();
    let host = cpal::default_host();
    // Kullanıcı ayarlardan bir aygıt seçtiyse o (bulunamazsa sistem varsayılanı).
    let chosen_device = preferred_device().and_then(|want| {
        let found = host
            .output_devices()
            .ok()?
            .find(|d| d.name().is_ok_and(|n| n == want));
        if found.is_none() {
            log::warn!("Seçilen ses aygıtı bulunamadı ({want}); sistem varsayılanı kullanılıyor");
        }
        found
    });
    let pinned = chosen_device.is_some();
    if pinned {
        id = None;
    }
    let device = match chosen_device {
        Some(device) => device,
        None => host
            .default_output_device()
            .ok_or("varsayılan ses aygıtı yok")?,
    };
    let name = device.name().unwrap_or_else(|_| "[adsız]".into());
    let default_config = device
        .default_output_config()
        .map_err(|e| format!("aygıtın varsayılan biçimi okunamadı: {e}"))?;
    let preferred = device
        .supported_output_configs()
        .map_err(|e| format!("aygıtın biçimleri okunamadı: {e}"))?
        .find(|c| c.channels() == CHANNELS as cpal::ChannelCount)
        .and_then(|c| {
            c.try_with_sample_rate(cpal::SampleRate(RATE))
                .or_else(|| c.try_with_sample_rate(default_config.sample_rate()))
        });
    let chosen = if preferred.is_some() {
        "stereo"
    } else {
        "aygıt varsayılanı"
    };
    let config = preferred.unwrap_or(default_config);

    let mut stream = match rodio::OutputStreamBuilder::default()
        .with_device(device.clone())
        .with_config(&config.config())
        .with_sample_format(cpal::SampleFormat::I16)
        .with_error_callback(stream_error(Arc::downgrade(ctl), epoch))
        .open_stream()
    {
        Ok(stream) => stream,
        Err(e) => {
            log::warn!(
                "Ses çıkışı S16 / {} Hz açılamadı ({e}), aygıtın biçimine düşülüyor",
                config.sample_rate().0
            );
            rodio::OutputStreamBuilder::from_device(device)
                .map_err(|e| format!("{e}"))?
                .with_error_callback(stream_error(Arc::downgrade(ctl), epoch))
                .open_stream_or_fallback()
                .map_err(|e| format!("{e}"))?
        }
    };
    stream.log_on_drop(false);
    let c = stream.config();
    log::info!("Ses aygıtı: {name}");
    log::debug!(
        "Ses çıkışı açıldı: aygıt \"{name}\", biçim {:?}, {} Hz, {} kanal ({chosen}), aygıt tamponu {:?}, kuyruk hedefi {} ms, aygıt gecikmesi (tahmin) {} ms",
        c.sample_format(),
        c.sample_rate(),
        c.channel_count(),
        c.buffer_size(),
        ctl.target_frames() * 1000 / RATE as u64,
        ctl.device_ns / 1_000_000,
    );
    stream
        .mixer()
        .add(RingSource::with_gen(Arc::clone(ctl), epoch));
    Ok((stream, id, pinned))
}

// ---------------------------------------------------------------------------
// Aygıt seçimi (Ayarlar → Ses → Çıkış aygıtı)
// ---------------------------------------------------------------------------

/// Kullanıcının seçtiği çıkış aygıtının adı (`None`: sistem varsayılanı).
static PREFERRED_DEVICE: Mutex<Option<String>> = Mutex::new(None);
/// Seçim her değiştiğinde artar: çalar iş parçacığı bunu görünce aygıtı değiştirir.
static PREF_GEN: AtomicU64 = AtomicU64::new(0);

fn preferred_device() -> Option<String> {
    PREFERRED_DEVICE.lock().ok().and_then(|p| p.clone())
}

/// Çıkış aygıtını seç (`None`: sistem varsayılanını izle). Çalıyorsa bir
/// sonraki yazmada kesintisiz geçilir; durmuşsa Çal'a basınca.
pub fn set_preferred_device(name: Option<String>) {
    let name = name.filter(|n| !n.trim().is_empty());
    if let Ok(mut preferred) = PREFERRED_DEVICE.lock() {
        if *preferred == name {
            return;
        }
        *preferred = name;
    }
    PREF_GEN.fetch_add(1, SeqCst);
}

/// Bilinen çıkış aygıtlarının adları, tekrarsız. Windows'ta birkaç yüz ms
/// sürebilir: arayüz iş parçacığında çağrılmaz.
pub fn output_device_names() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let mut names = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for name in devices.filter_map(|d| d.name().ok()) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Sistemin varsayılan çıkış aygıtının adı.
pub fn default_output_device_name() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}
