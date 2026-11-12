//! Çalma senaryo testleri: sahte çalar (`FakePort`), aygıtsız sanal saatli çıkış
//! ve gerçek `Queue` ile. Ağ, ses kartı ve librespot oturumu gerekmez. Her
//! test `Playback`e bir sinyal / komut verir; çalara giden çağrılara
//! (`Call`) ve `Cx::fx`e yazılan etkilere (`Effect`) bakar.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use librespot::core::SpotifyUri;

use super::{
    Cx, Effect, Gate, LinkFacts, Playback, PlayerSignal, TransportCmd,
    testkit::{self, Call, Spy},
};
use crate::engine::{
    Command, Event, NoticeKind, OutputCtl, PlaybackEvent, RepeatMode, TrackInfo, TrackRow,
    output::Writer,
    queue::{Queue, UserItem},
    recovery,
};

/// Geçerli biçimli (22 haneli taban-62) şarkı adresleri.
fn uri(i: usize) -> String {
    format!("spotify:track:{i:022}")
}

fn info(i: usize) -> TrackInfo {
    TrackInfo {
        uri: uri(i),
        name: format!("Şarkı {i}"),
        artists: "Sanatçı".into(),
        duration_ms: 200_000,
        cover_url: Some(format!("https://i.scdn.co/image/{i}")),
    }
}

fn is_load(call: &Call, i: usize, position_ms: u32) -> bool {
    matches!(call, Call::Load { uri: u, play: true, position_ms: p } if *u == uri(i) && *p == position_ms)
}

/// Çalma yatağı: `Playback` + sıra + sahte çalar. `started(n)` ilk şarkıyı
/// kullanıcı gibi başlatmış, çağrı kaydını sıfırlamış hâlde verir.
struct Bench {
    pb: Playback,
    queue: Queue,
    meta: HashMap<String, TrackRow>,
    link: LinkFacts,
    spy: Spy,
    out: Arc<OutputCtl>,
    events: async_channel::Receiver<Event>,
    view: async_channel::Receiver<PlaybackEvent>,
}

impl Bench {
    fn new(tracks: usize) -> Self {
        let (deck, spy, out) = testkit::fake_deck();
        let (tx, events) = async_channel::unbounded();
        let (ptx, view) = async_channel::unbounded();
        let mut pb = Playback::new(Gate::new(tx, ptx), 0);
        pb.deck = Some(deck);
        let mut queue = Queue::new();
        queue
            .restart(None, (0..tracks).map(uri).collect(), None, 0, 1, true)
            .unwrap();
        Self {
            pb,
            queue,
            meta: HashMap::new(),
            link: LinkFacts::default(),
            spy,
            out,
            events,
            view,
        }
    }

    fn started(tracks: usize) -> Self {
        let mut b = Self::new(tracks);
        let first = b.queue.current_uri().unwrap().to_owned();
        b.run(|pb, cx| pb.begin(cx, first, 0));
        b.spy.take();
        b
    }

