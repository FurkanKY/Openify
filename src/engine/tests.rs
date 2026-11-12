//! Motor sınamaları: bağlantısız motor akışı (kaldığı yer, sıra kuralları,
//! çalma etkilerinin sırası). Çalma kararlarının kendisi `playback/tests.rs`'te.

use super::*;

/// Ayarlar sayfasının ses kalitesi ve eşitleme seçimi çalara ulaşıyor:
/// çalar bir şarkıyı yüklerken `config::live`'dan okur (vendor yaması).
#[test]
fn audio_settings_reach_the_player_config() {
    use librespot::playback::config::live;
    let seen = |default: Bitrate| live::bitrate(default);
    apply_audio(96, false);
    assert_eq!(seen(Bitrate::Bitrate320), Bitrate::Bitrate96);
    assert!(!live::normalisation(true));
    apply_audio(160, true);
    assert_eq!(seen(Bitrate::Bitrate320), Bitrate::Bitrate160);
    assert!(live::normalisation(false));
    apply_audio(320, true);
    assert_eq!(seen(Bitrate::Bitrate96), Bitrate::Bitrate320);
    // Ayar verilmemişse çaların kendi yapılandırması geçerli.
    live::set_bitrate(None);
    live::set_normalisation(None);
    assert_eq!(seen(Bitrate::Bitrate160), Bitrate::Bitrate160);
    assert!(live::normalisation(true));
}

/// Gerçek (ama geçerli biçimli) Spotify adresleri: model testleri
/// `t0, t1…` kullanıyor, burada çalara verilecek adres çözümlemesi sınanır.
fn track_uri(i: usize) -> String {
    format!("spotify:track:{}", &["2WUy2Uywcj5cP0IXQagO3z", "4uLU6hMCjMI75M1A2tKUQC"][i % 2])
}

fn queue(len: usize, index: usize) -> Queue {
    let mut q = Queue::new();
    q.restart(
        Some(CollectionId::Liked),
        (0..len).map(track_uri).collect(),
        None,
        index,
        1,
        true,
    );
    q
}

#[test]
fn queue_state_edges() {
    let s = queue_state(&queue(3, 0));
    assert!(s.has_next && !s.has_previous);
    let s = queue_state(&queue(3, 2));
    assert!(!s.has_next && s.has_previous);
    let s = queue_state(&queue(1, 0));
    assert!(!s.has_next && !s.has_previous);
}

#[test]
fn queue_state_reports_context_and_original_index() {
    let mut q = queue(5, 3);
    let s = queue_state(&q);
    assert_eq!(s.context, Some(CollectionId::Liked));
    assert_eq!(s.index, 3);
    // Karıştırınca indeks özgün bağlam sırasında kalır.
    q.set_shuffle(true);
    q.next();
    let s = queue_state(&q);
    assert_eq!(s.context, Some(CollectionId::Liked));
    assert!(s.index < 5);
    // Kullanıcı sırasından çalan şarkı bir listenin satırı sayılmaz.
    q.enqueue(vec![UserItem { uri: track_uri(0), track: None }]);
    q.next();
    let s = queue_state(&q);
    assert_eq!((s.context, s.index), (None, 0));
    assert!(s.has_previous);
}

#[test]
fn queue_addresses_parse_for_the_player() {
    let q = queue(2, 0);
    for uri in q.context_uris() {
        assert!(SpotifyUri::from_uri(uri).is_ok(), "{uri}");
    }
    assert!(SpotifyUri::from_uri("bozuk").is_err());
}

#[test]
fn seed_changes_between_contexts() {
    assert_ne!(fresh_seed(), fresh_seed());
}

// ── Motor akışı (bağlantısız; ağ ve çalma yok) ──

struct Rig {
    engine: Engine,
    events: async_channel::Receiver<Event>,
    playback: async_channel::Receiver<PlaybackEvent>,
    _internal: mpsc::UnboundedReceiver<Internal>,
    _rt: tokio::runtime::Runtime,
}

fn rig(db: Arc<Db>) -> Rig {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (events_tx, events) = async_channel::unbounded();
    let (playback_tx, playback) = async_channel::unbounded();
    let (internal_tx, internal) = mpsc::unbounded_channel();
    let engine = Engine::new(
        rt.handle().clone(),
        Some(db),
        events_tx,
        playback_tx,
        internal_tx,
        AudioLevels::new(),
    );
    Rig { engine, events, playback, _internal: internal, _rt: rt }
}

fn row(i: usize) -> TrackRow {
    TrackRow {
        uri: track_uri(i),
        name: format!("Şarkı {i}"),
        artists: "Sanatçı".into(),
        album: "Albüm".into(),
        duration_ms: 200_000,
        image: String::new(),
    }
}

