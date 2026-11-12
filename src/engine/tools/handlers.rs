//! Kütüphane araçlarının motor tarafı komutları (`LibraryCommand::RecordPlay`,
//! `PlayStats`, `ExportBackup`; `wire.rs` bunlara yönlendirir).
//!
//! Üçü de YEREL: çalma geçmişi library.db'ye yazılır / oradan okunur, yedek
//! kullanıcının seçtiği dosyaya yazılır. Spotify'a hiçbir istek ya da yazma
//! gitmez; edit kuyruğuna girmezler.

use std::{path::PathBuf, sync::Arc};

use super::{
    backup, civil,
    stats::{self, PlayLog, StatsRange},
};
use crate::engine::{Engine, Event, wire::LibraryEvent};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Engine {
    /// Bir çalma kaydı. Gizli oturumda yazılmaz. Eşzamanlı yazılır (tek satır,
    /// WAL): `Count` ile hemen ardından gelen `Extend` sırasını yitirmesin.
    pub(in crate::engine) fn tools_record_play(&mut self, log: PlayLog) {
        if stats::private_session() {
            return;
        }
        let Some(db) = &self.db else { return };
        let written = match &log {
            PlayLog::Count(rec) => db.record_play(rec),
            PlayLog::Extend { uri, played_at, add_ms } => db.extend_play(uri, *played_at, *add_ms),
        };
        if let Err(e) = written {
            log::warn!("Çalma geçmişi yazılamadı: {e:#}");
        }
    }

    /// İstatistik raporu (arka planda hesaplanır, `LibraryEvent::PlayStats`).
    pub(in crate::engine) fn tools_play_stats(&mut self, range: StatsRange) {
        let Some(db) = self.db.clone() else { return };
        let events = self.events.clone();
        self.bg.spawn_blocking(move || {
            match stats::report(&db, range, now_ms(), civil::local_offset_minutes()) {
                Ok(report) => {
                    let _ = events.try_send(Event::Library(LibraryEvent::PlayStats(Arc::new(report))));
                }
                Err(e) => log::warn!("İstatistik okunamadı: {e:#}"),
            }
        });
    }

    /// Kütüphane yedeği (arka planda yazılır, `BackupDone` / `BackupFailed`).
    pub(in crate::engine) fn tools_export_backup(&mut self, path: PathBuf) {
        let events = self.events.clone();
        let Some(db) = self.db.clone() else {
            let _ = events.try_send(Event::Library(LibraryEvent::BackupFailed));
            return;
        };
        self.bg.spawn_blocking(move || {
            let event = match backup::export(&db, &path, now_ms(), civil::local_offset_minutes()) {
                Ok(summary) => LibraryEvent::BackupDone(summary),
                Err(e) => {
                    log::warn!("Yedek yazılamadı ({}): {e:#}", path.display());
                    LibraryEvent::BackupFailed
                }
            };
            let _ = events.try_send(Event::Library(event));
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;
    use crate::engine::{
        AudioLevels,
        cache::Db,
        library::{CollectionId, Entry, Library, LibraryItem, TrackRow},
        tools::stats::PlayRecord,
        wire::LibraryCommand,
    };

    struct Rig {
        engine: Engine,
        events: async_channel::Receiver<Event>,
        db: Arc<Db>,
        _rt: tokio::runtime::Runtime,
    }

    fn rig() -> Rig {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (events_tx, events) = async_channel::unbounded();
        let (playback_tx, _playback) = async_channel::unbounded();
        let (internal_tx, _internal) = mpsc::unbounded_channel();
        let db = Arc::new(Db::open_in_memory().unwrap());
        let engine =
            Engine::new(rt.handle().clone(), Some(db.clone()), events_tx, playback_tx, internal_tx, AudioLevels::new());
        Rig { engine, events, db, _rt: rt }
    }

    fn record(n: u32, at: i64) -> PlayRecord {
        PlayRecord {
            uri: format!("spotify:track:{n:022}"),
            name: format!("Şarkı {n}"),
            artists: "Sanatçı".into(),
            album: "Albüm".into(),
            duration_ms: 200_000,
            listened_ms: 31_000,
            played_at: at,
        }
    }

    fn next_event(rig: &Rig) -> Option<LibraryEvent> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(Event::Library(e)) = rig.events.try_recv() {
                return Some(e);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn counted_plays_are_written_extended_and_reported() {
        let _guard = stats::FLAG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rig = rig();
        let at = now_ms() - 1000;
        rig.engine.on_library_command(LibraryCommand::RecordPlay(PlayLog::Count(record(1, at))));
        rig.engine.on_library_command(LibraryCommand::RecordPlay(PlayLog::Extend {
            uri: record(1, at).uri,
            played_at: at,
            add_ms: 9_000,
        }));
        let rows = rig.db.plays_since(0).unwrap();
        assert_eq!((rows.len(), rows[0].listened_ms), (1, 40_000));
        rig.engine.on_library_command(LibraryCommand::PlayStats(StatsRange::Weeks4));
        match next_event(&rig) {
            Some(LibraryEvent::PlayStats(report)) => {
                assert_eq!((report.range, report.plays, report.listened_ms), (StatsRange::Weeks4, 1, 40_000));
                assert_eq!(report.tracks[0].name, "Şarkı 1");
            }
            other => panic!("istatistik olayı beklendi: {other:?}"),
        }
    }

    #[test]
    fn nothing_is_written_in_a_private_session() {
        let _guard = stats::FLAG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rig = rig();
        stats::set_private_session(true);
        rig.engine.on_library_command(LibraryCommand::RecordPlay(PlayLog::Count(record(1, 5))));
        stats::set_private_session(false);
        assert!(rig.db.plays_since(0).unwrap().is_empty(), "gizli oturumda geçmiş yazılmaz");
        rig.engine.on_library_command(LibraryCommand::RecordPlay(PlayLog::Count(record(2, 6))));
        assert_eq!(rig.db.plays_since(0).unwrap().len(), 1, "oturum bitince yeniden yazılır");
    }

    #[test]
    fn backup_command_writes_the_file_and_reports_a_summary_or_failure() {
        let mut rig = rig();
        let dir = std::env::temp_dir().join(format!("openify-handlers-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Önbellekte kütüphane yok: başarısız, dosya yok.
        let path = dir.join("yedek.json");
        rig.engine.on_library_command(LibraryCommand::ExportBackup(path.clone()));
        assert!(matches!(next_event(&rig), Some(LibraryEvent::BackupFailed)));
        assert!(!path.exists());
        // Kütüphane önbelleğe girince yazılır.
        let lib = Library {
            liked_count: 1,
            playlists: vec![LibraryItem {
                id: CollectionId::Playlist("spotify:playlist:1".into()),
                name: "Bir".into(),
                subtitle: "1 şarkı".into(),
                image: String::new(),
            }],
            ..Default::default()
        };
        rig.db.save_library(&lib).unwrap();
        let row = TrackRow {
            uri: "spotify:track:a".into(),
            name: "A".into(),
            artists: "X".into(),
            album: "Y".into(),
            duration_ms: 1,
            image: String::new(),
        };
        rig.db.save_tracks(std::slice::from_ref(&row)).unwrap();
        rig.db
            .save_collection_entries(&CollectionId::Playlist("spotify:playlist:1".into()), Some(b"1"), &[Entry::untimed(row.uri)])
            .unwrap();
        rig.engine.on_library_command(LibraryCommand::ExportBackup(path.clone()));
        match next_event(&rig) {
            Some(LibraryEvent::BackupDone(s)) => assert_eq!((s.lists, s.tracks, s.path.clone()), (1, 1, path.clone())),
            other => panic!("yedek özeti beklendi: {other:?}"),
        }
        assert!(path.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