    fn run<R>(&mut self, f: impl FnOnce(&mut Playback, &mut Cx<'_>) -> R) -> (R, Vec<Effect>) {
        let mut fx = Vec::new();
        let result = f(
            &mut self.pb,
            &mut Cx {
                queue: &mut self.queue,
                meta: &self.meta,
                link: self.link,
                fx: &mut fx,
            },
        );
        (result, fx)
    }

    fn signal(&mut self, signal: PlayerSignal) -> Vec<Effect> {
        self.run(|pb, cx| pb.on_signal(cx, signal)).1
    }

    fn transport(&mut self, cmd: TransportCmd) -> Vec<Effect> {
        self.run(|pb, cx| pb.transport(cx, cmd)).1
    }

    fn current(&self) -> String {
        self.queue.current_uri().unwrap().to_owned()
    }

    /// Çıkışın halkasına 1 sn ses yazar: tutma sınaması için "önceki şarkının
    /// kuyruktaki sonu" (sınır henüz duyulmaz; sanal saat ilerlemez).
    fn with_audio(&self) -> Writer {
        let mut w = Writer::new(self.out.clone());
        w.start();
        w.sync();
        self.out.set_track_base(0);
        w.push(&vec![0.1; 44_100 * 2]).unwrap();
        w
    }
}

impl Drop for Bench {
    /// Tutma sürüyorsa bırakma iş parçacığı boşuna dönmesin.
    fn drop(&mut self) {
        self.pb.drop_handover();
    }
}

#[test]
fn test_uris_parse() {
    for i in [0, 1, 9, 123] {
        assert!(SpotifyUri::from_uri(&uri(i)).is_ok(), "{}", uri(i));
    }
}

// ── Deste (5.2) ─────────────────────────────────────────────────────────

fn parsed(i: usize) -> SpotifyUri {
    SpotifyUri::from_uri(&uri(i)).unwrap()
}

#[test]
fn deck_pause_ducks_then_marks_then_pauses() {
    let (deck, spy, _out) = testkit::fake_deck();
    deck.pause();
    let calls = spy.take();
    assert!(
        matches!(calls.as_slice(), [Call::Mark(_), Call::Pause]),
        "{calls:?}"
    );
}

#[test]
fn deck_seek_flushes_then_marks_then_seeks() {
    let (deck, spy, _out) = testkit::fake_deck();
    deck.seek(42_000);
    let calls = spy.take();
    assert!(
        matches!(calls.as_slice(), [Call::Mark(_), Call::Seek(42_000)]),
        "{calls:?}"
    );
    // Her işaret öncekinden farklı: çıkış kapısı komutları ayırt eder.
    deck.pause();
    let Some(Call::Mark(second)) = spy.take().first().cloned() else {
        panic!("işaret yok")
    };
    assert!(matches!(calls[0], Call::Mark(first) if first < second));
}

#[test]
fn deck_user_load_flushes_but_automatic_load_does_not() {
    let (deck, spy, _out) = testkit::fake_deck();
    deck.load(parsed(3), true, 7_000, true);
    let user = spy.take();
    assert!(
        matches!(user.as_slice(), [Call::Mark(_), c] if is_load(c, 3, 7_000)),
        "kullanıcı yüklemesi önce kuyruğu atar: {user:?}"
    );
    deck.load(parsed(4), true, 0, false);
    let auto = spy.take();
    assert!(
        matches!(auto.as_slice(), [c] if is_load(c, 4, 0)),
        "kendiliğinden geçişte kuyruk atılmaz (boşluksuz): {auto:?}"
    );
}

#[test]
fn deck_play_unducks_and_plays() {
    let (deck, spy, _out) = testkit::fake_deck();
    deck.play();
    assert_eq!(spy.take(), [Call::Play]);
}

// ── Sırada ilerleme ─────────────────────────────────────────────────────

#[test]
fn natural_transition_loads_the_next_track_without_flushing_and_holds_ui_events() {
    let mut b = Bench::started(3);
    let _audio = b.with_audio();
    let fx = b.signal(PlayerSignal::EndOfTrack);
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [c] if is_load(c, 1, 0)),
        "{calls:?}"
    );
    assert!(matches!(fx.as_slice(), [Effect::QueueChanged]), "{fx:?}");
    assert_eq!(b.current(), uri(1));
    assert!(
        b.pb.gate.is_holding(),
        "arayüz sınır duyulana dek önceki şarkıda kalır"
    );
    // Tutma sürerken gelen olaylar arayüze gitmez.
    b.pb.gate.send(Event::TrackChanged(info(1)));
    assert!(b.events.try_recv().is_err() && b.view.try_recv().is_err());
}

#[test]
fn repeat_one_reloads_the_same_track_at_the_end() {
    let mut b = Bench::started(3);
    b.queue.set_repeat(RepeatMode::One);
    b.signal(PlayerSignal::EndOfTrack);
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [c] if is_load(c, 0, 0)),
        "{calls:?}"
    );
    assert_eq!(b.current(), uri(0));
}

#[test]
fn end_of_the_queue_announces_the_end_and_saves_position_zero() {
    let mut b = Bench::started(1);
    let fx = b.signal(PlayerSignal::EndOfTrack);
    assert!(b.spy.take().is_empty(), "yüklenecek şarkı yok");
    assert!(
        matches!(
            fx.as_slice(),
            [Effect::Emit(Event::Ended), Effect::Persist(Some(0))]
        ),
        "{fx:?}"
    );
}

#[test]
fn user_next_flushes_the_old_audio_and_reports_the_queue_change() {
    let mut b = Bench::started(3);
    let fx = b.transport(TransportCmd::Next);
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [Call::Mark(_), c] if is_load(c, 1, 0)),
        "{calls:?}"
    );
    assert!(matches!(fx.as_slice(), [Effect::QueueChanged]), "{fx:?}");
}

// ── Önyükleme ───────────────────────────────────────────────────────────