fn next_playback(rx: &async_channel::Receiver<PlaybackEvent>) -> Option<PlaybackEvent> {
    for _ in 0..300 {
        if let Ok(ev) = rx.try_recv() {
            return Some(ev);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

/// Önceki oturumun kaydı: karıştırma + tekrar "hepsi", kullanıcı sırasında
/// bir şarkı, 83 sn'de duraklatılmış.
fn previous_session(db: &Arc<Db>) -> String {
    let mut r = rig(db.clone());
    let e = &mut r.engine;
    e.saver = Some(resume::Saver::spawn(db.clone()));
    e.queue.set_shuffle(true);
    e.queue.set_repeat(RepeatMode::All);
    let uris: Vec<String> = (0..10).map(track_uri).collect();
    let current = e.queue.restart(Some(CollectionId::Liked), uris, None, 3, 7, true).unwrap();
    e.queue.enqueue(vec![UserItem { uri: track_uri(1), track: Some(row(1)) }]);
    e.pb.track_info = Some(TrackInfo {
        uri: current.clone(),
        name: "Kaldığı şarkı".into(),
        artists: "A, B".into(),
        duration_ms: 200_000,
        cover_url: Some("https://i.scdn.co/image/abc".into()),
    });
    e.pb.last_position = Some((83_000, std::time::Instant::now(), false));
    e.persist();
    assert!(e.saver.as_ref().unwrap().flush(std::time::Duration::from_secs(5)));
    current
}

#[test]
fn restart_shows_the_saved_track_and_queue_without_playing() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let current = previous_session(&db);

    let mut r = rig(db.clone());
    r.engine.restore_playback();
    assert_eq!(r.engine.pb.restored, Some(83_000), "kaydedilen konum");
    assert!(r.engine.host.conn.is_none(), "Spotify'a bağlanılmadı, anahtar istenmedi");
    assert_eq!(r.engine.queue.current_uri(), Some(current.as_str()));

    let Some(PlaybackEvent::Restored(restored)) = next_playback(&r.playback) else {
        panic!("Restored olayı gelmedi");
    };
    assert_eq!(restored.track.uri, current);
    assert_eq!(restored.track.name, "Kaldığı şarkı");
    assert_eq!(restored.track.cover_url.as_deref(), Some("https://i.scdn.co/image/abc"));
    assert_eq!(restored.position_ms, 83_000);
    assert_eq!(restored.queue.context, Some(CollectionId::Liked));
    // Karıştırarak başlandı: geçmiş yok, ileri var.
    assert!(restored.queue.has_next && !restored.queue.has_previous);
    assert!(restored.view.shuffle && restored.view.repeat == RepeatMode::All);
    assert_eq!(restored.view.user_len, 1);
    assert_eq!(restored.view.upcoming[0].uri, track_uri(1));
    // Kullanıcı şarkısı + bağlamın kalanı (9) + tekrar "hepsi" olduğundan sıradaki tur (10).
    assert_eq!(restored.view.upcoming.len(), 1 + 9 + 10);
    assert_eq!(restored.view.context_remaining, 9);

    // Çalma olayı yok: açılışta kendiliğinden çalmaz.
    while let Ok(ev) = r.events.try_recv() {
        assert!(
            !matches!(ev, Event::Loading | Event::Playing { .. } | Event::TrackChanged(_)),
            "beklenmeyen olay: {ev:?}"
        );
    }

    // Hiçbir şey çalmadan kapanıp açılınca kayıt aynı kalır.
    r.engine.persist();
    assert!(r.engine.saver.as_ref().unwrap().flush(std::time::Duration::from_secs(5)));
    let mut again = rig(db);
    again.engine.restore_playback();
    assert_eq!(again.engine.pb.restored, Some(83_000));
    assert_eq!(again.engine.queue.current_uri(), Some(current.as_str()));
    assert!(again.engine.queue.shuffle());
}

#[test]
fn seek_pause_and_edits_work_on_a_restored_track_before_connecting() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    previous_session(&db);
    let mut r = rig(db);
    r.engine.restore_playback();
    while r.events.try_recv().is_ok() {}

    // Sarma yalnız kayıtlı konumu değiştirir ve bildirir.
    r.engine.on_command(Command::Seek(90_000));
    assert_eq!(r.engine.pb.restored, Some(90_000));
    assert!(matches!(r.events.try_recv(), Ok(Event::Position { position_ms: 90_000 })));
    // Süreden öteye sarılamaz.
    r.engine.on_command(Command::Seek(999_999));
    assert_eq!(r.engine.pb.restored, Some(200_000));
    while r.events.try_recv().is_ok() {}
    // Duraklat: zaten duraklatılmış, olay yok.
    r.engine.on_command(Command::Pause);
    assert!(r.events.try_recv().is_err());
    // Çal bağlantı gelene kadar bekler (çalmaz, anahtar istemez).
    r.engine.on_command(Command::Play);
    assert_eq!(r.engine.pending.len(), 1);
    assert!(r.engine.pb.restored.is_some());

    // Sıra düzenleme bağlantısız çalışır ve hem eski hem yeni olayı verir.
    while r.playback.try_recv().is_ok() {}
    r.engine.on_command(Command::Enqueue(vec![row(0)]));
    let Ok(Event::QueueChanged(state)) = r.events.try_recv() else {
        panic!("QueueChanged gelmedi");
    };
    assert!(state.has_next);
    let Some(PlaybackEvent::Queue(view)) = next_playback(&r.playback) else {
        panic!("Queue görünümü gelmedi");
    };
    assert_eq!(view.user_len, 2);
    r.engine.on_command(Command::MoveInQueue(1, 0));
    r.engine.on_command(Command::RemoveFromQueue(0));
    r.engine.on_command(Command::ClearQueue);
    assert_eq!(r.engine.queue.user_len(), 0);
    r.engine.on_command(Command::SetShuffle(false));
    r.engine.on_command(Command::SetRepeat(RepeatMode::One));
    assert!(!r.engine.queue.shuffle() && r.engine.queue.repeat() == RepeatMode::One);
}

#[test]
fn position_estimate_for_the_saved_state() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let e = &mut r.engine;
    assert_eq!(e.pb.position_now(), 0);
    e.pb.load_position = 5_000;
    assert_eq!(e.pb.position_now(), 5_000, "yüklendi, henüz çalmaya başlamadı");
    e.pb.last_position = Some((1_000, std::time::Instant::now(), false));
    assert_eq!(e.pb.position_now(), 1_000, "duraklatılmış: sabit");
    e.pb.last_position = Some((
        1_000,
        std::time::Instant::now() - std::time::Duration::from_secs(2),
        true,
    ));
    let p = e.pb.position_now();
    assert!((3_000..3_500).contains(&p), "çalıyor: geçen süre eklenir ({p})");
    e.pb.track_info = Some(TrackInfo {
        uri: String::new(),
        name: String::new(),
        artists: String::new(),
        duration_ms: 2_000,
        cover_url: None,
    });
    assert_eq!(e.pb.position_now(), 2_000, "süreyi aşmaz");
    e.pb.restored = Some(777);
    assert_eq!(e.pb.position_now(), 777, "geri yüklenen konum önceliklidir");
}

#[test]
fn corrupt_saved_state_is_ignored() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    db.save_playback("{bozuk json", Some("bozuk")).unwrap();
    let mut r = rig(db);
    r.engine.restore_playback();
    assert!(r.engine.pb.restored.is_none() && r.engine.queue.is_idle());
    assert!(r.playback.try_recv().is_err());
}

#[test]
fn engine_without_saved_state_starts_clean() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    r.engine.restore_playback();
    assert!(r.engine.pb.restored.is_none() && r.engine.queue.is_idle());
}

