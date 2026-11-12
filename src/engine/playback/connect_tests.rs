//! Sürücü geçişleri ve Spirc kolu testleri (PLAN-MOTOR-CONNECT.md 7.5, 7.6):
//! sahte çalar (`FakePort`), sahte Spirc ucu (`FakeConnect`) ve sahte Spirc
//! olayları; ağ, ses kartı, librespot oturumu ve gerçek Spirc gerekmez.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use librespot::connect::{ClusterView, DeviceView, HostView, SpircEvent};

use super::{
    Cx, Driver, Effect, Gate, LinkFacts, Playback, PlayerSignal, TransportCmd,
    testkit::{self, Call, Spy},
};
use crate::engine::{
    ConnectDriver, ConnectPort, ConnectStatus, Event, OutputCtl, PlaybackEvent, TrackInfo, queue::Queue,
};

fn uri(i: usize) -> String {
    format!("spotify:track:{i:022}")
}

fn info(i: usize) -> TrackInfo {
    TrackInfo {
        uri: uri(i),
        name: format!("Şarkı {i}"),
        artists: "Sanatçı".into(),
        duration_ms: 200_000,
        cover_url: None,
    }
}

/// Sahte Spirc ucu: komutları sırayla kaydeder; duraklatma çağrıldığı anda
/// çıkışın susturulmuş olup olmadığını da (gecikme sınaması).
struct FakeConnect {
    calls: Mutex<Vec<String>>,
    out: Arc<OutputCtl>,
    silent_at_pause: Mutex<Option<bool>>,
}