#[test]
fn preload_is_requested_once_and_refreshed_when_the_queue_changes() {
    let mut b = Bench::started(3);
    b.signal(PlayerSignal::PreloadDue);
    assert_eq!(b.spy.take(), [Call::Preload(uri(1))]);
    // Aynı şarkı yeniden istenmez.
    b.signal(PlayerSignal::PreloadDue);
    assert!(b.spy.take().is_empty());
    // Sıraya "sonra çal" eklenince doğru sıradaki yeniden önyüklenir.
    b.queue.play_next(vec![UserItem {
        uri: uri(9),
        track: None,
    }]);
    b.pb.refresh_preload(&b.queue);
    assert_eq!(b.spy.take(), [Call::Preload(uri(9))]);
    // Önyükleme hiç istenmediyse sıra değişimi çalara bir şey söylemez.
    let mut idle = Bench::started(3);
    idle.queue.play_next(vec![UserItem {
        uri: uri(9),
        track: None,
    }]);
    idle.pb.refresh_preload(&idle.queue);
    assert!(idle.spy.take().is_empty());
}

// ── Kurtarma ────────────────────────────────────────────────────────────

#[test]
fn key_failure_reconnects_and_retries_the_same_track_from_where_it_was() {
    let mut b = Bench::started(3);
    b.signal(PlayerSignal::Playing {
        position_ms: 12_000,
    });
    b.spy.fail_key();
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(uri(0)) });
    assert!(
        matches!(
            fx.as_slice(),
            [Effect::Reconnect {
                retry_current: true,
                skip_if_refused: true,
                started: Some(_)
            }]
        ),
        "{fx:?}"
    );
    assert!(
        b.spy.take().is_empty(),
        "bağlanma bitmeden çalara dokunulmaz"
    );
    // Motor bağlanmayı başlattı (`reconnect(true)`) ve bağlanma bitti.
    b.pb.retry_after_reconnect = true;
    b.run(|pb, cx| pb.after_reconnect(cx));
    let calls = b.spy.take();
    let [
        Call::Mark(_),
        Call::Load {
            uri: u,
            play: true,
            position_ms,
        },
    ] = calls.as_slice()
    else {
        panic!("{calls:?}");
    };
    assert_eq!(*u, uri(0), "aynı şarkı");
    assert!(
        (12_000..12_500).contains(position_ms),
        "kaldığı konumdan: {position_ms}"
    );
    assert_eq!(b.pb.retried_uri.as_deref(), Some(uri(0).as_str()));
    assert!(
        b.pb.ignore_stale_failure,
        "eski yüklemenin geç hatası yok sayılacak"
    );
}

#[test]
fn a_second_failure_of_the_same_track_skips_to_the_next() {
    let mut b = Bench::started(3);
    b.pb.retried_uri = Some(uri(0));
    b.spy.fail_key();
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(uri(0)) });
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [c] if is_load(c, 1, 0)),
        "atlama kuyruğu atmaz: {calls:?}"
    );
    assert!(
        matches!(
            fx.as_slice(),
            [
                Effect::QueueChanged,
                Effect::Notice {
                    kind: NoticeKind::Info,
                    ..
                }
            ]
        ),
        "{fx:?}"
    );
    assert!(!b.pb.retry_after_reconnect);
}

#[test]
fn the_fourth_consecutive_failure_stops_playback() {
    let mut b = Bench::started(6);
    // Geri çekilme sürüyor: yeniden bağlanma yok, her hata atlatır.
    b.link.since_reconnect = Some(Duration::from_secs(1));
    for n in 1..=3 {
        let current = b.current();
        let fx = b.signal(PlayerSignal::Unavailable { uri: Some(current) });
        let calls = b.spy.take();
        assert!(
            matches!(calls.as_slice(), [c] if is_load(c, n, 0)),
            "{n}. hata atlatır: {calls:?}"
        );
        assert!(
            matches!(fx.as_slice(), [Effect::QueueChanged, Effect::Notice { .. }]),
            "{fx:?}"
        );
    }
    let current = b.current();
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(current) });
    assert!(b.spy.take().is_empty(), "4. hatada başka şarkı yüklenmez");
    assert!(
        matches!(
            fx.as_slice(),
            [
                Effect::Emit(Event::Unavailable),
                Effect::Notice {
                    kind: NoticeKind::Problem,
                    ..
                }
            ]
        ),
        "{fx:?}"
    );
    assert_eq!(b.pb.failures, 0, "sayaç sıfırlanır");
}

#[test]
fn a_preload_failure_does_not_skip_the_playing_track() {
    let mut b = Bench::started(3);
    // Önyüklenen sıradaki şarkı çalınamadı, çalan şarkı sürüyor.
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(uri(1)) });
    assert!(fx.is_empty(), "{fx:?}");
    assert!(b.spy.take().is_empty());
    assert_eq!((b.pb.failures, b.current()), (0, uri(0)));
    // Anahtar hatasıyla geldiyse oturum şüpheli: yalnız yeniden bağlan,
    // çalan şarkıyı yeniden yükleme.
    b.spy.fail_key();
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(uri(1)) });
    assert!(
        matches!(
            fx.as_slice(),
            [Effect::Reconnect {
                retry_current: false,
                skip_if_refused: false,
                started: None
            }]
        ),
        "{fx:?}"
    );
    assert!(b.spy.take().is_empty());
}