// ── Karıştırma ve sıra kuralları, eşitleme düzeyi ──

/// Kural denemeleri için benzersiz adresler (`track_uri` yalnız iki farklı adres verir).
fn rule_uri(i: usize) -> String {
    format!("spotify:track:rule{i:018}")
}

/// `i % 3 == 0` olanlar "Engelli" sanatçının, ötekiler "Başka"nın şarkısı.
fn rule_row(i: usize) -> TrackRow {
    TrackRow {
        uri: rule_uri(i),
        name: format!("Şarkı {i}"),
        artists: if i % 3 == 0 { "Engelli".into() } else { "Başka".into() },
        album: "Albüm".into(),
        duration_ms: 200_000,
        image: String::new(),
    }
}

fn upcoming(e: &Engine) -> Vec<String> {
    e.queue.view(&|_| None).upcoming.into_iter().map(|u| u.uri).collect()
}

/// Kaldığı yerden sürülen sıra (satırsız, kayıt blobundan): kurallar
/// değişince şarkı bilgisi önbellekten yüklenir ve kara listedekiler düşer;
/// çalan şarkıya dokunulmaz.
#[test]
fn rules_filter_a_restored_queue_using_the_cache() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let rows: Vec<TrackRow> = (0..12).map(rule_row).collect();
    db.save_tracks(&rows).unwrap();
    // Önceki oturum: sıralı çalma, 3. şarkıda (Engelli sanatçı).
    {
        let mut r = rig(db.clone());
        let e = &mut r.engine;
        e.saver = Some(resume::Saver::spawn(db.clone()));
        let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
        e.queue.restart(Some(CollectionId::Liked), uris, None, 3, 7, true).unwrap();
        e.pb.track_info = Some(TrackInfo {
            uri: rule_uri(3),
            name: "x".into(),
            artists: "y".into(),
            duration_ms: 200_000,
            cover_url: None,
        });
        e.persist();
        assert!(e.saver.as_ref().unwrap().flush(std::time::Duration::from_secs(5)));
    }
    let mut r = rig(db);
    // Kurallar açılıştan önce de gelebilir.
    r.engine.on_command(Command::SetQueueRules(Rules::new(false, false, ["Engelli"])));
    r.engine.restore_playback();
    let engelli = |u: &String| (0..12).filter(|i| i % 3 == 0).any(|i| *u == rule_uri(i));
    assert_eq!(r.engine.queue.current_uri(), Some(rule_uri(3).as_str()), "çalan şarkıya dokunulmaz");
    let left = upcoming(&r.engine);
    assert!(!left.is_empty() && !left.iter().any(engelli), "kalanda kara listedeki yok: {left:?}");
    assert_eq!(left.len(), 6, "3 sonrası 8 şarkıdan Engelli olan 6 ve 9 düşer");
}

#[test]
fn set_queue_rules_reshapes_the_running_queue_and_back() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut r = rig(db);
    let rows: Arc<Vec<TrackRow>> = Arc::new((0..9).map(rule_row).collect());
    let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
    let e = &mut r.engine;
    e.queue.restart(None, uris, Some(rows), 1, 5, true).unwrap();
    assert_eq!(upcoming(e).len(), 7);
    e.on_command(Command::SetQueueRules(Rules::new(false, false, ["engelli"])));
    let left = upcoming(e);
    assert_eq!(left, [rule_uri(2), rule_uri(4), rule_uri(5), rule_uri(7), rule_uri(8)], "3 ve 6 düştü");
    // Aynı kural yeniden gelirse bir şey değişmez.
    let before = e.queue.order_gen();
    e.on_command(Command::SetQueueRules(Rules::new(false, false, ["engelli"])));
    assert_eq!(e.queue.order_gen(), before);
    // Kural kalkınca geri gelir.
    e.on_command(Command::SetQueueRules(Rules::default()));
    assert_eq!(upcoming(e).len(), 7);
}

#[test]
fn skip_explicit_reads_flags_from_the_cache_and_new_contexts_obey() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let rows: Vec<TrackRow> = (0..6).map(rule_row).collect();
    db.save_track_traits(&[(rule_uri(2), Traits { explicit: true, popularity: 10 }), (rule_uri(4), Traits { explicit: true, popularity: 0 })])
        .unwrap();
    let mut r = rig(db);
    r.engine.on_command(Command::SetQueueRules(Rules::new(false, true, [])));
    let rows = Arc::new(rows);
    let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
    // `start_queue` bağlantı ister; modele aynı yolla verilir.
    let (info, traits) = r.engine.rule_data(&uris, true);
    assert!(info.is_none(), "satırlar verildi, önbellekten istenmez");
    assert_eq!(traits.len(), 6);
    r.engine.queue.restart_with(None, uris, Some(rows), 0, 1, true, info, traits).unwrap();
    assert_eq!(upcoming(&r.engine), [rule_uri(1), rule_uri(3), rule_uri(5)], "açık sözlüler 2 ve 4 yok");
    assert!(r.engine.traits_known.contains(&rule_uri(2)));
}

#[test]
fn tracks_saved_with_fresh_metadata_keep_their_traits() {
    let db = Db::open_in_memory().unwrap();
    let rows: Vec<TrackRow> = (20..23).map(rule_row).collect();
    traits::note(&rows[1].uri, traits::from_spotify(true, 61));
    db.save_tracks(&rows).unwrap();
    let found = db.track_traits(&rows.iter().map(|r| r.uri.clone()).collect::<Vec<_>>()).unwrap();
    assert_eq!(found.len(), 1, "yalnız bildirilen şarkının özelliği var");
    assert_eq!(found[&rows[1].uri], Traits { explicit: true, popularity: 61 });
    // Tablo hiç kurulmamışsa okuma "bilinmiyor" döner, hata değil.
    let fresh = Db::open_in_memory().unwrap();
    assert!(fresh.track_traits(&[rule_uri(1)]).unwrap().is_empty());
}

