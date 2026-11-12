//! Dinleme çekirdeği, motor tarafı: radyo (başlatma ve sonsuz devam) ve yerel
//! şarkı gizleme. Saf parçalar `radio.rs` (çözümleme, zamanlama) ve
//! `hidden.rs` (dizin, tablo); burada yalnız motorun durumuna bağlanan yapıştırıcı.
//!
//! - **Radyo**: `Command::StartRadio` tohum için istasyonu alır (tek istek,
//!   salt okuma), şarkı bilgilerini toplu istekle tamamlar ve bağlamsız
//!   (`context: None`) yeni bir çalma bağlamı olarak başlatır; sıra
//!   `RadioInfo` taşır. Çalarken (`persist_tick`, ~5 sn'de bir) kalan şarkı
//!   `radio::REFILL_AT`'a inince, aralık (`radio::MIN_INTERVAL`) dolmuşsa ve
//!   yoldaki istek yoksa devamı istenir; gelenler çalma sırasının sonuna
//!   kurallardan geçerek eklenir. Bir radyo başlayınca eski radyonun gecikmiş
//!   yanıtları nesil numarasıyla düşer.
//! - **Gizleme**: `LibraryCommand::SetHidden` `hidden_songs` tablosunu yazar,
//!   sıra modelinin gizlenenlerini yeniler ve arayüze yeni dizini yollar.
//!   Spotify'a hiçbir şey gitmez.

use std::{collections::HashMap, sync::Arc, time::Instant};

use super::{
    Engine, Event, Internal, NoticeKind, blank_row,
    cache::Db,
    hidden::{HiddenList, HiddenSong},
    library::{self, TrackRow},
    radio::{self, RadioFailure, RadioInfo},
    rules::Traits,
    wire::LibraryEvent,
};
use crate::t;

/// Radyo isteğinin sonucu (arka plandan motora; bkz. `Internal::Radio`).
pub(super) struct RadioBatch {
    pub seed: String,
    /// Menüden gelen ad (boşsa Spotify'ın istasyon başlığı kullanılır).
    pub title: String,
    /// Yeni radyo mu (`false`: sıranın devamı)?
    pub start: bool,
    /// Devam isteğinin yapıldığı andaki radyo nesli (eskimiş yanıt düşer).
    pub generation: u64,
    pub result: Result<Fetched, RadioFailure>,
}

/// Çalınabilir hâle getirilmiş istasyon.
pub(super) struct Fetched {
    pub rows: Vec<TrackRow>,
    pub traits: HashMap<String, Traits>,
    pub station_title: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn failure_text(why: RadioFailure) -> &'static str {
    match why {
        RadioFailure::RateLimited => t!(NoticeRadioLimited),
        RadioFailure::Denied => t!(NoticeRadioDenied),
        RadioFailure::Offline | RadioFailure::Unavailable | RadioFailure::Empty => {
            t!(NoticeRadioFailed)
        }
    }
}