#[test]
fn a_stale_failure_from_before_the_reload_is_ignored() {
    let mut b = Bench::started(3);
    b.pb.ignore_stale_failure = true;
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(uri(0)) });
    assert!(fx.is_empty() && b.spy.take().is_empty());
    assert_eq!(b.pb.failures, 0);
    // Yeni yüklemenin ilk olayı bayrağı kaldırır: sonraki hata sayılır.
    b.signal(PlayerSignal::Loading);
    assert!(!b.pb.ignore_stale_failure);
    let fx = b.signal(PlayerSignal::Unavailable { uri: Some(uri(0)) });
    assert!(!fx.is_empty());
    assert_eq!(b.pb.failures, 1);
}

#[test]
fn stall_deadline_only_while_a_fresh_load_is_pending() {
    let mut b = Bench::started(3);
    let current = b.current();
    let started = b.pb.load_started.expect("yükleme başladı");
    assert_eq!(
        b.pb.stall_deadline(Some(current.as_str()), false),
        Some(started + recovery::LOAD_STALL)
    );
    assert_eq!(
        b.pb.stall_deadline(Some(current.as_str()), true),
        None,
        "yeniden bağlanma sürüyor"
    );
    b.pb.retried_uri = Some(current.clone());
    assert_eq!(
        b.pb.stall_deadline(Some(current.as_str()), false),
        None,
        "bu şarkı için yeniden denendi"
    );
    assert_eq!(
        b.pb.stall_deadline(Some(uri(1).as_str()), false),
        Some(started + recovery::LOAD_STALL),
        "başka şarkının yeniden denemesi sayılmaz"
    );
    b.pb.retried_uri = None;
    b.pb.stall_fired = true;
    assert_eq!(
        b.pb.stall_deadline(Some(current.as_str()), false),
        None,
        "bu yükleme için zaten işlendi"
    );
    b.pb.stall_fired = false;
    b.signal(PlayerSignal::Playing { position_ms: 0 });
    assert_eq!(
        b.pb.stall_deadline(Some(current.as_str()), false),
        None,
        "çalmaya başladı"
    );
}

#[test]
fn a_stalled_load_reconnects_once_and_respects_backoff() {
    let mut b = Bench::started(3);
    // Henüz erken: işaretlenir ama bağlanma istenmez.
    b.pb.load_started = Some(Instant::now() - Duration::from_secs(1));
    let (_, fx) = b.run(|pb, cx| pb.on_load_stall(cx));
    assert!(fx.is_empty() && b.pb.stall_fired);
    // Eşik aşıldı: oturum ölü sayılır.
    b.pb.stall_fired = false;
    b.pb.load_started = Some(Instant::now() - Duration::from_secs(6));
    let (_, fx) = b.run(|pb, cx| pb.on_load_stall(cx));
    assert!(
        matches!(
            fx.as_slice(),
            [Effect::Reconnect {
                retry_current: true,
                skip_if_refused: false,
                started: Some(_)
            }]
        ),
        "{fx:?}"
    );
    // Bağlanma sürüyorsa ya da geri çekilme dolmadıysa istenmez.
    b.link.reconnecting = true;
    b.pb.load_started = Some(Instant::now() - Duration::from_secs(6));
    assert!(b.run(|pb, cx| pb.on_load_stall(cx)).1.is_empty());
    b.link = LinkFacts {
        since_reconnect: Some(Duration::from_secs(2)),
        ..LinkFacts::default()
    };
    b.pb.load_started = Some(Instant::now() - Duration::from_secs(6));
    assert!(b.run(|pb, cx| pb.on_load_stall(cx)).1.is_empty());
}

// ── Geri yüklenen şarkı ─────────────────────────────────────────────────

