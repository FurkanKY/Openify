//! Spotify Connect: arayüz sözleşmesi, Spirc yaşam döngüsü ve çalara bağlama
//! (PLAN-MOTOR-CONNECT.md §5 7.4, §7). Sürücü mantığı `playback/connect.rs`te.
//!
//! **Varsayılan AÇIK** (13 Kasım'dan beri; `Settings::connect_enabled`): kapalıyken Spirc hiç
//! kurulmaz, oturum bugünkü gibi rastgele aygıt kimliğiyle açılır, hiçbir
//! Spotify isteği, olay ya da çalar komutu değişmez. Açıkken:
//!
//! - oturum KALICI aygıt kimliğiyle açılır (`apply_connect_identity`; telefonun
//!   listesinde hep aynı aygıt, hayalet yok);
//! - her takılan oturum için BİR Spirc (`Spirc::with_session`, `openify-net`
//!   çalışma ortamında; dealer kurucusu tek kullanımlık): oturum değişince
//!   (`Install::Replace`) eskisi bırakılır, yenisi kurulur;
//! - Spirc'in çaları sürmesi `Driver::Spirc` ile (bkz. `playback/connect.rs`).
//!
//! Parçalar:
//! - `ConnectPort`: motorun Spirc'ten istedikleri (sahte uç `connect/tests.rs`te);
//!   gerçek uç `SpircLink`.
//! - `OutMixer`: Spirc'in ses düzeyi karıştırıcısı; telefondan gelen düzeyi
//!   doğrudan çıkışa uygular (`VolumeChanged` olayını beklemez).
//! - `ConnectHost`: Spirc'in yaşam döngüsü kararları (ağsız, birim sınanır).
//! - `Engine`in `impl`i: komutlar, Spirc'i başlatma (arka planda), olaylar.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use librespot::{
    connect::{ConnectConfig, Spirc, SpircEvent},
    core::{Error, config::DeviceType, session::Session},
    playback::{
        mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
        player::Player,
    },
};
use tokio::task::JoinHandle;

use super::{Engine, Internal, OutputCtl};

// ── Arayüz sözleşmesi (PLAN §7) ─────────────────────────────────────────

/// Arayüzün Connect komutları (`Command::Connect`).
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectCommand {
    /// Ayarlardan aç / kapat. Açınca (gerekirse oturum yenilenip) Spirc
    /// başlar; kapatınca durur, telefon bu aygıtı listede görmez.
    SetEnabled(bool),
    /// Telefonun listesinde görünen ad. Çalışan Spirc yeni adla yeniden
    /// kurulur (oturum yenilenir).
    SetName(String),
    /// "Buraya aktar": başka aygıtta çalan oturumu bu aygıta al.
    TransferHere,
}

/// Çalmayı kim yürütüyor (bkz. `playback/driver.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectDriver {
    /// Openify'ın kendi çalması (varsayılan; Connect kapalıyken hep bu).
    #[default]
    Local,
    /// Telefon (Spotify) bu aygıtta çalıyor.
    Spirc,
    /// Başka bir aygıtta çalıyor; bu aygıtta çalan yok.
    Remote,
}

/// Başka aygıtta çalan oturum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNow {
    /// Çalan aygıtın görünen adı ("Pixel 8").
    pub device_name: String,
}

/// `PlaybackEvent::Connect`: Connect durumu; yalnız değişince gelir.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConnectStatus {
    pub driver: ConnectDriver,
    /// Spotify'da görünen adımız; boş: Connect kapalı ya da henüz hazır değil.
    pub device_name: String,
    /// `driver == Remote` iken çalan aygıt.
    pub remote: Option<RemoteNow>,
}

// ── Kalıcı kimlik ───────────────────────────────────────────────────────

/// Oturumun kullanacağı kalıcı aygıt kimliği; `Some`: Connect AÇIK.
static SESSION_ID: Mutex<Option<String>> = Mutex::new(None);