#[test]
fn normalisation_level_reaches_the_player_config() {
    use librespot::playback::config::live;
    apply_normalisation_level(-9.0);
    assert_eq!(live::normalisation_pregain_db(0.0), -9.0);
    apply_normalisation_level(3.0);
    assert_eq!(live::normalisation_pregain_db(0.0), 3.0);
    apply_normalisation_level(0.0);
    assert_eq!(live::normalisation_pregain_db(5.0), 0.0, "Normal: sıfır, çaların varsayılanı değil");
    live::set_normalisation_pregain_db(None);
    assert_eq!(live::normalisation_pregain_db(5.0), 5.0, "ayar verilmediyse çaların kendi değeri");
    live::set_normalisation_pregain_db(Some(99.0));
    assert_eq!(live::normalisation_pregain_db(0.0), 30.0, "uç değerler sıkışır");
    live::set_normalisation_pregain_db(None);
}

/// Ekran sınaması için küçük bir "Beğenilenler" (yalnız elle çalıştırılır):
/// `OPENIFY_SEED_DIR=<boş klasör> cargo test --release --target-dir
/// target/liste seed_rules_demo_library -- --ignored`, sonra uygulama
/// `OPENIFY_TEST_CACHE_DIR=<aynı klasör>` ile. Adlar uydurma; kapak yok.
#[test]
#[ignore = "ekran sınaması için elle çalıştırılır (OPENIFY_SEED_DIR)"]
fn seed_rules_demo_library() {
    let dir = std::path::PathBuf::from(std::env::var_os("OPENIFY_SEED_DIR").expect("OPENIFY_SEED_DIR verilmedi"));
    std::fs::create_dir_all(&dir).unwrap();
    let songs = [
        ("Gece Yarısı", "Mavi Orman"),
        ("Gece Yarısı - Remastered 2015", "Mavi Orman"),
        ("Uzun Yol", "Kuzey"),
        ("Uzun Yol (Live)", "Kuzey"),
        ("Sessiz Ev", "Deniz Aydın"),
        ("Yağmur Sonrası", "Mavi Orman, Kuzey"),
        ("Pazar Sabahı", "Deniz Aydın"),
        ("Kıyıda", "Kuzey"),
        ("Limanlar", "Ayşe Taş"),
        ("Eski Fotoğraf", "Ayşe Taş"),
    ];
    let rows: Vec<TrackRow> = songs
        .iter()
        .enumerate()
        .map(|(i, (name, artists))| TrackRow {
            uri: format!("spotify:track:{i:022}"),
            name: (*name).into(),
            artists: (*artists).into(),
            album: "Demo".into(),
            duration_ms: 200_000 + i as u32 * 1_000,
            image: String::new(),
        })
        .collect();
    let db = Db::open(&dir.join("library.db")).unwrap();
    db.save_tracks(&rows).unwrap();
    let library = Library { liked_count: rows.len(), albums: Vec::new(), playlists: Vec::new(), artists: Vec::new(), ..Default::default() };
    db.save_library(&library).unwrap();
    let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
    db.save_collection(&CollectionId::Liked, Some(b"1"), &uris).unwrap();
}

// ── Çalma etkileri ve kaldığı yer kaydı (5.3 / 5.4) ──

use super::playback::{Effect, PlayerSignal, testkit};

/// Bir `Playback` çağrısının etkileri yazıldıkları sırayla yürütülür: olay
/// sırası doğrudan çağırmakla aynı.
#[test]
fn effects_run_in_the_order_they_were_written() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    r.engine.with_pb(|_, cx| {
        cx.emit(Event::Loading);
        cx.notice(NoticeKind::Info, "ara");
        cx.emit(Event::Ended);
        cx.fx.push(Effect::QueueChanged);
        cx.emit(Event::Unavailable);
    });
    let seen: Vec<String> = std::iter::from_fn(|| r.events.try_recv().ok())
        .map(|e| match e {
            Event::Loading => "loading".to_string(),
            Event::Notice { message, .. } => message,
            Event::Ended => "ended".to_string(),
            Event::QueueChanged(_) => "queue".to_string(),
            Event::Unavailable => "unavailable".to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(seen, ["loading", "ara", "ended", "queue", "unavailable"]);
}

/// Yeniden bağlanma reddedilirse (giriş gerekli) çalınamayan şarkı bırakılır,
/// sıradaki yüklenir; atlama etkileri aynı sırada akar.
#[test]
fn a_refused_reconnect_skips_the_unplayable_track() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let e = &mut r.engine;
    let (deck, spy, _out) = testkit::fake_deck();
    e.pb.deck = Some(deck);
    e.host.link = login::LinkState::NeedsLogin;
    let uris: Vec<String> = (0..3).map(track_uri).collect();
    let current = e.queue.restart(Some(CollectionId::Liked), uris, None, 0, 1, true).unwrap();
    e.with_pb(|pb, cx| pb.on_signal(cx, PlayerSignal::Unavailable { uri: Some(current) }));
    let calls = spy.take();
    assert!(
        matches!(calls.as_slice(), [testkit::Call::Load { uri, play: true, position_ms: 0 }] if *uri == track_uri(1)),
        "{calls:?}"
    );
    let events: Vec<Event> = std::iter::from_fn(|| r.events.try_recv().ok()).collect();
    assert!(events.iter().any(|e| matches!(e, Event::QueueChanged(_))));
    assert!(events.iter().any(|e| matches!(e, Event::Notice { kind: NoticeKind::Info, .. })));
    assert!(!e.pb.retry_after_reconnect);
}

/// Boşluksuz geçişte tutma sürerken tahmini konumla kayıt yazılmaz (sıra yeni
/// şarkıya geçti, arayüz hâlâ eskisinde: şarkı ile konum tutmazdı); açık
/// konumlu kayıt (biten sıra) yine yazılır; tutma bitince kayıt yeniden yazılır.
#[test]
fn saving_is_skipped_while_a_handover_is_held() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut r = rig(db.clone());
    let e = &mut r.engine;
    e.saver = Some(resume::Saver::spawn(db.clone()));
    let uris: Vec<String> = (0..4).map(track_uri).collect();
    e.queue.restart(Some(CollectionId::Liked), uris, None, 1, 7, true).unwrap();
    let flush = |e: &Engine| assert!(e.saver.as_ref().unwrap().flush(std::time::Duration::from_secs(5)));

    let (_deck, _spy, out) = testkit::fake_deck();
    e.pb.gate.hold_for_test(out, Some(track_uri(0)));
    e.pb.last_position = Some((1_000, std::time::Instant::now(), true));
    e.persist();
    e.persist_tick();
    flush(e);
    assert!(db.load_playback().unwrap().is_none(), "tutma sürerken kayıt yazılmadı");

    e.persist_at(Some(0));
    flush(e);
    assert!(db.load_playback().unwrap().is_some(), "açık konumlu kayıt tutmada da yazılır");

    e.pb.gate.release_for_test();
    e.persist();
    flush(e);
    assert!(db.load_playback().unwrap().is_some());
}

