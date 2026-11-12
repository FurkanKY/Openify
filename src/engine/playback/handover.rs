//! Boşluksuz geçişte arayüz şarkıyı DUYULDUĞU anda değiştirsin.
//!
//! Şarkı sonu payıyla (bkz. `output.rs` başlığı) kuyruk ~4 sn derin: çalar
//! önceki şarkının sonunu önden çözüp bitirince (`EndOfTrack`) sıradakine
//! geçer, yani sıra değişimi, `TrackChanged`, `Playing` ve kapak duyulan
//! sınırdan ~4 sn önce gelir. Motor bu çalma olaylarını tutar (`Gate`) ve
//! yeni şarkının ilk karesi duyulurken (arayüzün işleme gecikmesi kadar,
//! `RELEASE_LEAD`, önce) sırasıyla gönderir. Arayüzün sözleşmesi değişmez:
//! aynı olaylar, aynı sırayla, yalnız duyulduğu anda.
//!
//! Tutma sürerken arayüz önceki şarkıyı gösterir; kullanıcının komutları ona
//! göre yorumlanır:
//! - Duraklat / Çal: konum, önceki şarkıda duyulan yer (beklemeden).
//! - Sar: önceki şarkı o konumdan yeniden yüklenir (sıra geri alınır).
//! - Önceki: sıra geri alınır, sonra her zamanki "Önceki".
//! - Sonraki, görünür listenin ilk satırı: yeni şarkı baştan (kuyruktaki
//!   önceki şarkı sonu atılır).
//! - Görünür listenin satır numarasıyla gelenler: tutulanlar hemen gönderilir,
//!   numara bir kaydırılır (yeni şarkı listeden çıkıp çalan oldu).
//! - Geri kalanı (sıraya ekle, karıştır, başka liste çal…): tutulanlar hemen
//!   gönderilir, komut her zamanki gibi.

use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    time::Duration,
};

use super::{Cx, Playback};
use crate::engine::{Command, Event, OutputCtl, PlaybackEvent, output::Handover};

/// Tutma sürerken sınırın duyulup duyulmadığına en geç bu aralıkla bakılır
/// (duraklatma / takılma tahmini değiştirebilir); tahmin daha yakınsa o an.
const POLL_MAX: Duration = Duration::from_millis(20);
/// Olaylar sınırdan bu kadar önce gönderilir: arayüzün olayı işlemesi
/// ölçümlerde 1 ms (boşta) ile 48 ms (meşgulken, önünde `QueueChanged`)
/// arası sürdü (7 Kasım). Ortası: başlık sınırın ±50 ms'sinde değişsin.
const RELEASE_LEAD: Duration = Duration::from_millis(25);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Arayüze giden çalma olaylarının kapısı: tutma yokken olduğu gibi geçirir.
#[derive(Clone)]
pub(in crate::engine) struct Gate {
    slot: Arc<Mutex<Option<Hold>>>,
    events: async_channel::Sender<Event>,
    playback: async_channel::Sender<PlaybackEvent>,
}

struct Hold {
    id: u64,
    out: Arc<OutputCtl>,
    held: Vec<Held>,
    /// Tutma sürerken çaların son durumu (`Playing` / `Paused`): bırakınca
    /// yeni şarkının duyulan konumuyla yeniden gönderilir.
    playing: Option<bool>,
    /// Geçişten önce çalan, arayüzün hâlâ gösterdiği şarkı.
    old_uri: Option<String>,
}

enum Held {
    Ui(Event),
    View(PlaybackEvent),
}

