//! Sürücü geçişleri: `Local` ↔ `Spirc` ↔ `Remote`. "Kim başlattıysa o sürer."
//!
//! | olay                                  | Local            | Spirc                       | Remote                  |
//! |---------------------------------------|------------------|-----------------------------|-------------------------|
//! | telefon aktardı (`RemoteAttached`)    | → Spirc          | -                           | → Spirc                 |
//! | telefon geri aldı (`RemoteDetached`)  | -                | → Local (+ Remote'a bakılır)| -                       |
//! | Openify'da yeni çalma başladı         | -                | Spirc'ten çık → Local       | → Local                 |
//! | başka aygıtta çalıyor (küme)          | boştaysa → Remote | -                          | ad güncellenir          |
//! | hiçbir aygıt çalmıyor (küme)          | -                | -                           | → Local                 |
//! | "Buraya aktar"                        | Spirc'e istek    | -                           | Spirc'e istek           |
//!
//! Spirc sürerken çalara KOMUT verilmez (komutu Spirc verir); deste yalnız
//! çıkışa dokunur: telefonun duraklatması sesi anında kısar (`silence`),
//! telefonun şarkı değiştirmesi / geri alması kuyruğu atar (`foreign_flush`),
//! kendi düğmelerimiz önce çıkışı hazırlar sonra Spirc'e iletilir. Sıra,
//! önyükleme, kurtarma ve kaldığı yer kaydı yalnız `Local`de işler.
//!
//! Spirc yokken (Connect kapalıysa hep) hiçbir şey burada çalışmaz: sürücü
//! `Local`, `link` boş.

use std::sync::Arc;

use librespot::connect::{ClusterView, SpircEvent};

use super::{
    Cx, Driver, Effect, Local, LocalSnapshot, Playback, RemoteDrive, SpircDrive, TransportCmd,
};
use crate::engine::{ConnectDriver, ConnectPort, ConnectStatus, Event, QueueState, RemoteNow};

/// Spotify Connect tarafının `Playback`teki durumu.
pub(in crate::engine) struct Link {
    /// Çalışan Spirc (yok: kapalı / henüz başlamadı / bitti).
    pub port: Option<Arc<dyn ConnectPort>>,
    /// Bu aygıtın Connect kimliği (kümede "biz" kimiz).
    pub self_id: String,
    /// Spotify'da görünen adımız; boş: Spirc yok.
    pub name: String,
    /// Son küme görünümü (hesaptaki aygıtlar, çalan oturum).
    pub cluster: Option<ClusterView>,
    /// Arayüze son bildirilen durum (aynısı yeniden gitmesin; başlangıç
    /// değeri varsayılan: Connect kapalıyken arayüze hiçbir şey gitmez).
    shown: ConnectStatus,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            port: None,
            self_id: String::new(),
            name: String::new(),
            cluster: None,
            shown: ConnectStatus::default(),
        }
    }
}

/// Kümede başka bir aygıt çalıyorsa (kimliği, adı).
fn other_playing(link: &Link) -> Option<(String, String)> {
    let c = link.cluster.as_ref()?;
    if c.active_device_id.is_empty() || c.active_device_id == link.self_id || !c.playing || c.paused
    {
        return None;
    }
    let name = c
        .devices
        .iter()
        .find(|d| d.id == c.active_device_id)?
        .name
        .clone();
    (!name.is_empty()).then(|| (c.active_device_id.clone(), name))
}

impl Playback {
    // ── Durum ───────────────────────────────────────────────────────────

    fn connect_status(&self) -> ConnectStatus {
        let (driver, remote) = match &self.driver {
            Driver::Local(_) => (ConnectDriver::Local, None),
            Driver::Spirc(_) => (ConnectDriver::Spirc, None),
            Driver::Remote(r) => (
                ConnectDriver::Remote,
                Some(RemoteNow {
                    device_name: r.device_name.clone(),
                }),
            ),
        };
        ConnectStatus {
            driver,
            device_name: self.link.name.clone(),
            remote,
        }
    }

    /// Durum değiştiyse arayüze bildir.
    pub(super) fn publish_connect(&mut self, cx: &mut Cx<'_>) {
        let status = self.connect_status();
        if status != self.link.shown {
            self.link.shown = status.clone();
            cx.fx.push(Effect::Connect(status));
        }
    }