// ── Ekolayzer komutu (6.6) ──

/// `Command::SetEq` bağlantı beklemeden tutulur, çıkış kurulunca uygulanır,
/// sonraki komutlar anında çıkışa ulaşır; sınırlar çekilir; çıkış yeniden
/// kurulursa (yeniden bağlanma) ayar kaybolmaz.
#[test]
fn set_eq_command_reaches_the_output() {
    use crate::engine::equalizer::{PREAMP_RANGE_DB, RANGE_DB};
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let gains = [6.0, 3.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.0, 4.5, 0.0];
    // Çıkış yok: komut düşmez, motor tutar (bekleyen komut kuyruğuna girmez).
    r.engine.on_command(Command::SetEq { enabled: true, gains_db: gains, preamp_db: 1.0 });
    assert!(r.engine.pb.deck.is_none() && r.engine.pending.is_empty());
    assert_eq!(r.engine.eq.gains_db, gains);
    assert!(r.engine.eq.enabled);

    // Çıkış kurulunca (attach) tutulan ayar uygulanır.
    let (deck, _spy, out) = testkit::fake_deck();
    assert_eq!(out.eq_published().map(|(on, _)| on), Some(false), "başlangıçta kapalı");
    r.engine.pb.deck = Some(deck);
    r.engine.apply_eq();
    let (on, p) = out.eq_published().expect("yayın okunur");
    assert!(on);
    assert_ne!(p.coef[0], super::output::eq::Params::IDENT.coef[0], "6 dB'lik bant katsayısı birim değil");
    assert_eq!(p.coef[2], super::output::eq::Params::IDENT.coef[2], "0 dB'lik bant birim");
    assert!(p.pre < 1.0 && p.pre > 0.0, "otomatik ön kazanç artı bantları dengeledi: {}", p.pre);

    // Çıkış varken komut hemen ulaşır: kapatma.
    r.engine.on_command(Command::SetEq { enabled: false, gains_db: gains, preamp_db: 0.0 });
    assert_eq!(out.eq_published().map(|(on, _)| on), Some(false));
    // Sınırlar: ±12 / ±12; sonlu olmayan sıfır.
    r.engine.on_command(Command::SetEq { enabled: true, gains_db: [99.0, -99.0, f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], preamp_db: 99.0 });
    assert_eq!(r.engine.eq.gains_db[0], RANGE_DB);
    assert_eq!(r.engine.eq.gains_db[1], -RANGE_DB);
    assert_eq!(r.engine.eq.gains_db[2], 0.0);
    assert_eq!(r.engine.eq.preamp_db, PREAMP_RANGE_DB);

    // Yeniden bağlanma: yeni çıkış, aynı ayar.
    let (deck2, _spy2, out2) = testkit::fake_deck();
    r.engine.pb.deck = Some(deck2);
    r.engine.apply_eq();
    assert_eq!(out2.eq_published().map(|(on, _)| on), Some(true), "yeni çıkış tutulan ayarı aldı");
}

// ── Karaoke komutu ──

/// `Command::SetKaraoke` bağlantı beklemeden tutulur, çıkış kurulunca uygulanır,
/// sonraki komutlar anında çıkışa ulaşır; miktar 0..1'e çekilir; çıkış
/// yeniden kurulursa (yeniden bağlanma) ayar kaybolmaz.
#[test]
fn set_karaoke_command_reaches_the_output() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    // Çıkış yok: komut düşmez, motor tutar.
    r.engine.on_command(Command::SetKaraoke { enabled: true, amount: 0.6 });
    assert!(r.engine.pb.deck.is_none() && r.engine.pending.is_empty());
    assert_eq!(r.engine.karaoke, (true, 0.6));

    // Çıkış kurulunca (attach) tutulan ayar uygulanır.
    let (deck, _spy, out) = testkit::fake_deck();
    assert_eq!(out.karaoke_published(), (false, 0.0), "başlangıçta kapalı");
    r.engine.pb.deck = Some(deck);
    r.engine.apply_karaoke();
    assert_eq!(out.karaoke_published(), (true, 0.6));

    // Çıkış varken komut hemen ulaşır: kapatma, sonra sınırlar.
    r.engine.on_command(Command::SetKaraoke { enabled: false, amount: 0.6 });
    assert_eq!(out.karaoke_published(), (false, 0.0));
    r.engine.on_command(Command::SetKaraoke { enabled: true, amount: 9.0 });
    assert_eq!(out.karaoke_published(), (true, 1.0));
    r.engine.on_command(Command::SetKaraoke { enabled: true, amount: f32::NAN });
    assert_eq!(out.karaoke_published(), (false, 0.0), "sonlu olmayan miktar 0: etkisiz");

    // Yeniden bağlanma: yeni çıkış, aynı ayar.
    r.engine.on_command(Command::SetKaraoke { enabled: true, amount: 0.25 });
    let (deck2, _spy2, out2) = testkit::fake_deck();
    r.engine.pb.deck = Some(deck2);
    r.engine.apply_karaoke();
    assert_eq!(out2.karaoke_published(), (true, 0.25), "yeni çıkış tutulan ayarı aldı");
}

// ── Dinleme çekirdeği: gizlenen şarkılar ve radyo (bkz. `listening.rs`) ──

fn hidden_song(uri: &str, context: Option<&str>) -> HiddenSong {
    HiddenSong { uri: uri.into(), context: context.map(Into::into), name: "Ad".into(), artists: "Sanatçı".into(), hidden_at: 0 }
}