impl FakeConnect {
    fn new(out: Arc<OutputCtl>) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::default(), out, silent_at_pause: Mutex::default() })
    }

    fn log(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

impl ConnectPort for FakeConnect {
    fn set_host_drives(&self, on: bool) {
        self.log(format!("host_drives({on})"));
    }
    fn transfer_here(&self) {
        self.log("transfer_here");
    }
    fn leave(&self) {
        self.log("leave");
    }
    fn play(&self) {
        self.log("play");
    }
    fn pause(&self) {
        *self.silent_at_pause.lock().unwrap() = Some(self.out.is_ducked());
        self.log("pause");
    }
    fn seek(&self, position_ms: u32) {
        self.log(format!("seek({position_ms})"));
    }
    fn next(&self) {
        self.log("next");
    }
    fn prev(&self) {
        self.log("prev");
    }
    fn set_volume(&self, volume: u16) {
        self.log(format!("volume({volume})"));
    }
    fn add_to_queue(&self, uri: &str) {
        self.log(format!("enqueue({uri})"));
    }
    fn shutdown(&self) {
        self.log("shutdown");
    }
}

struct Bench {
    pb: Playback,
    queue: Queue,
    meta: HashMap<String, crate::engine::TrackRow>,
    spy: Spy,
    out: Arc<OutputCtl>,
    port: Arc<FakeConnect>,
    events: async_channel::Receiver<Event>,
    #[allow(dead_code)]
    view: async_channel::Receiver<PlaybackEvent>,
}

impl Bench {
    /// Yerelde 3 şarkılık sıra başlamış, 1. şarkı 42 sn'de çalıyor; Spirc hazır.
    fn playing() -> Self {
        let (deck, spy, out) = testkit::fake_deck();
        let (tx, events) = async_channel::unbounded();
        let (ptx, view) = async_channel::unbounded();
        let mut pb = Playback::new(Gate::new(tx, ptx), 0);
        pb.deck = Some(deck);
        let mut queue = Queue::new();
        queue.restart(None, (0..3).map(uri).collect(), None, 0, 1, true).unwrap();
        let port = FakeConnect::new(out.clone());
        let mut b = Self { pb, queue, meta: HashMap::new(), spy, out, port, events, view };
        let first = b.queue.current_uri().unwrap().to_owned();
        b.run(|pb, cx| pb.begin(cx, first, 0));
        b.signal(PlayerSignal::TrackChanged(info(0)));
        b.signal(PlayerSignal::Playing { position_ms: 42_000 });
        b.spy.take();
        b
    }

    /// `playing` + Spirc bağlandı (ad "Openify (test)", kimlik "me").
    fn with_spirc() -> Self {
        let mut b = Self::playing();
        let port: Arc<dyn ConnectPort> = b.port.clone();
        b.run(|pb, cx| pb.connect_up(cx, port, "me".into(), "Openify (test)".into()));
        b
    }

    fn run<R>(&mut self, f: impl FnOnce(&mut Playback, &mut Cx<'_>) -> R) -> (R, Vec<Effect>) {
        let mut fx = Vec::new();
        let result = f(
            &mut self.pb,
            &mut Cx { queue: &mut self.queue, meta: &self.meta, link: LinkFacts::default(), fx: &mut fx },
        );
        (result, fx)
    }

    fn signal(&mut self, signal: PlayerSignal) -> Vec<Effect> {
        self.run(|pb, cx| pb.on_signal(cx, signal)).1
    }

    fn transport(&mut self, cmd: TransportCmd) -> Vec<Effect> {
        self.run(|pb, cx| pb.transport(cx, cmd)).1
    }

    fn spirc(&mut self, event: SpircEvent) -> Vec<Effect> {
        self.run(|pb, cx| pb.on_spirc(cx, event)).1
    }

    /// Çalara giden komutlar (işaretler hariç: onlar çıkış kapısının).
    fn player_commands(&self) -> Vec<Call> {
        self.spy.take().into_iter().filter(|c| !matches!(c, Call::Mark(_))).collect()
    }

    fn driver(&self) -> ConnectDriver {
        match self.pb.driver {
            Driver::Local(_) => ConnectDriver::Local,
            Driver::Spirc(_) => ConnectDriver::Spirc,
            Driver::Remote(_) => ConnectDriver::Remote,
        }
    }

    /// Telefon aktardı (bant içi işaret).
    fn attach(&mut self) -> Vec<Effect> {
        self.signal(PlayerSignal::RemoteAttached)
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        self.pb.drop_handover();
    }
}

fn statuses(fx: &[Effect]) -> Vec<ConnectStatus> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Connect(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

fn emitted(fx: &[Effect]) -> Vec<&Event> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Emit(ev) => Some(ev),
            _ => None,
        })
        .collect()
}

fn host(active: bool, playing: bool, paused: bool) -> SpircEvent {
    SpircEvent::State(HostView { active, playing, paused, ..HostView::default() })
}

/// Küme: `active` aygıt `playing` çalıyor; aygıtlar "me" (bu) ve "phone" ("Pixel 8").
fn cluster(active: &str, playing: bool) -> SpircEvent {
    SpircEvent::Cluster(ClusterView {
        active_device_id: active.into(),
        devices: vec![
            DeviceView { id: "me".into(), name: "Openify (test)".into(), device_type: 1, volume: 1 },
            DeviceView { id: "phone".into(), name: "Pixel 8".into(), device_type: 3, volume: 1 },
        ],
        playing,
        paused: !playing,
        ..ClusterView::default()
    })
}

// ── Connect kapalıyken hiçbir şey değişmez ──────────────────────────────