    /// Spirc başladı: `name` Spotify'da görünen adımız, `self_id` Connect kimliğimiz.
    pub fn connect_up(
        &mut self,
        cx: &mut Cx<'_>,
        port: Arc<dyn ConnectPort>,
        self_id: String,
        name: String,
    ) {
        self.link.port = Some(port);
        self.link.self_id = self_id;
        self.link.name = name;
        self.publish_connect(cx);
    }

    /// Spirc yok oldu / kapatıldı / oturum değişti: sürücü `Local`e döner.
    pub fn connect_down(&mut self, cx: &mut Cx<'_>) {
        self.link.port = None;
        self.link.name.clear();
        self.link.cluster = None;
        match self.driver {
            Driver::Spirc(_) => self.detach_spirc(cx),
            Driver::Remote(_) => self.driver = Driver::Local(Local),
            Driver::Local(_) => {}
        }
        self.publish_connect(cx);
    }

    // ── Sürücü geçişleri ────────────────────────────────────────────────

    /// Telefon bu aygıtı sürmeye başladı (aktarma / "burada çal"). İdempotent.
    pub(super) fn attach_spirc(&mut self, cx: &mut Cx<'_>) {
        if matches!(self.driver, Driver::Spirc(_)) {
            return;
        }
        let Some(deck) = &self.deck else { return };
        let snapshot = LocalSnapshot {
            track: self.track_info.clone(),
            position_ms: self.position_now(),
        };
        // Yerel çalma artık sürmüyor: önyükleme, yükleme izleme, kurtarma
        // durumu bırakılır; kaldığı yer kaydı donar (bkz. `Engine::persist_at`).
        self.restored = None;
        self.last_position = None;
        self.preload_armed = false;
        self.preloaded = None;
        self.load_started = None;
        self.stall_fired = false;
        self.retry_after_reconnect = false;
        self.retried_uri = None;
        self.ignore_stale_failure = false;
        self.failures = 0;
        self.drop_handover();
        // Yerelde çalan sesin kuyruktaki kalanı telefonun şarkısından önce
        // çalmasın; derin şarkı sonu payı da iner.
        deck.out.set_tail(false);
        deck.foreign_flush();
        self.driver = Driver::Spirc(SpircDrive {
            snapshot,
            silenced: false,
            load_expected: true,
        });
        // Sıra telefonun: ileri / geri düğmeleri açık, bizim sıramız gösterilmez.
        cx.emit(Event::QueueChanged(QueueState {
            context: None,
            index: 0,
            has_next: true,
            has_previous: true,
        }));
        self.publish_connect(cx);
    }

    /// Telefon aygıtı bıraktı (geri aldı, başka yere aktardı, bağlantı gitti).
    /// Arayüz Openify'ın kendi durumuna (duraklatılmış) döner. İdempotent.
    pub(super) fn detach_spirc(&mut self, cx: &mut Cx<'_>) {
        if !matches!(self.driver, Driver::Spirc(_)) {
            return;
        }
        let Driver::Spirc(drive) = std::mem::replace(&mut self.driver, Driver::Local(Local)) else {
            return;
        };
        if let Some(deck) = &self.deck {
            // Telefonun sesinin kuyruktaki kalanı atılır (Spirc `stop`u izler).
            deck.foreign_flush();
            deck.unsilence();
        }
        self.last_position = None;
        self.load_started = None;
        self.back_to_local(cx, drive.snapshot);
        self.reeval_remote(cx);
        self.publish_connect(cx);
    }

    /// Spirc'ten ayrılınca arayüzü yerel duruma döndürür: aktarmadan önce
    /// gösterilen şarkı kaldığı konumda duraklatılmış (Çal oradan yükler);
    /// yerelde şarkı yoktuysa "çalan yok".
    fn back_to_local(&mut self, cx: &mut Cx<'_>, snapshot: LocalSnapshot) {
        match snapshot.track {
            Some(info) => {
                self.restored = Some(snapshot.position_ms);
                self.track_info = Some(info.clone());
                if let Some(url) = info.cover_url.clone() {
                    cx.fx.push(Effect::FetchCover {
                        uri: info.uri.clone(),
                        url,
                    });
                }
                cx.emit(Event::TrackChanged(info));
                cx.queue_changed();
                cx.emit(Event::Paused {
                    position_ms: snapshot.position_ms,
                });
            }
            None => {
                self.track_info = None;
                cx.emit(Event::Ended);
            }
        }
    }