fn last_hidden_event(rx: &async_channel::Receiver<Event>) -> Option<Arc<HiddenList>> {
    let mut found = None;
    while let Ok(ev) = rx.try_recv() {
        if let Event::Library(LibraryEvent::Hidden(list)) = ev {
            found = Some(list);
        }
    }
    found
}

#[test]
fn hiding_a_song_persists_rebuilds_the_queue_and_tells_the_ui() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut r = rig(db.clone());
    let rows = radio_rows(0, 5);
    let uris: Vec<String> = rows.iter().map(|t| t.uri.clone()).collect();
    r.engine.queue.restart(None, uris.clone(), Some(Arc::new(rows)), 0, 1, true);
    while r.events.try_recv().is_ok() {}

    // Her yerde gizle: kayıt yazılır, sıradan düşer, arayüz yeni dizini alır.
    r.engine.on_command(Command::Library(LibraryCommand::SetHidden { songs: vec![hidden_song(&uris[2], None)], on: true }));
    assert_eq!(db.hidden_songs().unwrap().len(), 1);
    assert!(r.engine.hidden.hidden_everywhere(&uris[2]));
    let upcoming: Vec<String> = r.engine.queue.view(&|_| None).upcoming.iter().map(|e| e.uri.clone()).collect();
    assert!(!upcoming.contains(&uris[2]), "gizli şarkı sırada kalmamalı");
    assert_eq!(upcoming.len(), 3);
    let list = last_hidden_event(&r.events).expect("LibraryEvent::Hidden gelmedi");
    assert!(list.hidden_everywhere(&uris[2]));

    // Aynı eylemi yinelemek yeni olay çıkarmaz (dizin değişmedi); zamanı yenilenir.
    r.engine.on_command(Command::Library(LibraryCommand::SetHidden { songs: vec![hidden_song(&uris[2], None)], on: true }));
    assert_eq!(db.hidden_songs().unwrap().len(), 1);

    // Göster: kayıt silinir, şarkı (henüz çalınmadığı için) sıraya döner.
    r.engine.on_command(Command::Library(LibraryCommand::SetHidden { songs: vec![hidden_song(&uris[2], None)], on: false }));
    assert!(db.hidden_songs().unwrap().is_empty());
    let upcoming: Vec<String> = r.engine.queue.view(&|_| None).upcoming.iter().map(|e| e.uri.clone()).collect();
    assert!(upcoming.contains(&uris[2]));
    assert!(last_hidden_event(&r.events).expect("Hidden olayı").is_empty());
}

#[test]
fn hidden_songs_are_loaded_at_startup_and_apply_to_a_restored_queue() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    // Önceki oturum: karıştırılmış, tekrar "hepsi", on şarkılık (ayrı adresli) bağlam.
    let current = {
        let mut r = rig(db.clone());
        r.engine.saver = Some(resume::Saver::spawn(db.clone()));
        r.engine.queue.set_shuffle(true);
        r.engine.queue.set_repeat(RepeatMode::All);
        let rows = radio_rows(0, 10);
        let uris: Vec<String> = rows.iter().map(|t| t.uri.clone()).collect();
        let current = r.engine.queue.restart(Some(CollectionId::Liked), uris, Some(Arc::new(rows)), 3, 7, true).unwrap();
        r.engine.pb.track_info =
            Some(TrackInfo { uri: current.clone(), name: "Kaldığı şarkı".into(), artists: "A".into(), duration_ms: 200_000, cover_url: None });
        r.engine.pb.last_position = Some((83_000, std::time::Instant::now(), false));
        r.engine.persist();
        assert!(r.engine.saver.as_ref().unwrap().flush(std::time::Duration::from_secs(5)));
        current
    };
    // Önceki oturumdan sonra bir şarkı gizlenmiş (kayıtlı sıranın kalanında duruyor).
    let victim = {
        let mut probe = rig(db.clone());
        probe.engine.restore_playback();
        probe.engine.queue.view(&|_| None).upcoming.iter().find(|e| e.source == QueueSource::Context).unwrap().uri.clone()
    };
    db.hide_song(&hidden_song(&victim, None)).unwrap();

    let mut r = rig(db);
    r.engine.init_hidden();
    assert!(last_hidden_event(&r.events).expect("açılışta Hidden olayı").hidden_everywhere(&victim));
    r.engine.restore_playback();
    assert_eq!(r.engine.queue.current_uri(), Some(current.as_str()), "çalan şarkı yerinde");
    let rest: Vec<String> = r.engine.queue.view(&|_| None).upcoming.iter().map(|e| e.uri.clone()).collect();
    // `victim` bağlamın kalanından düşmüş; tekrar "hepsi" açıkken sıradaki turda da yok.
    assert!(!rest.contains(&victim), "gizli şarkı kaldığı yerden sürülen sırada çalınmaz");
    assert_eq!(rest.len(), 8 + 9, "kalan 8 + sıradaki tur 9 (gizli olan her iki turda da düşük)");
}

fn radio_rows(from: usize, to: usize) -> Vec<TrackRow> {
    (from..to)
        .map(|i| TrackRow {
            uri: format!("spotify:track:radio{i}"),
            name: format!("Radyo {i}"),
            artists: format!("Sanatçı {}", i % 5),
            album: String::new(),
            duration_ms: 180_000,
            image: String::new(),
        })
        .collect()
}

fn fetched(rows: Vec<TrackRow>) -> listening::Fetched {
    listening::Fetched { rows, traits: HashMap::new(), station_title: "Spotify başlığı".into() }
}

/// Radyo çalıyormuş gibi kurulu motor (bağlantı yok; `begin_radio` çalar ister, burada sıra doğrudan kurulur).
fn radio_engine() -> Rig {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let rows = radio_rows(0, 12);
    let uris: Vec<String> = rows.iter().map(|t| t.uri.clone()).collect();
    r.engine.queue.restart(None, uris, Some(Arc::new(rows)), 0, 3, true);
    r.engine.queue.set_radio(Some(RadioInfo { seed: "spotify:track:radioseed".into(), title: "Tohum".into() }));
    r.engine.radio.started(std::time::Instant::now());
    while r.events.try_recv().is_ok() {}
    r
}