/// İstasyonu alır ve şarkı bilgilerini tamamlar: önce önbellek, sonra Spotify'ın
/// toplu şarkı isteği (yeni gelenler önbelleğe yazılır); bilgisi hiç gelmeyen
/// şarkı için yanıtın taşıdığı künye (varsa) yedek olur, o da yoksa şarkı atlanır.
async fn gather(
    session: &librespot::core::session::Session,
    db: Option<Arc<Db>>,
    seed: &str,
    prev: &[String],
    first: bool,
) -> Result<Fetched, RadioFailure> {
    let station = radio::load(session, seed, prev, first).await?;
    let mut uris: Vec<String> = Vec::new();
    // Şarkı radyosu tohum şarkıyla başlar (yanıtta zaten varsa tekrar edilmez).
    if first && radio::is_track_seed(seed) {
        uris.push(seed.to_string());
    }
    for t in &station.tracks {
        if !uris.contains(&t.uri) {
            uris.push(t.uri.clone());
        }
    }
    let hinted: HashMap<String, TrackRow> = station
        .tracks
        .iter()
        .filter_map(|t| t.row.clone().map(|r| (t.uri.clone(), r)))
        .collect();

    let mut rows: HashMap<String, TrackRow> = HashMap::new();
    if let Some(db) = db.clone() {
        let wanted = uris.clone();
        let cached = tokio::task::spawn_blocking(move || db.tracks(&wanted).unwrap_or_default())
            .await
            .unwrap_or_default();
        rows.extend(cached);
    }
    let missing: Vec<String> = uris
        .iter()
        .filter(|u| !rows.contains_key(*u))
        .cloned()
        .collect();
    if !missing.is_empty() {
        let fetched = library::fetch_tracks(session, missing).await;
        if let Some(db) = db.clone()
            && !fetched.is_empty()
        {
            let to_save = fetched.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = db.save_tracks(&to_save) {
                    log::warn!("Radyo şarkılarının bilgisi yazılamadı: {e:#}");
                }
            })
            .await;
        }
        rows.extend(fetched.into_iter().map(|r| (r.uri.clone(), r)));
    }
    for (uri, row) in hinted {
        rows.entry(uri).or_insert(row);
    }
    let ordered: Vec<TrackRow> = uris.iter().filter_map(|u| rows.remove(u)).collect();
    if ordered.is_empty() {
        log::info!(
            "radyo: {} şarkı geldi ama hiçbirinin bilgisi alınamadı",
            uris.len()
        );
        return Err(RadioFailure::Unavailable);
    }
    let traits = match db {
        Some(db) => {
            let wanted: Vec<String> = ordered.iter().map(|r| r.uri.clone()).collect();
            tokio::task::spawn_blocking(move || db.track_traits(&wanted).unwrap_or_default())
                .await
                .unwrap_or_default()
        }
        None => HashMap::new(),
    };
    Ok(Fetched {
        rows: ordered,
        traits,
        station_title: station.title,
    })
}

impl Engine {
    // ── Radyo ───────────────────────────────────────────────────────────

    /// `Command::StartRadio`: tohum için radyoyu getir ve çal.
    pub(super) fn start_radio(&mut self, seed: String, title: String) {
        if !radio::seed_ok(&seed) {
            self.notice(NoticeKind::Problem, t!(NoticeRadioFailed));
            return;
        }
        self.radio.want(&seed);
        self.fetch_radio(seed, title, Vec::new(), true);
    }

    fn fetch_radio(&mut self, seed: String, title: String, prev: Vec<String>, start: bool) {
        let Some(conn) = &self.host.conn else { return };
        let session = conn.session.clone();
        let db = self.db.clone();
        let internal = self.internal.clone();
        if !start {
            self.radio.begin(Instant::now());
        }
        let generation = self.radio.generation();
        self.bg.spawn(async move {
            let result = gather(&session, db, &seed, &prev, start).await;
            let _ = internal.send(Internal::Radio(RadioBatch {
                seed,
                title,
                start,
                generation,
                result,
            }));
        });
    }

    /// Sıra sonuna yaklaşıldıysa radyonun devamını ister. Yalnız çalarken
    /// (`persist_tick`) çağrılır: duraklatılmış / kaldığı yerden açılmış radyo
    /// istek atmaz.
    pub(super) fn radio_tick(&mut self) {
        let Some(info) = self.queue.radio().cloned() else {
            return;
        };
        if self.host.conn.is_none()
            || !self.radio.due(
                Instant::now(),
                self.queue.context_remaining(),
                self.queue.ctx_uris().len(),
            )
        {
            return;
        }
        let prev = self.queue.ctx_uris().to_vec();
        self.fetch_radio(info.seed, info.title, prev, false);
    }

