//! Kaldığı yerden sürme: sıranın (bağlam, kullanıcı sırası, geçmiş,
//! karıştırma, tekrar) ve çalan şarkının konumunun SQLite'a yazılması ve
//! açılışta geri okunması.
//!
//! İki parça, `playback` tablosunda iki satır:
//!   - `state`: küçük (birkaç KB): çalan şarkı, konum, kullanıcı sırası,
//!     geçmiş, imleç, karıştırma / tekrar. Beş saniyede bir yazılabilir.
//!   - `context`: büyük (6.000 şarkılık listede ~250 KB): bağlamın şarkı
//!     adresleri ve çalma sırası. Yalnız bağlam ya da sıra değişince yazılır;
//!     `state` içindeki `blob_gen` ile eşleşir.
//!
//! Yazma ayrı bir iş parçacığında (`Saver`): motorun komut döngüsü SQLite
//! kilidinde (kütüphane tazeleme uzun bir işlem tutabilir) beklemez; ardışık
//! istekler birleştirilir (yalnız en yenisi yazılır), kapanışta `flush`.

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use super::{
    cache::Db,
    library::{CollectionId, TrackRow},
    queue::{Origin, Played, Queue, RepeatMode, UserItem},
};

/// Kayıt biçimi değişirse artır: eski kayıt yok sayılır (kimseyi kırmadan).
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapItem {
    pub uri: String,
    /// Bağlamdaki indeksi; kullanıcı şarkısı (ya da başka bağlamdan kalma) ise yok.
    pub ctx: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapUser {
    pub uri: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artists: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(default)]
    pub image: String,
}

/// Çalan şarkının görüntü bilgisi (arayüz açılışta hemen gösterebilsin diye;
/// Spotify'a ya da önbellekteki şarkı tablosuna bağlı değil).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapTrack {
    pub uri: String,
    pub name: String,
    pub artists: String,
    pub duration_ms: u32,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    /// Bağlam satırının hangi sürümüyle eşleştiği.
    pub blob_gen: u64,
    /// `CollectionId::key()`; sanatçı ve tek şarkı bağlamlarında yok.
    pub context: Option<String>,
    pub cursor: usize,
    pub current: Option<SnapItem>,
    pub history: Vec<SnapItem>,
    pub user: Vec<SnapUser>,
    pub last_ctx: Option<usize>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub seed: u64,
    pub lap: u32,
    pub position_ms: u32,
    pub track: Option<SnapTrack>,
    /// Çalan bağlam bir radyoysa (tohum, başlık): açılışta devamı sürsün diye.
    #[serde(default)]
    pub radio: Option<SnapRadio>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapRadio {
    pub seed: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Blob {
    generation: u64,
    uris: Vec<String>,
    order: Vec<usize>,
    /// `uris` ile aynı indeksli sanatçı kimlikleri (`Queue::artists`; 0
    /// bilinmiyor). 7 Kasım'dan önceki kayıtlarda yok: o zaman hepsi
    /// bilinmiyor sayılır, karıştırma o bağlamda düz rastgele olur.
    #[serde(default)]
    artists: Vec<u32>,
}

/// Geri okunan durum.
pub struct Restore {
    pub queue: Queue,
    pub position_ms: u32,
    pub track: Option<SnapTrack>,
    /// Bağlam satırı sağlam okundu mu (sağlamsa yeniden yazmaya gerek yok)?
    pub blob_ok: bool,
}

fn snap_item(p: &Played) -> SnapItem {
    SnapItem {
        uri: p.uri.clone(),
        ctx: match p.origin {
            Origin::Context(i) => Some(i),
            Origin::User => None,
        },
    }
}

fn played(item: SnapItem) -> Played {
    Played {
        uri: item.uri,
        origin: item.ctx.map_or(Origin::User, Origin::Context),
        track: None,
    }
}

/// Sıranın küçük kaydı.
pub fn snapshot(queue: &Queue, position_ms: u32, track: Option<SnapTrack>) -> Snapshot {
    Snapshot {
        v: VERSION,
        blob_gen: queue.order_gen,
        context: queue.context.as_ref().map(|c| c.key().to_string()),
        cursor: queue.cursor,
        current: queue.current.as_ref().map(snap_item),
        history: queue.history.iter().map(snap_item).collect(),
        user: queue
            .user
            .iter()
            .map(|u| match &u.track {
                Some(t) => SnapUser {
                    uri: u.uri.clone(),
                    name: t.name.clone(),
                    artists: t.artists.clone(),
                    album: t.album.clone(),
                    duration_ms: t.duration_ms,
                    image: t.image.clone(),
                },
                None => SnapUser {
                    uri: u.uri.clone(),
                    name: String::new(),
                    artists: String::new(),
                    album: String::new(),
                    duration_ms: 0,
                    image: String::new(),
                },
            })
            .collect(),
        last_ctx: queue.last_ctx,
        shuffle: queue.shuffle,
        repeat: queue.repeat,
        seed: queue.seed,
        lap: queue.lap,
        position_ms,
        track,
        radio: queue.radio.as_ref().map(|r| SnapRadio { seed: r.seed.clone(), title: r.title.clone() }),
    }
}

pub fn encode(snapshot: &Snapshot) -> String {
    serde_json::to_string(snapshot).unwrap_or_default()
}

/// Bağlam satırı: büyük, yalnız sıra değişince yazılır.
pub fn encode_blob(queue: &Queue) -> String {
    #[derive(Serialize)]
    struct BlobRef<'a> {
        generation: u64,
        uris: &'a [String],
        order: &'a [usize],
        #[serde(skip_serializing_if = "<[u32]>::is_empty")]
        artists: &'a [u32],
    }
    let blob = BlobRef {
        generation: queue.order_gen,
        uris: &queue.ctx,
        order: &queue.order,
        artists: &queue.artists,
    };
    serde_json::to_string(&blob).unwrap_or_default()
}