impl Gate {
    pub fn new(
        events: async_channel::Sender<Event>,
        playback: async_channel::Sender<PlaybackEvent>,
    ) -> Self {
        Self {
            slot: Arc::default(),
            events,
            playback,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Hold>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sınama: tutmayı elle başlat (çıkış `out`, arayüzün gösterdiği şarkı
    /// `old_uri`); bırakma iş parçacığı açılmaz.
    #[cfg(test)]
    pub(in crate::engine) fn hold_for_test(&self, out: Arc<OutputCtl>, old_uri: Option<String>) {
        *self.lock() = Some(Hold {
            id: 0,
            out,
            held: Vec::new(),
            playing: None,
            old_uri,
        });
    }

    /// Sınama: tutmayı bırak (`hold_for_test`'in tersi).
    #[cfg(test)]
    pub(in crate::engine) fn release_for_test(&self) {
        if let Some(hold) = self.lock().take() {
            hold.out.end_handover();
        }
    }

    /// Tutma sürüyor mu (çalar bir şarkı sonunu önden çözdü, sınır henüz
    /// duyulmadı; arayüz önceki şarkıyı gösteriyor)?
    pub fn is_holding(&self) -> bool {
        self.lock().is_some()
    }

    /// `Engine::emit`'in yolu.
    pub fn send(&self, event: Event) {
        let mut slot = self.lock();
        let Some(hold) = slot.as_mut() else {
            let _ = self.events.try_send(event);
            return;
        };
        match event {
            // Konum bildirenler: arayüz önceki şarkıyı gösteriyor, onun konumu.
            Event::Playing { .. } | Event::Paused { .. } | Event::Position { .. } => {
                let paused = matches!(event, Event::Paused { .. });
                match hold.out.handover_old_ms(paused) {
                    Some(position_ms) => {
                        let event = match event {
                            Event::Playing { .. } => {
                                hold.playing = Some(true);
                                Event::Playing { position_ms }
                            }
                            Event::Paused { .. } => {
                                hold.playing = Some(false);
                                Event::Paused { position_ms }
                            }
                            _ => Event::Position { position_ms },
                        };
                        let _ = self.events.try_send(event);
                    }
                    None => {
                        // Sınır duyuldu (ya da geçiş iptal): önce tutulanlar,
                        // sonra yeni şarkının bu olayı.
                        self.release(&mut slot, None);
                        let _ = self.events.try_send(event);
                    }
                }
            }
            Event::QueueChanged(_)
            | Event::TrackChanged(_)
            | Event::Cover { .. }
            | Event::Loading
            | Event::Ended
            | Event::Unavailable => hold.held.push(Held::Ui(event)),
            other => {
                let _ = self.events.try_send(other);
            }
        }
    }

    /// `PlaybackEvent::Queue`'nun yolu (sıra görünümü de şarkıyla birlikte).
    pub fn send_view(&self, event: PlaybackEvent) {
        let mut slot = self.lock();
        match slot.as_mut() {
            Some(hold) => hold.held.push(Held::View(event)),
            None => {
                let _ = self.playback.try_send(event);
            }
        }
    }

    /// Tutulanları sırasıyla gönderir. `new_ms`: yeni şarkının duyulan konumu
    /// (sınıra gelindiyse). Çalıyorsa arayüz `TrackChanged`'i aldığı an
    /// konumu 0'dan başlatır: ayrıca `Playing` gönderilmez (arayüz olayları
    /// kare başına işliyor, ~40 ms sonra gelecek bir `Playing` saati
    /// geciktirirdi). Duraklatıldıysa yeni şarkının konumuyla `Paused`.
    fn release(&self, slot: &mut Option<Hold>, new_ms: Option<u32>) {
        let Some(hold) = slot.take() else { return };
        hold.out.end_handover();
        let has_track = hold
            .held
            .iter()
            .any(|h| matches!(h, Held::Ui(Event::TrackChanged(_))));
        for held in hold.held {
            match held {
                Held::Ui(event) => {
                    let _ = self.events.try_send(event);
                }
                Held::View(event) => {
                    let _ = self.playback.try_send(event);
                }
            }
        }
        match (new_ms, hold.playing) {
            (Some(position_ms), Some(false)) => {
                let _ = self.events.try_send(Event::Paused { position_ms });
            }
            (Some(position_ms), Some(true)) if !has_track => {
                let _ = self.events.try_send(Event::Playing { position_ms });
            }
            _ => {}
        }
    }

    /// Sınır duyulana dek bekler, sonra bırakır. Kendi iş parçacığında: tokio
    /// uyuması Windows'ta zamanlayıcı taneciğine (~16 ms) yuvarlanıyor, 7
    /// Kasım ölçümünde 31 ms geç uyandı; `thread::sleep` yüksek çözünürlüklü
    /// bekleme kullanıyor. Geçiş başına bir iş parçacığı (~3 dk'da bir).
    fn spawn_release(&self, id: u64) {
        let gate = self.clone();
        let spawned = std::thread::Builder::new()
            .name("openify-handover".into())
            .spawn(move || {
                loop {
                    let wait = {
                        let mut slot = gate.lock();
                        let Some(hold) = slot.as_ref() else { return };
                        if hold.id != id {
                            return;
                        }
                        match hold.out.handover_state() {
                            Handover::Pending { eta } if eta <= RELEASE_LEAD => {
                                log::debug!(
                                    "Geçiş: arayüze sınırdan {} ms önce bildiriliyor",
                                    eta.as_millis()
                                );
                                gate.release(&mut slot, Some(0));
                                return;
                            }
                            Handover::Pending { eta } => {
                                (eta - RELEASE_LEAD).clamp(Duration::from_millis(1), POLL_MAX)
                            }
                            Handover::Audible { new_ms } => {
                                log::debug!(
                                    "Geçiş duyuldu: arayüze bildiriliyor (yeni şarkıda {new_ms} ms)"
                                );
                                gate.release(&mut slot, Some(new_ms));
                                return;
                            }
                            Handover::None => {
                                gate.release(&mut slot, None);
                                return;
                            }
                        }
                    };
                    std::thread::sleep(wait);
                }
            });
        if let Err(e) = spawned {
            // Olmazsa tutulanlar hemen gider (eski davranış: ~4 sn erken).
            log::warn!("Geçiş iş parçacığı açılamadı ({e}); şarkı değişimi hemen bildiriliyor");
            self.release(&mut self.lock(), None);
        }
    }
}

impl Playback {
    /// Şarkı kendiliğinden bitti (`EndOfTrack`, sıradakini yüklemeden ÖNCE):
    /// sınırın karesi alınır, çalma olayları sınır duyulana dek tutulur.
    pub fn begin_handover(&mut self, cx: &Cx<'_>) {
        let Some(deck) = &self.deck else { return };
        deck.out.begin_handover();
        let mut slot = self.gate.lock();
        if slot.is_some() {
            // Önceki geçiş henüz duyulmadı (çok kısa şarkı): aynı tutma sürer,
            // arayüz hâlâ onun önceki şarkısını gösteriyor.
            return;
        }
        let id = NEXT_ID.fetch_add(1, Relaxed);
        *slot = Some(Hold {
            id,
            out: Arc::clone(&deck.out),
            held: Vec::new(),
            playing: None,
            old_uri: cx.queue.current_uri().map(str::to_owned),
        });
        drop(slot);
        self.gate.spawn_release(id);
    }

    /// Tutulanları hemen gönderir.
    fn release_handover(&self) {
        let mut slot = self.gate.lock();
        self.gate.release(&mut slot, None);
    }

    /// Tutulanları atar (önceki şarkıya dönülüyor: arayüz onu zaten gösteriyor).
    pub(super) fn drop_handover(&self) {
        if let Some(hold) = self.gate.lock().take() {
            hold.out.end_handover();
        }
    }

    /// Tutma sürerken gelen kullanıcı komutu (bkz. modül notu). Komut
    /// burada bitti ise `None`, yoksa her zamanki yoldan çalıştırılacak komut.
    pub fn handover_command(&mut self, cx: &mut Cx<'_>, command: Command) -> Option<Command> {
        let (old_uri, playing) = {
            let slot = self.gate.lock();
            let Some(hold) = slot.as_ref() else {
                return Some(command);
            };
            if hold.out.handover_old_ms(false).is_none() {
                // Sınır zaten duyuldu: bırak, komut yeni şarkıya.
                drop(slot);
                self.release_handover();
                return Some(command);
            }
            (hold.old_uri.clone(), hold.playing)
        };
        // Çalar arayüzün gösterdiğinden başka şarkıda mı (tekrar "tek"te aynı)?
        let moved = old_uri.is_some() && old_uri.as_deref() != cx.queue.current_uri();
        match command {
            // Önceki şarkıda olduğu gibi çalışır ya da şarkıdan bağımsız.
            Command::Play
            | Command::Pause
            | Command::SetVolume(_)
            | Command::SetCrossfade(_)
            | Command::SetEq { .. }
            | Command::SetKaraoke { .. }
            | Command::OpenCollection(_)
            | Command::Search { .. }
            | Command::RefreshLibrary
            | Command::FetchImages(_)
            | Command::FetchLyrics(_)
            | Command::PrefetchLyrics(_)
            | Command::Flush(_)
            | Command::Library(_) => Some(command),
            Command::Seek(ms) => match old_uri {
                Some(uri) => {
                    self.return_to_old(cx, &uri, moved, ms, playing != Some(false));
                    None
                }
                None => {
                    self.release_handover();
                    Some(command)
                }
            },
            Command::Previous => match old_uri {
                Some(uri) => {
                    self.drop_handover();
                    if moved {
                        cx.queue.previous();
                    }
                    if cx.queue.has_previous() {
                        Some(command)
                    } else {
                        self.return_to_old(cx, &uri, false, 0, playing != Some(false));
                        None
                    }
                }
                None => {
                    self.release_handover();
                    Some(command)
                }
            },
            Command::Next | Command::PlayQueueItem(0) if moved => {
                // Arayüzde "sıradaki" = çaların zaten çaldığı yeni şarkı: baştan.
                self.release_handover();
                if let Some(deck) = &self.deck {
                    deck.seek(0);
                }
                None
            }
            Command::PlayQueueItem(i) if moved => {
                self.release_handover();
                Some(Command::PlayQueueItem(i - 1))
            }
            Command::RemoveFromQueue(i) if moved => {
                self.release_handover();
                // İlk satır çalmaya başlamış yeni şarkı: kaldırmak = geçmek.
                Some(if i == 0 {
                    Command::Next
                } else {
                    Command::RemoveFromQueue(i - 1)
                })
            }
            Command::MoveInQueue(from, to) if moved => {
                self.release_handover();
                // Çalmaya başlamış yeni şarkı taşınamaz.
                (from > 0).then(|| Command::MoveInQueue(from - 1, to.saturating_sub(1)))
            }
            other => {
                self.release_handover();
                Some(other)
            }
        }
    }

    /// Tutma iptal; önceki şarkı `ms`'den yeniden yüklenir (`moved`: sıra
    /// geçişte ilerlemişti, geri alınır).
    fn return_to_old(&mut self, cx: &mut Cx<'_>, uri: &str, moved: bool, ms: u32, play: bool) {
        self.drop_handover();
        if moved {
            cx.queue.previous();
        }
        self.restored = None;
        if self.load_uri(uri, ms, true) {
            if !play && let Some(deck) = &self.deck {
                deck.pause();
            }
            cx.queue_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;
    use crate::engine::{
        NoticeKind, QueueState, QueueView, TrackInfo,
        levels::AudioLevels,
        output::{RingSource, Writer},
    };

    fn track(uri: &str) -> TrackInfo {
        TrackInfo {
            uri: uri.into(),
            name: uri.into(),
            artists: String::new(),
            duration_ms: 1000,
            cover_url: None,
        }
    }

    /// Önceki şarkının 1 sn'si kuyrukta, sınırda tutma başlar; yeni şarkının
    /// olayları sınır duyulunca, sırasıyla ve duyulan konumla gider.
    #[test]
    fn holds_track_change_until_boundary_is_heard_then_releases_in_order() {
        let clock = Arc::new(AtomicU64::new(1_000_000_000));
        let out = OutputCtl::with_clock(AudioLevels::new(), clock.clone());
        let mut src = RingSource::new(out.clone());
        let mut w = Writer::new(out.clone());
        w.start();
        w.sync();
        out.set_track_base(0);
        w.push(&vec![0.25; 44_100 * 2]).unwrap();
        let mut ticks = 0u32;
        let tick = |src: &mut RingSource, ticks: &mut u32| {
            clock.fetch_add(10_000_000, Relaxed);
            for _ in 0..441 * 2 {
                src.next();
            }
            *ticks += 1;
        };
        for _ in 0..20 {
            tick(&mut src, &mut ticks);
        }
        // EndOfTrack: önceki şarkının 1 sn'si yazıldı, 200 ms'si çalındı.
        out.begin_handover();
        let (tx, rx) = async_channel::unbounded();
        let (ptx, prx) = async_channel::unbounded();
        let gate = Gate::new(tx, ptx);
        *gate.lock() = Some(Hold {
            id: 1,
            out: out.clone(),
            held: Vec::new(),
            playing: None,
            old_uri: Some("eski".into()),
        });
        gate.send(Event::QueueChanged(QueueState::default()));
        gate.send_view(PlaybackEvent::Queue(QueueView::default()));
        gate.send(Event::TrackChanged(track("yeni")));
        gate.send(Event::Playing { position_ms: 0 });
        gate.send(Event::Notice {
            kind: NoticeKind::Info,
            message: "geçer".into(),
        });
        // Arayüze yalnız önceki şarkının duyulan konumuyla `Playing` ve
        // şarkıdan bağımsız bildirim gitti.
        match rx.try_recv() {
            Ok(Event::Playing { position_ms }) => assert!(
                (150..=210).contains(&position_ms),
                "önceki şarkı {position_ms} ms"
            ),
            other => panic!("beklenmeyen {other:?}"),
        }
        assert!(matches!(rx.try_recv(), Ok(Event::Notice { .. })));
        assert!(
            rx.try_recv().is_err() && prx.try_recv().is_err(),
            "tutulan olay sızdı"
        );
        // Yeni şarkı yazılır; ses kartı çalmayı sürdürür.
        out.set_track_base(0);
        w.push(&vec![0.25; 44_100 * 2]).unwrap();
        let new_ms = loop {
            tick(&mut src, &mut ticks);
            assert!(ticks < 300, "sınır duyulmadı");
            match out.handover_state() {
                Handover::Audible { new_ms } => break new_ms,
                Handover::Pending { .. } => {}
                Handover::None => panic!("geçiş kayboldu"),
            }
        };
        // Sınır: 1000 ms'lik önceki şarkı + aygıt gecikmesi (30 ms) sonra duyulur.
        let at_ms = ticks * 10;
        assert!(
            (1020..=1050).contains(&at_ms),
            "sınır {at_ms} ms'de duyuldu"
        );
        gate.release(&mut gate.lock(), Some(new_ms));
        assert!(matches!(rx.try_recv(), Ok(Event::QueueChanged(_))));
        assert!(matches!(rx.try_recv(), Ok(Event::TrackChanged(t)) if t.uri == "yeni"));
        // Çalıyor: arayüz konumu `TrackChanged`'i aldığında 0'dan başlatır,
        // ayrıca `Playing` yok (yeni şarkıda {new_ms} ms'de bırakıldı).
        assert!(new_ms <= 10, "yeni şarkı {new_ms} ms");
        assert!(rx.try_recv().is_err());
        assert!(matches!(prx.try_recv(), Ok(PlaybackEvent::Queue(_))));
        assert_eq!(out.handover_state(), Handover::None);
        // Tutma bitti: olaylar olduğu gibi geçer.
        gate.send(Event::Paused { position_ms: 77 });
        assert!(matches!(
            rx.try_recv(),
            Ok(Event::Paused { position_ms: 77 })
        ));
    }

    /// Gerçek zamanda: 10 ms'lik geri çağrılarla çeken bir "ses kartı"
    /// iş parçacığı; bırakma iş parçacığı olayları sınırın duyulmasından
    /// `RELEASE_LEAD` önce (±8 ms) gönderir. (7 Kasım: tokio uyuması 31 ms geç
    /// kalıyordu.)
    #[test]
    // Gerçek zamanlı ölçüm (±8 ms): paralel test koşusunda ya da yüklü makinede
    // uyku taşar ve yanlış düşer. Sakin makinede ayrıca koşulur:
    // `cargo test --release -- --ignored release_thread_fires`.
    // Mantığın kendisi sanal saatli testlerde her koşuda sınanıyor.
    #[ignore = "gerçek zaman; sakin makinede --ignored ile koş"]
    fn release_thread_fires_just_before_the_boundary_in_real_time() {
        use std::{
            sync::atomic::AtomicBool,
            thread,
            time::{Duration, Instant},
        };
        let out = OutputCtl::new(AudioLevels::new(), 1.0);
        let mut w = Writer::new(out.clone());
        w.start();
        w.sync();
        out.set_track_base(0);
        // Önceki şarkının 0,5 sn'si, sınır (EndOfTrack), yeni şarkının 0,5 sn'si.
        w.push(&vec![0.1; 44_100]).unwrap();
        out.begin_handover();
        w.push(&vec![0.1; 44_100]).unwrap();
        let (boundary, total) = (22_050u64, 44_100u64);
        let stop = Arc::new(AtomicBool::new(false));
        let truth: Arc<Mutex<Option<Instant>>> = Arc::default();
        let device = {
            let (out, stop, truth) = (out.clone(), stop.clone(), truth.clone());
            thread::spawn(move || {
                let mut src = RingSource::new(out.clone());
                let device_ns = crate::engine::levels::device_delay_ns();
                while !stop.load(Relaxed) {
                    let t_cb = Instant::now();
                    // Okunan kare = yazılan - kuyruk.
                    let before = total - out.queued_frames().min(total);
                    for _ in 0..441 * 2 {
                        src.next();
                    }
                    let read = total - out.queued_frames().min(total);
                    let mut t = truth.lock().unwrap();
                    if t.is_none() && read >= boundary {
                        let offset = (boundary - before) as f64 / 44_100.0;
                        *t = Some(
                            t_cb + Duration::from_secs_f64(offset)
                                + Duration::from_nanos(device_ns),
                        );
                    }
                    drop(t);
                    thread::sleep(Duration::from_millis(10));
                }
            })
        };
        thread::sleep(Duration::from_millis(100));
        let (tx, rx) = async_channel::unbounded();
        let (ptx, _prx) = async_channel::unbounded();
        let gate = Gate::new(tx, ptx);
        *gate.lock() = Some(Hold {
            id: 7,
            out: out.clone(),
            held: Vec::new(),
            playing: None,
            old_uri: None,
        });
        gate.send(Event::TrackChanged(track("yeni")));
        gate.spawn_release(7);
        let start = Instant::now();
        let released = loop {
            if rx.try_recv().is_ok() {
                break Instant::now();
            }
            assert!(start.elapsed() < Duration::from_secs(2), "bırakılmadı");
            std::hint::spin_loop();
        };
        while truth.lock().unwrap().is_none() && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Relaxed);
        device.join().unwrap();
        let truth = truth.lock().unwrap().expect("sınır çekilmedi");
        let lead_ms = (truth.saturating_duration_since(released).as_secs_f64()
            - released.saturating_duration_since(truth).as_secs_f64())
            * 1000.0;
        let want = RELEASE_LEAD.as_secs_f64() * 1000.0;
        eprintln!("bırakma: sınırdan {lead_ms:.1} ms önce (hedef {want})");
        assert!(
            (lead_ms - want).abs() <= 8.0,
            "sınırdan {lead_ms:.1} ms önce bırakıldı (hedef {want})"
        );
    }
}
