//! Kaldığı yerden sürme: sırayı ve konumu kaydet (`persist*`), açılışta kayıtlı
//! durumu geri yükle (`restore_playback`). Motorun `impl`i; durum `Playback`te
//! (`restored`, `track_info`, konum), kayıt `resume.rs`'te.

use super::Driver;
use crate::engine::{
    Engine, Event, PlaybackEvent, RESUME, Restored, TrackInfo, cache, palette, queue::Queue,
    queue_state, resume,
};

impl Engine {
    /// Sırayı ve konumu kaydeder (ayrı iş parçacığında yazılır).
    pub(in crate::engine) fn persist(&mut self) {
        self.persist_at(None);
    }

    /// `position`: verilirse konum olarak o yazılır (ör. biten sırada 0).
    pub(in crate::engine) fn persist_at(&mut self, position: Option<u32>) {
        if self.saver.is_none() {
            return;
        }
        // Telefon sürerken kaldığı yer kaydı donar: sıra bizim değil, konum ve
        // şarkı telefonunki (bkz. `playback/connect.rs`). Telefon bırakınca
        // yerel durum geri gelir ve yeniden yazılır.
        if !matches!(self.pb.driver, Driver::Local(_) | Driver::Remote(_)) {
            return;
        }
        // Boşluksuz geçişte tutma sürerken tahmini konumla kayıt yazılmaz: sıra
        // yeni şarkıya geçti ama arayüz (ve duyulan ses) hâlâ eskisinde; o an
        // yazılan kayıtta şarkı ile konum birbirini tutmazdı. Tutma bitince bir
        // sonraki kayıt (en geç `PERSIST_INTERVAL` sonra) doğrusunu yazar.
        if position.is_none() && self.pb.gate.is_holding() {
            return;
        }
        let (position, track) = self.with_pb(|pb, cx| pb.resume_state(cx, position));
        let Some(saver) = &self.saver else { return };
        let snapshot = resume::snapshot(&self.queue, position, track);
        let generation = self.queue.order_gen();
        // Büyük kayıt (bağlamın şarkı listesi) yalnız sıra değişince yazılır.
        let blob = (self.saved_gen != Some(generation)).then(|| resume::encode_blob(&self.queue));
        saver.save(resume::encode(&snapshot), blob);
        self.saved_gen = Some(generation);
    }

    /// Her `PERSIST_INTERVAL`'de, çalıyorsa konumu kaydeder.
    pub(in crate::engine) fn persist_tick(&mut self) {
        if matches!(self.pb.driver, Driver::Local(_))
            && matches!(self.pb.last_position, Some((_, _, true)))
        {
            self.persist();
            // Radyo sona yaklaştıysa devamı (yalnız çalarken; bkz. `listening.rs`).
            self.radio_tick();
        }
    }

    /// Açılışta kayıtlı durumu geri yükler: sıra modele konur, arayüze
    /// `PlaybackEvent::Restored` gider; çalara HİÇBİR şey yüklenmez.
    pub(in crate::engine) fn restore_playback(&mut self) {
        let Some(db) = self.db.clone() else { return };
        self.saver = Some(resume::Saver::spawn(db.clone()));
        if !RESUME.load(std::sync::atomic::Ordering::Relaxed) {
            log::info!("kaldığı yer: ayarlardan kapalı, geri yüklenmiyor");
            return;
        }
        let loaded = match db.load_playback() {
            Ok(Some(loaded)) => loaded,
            Ok(None) => return,
            Err(e) => {
                log::warn!("Kaldığı yer okunamadı: {e:#}");
                return;
            }
        };
        let Some(restored) = resume::restore(&loaded.0, loaded.1.as_deref()) else {
            log::warn!("Kaldığı yer kaydı geçersiz, yok sayıldı");
            return;
        };
        self.queue = restored.queue;
        self.queue.adopt_rules(self.rules.clone());
        self.queue.adopt_hidden(self.hidden.clone());
        self.saved_gen = restored.blob_ok.then(|| self.queue.order_gen());
        // Kayıtlı sıra kurallara uyuyor mu: gerekirse yalnız şimdi (satırlar
        // önbellekten gelince) süzülür; uyuyorsa kayıtlı sıra olduğu gibi kalır.
        self.apply_rule_data();
        // Kayıttan sonra gizlenen şarkı kalan sıradaysa yalnız o düşer.
        self.queue.refresh_if_needed();

        // Sıradaki şarkıların bilgisi (açılışta ~55 tek satırlık okuma).
        let first = self.queue.view(&|_| None);
        let missing = Queue::missing_meta(&first);
        if let Ok(found) = db.tracks(&missing) {
            self.meta.extend(found);
        }
        self.meta_requested.extend(missing);

        let Some(uri) = self.queue.current_uri().map(str::to_owned) else {
            // Çalan yok (yalnız sıra ya da karıştırma / tekrar durumu).
            self.emit(Event::QueueChanged(queue_state(&self.queue)));
            let view = self.queue.view(&|u| self.meta.get(u).cloned());
            let _ = self.playback.try_send(PlaybackEvent::Queue(view));
            return;
        };
        let row = self
            .queue
            .current_row()
            .or_else(|| self.meta.get(&uri).cloned());
        let info = match (restored.track, row) {
            (Some(t), _) if t.uri == uri => TrackInfo {
                uri,
                name: t.name,
                artists: t.artists,
                duration_ms: t.duration_ms,
                cover_url: t.cover_url,
            },
            (_, Some(r)) => TrackInfo {
                uri,
                name: r.name,
                artists: r.artists,
                duration_ms: r.duration_ms,
                cover_url: (!r.image.is_empty()).then_some(r.image),
            },
            _ => TrackInfo {
                uri,
                name: String::new(),
                artists: String::new(),
                duration_ms: 0,
                cover_url: None,
            },
        };
        let position_ms = match info.duration_ms {
            0 => restored.position_ms,
            d => restored.position_ms.min(d),
        };
        self.pb.restored = Some(position_ms);
        self.pb.track_info = Some(info.clone());

        let queue = queue_state(&self.queue);
        let view = self.queue.view(&|u| self.meta.get(u).cloned());
        let more = Queue::missing_meta(&view);
        self.request_meta(more);
        // Kapak diskteyse (varsa) okunup vurgu rengiyle gönderilir; ağa çıkılmaz.
        let playback = self.playback.clone();
        self.bg.spawn_blocking(move || {
            let cover = info
                .cover_url
                .as_deref()
                .and_then(cache::read_cover)
                .filter(|b| !b.is_empty())
                .map(|bytes| {
                    let accent = palette::accent_from_image(&bytes);
                    (bytes, accent)
                });
            let _ = playback.try_send(PlaybackEvent::Restored(Box::new(Restored {
                track: info,
                position_ms,
                queue,
                view,
                cover,
            })));
        });
    }
}