#[test]
fn without_spirc_everything_stays_local_and_nothing_is_published() {
    let mut b = Bench::playing();
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert!(b.pb.link.port.is_none() && b.pb.link.name.is_empty());
    // Yerel çalmanın her adımı: ne bir sürücü geçişi ne bir Connect olayı.
    let mut all = Vec::new();
    all.extend(b.signal(PlayerSignal::PreloadDue));
    all.extend(b.signal(PlayerSignal::Paused { position_ms: 50_000 }));
    all.extend(b.transport(TransportCmd::Play));
    all.extend(b.signal(PlayerSignal::EndOfTrack));
    all.extend(b.transport(TransportCmd::Next));
    all.extend(b.transport(TransportCmd::Pause));
    assert!(statuses(&all).is_empty(), "Connect kapalıyken arayüze Connect durumu gitmez");
    assert_eq!(b.driver(), ConnectDriver::Local);
    // Çalar komutları bugünkü gibi gidiyor (preload, load, pause…).
    let calls = b.player_commands();
    assert!(calls.iter().any(|c| matches!(c, Call::Preload(_))), "{calls:?}");
    assert!(calls.iter().any(|c| matches!(c, Call::Load { .. })), "{calls:?}");
    assert!(calls.iter().any(|c| matches!(c, Call::Pause)), "{calls:?}");
}

#[test]
fn spirc_events_without_a_link_are_harmless() {
    // Spirc bağlı değilken (kapalı) küme bildirimi gelse de sürücü değişmez.
    let mut b = Bench::playing();
    b.signal(PlayerSignal::Paused { position_ms: 1 });
    let fx = b.spirc(cluster("phone", true));
    // Küme kimliksiz (self_id boş) olsa da: başka aygıt çalıyor ve yerel boşta
    // → Remote; Spirc yokken bu olay zaten gelmez, ama gelse de çökmez.
    assert!(matches!(b.driver(), ConnectDriver::Remote | ConnectDriver::Local), "{fx:?}");
}

// ── Telefon aktarır (Local → Spirc) ─────────────────────────────────────

#[test]
fn phone_transfer_attaches_spirc_flushes_old_audio_and_sends_no_player_command() {
    let mut b = Bench::with_spirc();
    let flushed_before = b.out.flush_requested();
    let fx = b.attach();
    assert_eq!(b.driver(), ConnectDriver::Spirc);
    // Yerelde çalan sesin kuyruktaki kalanı atılır: işaret var, çalar komutu yok.
    assert!(b.out.flush_requested() > flushed_before, "kuyruk atılmadı");
    let calls = b.spy.take();
    assert!(matches!(calls.as_slice(), [Call::Mark(_)]), "yalnız işaret: {calls:?}");
    // Arayüz: ileri / geri açık; Connect durumu Spirc.
    assert!(
        emitted(&fx).iter().any(|e| matches!(e, Event::QueueChanged(q) if q.has_next && q.has_previous)),
        "{fx:?}"
    );
    let shown = statuses(&fx);
    assert_eq!(shown.len(), 1);
    assert_eq!((shown[0].driver, shown[0].device_name.as_str()), (ConnectDriver::Spirc, "Openify (test)"));
    // Tekrarı etkisiz (bant içi işaret + durum bildirimi ikisi de gelir).
    let again = b.attach();
    assert!(again.is_empty() && b.spy.take().is_empty());
    let fx = b.spirc(host(true, true, false));
    assert!(statuses(&fx).is_empty());
}

#[test]
fn spirc_state_event_attaches_when_the_in_band_signal_is_missing() {
    let mut b = Bench::with_spirc();
    b.spirc(host(true, true, false));
    assert_eq!(b.driver(), ConnectDriver::Spirc);
}

#[test]
fn spirc_arm_never_commands_the_player() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.spy.take();
    // Çaldan gelen her olay: Spirc'in işi, bizim değil.
    for s in [
        PlayerSignal::Loading,
        PlayerSignal::TrackChanged(info(7)),
        PlayerSignal::Playing { position_ms: 0 },
        PlayerSignal::PreloadDue,
        PlayerSignal::Position { position_ms: 9_000 },
        PlayerSignal::EndOfTrack,
        PlayerSignal::Unavailable { uri: Some(uri(8)) },
        PlayerSignal::Loading,
        PlayerSignal::Playing { position_ms: 0 },
        PlayerSignal::Paused { position_ms: 3_000 },
    ] {
        b.signal(s);
    }
    // Kendi düğmelerimiz: Spirc'e iletilir.
    for c in [
        TransportCmd::Play,
        TransportCmd::Pause,
        TransportCmd::Seek(5_000),
        TransportCmd::Next,
        TransportCmd::Previous,
        TransportCmd::SetVolume(0.5),
    ] {
        b.transport(c);
    }
    assert!(b.player_commands().is_empty(), "çalara komut gitmedi");
    assert_eq!(
        b.port.take(),
        ["play", "pause", "seek(5000)", "next", "prev", "volume(32767)"],
        "komutlar Spirc'e gitti"
    );
    assert!(b.pb.load_started.is_none() && !b.pb.preload_armed, "yükleme izleme / önyükleme yerelin işi");
}