    /// Openify'da yeni çalma başlıyor: Spirc'ten çık (telefon bırakıldı,
    /// Spirc'in oynatıcı olaylarına tepkisi susturulur), sürücü `Local`.
    /// Çalara komut VERİLMEZ (Spirc'in bırakma isteği de çalara dokunmaz).
    fn leave_spirc_for_local(&mut self, cx: &mut Cx<'_>) {
        if !matches!(self.driver, Driver::Spirc(_)) {
            return;
        }
        if let Some(port) = &self.link.port {
            port.set_host_drives(true);
            port.leave();
        }
        if let Some(deck) = &self.deck {
            deck.unsilence();
        }
        self.driver = Driver::Local(Local);
        self.publish_connect(cx);
    }

    /// Başka aygıtta çalma bitti ya da burada çalma başladı: `Local`.
    pub(super) fn leave_remote(&mut self, cx: &mut Cx<'_>) {
        if matches!(self.driver, Driver::Remote(_)) {
            self.driver = Driver::Local(Local);
            self.publish_connect(cx);
        }
    }

    /// Yerelde çalan / yüklenen yok mu (başka aygıt çalıyorsa "orada çalıyor"
    /// denebilir)?
    fn local_idle(&self) -> bool {
        !matches!(self.last_position, Some((_, _, true))) && self.load_started.is_none()
    }

    /// Küme görünümüne göre `Remote`a gir / çık / adı güncelle.
    pub(super) fn reeval_remote(&mut self, cx: &mut Cx<'_>) {
        let other = other_playing(&self.link);
        let idle = self.local_idle();
        let next = match (&self.driver, other) {
            (Driver::Local(_), Some((device_id, device_name))) if idle => {
                Some(Driver::Remote(RemoteDrive {
                    device_id,
                    device_name,
                }))
            }
            (Driver::Remote(_), Some((device_id, device_name))) => {
                Some(Driver::Remote(RemoteDrive {
                    device_id,
                    device_name,
                }))
            }
            (Driver::Remote(_), None) => Some(Driver::Local(Local)),
            _ => None,
        };
        if let Some(driver) = next {
            self.driver = driver;
        }
        self.publish_connect(cx);
    }

    /// Spirc'in bildirimleri (`Engine::on_connect_internal`).
    pub fn on_spirc(&mut self, cx: &mut Cx<'_>, event: SpircEvent) {
        match event {
            SpircEvent::State(host) => {
                // Bant içi işaretin yedeği: etkinleşme / bırakma burada da
                // görülür (ikisi de idempotent).
                if host.active && !matches!(self.driver, Driver::Spirc(_)) {
                    self.attach_spirc(cx);
                } else if !host.active && matches!(self.driver, Driver::Spirc(_)) {
                    return self.detach_spirc(cx);
                }
                // Duraklatma çalardan ÖNCE bildirilebilir: çıkışı hemen kıs.
                if host.paused && !host.playing {
                    self.spirc_silence();
                } else if host.playing {
                    self.spirc_resumed();
                }
            }
            SpircEvent::Cluster(cluster) => {
                self.link.cluster = Some(cluster);
                self.reeval_remote(cx);
            }
        }
    }

    // ── Spirc sürerken çıkış ────────────────────────────────────────────

    /// Çıkışı sustur (duraklatma: telefonun ya da bizim). Bir kez.
    pub(super) fn spirc_silence(&mut self) {
        let (Some(deck), Driver::Spirc(drive)) = (&self.deck, &mut self.driver) else {
            return;
        };
        if !drive.silenced {
            drive.silenced = true;
            deck.silence();
        }
    }

    /// Çalma sürdü: susturma kalkar.
    pub(super) fn spirc_resumed(&mut self) {
        let (Some(deck), Driver::Spirc(drive)) = (&self.deck, &mut self.driver) else {
            return;
        };
        if std::mem::take(&mut drive.silenced) {
            deck.unsilence();
        }
    }