/// Arayüz, ayarlar yüklenince (motor başlamadan) ve her değişiklikte çağırır:
/// Connect açıksa oturumlar `device_id` ile açılır (telefonun listesinde hep
/// aynı aygıt), kapalıysa bugünkü gibi rastgele kimlikle. Çalışan oturumu
/// değiştirmez; motor `Command::Connect(SetEnabled)` ile haberdar edilir.
pub fn apply_connect_identity(enabled: bool, device_id: &str) {
    let id = (enabled && !device_id.is_empty()).then(|| device_id.to_string());
    *SESSION_ID.lock().unwrap_or_else(|e| e.into_inner()) = id;
}

/// Oturum açılırken kullanılacak kimlik (`None`: bugünkü rastgele kimlik).
pub(super) fn session_device_id() -> Option<String> {
    SESSION_ID.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// ── Motorun Spirc'ten istedikleri ───────────────────────────────────────

/// Çalışan Spirc'e komutlar. Hepsi beklemeden döner. Çalar komutu YOKTUR:
/// Spirc çalara kendi komutlarını verir.
pub(in crate::engine) trait ConnectPort: Send + Sync {
    /// Ana uygulama çalmayı kendisi yürütüyor: Spirc çalar olaylarına karışmasın.
    fn set_host_drives(&self, on: bool);
    /// "Buraya aktar": Spotify'dan çalmayı bu aygıta ver (`Spirc::transfer`).
    fn transfer_here(&self);
    /// Etkin aygıtlıktan çık, çalara dokunma (`disconnect(false)`).
    fn leave(&self);
    fn play(&self);
    fn pause(&self);
    fn seek(&self, position_ms: u32);
    fn next(&self);
    fn prev(&self);
    /// 0..=65535.
    fn set_volume(&self, volume: u16);
    /// Telefonun sırasının sonuna ekle (P4).
    fn add_to_queue(&self, uri: &str);
    /// Spirc'i kapat (Spotify'dan aygıtı siler; etkinse duraklatır).
    fn shutdown(&self);
    /// Spirc görevinin tutamağı (kapanışı beklemek için; bir kez alınır).
    fn join_handle(&self) -> Option<JoinHandle<()>> {
        None
    }
}

/// Gerçek uç: librespot'un Spirc'i.
pub(in crate::engine) struct SpircLink {
    spirc: Spirc,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl ConnectPort for SpircLink {
    fn set_host_drives(&self, on: bool) {
        self.spirc.set_host_drives(on);
    }

    fn transfer_here(&self) {
        let _ = self.spirc.transfer(None);
    }

    fn leave(&self) {
        let _ = self.spirc.disconnect(false);
    }

    fn play(&self) {
        let _ = self.spirc.play();
    }

    fn pause(&self) {
        let _ = self.spirc.pause();
    }

    fn seek(&self, position_ms: u32) {
        let _ = self.spirc.set_position_ms(position_ms);
    }

    fn next(&self) {
        let _ = self.spirc.next();
    }

    fn prev(&self) {
        let _ = self.spirc.prev();
    }

    fn set_volume(&self, volume: u16) {
        let _ = self.spirc.set_volume(volume);
    }

    fn add_to_queue(&self, uri: &str) {
        let _ = self.spirc.add_to_queue(uri);
    }

    fn shutdown(&self) {
        let _ = self.spirc.shutdown();
    }

    fn join_handle(&self) -> Option<JoinHandle<()>> {
        self.task.lock().ok()?.take()
    }
}

/// Spirc'in ses düzeyi karıştırıcısı: Deste ile AYNI yazılım düzeyini
/// paylaşır; Spirc ses düzeyini değiştirince (telefondan) çıkışın kazancını
/// hemen ayarlar. (Spike'ta `VolumeChanged` olayını bekliyorduk; olay
/// çıkış kapısı için ayrılmıştı, bkz. `output/mod.rs`.)
pub(in crate::engine) struct OutMixer {
    soft: SoftMixer,
    out: Arc<OutputCtl>,
}

impl OutMixer {
    pub fn new(soft: SoftMixer, out: Arc<OutputCtl>) -> Self {
        Self { soft, out }
    }
}

impl Mixer for OutMixer {
    fn open(_: MixerConfig) -> Result<Self, Error> {
        Err(Error::unimplemented("OutMixer is built by the engine only"))
    }

    fn volume(&self) -> u16 {
        self.soft.volume()
    }

    fn set_volume(&self, volume: u16) {
        self.soft.set_volume(volume);
        self.out.set_gain(self.soft.get_soft_volume().attenuation_factor() as f32);
    }

    fn get_soft_volume(&self) -> Box<dyn librespot::playback::mixer::VolumeGetter + Send> {
        self.soft.get_soft_volume()
    }
}

// ── Yaşam döngüsü kararları (ağsız) ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Life {
    Off,
    /// Kuruluyor (arka planda `with_session`).
    Starting(u64),
    Running(u64),
}

/// `ConnectHost::decide`in kararı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::engine) enum Decision {
    Nothing,
    /// Bu sıra numarasıyla Spirc'i kur.
    Launch(u64),
    /// Çalışan / kurulan Spirc'i bırak.
    Stop,
    /// Oturum kalıcı kimlikle açılmamış: yeniden bağlanıp tekrar bak.
    RestartSession,
}