#[test]
fn spirc_arm_surfaces_the_phones_track_and_position_to_the_ui() {
    let mut b = Bench::with_spirc();
    b.attach();
    let fx = b.signal(PlayerSignal::TrackChanged(info(9)));
    assert!(emitted(&fx).iter().any(|e| matches!(e, Event::TrackChanged(t) if t.uri == uri(9))), "{fx:?}");
    let fx = b.signal(PlayerSignal::Playing { position_ms: 1_000 });
    assert!(emitted(&fx).iter().any(|e| matches!(e, Event::Playing { .. })));
    assert_eq!(b.pb.track_info.as_ref().map(|t| t.uri.clone()), Some(uri(9)));
}

// ── Duraklatma anında susar ─────────────────────────────────────────────

#[test]
fn phone_pause_silences_the_output_without_waiting_for_the_queue() {
    // Telefonun duraklatması çalardan ÖNCE Spirc'in durum bildirimiyle gelir:
    // çıkış hemen kısılır (kuyruktaki ses beklenmez).
    let mut b = Bench::with_spirc();
    b.attach();
    b.spirc(host(true, true, false));
    assert!(!b.out.is_ducked());
    b.spirc(host(true, false, true));
    assert!(b.out.is_ducked(), "ses hemen kısılmalı");
    // Çalar `Paused`ı sonra bildirince ikinci kez susturulmaz (işaret sayısı artmaz).
    let marks = b.out.flush_requested();
    let before = b.spy.take().len();
    b.signal(PlayerSignal::Paused { position_ms: 10_000 });
    assert_eq!(b.spy.take().len(), 0, "ikinci işaret yok ({before} önceki)");
    assert_eq!(b.out.flush_requested(), marks);
    assert!(b.out.is_ducked());
}

#[test]
fn phone_pause_is_silenced_by_the_player_event_alone_too() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.signal(PlayerSignal::Playing { position_ms: 0 });
    b.signal(PlayerSignal::Paused { position_ms: 10_000 });
    assert!(b.out.is_ducked());
    // Telefon yeniden çalınca (çalar `Playing`): susturma kalkar.
    b.signal(PlayerSignal::Playing { position_ms: 10_000 });
    assert!(!b.out.is_ducked(), "çalma sürünce ses geri gelir");
}

#[test]
fn own_pause_silences_before_spirc_is_asked_to_pause() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.transport(TransportCmd::Pause);
    assert_eq!(*b.port.silent_at_pause.lock().unwrap(), Some(true), "önce sustur, sonra Spirc'e ilet");
    assert!(b.player_commands().is_empty());
    // Çal: önce susturma kalkar.
    b.transport(TransportCmd::Play);
    assert!(!b.out.is_ducked());
    assert_eq!(b.port.take(), ["pause", "play"]);
}

// ── Telefon şarkı değiştirir / sarar ────────────────────────────────────