    /// Telefondan (ya da etkinleşmeden) yükleme: kendiliğinden geçiş ya da
    /// bizim komutumuz değilse kuyruktaki eski ses atılır.
    pub(super) fn spirc_loading(&mut self) {
        let (Some(deck), Driver::Spirc(drive)) = (&self.deck, &mut self.driver) else {
            return;
        };
        if !std::mem::take(&mut drive.load_expected) {
            deck.foreign_flush();
        }
    }

    /// Şarkı bitti: Spirc sıradakini yükler, o yükleme kuyruğu atmaz.
    pub(super) fn spirc_end_of_track(&mut self) {
        if let Driver::Spirc(drive) = &mut self.driver {
            drive.load_expected = true;
        }
    }

    // ── Komutlar ────────────────────────────────────────────────────────

    /// Spirc sürerken taşıma komutu: telefonun sırasında / durumunda ilerler,
    /// biz yalnız çıkışı hazırlayıp Spirc'e ileteceğiz. Çalara komut yok.
    pub(super) fn spirc_transport(&mut self, cx: &mut Cx<'_>, cmd: TransportCmd) {
        let port = self.link.port.clone();
        match cmd {
            TransportCmd::Play => {
                self.spirc_resumed();
                if let Some(p) = &port {
                    p.play();
                }
            }
            TransportCmd::Pause => {
                // Ses beklemeden susar; sonra Spirc duraklatır.
                self.spirc_silence();
                if let Some(p) = &port {
                    p.pause();
                }
            }
            TransportCmd::Seek(ms) => {
                if let Some(deck) = &self.deck {
                    deck.foreign_flush();
                }
                if let Some(p) = &port {
                    p.seek(ms);
                }
            }
            TransportCmd::Next | TransportCmd::Previous => {
                if let Some(deck) = &self.deck {
                    deck.foreign_flush();
                }
                // Komutumuzun yükleme olayı yeniden atmasın.
                if let Driver::Spirc(drive) = &mut self.driver {
                    drive.load_expected = true;
                }
                if let Some(p) = &port {
                    if cmd == TransportCmd::Next {
                        p.next();
                    } else {
                        p.prev();
                    }
                }
            }
            TransportCmd::SetVolume(volume) => {
                self.set_volume(volume);
                if let Some(p) = &port {
                    p.set_volume(super::volume_to_u16(volume));
                }
            }
            // Openify'da yeni çalma: "kim başlattıysa o sürer" → Spirc'ten çık.
            TransportCmd::Start { .. } | TransportCmd::JumpTo(_) => {
                self.leave_spirc_for_local(cx);
                self.local_transport(cx, cmd);
            }
        }
    }

    /// Başka aygıtta çalarken taşıma komutu: uzak aygıta komut yok. Burada
    /// çalma başlatan komutlar yerel çalmayı başlatır (`Remote`tan çıkılır);
    /// duraklat / sar zaten boştaki yerel çalara gider.
    pub(super) fn remote_transport(&mut self, cx: &mut Cx<'_>, cmd: TransportCmd) {
        if !matches!(
            cmd,
            TransportCmd::Pause | TransportCmd::Seek(_) | TransportCmd::SetVolume(_)
        ) {
            self.leave_remote(cx);
        }
        self.local_transport(cx, cmd);
    }

    /// "Buraya aktar": Spotify'dan çalmayı bu aygıta vermesini iste. Cevap,
    /// telefondaki gibi Spirc'in etkinleşmesidir (`RemoteAttached`). Spirc
    /// yoksa ya da zaten sürüyorsa bir şey yapmaz.
    pub fn transfer_here(&self) {
        if matches!(self.driver, Driver::Spirc(_)) {
            return;
        }
        if let Some(port) = &self.link.port {
            port.transfer_here();
        }
    }

    /// Telefon sürerken "sıraya ekle": Spirc'in sırasına (P4), sonra çal da
    /// sona eklenir (Spirc yalnız sona ekler). `true`: işlendi, yerel sıraya
    /// dokunulmaz.
    pub fn spirc_enqueue(&self, uris: &[String]) -> bool {
        if !matches!(self.driver, Driver::Spirc(_)) {
            return false;
        }
        if let Some(port) = &self.link.port {
            for uri in uris {
                port.add_to_queue(uri);
            }
        }
        true
    }
}