fn user_item(u: SnapUser) -> UserItem {
    let track = (!u.name.is_empty()).then(|| TrackRow {
        uri: u.uri.clone(),
        name: u.name,
        artists: u.artists,
        album: u.album,
        duration_ms: u.duration_ms,
        image: u.image,
    });
    UserItem { uri: u.uri, track }
}

/// Kaydı geri okur. Bozuk ya da eski sürüm kayıtta `None` (sıfırdan başlanır).
/// Çalan da sıradaki de yoksa (yalnız karıştırma / tekrar durumu kayıtlıysa)
/// boş bir sıra döner.
/// Bağlam satırı yoksa / eşleşmiyorsa / tutarsızsa yalnız çalan şarkı ve
/// kullanıcı sırasıyla (bağlamsız) geri yüklenir.
pub fn restore(state: &str, blob: Option<&str>) -> Option<Restore> {
    let snap: Snapshot = serde_json::from_str(state).ok()?;
    if snap.v != VERSION {
        return None;
    }
    let current = snap.current.clone();
    let user: std::collections::VecDeque<UserItem> = snap.user.iter().cloned().map(user_item).collect();
    let mut queue = Queue::new();
    queue.user = user;
    queue.shuffle = snap.shuffle;
    queue.repeat = snap.repeat;
    queue.seed = snap.seed;
    queue.lap = snap.lap;
    queue.history = snap.history.iter().cloned().map(played).collect();

    let full = blob
        .and_then(|b| serde_json::from_str::<Blob>(b).ok())
        .filter(|b| b.generation == snap.blob_gen);
    let mut blob_ok = false;
    if let Some(b) = full {
        let mut full_queue = queue.clone();
        full_queue.context = snap.context.as_deref().and_then(CollectionId::from_key);
        // Sanatçı kimlikleri bağlamla eşleşmiyorsa (bozuk) bilinmiyor sayılır.
        full_queue.artists = if b.artists.len() == b.uris.len() { b.artists } else { Vec::new() };
        full_queue.ctx = b.uris;
        full_queue.order = b.order;
        full_queue.cursor = snap.cursor;
        full_queue.last_ctx = snap.last_ctx;
        full_queue.current = current.clone().map(played);
        full_queue.order_gen = b.generation;
        if full_queue.is_consistent() {
            queue = full_queue;
            blob_ok = true;
            // Radyonun bağlamı kayıtlıysa radyo sürer (bağlamsız geri yüklemede değil).
            queue.radio = snap.radio.clone().map(|r| super::radio::RadioInfo { seed: r.seed, title: r.title });
        }
    }
    if !blob_ok {
        // Bağlamsız: çalan şarkı tek başına (geçmişin bağlam indeksleri de
        // anlamsız).
        queue.current = current.map(|c| Played { uri: c.uri, origin: Origin::User, track: None });
        for h in queue.history.iter_mut() {
            h.origin = Origin::User;
        }
        queue.bump();
    }
    Some(Restore {
        queue,
        position_ms: snap.position_ms,
        track: snap.track,
        blob_ok,
    })
}

// ── Yazıcı ──────────────────────────────────────────────────────────────