#[test]
fn phone_skip_flushes_but_an_automatic_advance_does_not() {
    let mut b = Bench::with_spirc();
    b.attach();
    // Etkinleşmenin ilk yüklemesi zaten yıkandı.
    b.signal(PlayerSignal::Loading);
    let base = b.out.flush_requested();
    // Şarkı kendiliğinden bitti, Spirc sıradakini yükledi: yıkama yok (boşluksuz).
    b.signal(PlayerSignal::EndOfTrack);
    b.signal(PlayerSignal::Loading);
    assert_eq!(b.out.flush_requested(), base, "kendiliğinden geçişte kuyruk atılmaz");
    // Telefonda "sonraki": yükleme şarkı bitmeden geldi → eski ses atılır.
    b.signal(PlayerSignal::Loading);
    assert!(b.out.flush_requested() > base, "telefon atladı: eski ses atılmalı");
    assert!(b.player_commands().is_empty());
}

#[test]
fn own_next_and_seek_flush_first_and_the_resulting_load_is_not_flushed_twice() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.signal(PlayerSignal::Loading);
    let before = b.out.flush_requested();
    b.transport(TransportCmd::Next);
    let after_cmd = b.out.flush_requested();
    assert!(after_cmd > before, "kendi Sonraki'miz kuyruğu önce atar");
    b.signal(PlayerSignal::Loading);
    assert_eq!(b.out.flush_requested(), after_cmd, "komutun yüklemesi yeniden atmaz");
    b.transport(TransportCmd::Seek(1_000));
    assert!(b.out.flush_requested() > after_cmd, "kendi sarmamız kuyruğu atar");
}

// ── Telefon geri alır (Spirc → Local) ───────────────────────────────────

#[test]
fn phone_taking_back_returns_the_ui_to_the_paused_local_track() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.signal(PlayerSignal::TrackChanged(info(9)));
    b.signal(PlayerSignal::Playing { position_ms: 5_000 });
    b.spy.take();
    let flushed = b.out.flush_requested();
    let fx = b.signal(PlayerSignal::RemoteDetached);
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert!(b.out.flush_requested() > flushed, "telefonun sesinin kuyruktaki kalanı atılır");
    assert!(b.player_commands().is_empty(), "çalara komut yok (telefon zaten `stop` verir)");
    // Arayüz: aktarmadan önce gösterilen yerel şarkı, kaldığı konumda duraklatılmış.
    let ev = emitted(&fx);
    assert!(ev.iter().any(|e| matches!(e, Event::TrackChanged(t) if t.uri == uri(0))), "{fx:?}");
    assert!(ev.iter().any(|e| matches!(e, Event::Paused { position_ms } if (42_000..=43_000).contains(position_ms))));
    assert!(fx.iter().any(|e| matches!(e, Effect::QueueChanged)));
    assert!(b.pb.restored.is_some(), "Çal yerel şarkıyı kaldığı yerden yükler");
    assert_eq!(statuses(&fx).last().map(|s| s.driver), Some(ConnectDriver::Local));
    // Çal: yerel sürücü yükler (kullanıcı yüklemesi: kuyruk atılır).
    b.transport(TransportCmd::Play);
    let calls = b.player_commands();
    assert!(matches!(calls.as_slice(), [Call::Load { uri: u, play: true, position_ms }] if *u == uri(0) && *position_ms >= 42_000), "{calls:?}");
}

#[test]
fn spirc_state_going_inactive_is_the_backup_for_the_detach() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.spirc(host(false, false, false));
    assert_eq!(b.driver(), ConnectDriver::Local);
}

#[test]
fn phone_takes_back_while_nothing_was_local_clears_the_player() {
    // Açılışta (yerelde şarkı yokken) telefon aktardı, sonra geri aldı.
    let (deck, _spy, out) = testkit::fake_deck();
    let (tx, _events) = async_channel::unbounded();
    let (ptx, _view) = async_channel::unbounded();
    let mut pb = Playback::new(Gate::new(tx, ptx), 0);
    pb.deck = Some(deck);
    let mut queue = Queue::new();
    let meta = HashMap::new();
    let port = FakeConnect::new(out);
    let mut fx = Vec::new();
    {
        let mut cx = Cx { queue: &mut queue, meta: &meta, link: LinkFacts::default(), fx: &mut fx };
        pb.connect_up(&mut cx, port, "me".into(), "Openify".into());
        pb.on_signal(&mut cx, PlayerSignal::RemoteAttached);
        cx.fx.clear();
        pb.on_signal(&mut cx, PlayerSignal::RemoteDetached);
    }
    assert!(emitted(&fx).iter().any(|e| matches!(e, Event::Ended)), "{fx:?}");
    assert!(pb.restored.is_none());
}