    /// Arka plandan gelen radyo yanıtı (bkz. `Internal::Radio`).
    pub(super) fn on_radio_batch(&mut self, batch: RadioBatch) {
        let RadioBatch {
            seed,
            title,
            start,
            generation,
            result,
        } = batch;
        if start {
            // Daha yeni bir radyo istenmişse bu yanıt eskidir.
            if !self.radio.take_wanted(&seed) {
                return;
            }
            match result {
                Ok(fetched) => self.begin_radio(seed, title, fetched),
                Err(why) => {
                    log::info!("radyo başlatılamadı: {why:?}");
                    self.notice(NoticeKind::Problem, failure_text(why));
                }
            }
            return;
        }
        // Devam: yalnız hâlâ aynı radyo çalıyorsa.
        if generation != self.radio.generation()
            || self.queue.radio().map(|r| r.seed.as_str()) != Some(seed.as_str())
        {
            return;
        }
        let added = match result {
            Ok(fetched) => self.extend_radio(fetched),
            Err(why) => {
                log::info!("radyonun devamı alınamadı: {why:?}");
                0
            }
        };
        self.radio.end(added);
        if added > 0 {
            self.queue_changed();
        }
    }

    /// Yeni radyoyu çalma bağlamı olarak başlatır.
    fn begin_radio(&mut self, seed: String, title: String, fetched: Fetched) {
        let Fetched {
            rows,
            traits,
            station_title,
        } = fetched;
        let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
        let start = radio::first_playable(
            &rows,
            &|u| traits.get(u).copied(),
            &self.rules,
            &self.hidden,
            radio::is_track_seed(&seed),
        );
        let title = if title.trim().is_empty() {
            station_title
        } else {
            title
        };
        self.start_queue(None, uris, Some(Arc::new(rows)), start);
        // Başlatma sıraya `radio`yu düşürdü (yeni bağlam): kimliğini şimdi koy.
        self.queue.set_radio(Some(RadioInfo { seed, title }));
        self.radio.started(Instant::now());
        self.queue_changed();
    }

    /// Gelen şarkıları bağlamın sonuna ekler; eklenen sayısı.
    fn extend_radio(&mut self, fetched: Fetched) -> usize {
        // Kaldığı yerden açılmış radyoda satırlar yok: adlar önbellekten (kurallar ve
        // sanatçı dengesi satırlara bakar).
        if self.queue.needs_info()
            && let Some(db) = &self.db
        {
            let uris = self.queue.ctx_uris().to_vec();
            let found = db.tracks(&uris).unwrap_or_default();
            let info: Vec<TrackRow> = uris
                .iter()
                .map(|u| found.get(u).cloned().unwrap_or_else(|| blank_row(u)))
                .collect();
            self.queue.attach_info(Arc::new(info));
        }
        let added = self.queue.extend_context(fetched.rows);
        if added > 0 {
            self.queue.merge_traits(&fetched.traits);
        }
        added
    }

    // ── Gizlenen şarkılar ───────────────────────────────────────────────

    /// Açılışta kayıtlı gizlenenleri yükler (kaldığı yer geri yüklenmeden önce) ve
    /// arayüze yollar.
    pub(super) fn init_hidden(&mut self) {
        let Some(db) = self.db.clone() else { return };
        match db.hidden_songs() {
            Ok(list) => {
                self.hidden = Arc::new(HiddenList::new(list));
                self.queue.adopt_hidden(self.hidden.clone());
                self.emit(Event::Library(LibraryEvent::Hidden(self.hidden.clone())));
            }
            Err(e) => log::warn!("Gizlenen şarkılar okunamadı: {e:#}"),
        }
    }

    /// `LibraryCommand::SetHidden`: şarkıları gizle (`on`) ya da göster. Yerel;
    /// Spotify'a hiçbir şey gitmez.
    pub(super) fn set_hidden(&mut self, songs: Vec<HiddenSong>, on: bool) {
        let Some(db) = self.db.clone() else { return };
        let now = now_ms();
        for s in &songs {
            let done = if on {
                db.hide_song(&HiddenSong {
                    hidden_at: now,
                    ..s.clone()
                })
            } else {
                db.show_song(&s.uri, s.context.as_deref()).map(|_| ())
            };
            if let Err(e) = done {
                log::warn!("Şarkı gizleme yazılamadı: {e:#}");
            }
        }
        let list = Arc::new(HiddenList::new(db.hidden_songs().unwrap_or_default()));
        if list == self.hidden {
            return;
        }
        self.hidden = list.clone();
        if self.queue.set_hidden(list.clone()) {
            self.queue_changed();
        }
        self.emit(Event::Library(LibraryEvent::Hidden(list)));
    }
}