fn batch(seed: &str, generation: u64, result: Result<listening::Fetched, radio::RadioFailure>) -> listening::RadioBatch {
    listening::RadioBatch { seed: seed.into(), title: String::new(), start: false, generation, result }
}

#[test]
fn a_radio_follow_up_extends_the_context_and_tells_the_ui() {
    let mut r = radio_engine();
    let generation = r.engine.radio.generation();
    let before = r.engine.queue.context_uris().len();
    r.engine.on_radio_batch(batch("spotify:track:radioseed", generation, Ok(fetched(radio_rows(10, 30)))));
    // 10 ve 11 zaten vardı: 18 yeni şarkı.
    assert_eq!(r.engine.queue.context_uris().len(), before + 18);
    assert!(r.engine.queue.radio().is_some(), "radyo sürüyor");
    assert!(matches!(next_playback(&r.playback), Some(PlaybackEvent::Queue(view)) if view.radio.is_some()));
    assert!(r.engine.queue.context_remaining() >= 29, "yeni şarkılar sıranın sonuna girdi");
}

#[test]
fn stale_follow_ups_and_failures_never_touch_the_queue() {
    let mut r = radio_engine();
    let generation = r.engine.radio.generation();
    let before = r.engine.queue.context_uris().len();
    // Başka bir radyonun (tohum) gecikmiş yanıtı.
    r.engine.on_radio_batch(batch("spotify:track:baska", generation, Ok(fetched(radio_rows(20, 30)))));
    // Eski nesil: yeni radyo başlamış.
    r.engine.on_radio_batch(batch("spotify:track:radioseed", generation.wrapping_sub(1), Ok(fetched(radio_rows(20, 30)))));
    assert_eq!(r.engine.queue.context_uris().len(), before);
    // Hata: bağlam aynı kalır, hata sayacı artar, bildirim yok (radyo çalmaya devam eder).
    r.engine.on_radio_batch(batch("spotify:track:radioseed", generation, Err(radio::RadioFailure::Offline)));
    assert_eq!(r.engine.queue.context_uris().len(), before);
    assert!(r.engine.queue.radio().is_some());
    while let Ok(ev) = r.events.try_recv() {
        assert!(!matches!(ev, Event::Notice { .. }), "devam hatası sessiz: {ev:?}");
    }
}

#[test]
fn a_failed_radio_start_says_so_and_a_replaced_start_stays_quiet() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let start = |seed: &str, result| listening::RadioBatch {
        seed: seed.into(),
        title: "Ad".into(),
        start: true,
        generation: 0,
        result,
    };
    // İstenmiş radyo hata verince kullanıcıya bildirim gider (türüne göre yazı).
    r.engine.radio.want("spotify:track:a");
    r.engine.on_radio_batch(start("spotify:track:a", Err(radio::RadioFailure::RateLimited)));
    let Ok(Event::Notice { kind, message }) = r.events.try_recv() else { panic!("bildirim gelmedi") };
    assert_eq!(kind, NoticeKind::Problem);
    assert_eq!(message, t!(NoticeRadioLimited));
    // Daha yeni bir radyo istenmişse eskisinin hatası sessiz kalır.
    r.engine.radio.want("spotify:track:a");
    r.engine.radio.want("spotify:track:b");
    r.engine.on_radio_batch(start("spotify:track:a", Err(radio::RadioFailure::Offline)));
    assert!(r.events.try_recv().is_err());
    // Geçersiz tohum bağlantısız bile hemen söylenir.
    r.engine.start_radio("liked".into(), String::new());
    let Ok(Event::Notice { message, .. }) = r.events.try_recv() else { panic!("bildirim gelmedi") };
    assert_eq!(message, t!(NoticeRadioFailed));
}

#[test]
fn the_radio_survives_a_restart_through_the_saved_state() {
    let db = Arc::new(Db::open_in_memory().unwrap());
    {
        let mut r = rig(db.clone());
        r.engine.saver = Some(resume::Saver::spawn(db.clone()));
        let rows = radio_rows(0, 12);
        let uris: Vec<String> = rows.iter().map(|t| t.uri.clone()).collect();
        let current = r.engine.queue.restart(None, uris, Some(Arc::new(rows)), 0, 3, true).unwrap();
        r.engine.queue.set_radio(Some(RadioInfo { seed: "spotify:artist:art".into(), title: "Sanatçı".into() }));
        r.engine.pb.track_info = Some(TrackInfo { uri: current, name: "Radyo 0".into(), artists: "S".into(), duration_ms: 1000, cover_url: None });
        r.engine.pb.last_position = Some((1_000, std::time::Instant::now(), false));
        r.engine.persist();
        assert!(r.engine.saver.as_ref().unwrap().flush(std::time::Duration::from_secs(5)));
    }
    let mut again = rig(db);
    again.engine.restore_playback();
    let info = again.engine.queue.radio().expect("radyo geri yüklenmedi");
    assert_eq!((info.seed.as_str(), info.title.as_str()), ("spotify:artist:art", "Sanatçı"));
}

// ── Oturum: `install_session` ve yeniden bağlanma kararları ──

/// Bağlanmamış bir oturum nesnesi (ağa hiç çıkmaz); bir tokio ortamı içinde kurulmalı.
fn test_session(device_id: &str) -> Session {
    use librespot::core::config::SessionConfig;
    Session::new(SessionConfig { device_id: device_id.to_string(), ..SessionConfig::default() }, None)
}