// ── Openify'da yeni çalma (Spirc → Local) ───────────────────────────────

#[test]
fn local_start_leaves_spirc_and_plays_locally() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.spy.take();
    b.port.take();
    let fx = b.transport(TransportCmd::Start { uri: uri(2), position_ms: 0 });
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert_eq!(b.port.take(), ["host_drives(true)", "leave"], "Spirc'e: ana uygulama sürüyor, etkinlikten çık");
    let calls = b.player_commands();
    assert!(matches!(calls.as_slice(), [Call::Load { uri: u, play: true, position_ms: 0 }] if *u == uri(2)), "{calls:?}");
    assert!(statuses(&fx).iter().any(|s| s.driver == ConnectDriver::Local), "{fx:?}");
    assert!(!b.out.is_ducked());
}

#[test]
fn jumping_in_the_queue_also_leaves_spirc() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.port.take();
    b.transport(TransportCmd::JumpTo(1));
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert_eq!(b.port.take(), ["host_drives(true)", "leave"]);
}

#[test]
fn enqueue_goes_to_the_phones_queue_only_while_spirc_drives() {
    let mut b = Bench::with_spirc();
    assert!(!b.pb.spirc_enqueue(&[uri(5)]), "yerelde yerel sıraya");
    assert!(b.port.take().is_empty());
    b.attach();
    assert!(b.pb.spirc_enqueue(&[uri(5), uri(6)]));
    assert_eq!(b.port.take(), [format!("enqueue({})", uri(5)), format!("enqueue({})", uri(6))]);
}

// ── Başka aygıtta çalıyor (7.6) ─────────────────────────────────────────

#[test]
fn another_device_playing_turns_an_idle_local_into_remote() {
    let mut b = Bench::with_spirc();
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    let fx = b.spirc(cluster("phone", true));
    assert_eq!(b.driver(), ConnectDriver::Remote);
    let shown = statuses(&fx);
    assert_eq!(shown.len(), 1, "{fx:?}");
    assert_eq!(shown[0].driver, ConnectDriver::Remote);
    assert_eq!(shown[0].remote.as_ref().map(|r| r.device_name.as_str()), Some("Pixel 8"));
    // Aynı küme yeniden gelince yeni bildirim yok.
    assert!(statuses(&b.spirc(cluster("phone", true))).is_empty());
    // Telefon durdurunca: yerel.
    let fx = b.spirc(cluster("phone", false));
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert_eq!(statuses(&fx).last().map(|s| s.driver), Some(ConnectDriver::Local));
}

#[test]
fn local_playback_is_never_turned_into_remote_by_a_cluster_update() {
    let mut b = Bench::with_spirc();
    b.spirc(cluster("phone", true));
    assert_eq!(b.driver(), ConnectDriver::Local, "burada çalarken 'başka aygıtta çalıyor' denmez");
    // Yerel sustu: artık orada çalıyor.
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    assert_eq!(b.driver(), ConnectDriver::Remote);
}

#[test]
fn this_device_being_the_active_one_is_not_remote() {
    let mut b = Bench::with_spirc();
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    b.spirc(cluster("me", true));
    assert_eq!(b.driver(), ConnectDriver::Local);
}