/// Spirc'in yaşam döngüsü: istenen durum ve çalışan Spirc. Ağ yok; kararları
/// `Engine` yürütür.
pub(in crate::engine) struct ConnectHost {
    pub enabled: bool,
    /// Spotify'da görünen ad.
    pub name: String,
    life: Life,
    epoch: u64,
    /// Kimlik uyuşmazlığı için bu oturumda zaten yeniden bağlanma istendi
    /// (döngüye girmesin).
    restart_asked: bool,
    /// Bu oturumda bir Spirc kuruldu (başarılı ya da değil): dealer kurucusu
    /// oturum başına tek kullanımlık, aynı oturumda yenisi kurulamaz; aç-kapa-aç
    /// için yeni oturum gerekir.
    session_used: bool,
}

impl ConnectHost {
    pub fn new() -> Self {
        Self { enabled: false, name: crate::brand::APP_NAME.to_string(), life: Life::Off, epoch: 0, restart_asked: false, session_used: false }
    }

    pub fn is_running(&self) -> bool {
        self.life != Life::Off
    }

    /// Bu sıra numaralı Spirc güncel mi (kuruluyor ya da çalışıyor)?
    pub fn is_current(&self, epoch: u64) -> bool {
        matches!(self.life, Life::Starting(g) | Life::Running(g) if g == epoch)
    }

    /// Şimdi ne yapılmalı? `ready`: bağlı oturum ve gerçek çalar var;
    /// `id_ok`: oturum kalıcı aygıt kimliğiyle açılmış.
    pub fn decide(&mut self, ready: bool, id_ok: bool) -> Decision {
        match (self.enabled, self.life) {
            (false, Life::Off) => Decision::Nothing,
            (false, _) => Decision::Stop,
            (true, Life::Off) if !ready => Decision::Nothing,
            (true, Life::Off) if !id_ok || self.session_used => {
                if std::mem::replace(&mut self.restart_asked, true) {
                    Decision::Nothing
                } else {
                    Decision::RestartSession
                }
            }
            (true, Life::Off) => {
                self.session_used = true;
                self.epoch += 1;
                self.life = Life::Starting(self.epoch);
                Decision::Launch(self.epoch)
            }
            (true, _) => Decision::Nothing,
        }
    }

    /// Yeni oturum takıldı: kimlik uyuşmazlığı hakkı ve Spirc kurma hakkı yeniden doğar.
    pub fn new_session(&mut self) {
        self.restart_asked = false;
        self.session_used = false;
    }

    /// Spirc kuruldu; kabul edildiyse `true`, değilse (eskimiş) çağıran
    /// Spirc'i kapatmalı.
    pub fn started(&mut self, epoch: u64) -> bool {
        if self.life == Life::Starting(epoch) {
            self.life = Life::Running(epoch);
            true
        } else {
            false
        }
    }

    /// Kurulum başarısız (güncelse `true`).
    pub fn failed(&mut self, epoch: u64) -> bool {
        if self.life == Life::Starting(epoch) {
            self.life = Life::Off;
            true
        } else {
            false
        }
    }