#[test]
fn a_restored_track_plays_from_its_saved_position() {
    let mut b = Bench::new(3);
    b.pb.restored = Some(83_000);
    let fx = b.transport(TransportCmd::Play);
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [Call::Mark(_), c] if is_load(c, 0, 83_000)),
        "kaldığı konumdan, kuyruk atılarak: {calls:?}"
    );
    assert!(fx.is_empty(), "{fx:?}");
    assert_eq!((b.pb.restored, b.pb.load_position), (None, 83_000));
    // Yüklenmemiş şarkıda sarma ve duraklatma çalara gitmez.
    let mut r = Bench::new(3);
    r.pb.restored = Some(10_000);
    let fx = r.transport(TransportCmd::Seek(20_000));
    assert!(r.spy.take().is_empty());
    assert!(matches!(
        fx.as_slice(),
        [
            Effect::Emit(Event::Position {
                position_ms: 20_000
            }),
            Effect::Persist(None)
        ]
    ));
    assert!(r.transport(TransportCmd::Pause).is_empty() && r.spy.take().is_empty());
    assert_eq!(r.pb.restored, Some(20_000));
}

// ── Boşluksuz geçişte tutma ─────────────────────────────────────────────

#[test]
fn a_seek_while_the_ui_still_shows_the_old_track_returns_to_it() {
    let mut b = Bench::started(3);
    let _audio = b.with_audio();
    b.signal(PlayerSignal::EndOfTrack);
    b.spy.take();
    assert_eq!(b.current(), uri(1), "çalar yeni şarkıya geçti");
    let (command, fx) = b.run(|pb, cx| pb.handover_command(cx, Command::Seek(5_000)));
    assert!(command.is_none(), "komut burada bitti");
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [Call::Mark(_), c] if is_load(c, 0, 5_000)),
        "arayüzün gösterdiği eski şarkı o konumdan yeniden yüklenir: {calls:?}"
    );
    assert_eq!(b.current(), uri(0), "sıra geri alındı");
    assert!(matches!(fx.as_slice(), [Effect::QueueChanged]), "{fx:?}");
    assert!(!b.pb.gate.is_holding());
}

#[test]
fn next_while_held_restarts_the_new_track_instead_of_skipping_it() {
    let mut b = Bench::started(3);
    let _audio = b.with_audio();
    b.signal(PlayerSignal::EndOfTrack);
    b.spy.take();
    let (command, _) = b.run(|pb, cx| pb.handover_command(cx, Command::Next));
    assert!(command.is_none());
    let calls = b.spy.take();
    assert!(
        matches!(calls.as_slice(), [Call::Mark(_), Call::Seek(0)]),
        "{calls:?}"
    );
    assert_eq!(
        b.current(),
        uri(1),
        "sıra yerinde: arayüzde 'sıradaki' zaten bu şarkıydı"
    );
    assert!(!b.pb.gate.is_holding());
}

#[test]
fn pause_while_held_passes_through_untouched() {
    let mut b = Bench::started(3);
    let _audio = b.with_audio();
    b.signal(PlayerSignal::EndOfTrack);
    let (command, _) = b.run(|pb, cx| pb.handover_command(cx, Command::Pause));
    assert!(matches!(command, Some(Command::Pause)));
    assert!(b.pb.gate.is_holding(), "duraklatma tutmayı bozmaz");
}

// ── Etki sırası ─────────────────────────────────────────────────────────

#[test]
fn track_change_effects_keep_their_order() {
    let mut b = Bench::started(2);
    b.pb.last_position = Some((5_000, Instant::now(), true));
    let fx = b.signal(PlayerSignal::TrackChanged(info(1)));
    assert!(
        matches!(
            fx.as_slice(),
            [
                Effect::FetchCover { .. },
                Effect::Persist(None),
                Effect::Emit(Event::TrackChanged(_))
            ]
        ),
        "{fx:?}"
    );
    assert!(
        b.pb.last_position.is_none(),
        "yeni şarkıda eski konum geçersiz"
    );
    assert_eq!(
        b.pb.track_info.as_ref().map(|t| t.uri.clone()),
        Some(uri(1))
    );
}

#[test]
fn pause_saves_before_it_reports() {
    let mut b = Bench::started(2);
    let fx = b.signal(PlayerSignal::Paused {
        position_ms: 30_000,
    });
    assert!(
        matches!(
            fx.as_slice(),
            [Effect::Persist(None), Effect::Emit(Event::Paused { .. })]
        ),
        "{fx:?}"
    );
    assert!(matches!(b.pb.last_position, Some((_, _, false))));
}

#[test]
fn resuming_after_a_pause_reports_the_audible_position() {
    let mut b = Bench::started(2);
    b.signal(PlayerSignal::Paused {
        position_ms: 30_000,
    });
    let fx = b.signal(PlayerSignal::Playing {
        position_ms: 30_000,
    });
    assert!(
        matches!(fx.as_slice(), [Effect::Emit(Event::Playing { .. })]),
        "{fx:?}"
    );
    assert!(matches!(b.pb.last_position, Some((_, _, true))));
    assert_eq!(b.pb.failures, 0);
}