#[test]
fn local_play_starting_leaves_remote() {
    let mut b = Bench::with_spirc();
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    b.spirc(cluster("phone", true));
    assert_eq!(b.driver(), ConnectDriver::Remote);
    let fx = b.signal(PlayerSignal::Loading);
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert_eq!(statuses(&fx).last().map(|s| s.driver), Some(ConnectDriver::Local));
}

#[test]
fn remote_gets_no_commands_only_transfer_here() {
    let mut b = Bench::with_spirc();
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    b.spirc(cluster("phone", true));
    b.port.take();
    // Duraklat / sar: uzak aygıta hiçbir şey gitmez.
    b.transport(TransportCmd::Pause);
    b.transport(TransportCmd::Seek(9_000));
    assert!(b.port.take().is_empty());
    assert_eq!(b.driver(), ConnectDriver::Remote);
    // Çal: burada yerel çalma başlar (uzak aygıta komut yok), Remote'tan çıkılır.
    b.spy.take();
    b.transport(TransportCmd::Play);
    assert!(b.port.take().is_empty(), "uzak aygıta komut gitmedi");
    assert_eq!(b.driver(), ConnectDriver::Local);
    // "Buraya aktar": Spirc'e istek.
    b.pb.transfer_here();
    assert_eq!(b.port.take(), ["transfer_here"]);
}

#[test]
fn transfer_here_is_ignored_while_spirc_already_drives_or_without_spirc() {
    let mut b = Bench::with_spirc();
    b.attach();
    b.port.take();
    b.pb.transfer_here();
    assert!(b.port.take().is_empty());
    // Spirc yoksa (kapalı): hiçbir şey olmaz, çökmez.
    let b = Bench::playing();
    b.pb.transfer_here();
}

#[test]
fn transferring_here_from_remote_ends_with_the_phone_attaching() {
    let mut b = Bench::with_spirc();
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    b.spirc(cluster("phone", true));
    b.pb.transfer_here();
    // Spotify aktardı: bant içi işaret → Spirc.
    let fx = b.attach();
    assert_eq!(b.driver(), ConnectDriver::Spirc);
    assert_eq!(statuses(&fx).last().map(|s| (s.driver, s.remote.clone())), Some((ConnectDriver::Spirc, None)));
}

// ── Oturum yenilenince / Spirc bitince ──────────────────────────────────

#[test]
fn connect_down_returns_to_local_and_clears_the_name() {
    let mut b = Bench::with_spirc();
    b.attach();
    let fx = b.run(|pb, cx| pb.connect_down(cx)).1;
    assert_eq!(b.driver(), ConnectDriver::Local);
    assert!(b.pb.link.port.is_none());
    let last = statuses(&fx).pop().expect("durum bildirildi");
    assert_eq!((last.driver, last.device_name.as_str()), (ConnectDriver::Local, ""));
    // Yeni oturumun Spirc'i yeniden bağlanır (aynı kalıcı kimlik).
    let port: Arc<dyn ConnectPort> = b.port.clone();
    let fx = b.run(|pb, cx| pb.connect_up(cx, port, "me".into(), "Openify (test)".into())).1;
    assert_eq!(statuses(&fx).pop().map(|s| s.device_name), Some("Openify (test)".to_string()));
}

#[test]
fn connect_down_from_remote_also_returns_to_local() {
    let mut b = Bench::with_spirc();
    b.signal(PlayerSignal::Paused { position_ms: 1_000 });
    b.spirc(cluster("phone", true));
    b.run(|pb, cx| pb.connect_down(cx));
    assert_eq!(b.driver(), ConnectDriver::Local);
}

#[test]
fn the_events_channel_is_untouched_by_status_publishing() {
    // `Effect::Connect` arayüz olay kanalına (`Event`) hiçbir şey yazmaz.
    let mut b = Bench::with_spirc();
    b.attach();
    // (Gate'e hiçbir Event gitmedi: yalnız fx listesindeki etkiler.)
    assert!(b.events.try_recv().is_err());
}