fn drain_events(rx: &async_channel::Receiver<Event>) -> Vec<Event> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// `Replace`: yeni oturum yerine geçer, eskisi kapanır, çalar korunur ve çalan
/// şarkı kaldığı konumdan yeniden yüklenir; bağlantı durumu ve hesap denetimi
/// turları yeniden kurulur. (Eskiden `on_reconnected`in içindeydi.)
#[test]
fn replace_swaps_the_session_keeps_the_player_and_retries_the_song() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let _guard = r._rt.enter();
    let e = &mut r.engine;
    let (deck, spy, _out) = testkit::fake_deck();
    e.pb.deck = Some(deck);
    let (old, new) = (test_session("eski-aygit-0123456789"), test_session("yeni-aygit-0123456789"));
    e.host.conn = Some(session::Connected { session: old.clone() });
    e.host.link = login::LinkState::Offline;
    e.host.premium_checks_left = 0;
    let uris: Vec<String> = (0..3).map(track_uri).collect();
    let current = e.queue.restart(Some(CollectionId::Liked), uris, None, 0, 1, true).unwrap();
    e.pb.retry_after_reconnect = true;
    e.pb.last_position = Some((12_000, std::time::Instant::now(), true));

    assert!(e.install_session(new.clone(), Install::Replace).is_none(), "çalar korunur: yeni olay kanalı yok");

    assert_eq!(e.host.session().unwrap().device_id(), "yeni-aygit-0123456789");
    assert!(old.is_invalid() && !new.is_invalid(), "eski oturum kapanır, yenisi açık kalır");
    assert_eq!(e.host.link, login::LinkState::Connected);
    assert_eq!(e.host.premium_checks_left, session::PREMIUM_CHECKS);
    let calls = spy.take();
    assert!(matches!(calls.first(), Some(testkit::Call::Rebind)), "önce çalar yeni oturuma bağlanır: {calls:?}");
    assert!(
        calls.iter().any(|c| matches!(c, testkit::Call::Load { uri, play: true, position_ms } if *uri == current && *position_ms >= 12_000)),
        "çalan şarkı kaldığı konumdan yeniden yüklenmeli: {calls:?}"
    );
    assert!(!e.pb.retry_after_reconnect);
    assert!(drain_events(&r.events).iter().any(|ev| matches!(ev, Event::Connected { .. })), "arayüze 'bağlandı' gider");
}

/// Çalar yokken (ses çıkışı yoktu / hesap Premium'a dönmüş) başarılı yeniden
/// bağlanma oturumu `First` için bekletir; sayaçlar sıfırlanır.
#[test]
fn a_reconnect_without_a_player_parks_the_session_for_first_install() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let _guard = r._rt.enter();
    let e = &mut r.engine;
    e.host.reconnecting = true;
    e.host.reconnect_failures = 3;
    e.host.reconnect_wanted = true;
    e.host.link = login::LinkState::Offline;
    e.on_reconnected(Ok(test_session("bekleyen-aygit-0123456")));
    assert!(!e.host.reconnecting && e.host.reconnect_failures == 0 && !e.host.reconnect_wanted);
    assert_eq!(e.host.link, login::LinkState::Connected);
    assert!(e.host.new_session.is_some(), "komut döngüsü `First` takacak");
    assert!(e.host.conn.is_none(), "çalar kurulmadan oturum motora girmez");
}

/// Ağ yüzünden başarısız yeniden bağlanma: oturum ve çalar yerinde kalır, geri
/// çekilme sayacı artar, sonra yeniden denenecek ve kullanıcıya "birazdan" denir.
#[test]
fn an_offline_reconnect_keeps_the_session_and_asks_to_try_again() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let _guard = r._rt.enter();
    let e = &mut r.engine;
    let old = test_session("kalan-aygit-0123456789");
    e.host.conn = Some(session::Connected { session: old.clone() });
    e.host.reconnecting = true;
    e.on_reconnected(Err(auth::ConnectError::Offline(anyhow::anyhow!("network: yok"))));
    assert!(!e.host.reconnecting && e.host.reconnect_failures == 1 && e.host.reconnect_wanted);
    assert!(e.host.reconnect_due(), "geri çekilme dolunca `check_health` yeniden dener");
    assert_eq!(e.host.session().unwrap().device_id(), "kalan-aygit-0123456789");
    assert!(!old.is_invalid());
    let events = drain_events(&r.events);
    assert!(
        events.iter().any(|ev| matches!(ev, Event::Notice { kind: NoticeKind::Working, message } if message == t!(NoticeReconnectSoon))),
        "{events:?}"
    );
}

/// Kayıtlı giriş reddedildi: yeniden denemek boşuna; giriş istenir, bekleyen
/// yeniden yükleme isteği düşer, bekleyen çalma komutları atılır.
#[test]
fn a_rejected_reconnect_asks_for_login_and_drops_pending_work() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let _guard = r._rt.enter();
    let e = &mut r.engine;
    e.host.reconnecting = true;
    e.host.reconnect_wanted = true;
    e.pb.retry_after_reconnect = true;
    e.pending.push(Command::Play);
    e.on_reconnected(Err(auth::ConnectError::Rejected));
    assert_eq!(e.host.link, login::LinkState::NeedsLogin);
    assert!(!e.host.reconnect_wanted && !e.host.reconnecting && !e.pb.retry_after_reconnect);
    assert!(e.pending.is_empty());
    assert!(drain_events(&r.events).iter().any(|ev| matches!(ev, Event::NeedsLogin)));
}

/// `reconnect`in karar kapıları (bağlanmayı BAŞLATAN yol ağa çıkar, burada
/// sınanmaz): giriş gerekirken denenmez, süren bağlanmaya katılınır, geri çekilme
/// sürerken vazgeçilir ve bekleyen yeniden yükleme isteği düşer.
#[test]
fn reconnect_gates_join_a_running_attempt_hold_back_or_refuse() {
    let mut r = rig(Arc::new(Db::open_in_memory().unwrap()));
    let _guard = r._rt.enter();
    let e = &mut r.engine;
    // Giriş gerekiyor: hiç denenmez, istek düşer.
    e.host.link = login::LinkState::NeedsLogin;
    e.pb.retry_after_reconnect = true;
    assert!(!e.reconnect(true));
    assert!(!e.pb.retry_after_reconnect && !e.host.reconnecting);
    // Süren bağlanmaya katılınır; yeniden yükleme isteği korunur.
    e.host.link = login::LinkState::Connected;
    e.host.reconnecting = true;
    assert!(e.reconnect(true));
    assert!(e.pb.retry_after_reconnect);
    // Geri çekilme sürüyor (az önce denendi): vazgeçilir, istek düşer.
    e.host.reconnecting = false;
    e.host.last_reconnect = Some(std::time::Instant::now());
    assert!(!e.reconnect(true));
    assert!(!e.pb.retry_after_reconnect && !e.host.reconnecting);
}