    /// Spirc'in görevi bitti (güncelse `true`).
    pub fn ended(&mut self, epoch: u64) -> bool {
        if self.is_current(epoch) {
            self.life = Life::Off;
            true
        } else {
            false
        }
    }

    /// Bırak: yarıda kalan kurulum da geçersiz olur (sıra numarası atlar).
    pub fn stop(&mut self) {
        self.epoch += 1;
        self.life = Life::Off;
    }
}

// ── Motor tarafı ────────────────────────────────────────────────────────

/// Arka planda biten Connect işleri (`Internal::Connect`).
pub(super) enum ConnectInternal {
    Started { epoch: u64, port: Arc<dyn ConnectPort>, self_id: String },
    Failed { epoch: u64, error: String },
    Event { epoch: u64, event: SpircEvent },
    Ended { epoch: u64 },
}

/// Ad en çok bu kadar karakter (Spotify listesinde taşmasın; ayarlar da keser).
const NAME_MAX: usize = 64;

impl Engine {
    /// `Command::Connect`.
    pub(super) fn on_connect_command(&mut self, command: ConnectCommand) {
        match command {
            ConnectCommand::SetEnabled(on) => {
                if self.connect.enabled != on {
                    log::info!("Spotify Connect {}", if on { "açıldı" } else { "kapandı" });
                    self.connect.enabled = on;
                }
                self.connect_sync();
            }
            ConnectCommand::SetName(name) => {
                let name: String = name.trim().chars().take(NAME_MAX).collect();
                if name.is_empty() || name == self.connect.name {
                    return;
                }
                self.connect.name = name;
                // Ad Spirc kurulurken verilir ve dealer tek kullanımlıktır:
                // çalışan Spirc yeni oturumla yeni adla kurulur.
                if self.connect.is_running() {
                    self.connect_restart_session();
                }
            }
            ConnectCommand::TransferHere => self.pb.transfer_here(),
        }
    }

    /// Spirc'in olması gereken durumuyla gerçek durumunu eşitler (oturum
    /// takılınca, açma / kapama komutunda).
    pub(super) fn connect_sync(&mut self) {
        let ready = self.host.conn.is_some() && self.pb.deck.as_ref().is_some_and(|d| d.port.player().is_some());
        let id_ok = match (&self.host.conn, session_device_id()) {
            (Some(conn), Some(want)) => conn.session.device_id() == want,
            _ => false,
        };
        match self.connect.decide(ready, id_ok) {
            Decision::Nothing => {}
            Decision::Launch(epoch) => self.launch_spirc(epoch),
            Decision::Stop => self.connect_stop(),
            Decision::RestartSession => {
                log::info!("Connect için oturum kalıcı aygıt kimliğiyle yeniden açılıyor");
                self.connect_restart_session();
            }
        }
    }

    /// Spirc bu oturumla bitti / bırakıldı: kapat ve sürücüyü `Local`e döndür.
    pub(super) fn connect_stop(&mut self) {
        let was_running = self.connect.is_running();
        self.connect.stop();
        if let Some(port) = self.pb.link.port.clone() {
            port.shutdown();
        }
        if was_running || self.pb.link.port.is_some() {
            self.with_pb(|pb, cx| pb.connect_down(cx));
        }
    }

    /// Motor kapanırken: Spirc'i kapat (Spotify'dan aygıtı sil) ve bitmesini
    /// kısa süre bekle; yoksa telefonun listesinde birkaç dakika asılı kalır.
    pub(super) async fn connect_close(&mut self) {
        let port = self.pb.link.port.clone();
        self.connect.stop();
        if let Some(port) = port {
            port.shutdown();
            if let Some(task) = port.join_handle() {
                let _ = tokio::time::timeout(Duration::from_millis(1500), task).await;
            }
        }
    }

    /// Yeni oturum için yeni Spirc (dealer tek kullanımlık): oturumu yenile.
    /// Geri çekilme sürerken `check_health` birazdan yeniden dener.
    fn connect_restart_session(&mut self) {
        self.host.reconnect_wanted = true;
        self.reconnect(false);
    }