enum Msg {
    Save { state: String, blob: Option<String> },
    Flush(Sender<()>),
}

/// Kaydı ayrı bir iş parçacığında yazar. Düşürülünce iş parçacığı biter.
pub struct Saver {
    tx: Option<Sender<Msg>>,
    handle: Option<JoinHandle<()>>,
}

impl Saver {
    pub fn spawn(db: Arc<Db>) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("openify-resume".into())
            .spawn(move || writer(db, rx))
            .ok();
        Self { tx: Some(tx), handle }
    }

    pub fn save(&self, state: String, blob: Option<String>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Save { state, blob });
        }
    }

    /// Bekleyen yazımlar bitene kadar bekler (en çok `timeout`). Kapanışta.
    pub fn flush(&self, timeout: Duration) -> bool {
        let Some(tx) = &self.tx else { return true };
        let (ack_tx, ack_rx) = mpsc::channel();
        tx.send(Msg::Flush(ack_tx)).is_ok() && ack_rx.recv_timeout(timeout).is_ok()
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn writer(db: Arc<Db>, rx: Receiver<Msg>) {
    while let Ok(first) = rx.recv() {
        let mut state: Option<String> = None;
        let mut blob: Option<String> = None;
        let mut acks = Vec::new();
        let mut handle = |msg: Msg| match msg {
            Msg::Save { state: s, blob: b } => {
                state = Some(s);
                // Ardışık isteklerde büyük satır en son verilenle birleşir
                // (sonrakinde yoksa öncekini koru).
                if b.is_some() {
                    blob = b;
                }
            }
            Msg::Flush(ack) => acks.push(ack),
        };
        handle(first);
        while let Ok(more) = rx.try_recv() {
            handle(more);
        }
        if let Some(state) = state {
            if let Err(e) = db.save_playback(&state, blob.as_deref()) {
                log::warn!("Kaldığı yer kaydedilemedi: {e:#}");
            }
        }
        for ack in acks {
            let _ = ack.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uris(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("spotify:track:{i}")).collect()
    }

    fn item(uri: &str, with_meta: bool) -> UserItem {
        UserItem {
            uri: uri.into(),
            track: with_meta.then(|| TrackRow {
                uri: uri.into(),
                name: format!("{uri} adı"),
                artists: "Sanatçı".into(),
                album: "Albüm".into(),
                duration_ms: 123_000,
                image: "https://i.scdn.co/image/x".into(),
            }),
        }
    }

    fn busy_queue() -> Queue {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        q.restart(Some(CollectionId::Playlist("spotify:playlist:p".into())), uris(40), None, 4, 0xABCD, true);
        for _ in 0..6 {
            q.next();
        }
        q.previous();
        q.enqueue(vec![item("spotify:track:u1", true), item("spotify:track:u2", false)]);
        q
    }

    fn track() -> Option<SnapTrack> {
        Some(SnapTrack {
            uri: "spotify:track:1".into(),
            name: "Şarkı".into(),
            artists: "A, B".into(),
            duration_ms: 200_000,
            cover_url: Some("https://i.scdn.co/image/c".into()),
        })
    }

    /// Bir sıranın gelecekteki bütün davranışını belirleyen görünüm.
    fn fingerprint(q: &Queue) -> String {
        format!(
            "{:?}|{:?}|{:?}|{}|{:?}|{:?}|{}|{:?}|{}|{}|{}|{:?}",
            q.context().map(|c| c.key().to_string()),
            q.order,
            q.current.as_ref().map(snap_item),
            q.cursor,
            q.history.iter().map(snap_item).collect::<Vec<_>>(),
            q.user.iter().map(|u| u.uri.clone()).collect::<Vec<_>>(),
            q.shuffle,
            q.repeat,
            q.seed,
            q.lap,
            q.ctx.len(),
            q.last_ctx,
        )
    }

    #[test]
    fn snapshot_roundtrip_restores_identical_queue() {
        let q = busy_queue();
        let state = encode(&snapshot(&q, 83_500, track()));
        let blob = encode_blob(&q);
        let back = restore(&state, Some(&blob)).unwrap();
        assert!(back.blob_ok);
        assert_eq!(back.position_ms, 83_500);
        assert_eq!(back.track, track());
        assert_eq!(fingerprint(&back.queue), fingerprint(&q));
        assert_eq!(back.queue.order_gen(), q.order_gen());
        // Kullanıcı şarkısının bilgisi de taşınır.
        assert_eq!(back.queue.user[0].track.as_ref().unwrap().name, "spotify:track:u1 adı");
        assert!(back.queue.user[1].track.is_none());
    }

    #[test]
    fn restored_queue_behaves_the_same_going_forward_and_back() {
        let original = busy_queue();
        let mut a = original.clone();
        let back = restore(
            &encode(&snapshot(&original, 0, None)),
            Some(&encode_blob(&original)),
        )
        .unwrap();
        let mut b = back.queue;
        for step in 0..60 {
            assert_eq!(a.peek_next_auto(), b.peek_next_auto(), "peek {step}");
            assert_eq!(a.next_auto(), b.next_auto(), "ileri {step}");
        }
        for step in 0..30 {
            assert_eq!(a.previous(), b.previous(), "geri {step}");
        }
    }

    #[test]
    fn context_blob_is_only_needed_when_order_changes() {
        // Konum / ilerleme `state`i değiştirir, `context`i değil.
        let mut q = busy_queue();
        let g = q.order_gen();
        let blob_before = encode_blob(&q);
        q.next();
        q.next();
        assert_eq!(q.order_gen(), g);
        let state = encode(&snapshot(&q, 5, None));
        let back = restore(&state, Some(&blob_before)).unwrap();
        assert!(back.blob_ok, "eski bağlam satırı yeni imleçle de geçerli");
        assert_eq!(fingerprint(&back.queue), fingerprint(&q));
    }

    #[test]
    fn mismatched_or_corrupt_context_falls_back_to_current_track_only() {
        let q = busy_queue();
        let state = encode(&snapshot(&q, 1_000, track()));

        for blob in [None, Some("{bozuk"), Some(r#"{"generation":777,"uris":[],"order":[]}"#)] {
            let back = restore(&state, blob).unwrap();
            assert!(!back.blob_ok);
            assert_eq!(back.position_ms, 1_000);
            assert_eq!(back.queue.current_uri(), q.current_uri());
            assert!(back.queue.context().is_none());
            assert_eq!(back.queue.user_len(), 2, "kullanıcı sırası korunur");
            assert!(back.queue.shuffle() && back.queue.repeat() == RepeatMode::All);
            assert!(back.queue.is_consistent());
        }

        // Tutarsız indeks (ctx'ten büyük): yine güvenli geri dönüş.
        let mut bad = q.clone();
        bad.order[0] = 9_999;
        let back = restore(&state, Some(&encode_blob(&bad)));
        let back = back.unwrap();
        assert!(!back.blob_ok);
    }

    #[test]
    fn garbage_and_old_version_restore_nothing_and_empty_keeps_modes() {
        assert!(restore("", None).is_none());
        assert!(restore("{}", None).is_none());
        assert!(restore("bozuk", None).is_none());
        let mut snap = snapshot(&busy_queue(), 0, None);
        snap.v = 99;
        assert!(restore(&encode(&snap), None).is_none());
        // Çalan da sıradaki de yok: yalnız karıştırma / tekrar durumu gelir.
        let mut modes = Queue::new();
        modes.set_shuffle(true);
        modes.set_repeat(RepeatMode::One);
        let back = restore(&encode(&snapshot(&modes, 0, None)), None).unwrap();
        assert!(back.queue.is_idle() && back.queue.shuffle() && back.queue.repeat() == RepeatMode::One);
    }

    /// Sanatçılı (dengeli karıştırılan) bağlam: 4 sanatçı × 10 şarkı.
    fn artist_queue() -> Queue {
        let rows: Vec<TrackRow> = uris(40)
            .into_iter()
            .enumerate()
            .map(|(i, uri)| TrackRow {
                uri,
                name: format!("Şarkı {i}"),
                artists: format!("Sanatçı {}", i % 4),
                album: String::new(),
                duration_ms: 1000,
                image: String::new(),
            })
            .collect();
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.restart(None, uris(40), Some(Arc::new(rows)), 2, 0xBEEF, true);
        q.next();
        q
    }

    #[test]
    fn artist_ids_survive_the_record_and_keep_shuffle_balanced() {
        let q = artist_queue();
        assert_eq!(q.artists.len(), 40);
        let back = restore(&encode(&snapshot(&q, 0, None)), Some(&encode_blob(&q))).unwrap();
        assert!(back.blob_ok);
        assert_eq!(back.queue.artists, q.artists);
        // Satır bilgisi kayıtta yok; karıştırmayı yeniden kurunca sıra yine
        // aynı (sanatçılar kayıttan).
        let (mut a, mut b) = (q.clone(), back.queue);
        for qq in [&mut a, &mut b] {
            qq.set_shuffle(false);
            qq.set_shuffle(true);
        }
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn old_context_record_without_artists_still_loads() {
        // 7 Kasım'dan önceki biçim: `artists` alanı yok.
        let q = artist_queue();
        let state = encode(&snapshot(&q, 7_000, None));
        let old_blob = format!(
            r#"{{"generation":{},"uris":{},"order":{}}}"#,
            q.order_gen(),
            serde_json::to_string(&q.ctx).unwrap(),
            serde_json::to_string(&q.order).unwrap(),
        );
        let back = restore(&state, Some(&old_blob)).unwrap();
        assert!(back.blob_ok, "eski bağlam satırı okunur");
        assert!(back.queue.artists.is_empty(), "sanatçı bilinmiyor");
        assert_eq!(back.queue.order, q.order, "kayıtlı sıra korunur");
        assert_eq!(back.queue.current_uri(), q.current_uri());
        // Karıştırma yeniden kurulursa düz rastgele (çökmeden, eksiksiz).
        let mut r = back.queue;
        r.set_shuffle(false);
        r.set_shuffle(true);
        assert!(r.is_consistent());
        let mut seen = 0;
        while r.next().is_some() {
            seen += 1;
        }
        assert_eq!(seen, 40 - 2, "çalınmış iki şarkı (geçmiş) dışında hepsi");

        // Sanatçı sayısı bağlamla uyuşmayan (bozuk) kayıt: bilinmiyor sayılır.
        let bad = format!(
            r#"{{"generation":{},"uris":{},"order":{},"artists":[1,2]}}"#,
            q.order_gen(),
            serde_json::to_string(&q.ctx).unwrap(),
            serde_json::to_string(&q.order).unwrap(),
        );
        let back = restore(&state, Some(&bad)).unwrap();
        assert!(back.blob_ok && back.queue.artists.is_empty());
    }

    #[test]
    fn queue_with_only_user_items_restores() {
        let mut q = Queue::new();
        q.enqueue(vec![item("spotify:track:u1", true)]);
        let back = restore(&encode(&snapshot(&q, 0, None)), Some(&encode_blob(&q))).unwrap();
        assert!(back.queue.has_next());
        assert_eq!(back.queue.user_len(), 1);
    }

    #[test]
    fn large_context_blob_is_reasonably_small() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.restart(Some(CollectionId::Liked), uris(6_000), None, 0, 1, true);
        let blob = encode_blob(&q);
        assert!(blob.len() < 400_000, "6.000 şarkılık bağlam {} bayt", blob.len());
        let state = encode(&snapshot(&q, 0, track()));
        assert!(state.len() < 2_000, "küçük kayıt {} bayt", state.len());
    }

    // ── Veritabanı ve yazıcı ──

    #[test]
    fn db_roundtrip_and_blob_is_kept_when_omitted() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.load_playback().unwrap().is_none());
        db.save_playback("s1", Some("b1")).unwrap();
        assert_eq!(db.load_playback().unwrap().unwrap(), ("s1".to_string(), Some("b1".to_string())));
        db.save_playback("s2", None).unwrap();
        assert_eq!(db.load_playback().unwrap().unwrap(), ("s2".to_string(), Some("b1".to_string())));
        db.save_playback("s3", Some("b3")).unwrap();
        assert_eq!(db.load_playback().unwrap().unwrap().1.as_deref(), Some("b3"));
    }

    #[test]
    fn saver_writes_latest_state_and_flushes() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let saver = Saver::spawn(db.clone());
        saver.save("bir".into(), Some("blob".into()));
        for i in 0..20 {
            saver.save(format!("durum{i}"), None);
        }
        assert!(saver.flush(Duration::from_secs(5)));
        let (state, blob) = db.load_playback().unwrap().unwrap();
        assert_eq!(state, "durum19");
        assert_eq!(blob.as_deref(), Some("blob"), "büyük satır korunur");
        drop(saver);
    }

    #[test]
    fn full_pipeline_through_db() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let q = busy_queue();
        let saver = Saver::spawn(db.clone());
        saver.save(encode(&snapshot(&q, 42_000, track())), Some(encode_blob(&q)));
        assert!(saver.flush(Duration::from_secs(5)));
        let (state, blob) = db.load_playback().unwrap().unwrap();
        let back = restore(&state, blob.as_deref()).unwrap();
        assert_eq!(back.position_ms, 42_000);
        assert_eq!(fingerprint(&back.queue), fingerprint(&q));
    }
}