    /// Oturum takıldı (`First` / `Replace`): eski oturumun Spirc'i zaten
    /// bırakıldı (`install_replace` başında `connect_stop`); yeni oturum için
    /// (açıksa) yenisi kurulur. Kapalıyken bir şey yapmaz.
    pub(super) fn connect_on_install(&mut self) {
        self.connect.new_session();
        self.connect_sync();
    }

    /// Spirc'i arka planda kurar (`Spirc::with_session`, `openify-net`'te).
    fn launch_spirc(&mut self, epoch: u64) {
        let (Some(conn), Some(deck)) = (&self.host.conn, &self.pb.deck) else { return };
        let Some(player) = deck.port.player() else { return };
        let session = conn.session.clone();
        let out = deck.out.clone();
        let mixer: Arc<dyn Mixer> = Arc::new(OutMixer::new(deck.mixer(), out));
        let config = ConnectConfig {
            name: self.connect.name.clone(),
            device_type: DeviceType::Computer,
            initial_volume: super::playback::volume_to_u16(self.pb.volume),
            ..ConnectConfig::default()
        };
        let internal = self.internal.clone();
        log::info!("Spotify Connect aygıtı kuruluyor: {}", config.name);
        tokio::spawn(async move {
            let started = spawn_spirc(config, session, player, mixer, epoch, &internal).await;
            if let Err(error) = started {
                let _ = internal.send(Internal::Connect(ConnectInternal::Failed { epoch, error: error.to_string() }));
            }
        });
    }

    /// `Internal::Connect`.
    pub(super) fn on_connect_internal(&mut self, message: ConnectInternal) {
        match message {
            ConnectInternal::Started { epoch, port, self_id } => {
                if !self.connect.started(epoch) {
                    // Bu arada oturum değişti / Connect kapandı.
                    port.shutdown();
                    return;
                }
                let name = self.connect.name.clone();
                log::info!("Spotify Connect aygıtı hazır: {name}");
                self.with_pb(|pb, cx| pb.connect_up(cx, port, self_id, name));
            }
            ConnectInternal::Failed { epoch, error } => {
                if self.connect.failed(epoch) {
                    // Yeniden denenmez (Spotify'ı boğma); sonraki oturumda ya da
                    // açıp kapatınca yeniden denenir.
                    log::warn!("Spotify Connect kurulamadı: {error}");
                }
            }
            ConnectInternal::Event { epoch, event } => {
                if self.connect.is_current(epoch) {
                    self.with_pb(|pb, cx| pb.on_spirc(cx, event));
                }
            }
            ConnectInternal::Ended { epoch } => {
                if self.connect.ended(epoch) {
                    log::warn!("Spotify Connect görevi bitti (oturum düşmüş olabilir)");
                    self.with_pb(|pb, cx| pb.connect_down(cx));
                }
            }
        }
    }

    /// Telefon sürerken "sıraya ekle" Spirc'in sırasına gider; `true`: işlendi.
    pub(super) fn connect_enqueue(&self, uris: &[String]) -> bool {
        self.pb.spirc_enqueue(uris)
    }
}

/// Spirc'i kurar, görevini ve olay akışını başlatır, motora haber verir.
async fn spawn_spirc(
    config: ConnectConfig,
    session: Session,
    player: Arc<Player>,
    mixer: Arc<dyn Mixer>,
    epoch: u64,
    internal: &tokio::sync::mpsc::UnboundedSender<Internal>,
) -> Result<(), Error> {
    let self_id = session.device_id().to_string();
    let (spirc, task, mut events) = Spirc::with_session(config, session, player, mixer).await?;
    let end = internal.clone();
    let handle = tokio::spawn(async move {
        task.await;
        let _ = end.send(Internal::Connect(ConnectInternal::Ended { epoch }));
    });
    let port: Arc<dyn ConnectPort> = Arc::new(SpircLink { spirc, task: Mutex::new(Some(handle)) });
    // `Started` olaylardan ÖNCE gider (aynı kanal, sıra korunur).
    let _ = internal.send(Internal::Connect(ConnectInternal::Started { epoch, port, self_id }));
    let pump = internal.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if pump.send(Internal::Connect(ConnectInternal::Event { epoch, event })).is_err() {
                break;
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests;
