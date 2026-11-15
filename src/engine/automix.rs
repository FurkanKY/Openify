//! AutoMix: şarkı çözümlemelerinin kalıcı önbelleği (`library.db`, tablo
//! `track_analysis`, şema sürümü 9).
//!
//! Çözümlemenin kendisi `output/beat.rs` (ölçüm) ve `output/automix.rs` (karar):
//! çalar iş parçacığı, çalınan şarkıyı çözerken sonucu çıkışa koyar
//! (`OutputCtl::take_analyses`), `Playback` onu bellekteki deftere (`beats`) ve
//! `Effect::SaveAnalysis` ile buraya yazar. Bir şarkı sonraki çalışında, geçişten
//! ÖNCE (şarkı sonu payı açılırken) giden / gelen ucu bilinir: payın derinliği ve
//! gelenin tempo / giriş bilgisi buradan gelir (bkz. `playback/crossfade.rs`).
//!
//! Veri sınıfı: çalarak yeniden hesaplanabilir (indirme gerekmez, Spotify'a istek
//! gitmez); göç bir sonraki sürümde de düşürmez, yalnız `ALGO` değişirse eski
//! satırlar yok sayılır (ölçüm sabitleri ayarlanınca). En çok `KEEP_MAX` satır
//! (en eski çalınan atılır).

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    cache::Db,
    output::{Analysis, ENV_SECS, Edge, TrackFeatures},
};

/// Çözümleme algoritmasının sürümü: ölçüm sabitleri / yöntemi değişince artırılır.
pub const ALGO: i64 = 1;
/// Version for the whole-track music feature estimator, independent of AutoMix.
pub const FEATURES_ALGO: i64 = 1;
/// Tabloda tutulan en çok şarkı.
pub const KEEP_MAX: i64 = 20_000;
/// Açılışta (AutoMix açıkken) belleğe alınan en çok şarkı (en yeni çalınanlar).
pub const LOAD_MAX: usize = 2_000;

/// 8 → 9: şarkı çözümlemeleri (yeni tablo, veri kaybı yok).
pub(super) fn track_analysis_step(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- AutoMix analysis per track (see output/beat.rs). Recomputable by playing the track again.
         -- head_* describe the first seconds after the music starts, tail_* the last seconds
         -- before it ends; flags: 1 = head valid, 2 = tail valid. *_env: 20 x i8 dBFS, one per second.
         CREATE TABLE IF NOT EXISTS track_analysis (
             uri          TEXT PRIMARY KEY,
             algo         INTEGER NOT NULL,
             flags        INTEGER NOT NULL,
             total_ms     INTEGER NOT NULL DEFAULT 0,
             lead_ms      INTEGER NOT NULL DEFAULT 0,
             tail_ms      INTEGER NOT NULL DEFAULT 0,
             head_bpm     REAL    NOT NULL DEFAULT 0,
             head_conf    REAL    NOT NULL DEFAULT 0,
             head_beat_ms INTEGER NOT NULL DEFAULT 0,
             tail_bpm     REAL    NOT NULL DEFAULT 0,
             tail_conf    REAL    NOT NULL DEFAULT 0,
             tail_beat_ms INTEGER NOT NULL DEFAULT 0,
             head_env     BLOB    NOT NULL,
             tail_env     BLOB    NOT NULL,
             analyzed_at  INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS track_analysis_at ON track_analysis (analyzed_at);",
    )
}

/// 13 → 14: whole-track BPM, key, and energy, kept separate from AutoMix data.
pub(super) fn track_features_step(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS track_features (
             uri TEXT PRIMARY KEY,
             algo INTEGER NOT NULL,
             bpm REAL NOT NULL,
             bpm_conf REAL NOT NULL,
             key_index INTEGER NOT NULL,
             key_minor INTEGER NOT NULL,
             key_conf REAL NOT NULL,
             energy INTEGER NOT NULL,
             analyzed_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS track_features_at ON track_features (analyzed_at);",
    )
}

fn env_blob(env: &[i8; ENV_SECS]) -> Vec<u8> {
    env.iter().map(|v| *v as u8).collect()
}

fn env_from(blob: Vec<u8>) -> [i8; ENV_SECS] {
    let mut env = [i8::MIN; ENV_SECS];
    for (slot, b) in env.iter_mut().zip(blob) {
        *slot = b as i8;
    }
    env
}

const COLUMNS: &str = "flags, total_ms, lead_ms, tail_ms, head_bpm, head_conf, head_beat_ms, \
                       tail_bpm, tail_conf, tail_beat_ms, head_env, tail_env";

fn from_row(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Analysis> {
    Ok(Analysis {
        flags: r.get::<_, i64>(at)? as u8,
        total_ms: r.get::<_, i64>(at + 1)? as u32,
        lead_ms: r.get::<_, i64>(at + 2)? as u32,
        tail_ms: r.get::<_, i64>(at + 3)? as u32,
        head: Edge {
            bpm: r.get::<_, f64>(at + 4)? as f32,
            conf: r.get::<_, f64>(at + 5)? as f32,
            beat_ms: r.get::<_, i64>(at + 6)? as u32,
        },
        tail: Edge {
            bpm: r.get::<_, f64>(at + 7)? as f32,
            conf: r.get::<_, f64>(at + 8)? as f32,
            beat_ms: r.get::<_, i64>(at + 9)? as u32,
        },
        head_env: env_from(r.get(at + 10)?),
        tail_env: env_from(r.get(at + 11)?),
        features: None,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn read_one(conn: &Connection, uri: &str) -> rusqlite::Result<Option<Analysis>> {
    let mut analysis = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM track_analysis WHERE uri = ?1 AND algo = ?2"),
            params![uri, ALGO],
            |r| from_row(r, 0),
        )
        .optional()?;
    if let Some(a) = &mut analysis {
        a.features = read_features(conn, uri)?;
    }
    Ok(analysis)
}

pub(super) fn read_features(
    conn: &Connection,
    uri: &str,
) -> rusqlite::Result<Option<TrackFeatures>> {
    conn.query_row(
        "SELECT bpm, bpm_conf, key_index, key_minor, key_conf, energy
         FROM track_features WHERE uri = ?1 AND algo = ?2",
        params![uri, FEATURES_ALGO],
        |r| {
            Ok(TrackFeatures {
                bpm: r.get::<_, f64>(0)? as f32,
                bpm_conf: r.get::<_, f64>(1)? as f32,
                key: r.get::<_, i64>(2)? as u8,
                minor: r.get::<_, i64>(3)? != 0,
                key_conf: r.get::<_, f64>(4)? as f32,
                energy: r.get::<_, i64>(5)? as u8,
            })
        },
    )
    .optional()
}

fn write_features(conn: &Connection, uri: &str, features: TrackFeatures) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO track_features (uri, algo, bpm, bpm_conf, key_index, key_minor, key_conf, energy, analyzed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(uri) DO UPDATE SET algo=excluded.algo, bpm=excluded.bpm,
            bpm_conf=excluded.bpm_conf, key_index=excluded.key_index, key_minor=excluded.key_minor,
            key_conf=excluded.key_conf, energy=excluded.energy, analyzed_at=excluded.analyzed_at",
        params![
            uri,
            FEATURES_ALGO,
            features.bpm,
            features.bpm_conf,
            features.key,
            i64::from(features.minor),
            features.key_conf,
            features.energy,
            now_ms()
        ],
    )?;
    conn.execute(
        "DELETE FROM track_features WHERE uri IN
            (SELECT uri FROM track_features ORDER BY analyzed_at DESC LIMIT -1 OFFSET ?1)",
        [KEEP_MAX],
    )?;
    Ok(())
}

impl Db {
    /// Şarkının önbellekteki çözümlemesi.
    #[allow(dead_code)] // testler ve tanı; motor `load_analyses` ile toplu okur
    pub fn analysis(&self, uri: &str) -> Result<Option<Analysis>> {
        Ok(read_one(&self.r(), uri)?)
    }

    /// Yeni çözümlemeyi kaydeder: aynı şarkının eski satırıyla birleşir (yeni geçerli
    /// uçlar eskinin yerine geçer, geçersiz ucu eskisi korunur), en çok `KEEP_MAX`
    /// satır kalır. Hiçbir uç geçerli değilse hiçbir şey yazılmaz.
    pub fn save_analysis(&self, uri: &str, analysis: &Analysis) -> Result<()> {
        if uri.is_empty() {
            return Ok(());
        }
        let conn = self.w();
        if analysis.flags == 0 {
            if let Some(features) = analysis.features {
                write_features(&conn, uri, features)?;
            }
            return Ok(());
        }
        let merged = match read_one(&conn, uri)? {
            Some(old) => old.merged(analysis),
            None => *analysis,
        };
        conn.execute(
            &format!(
                "INSERT INTO track_analysis (uri, algo, {COLUMNS}, analyzed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT (uri) DO UPDATE SET
                    algo = excluded.algo, flags = excluded.flags, total_ms = excluded.total_ms,
                    lead_ms = excluded.lead_ms, tail_ms = excluded.tail_ms,
                    head_bpm = excluded.head_bpm, head_conf = excluded.head_conf, head_beat_ms = excluded.head_beat_ms,
                    tail_bpm = excluded.tail_bpm, tail_conf = excluded.tail_conf, tail_beat_ms = excluded.tail_beat_ms,
                    head_env = excluded.head_env, tail_env = excluded.tail_env, analyzed_at = excluded.analyzed_at"
            ),
            params![
                uri,
                ALGO,
                merged.flags as i64,
                merged.total_ms as i64,
                merged.lead_ms as i64,
                merged.tail_ms as i64,
                merged.head.bpm as f64,
                merged.head.conf as f64,
                merged.head.beat_ms as i64,
                merged.tail.bpm as f64,
                merged.tail.conf as f64,
                merged.tail.beat_ms as i64,
                env_blob(&merged.head_env),
                env_blob(&merged.tail_env),
                now_ms(),
            ],
        )?;
        if let Some(f) = merged.features {
            write_features(&conn, uri, f)?;
        }
        conn.execute(
            "DELETE FROM track_analysis WHERE uri IN
                (SELECT uri FROM track_analysis ORDER BY analyzed_at DESC LIMIT -1 OFFSET ?1)",
            [KEEP_MAX],
        )?;
        Ok(())
    }

    /// En son çözümlenen en çok `limit` şarkı (açılışta belleğe).
    pub fn load_analyses(&self, limit: usize) -> Result<Vec<(String, Analysis)>> {
        let conn = self.r();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT uri, {COLUMNS} FROM track_analysis WHERE algo = ?1 ORDER BY analyzed_at DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![ALGO, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, from_row(r, 1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(uri, mut analysis)| {
                analysis.features = read_features(&conn, &uri)?;
                Ok((uri, analysis))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        migrate,
        output::{HAS_HEAD, HAS_TAIL},
    };

    fn sample(flags: u8, seed: f32) -> Analysis {
        let mut a = Analysis::EMPTY;
        a.flags = flags;
        if flags & HAS_HEAD != 0 {
            a.lead_ms = 120;
            a.head = Edge {
                bpm: 120.0 + seed,
                conf: 0.5,
                beat_ms: 210,
            };
            a.head_env = [-14 + seed as i8; ENV_SECS];
        }
        if flags & HAS_TAIL != 0 {
            a.total_ms = 200_000;
            a.tail_ms = 1_500;
            a.tail = Edge {
                bpm: 128.0 + seed,
                conf: 0.7,
                beat_ms: 55,
            };
            a.tail_env = [-20 + seed as i8; ENV_SECS];
        }
        a
    }

    #[test]
    fn analysis_round_trips_through_the_table() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.analysis("t:a").unwrap(), None);
        let a = sample(HAS_HEAD | HAS_TAIL, 1.0);
        db.save_analysis("t:a", &a).unwrap();
        let back = db.analysis("t:a").unwrap().unwrap();
        assert_eq!(back.flags, a.flags);
        assert_eq!(
            (back.lead_ms, back.tail_ms, back.total_ms),
            (120, 1_500, 200_000)
        );
        assert!((back.head.bpm - 121.0).abs() < 1e-3 && (back.tail.conf - 0.7).abs() < 1e-3);
        assert_eq!((back.head.beat_ms, back.tail.beat_ms), (210, 55));
        assert_eq!(back.head_env, a.head_env);
        assert_eq!(back.tail_env, a.tail_env);
    }

    #[test]
    fn whole_track_features_round_trip_in_their_own_algorithm_version() {
        let db = Db::open_in_memory().unwrap();
        let mut a = sample(HAS_TAIL, 0.0);
        a.features = Some(TrackFeatures {
            bpm: 123.5,
            bpm_conf: 0.91,
            key: 9,
            minor: true,
            key_conf: 0.82,
            energy: 73,
        });
        db.save_analysis("t:features", &a).unwrap();
        assert_eq!(
            db.analysis("t:features").unwrap().unwrap().features,
            a.features
        );
        let conn = db.r();
        conn.execute(
            "UPDATE track_features SET algo = ?1 WHERE uri = ?2",
            params![FEATURES_ALGO + 1, "t:features"],
        )
        .unwrap();
        drop(conn);
        assert_eq!(db.analysis("t:features").unwrap().unwrap().features, None);
    }

    #[test]
    fn features_persist_without_an_automix_edge() {
        let db = Db::open_in_memory().unwrap();
        let features = TrackFeatures {
            bpm: 123.5,
            bpm_conf: 0.91,
            key: 9,
            minor: true,
            key_conf: 0.82,
            energy: 73,
        };
        db.save_analysis(
            "t:short",
            &Analysis {
                features: Some(features),
                ..Analysis::EMPTY
            },
        )
        .unwrap();
        assert_eq!(read_features(&db.r(), "t:short").unwrap(), Some(features));
        assert_eq!(db.analysis("t:short").unwrap(), None);
    }

    #[test]
    fn features_migration_keeps_existing_automix_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        track_analysis_step(&tx, 8).unwrap();
        tx.execute(
            "INSERT INTO track_analysis (uri, algo, flags, head_env, tail_env, analyzed_at)
             VALUES ('t:old', ?1, 1, x'', x'', 1)",
            [ALGO],
        )
        .unwrap();
        track_features_step(&tx, 13).unwrap();
        tx.commit().unwrap();
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM track_analysis WHERE uri='t:old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1);
        assert!(migrate::schema_columns(&conn).contains_key("track_features"));
    }

    #[test]
    fn a_later_head_only_play_keeps_the_known_tail_and_the_other_way_round() {
        let db = Db::open_in_memory().unwrap();
        db.save_analysis("t:a", &sample(HAS_TAIL, 0.0)).unwrap();
        db.save_analysis("t:a", &sample(HAS_HEAD, 3.0)).unwrap();
        let a = db.analysis("t:a").unwrap().unwrap();
        assert_eq!(a.flags, HAS_HEAD | HAS_TAIL);
        assert!(
            (a.tail.bpm - 128.0).abs() < 1e-3,
            "son uç eskisi: {}",
            a.tail.bpm
        );
        assert!(
            (a.head.bpm - 123.0).abs() < 1e-3,
            "baş uç yeni: {}",
            a.head.bpm
        );
        // Yeni son uç eskisinin yerine geçer.
        db.save_analysis("t:a", &sample(HAS_TAIL, 5.0)).unwrap();
        let a = db.analysis("t:a").unwrap().unwrap();
        assert!((a.tail.bpm - 133.0).abs() < 1e-3 && (a.head.bpm - 123.0).abs() < 1e-3);
        // Hiçbir ucu geçerli olmayan çözümleme yazılmaz.
        db.save_analysis("t:z", &Analysis::EMPTY).unwrap();
        assert_eq!(db.analysis("t:z").unwrap(), None);
    }

    #[test]
    fn newest_first_and_capped() {
        let db = Db::open_in_memory().unwrap();
        for i in 0..5 {
            db.save_analysis(&format!("t:{i}"), &sample(HAS_TAIL, i as f32))
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let all = db.load_analyses(3).unwrap();
        assert_eq!(
            all.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>(),
            ["t:4", "t:3", "t:2"]
        );
        // Tablo sınırı: KEEP_MAX'ten fazlası en eskiden atılır.
        {
            let conn = db.w();
            for i in 0..(KEEP_MAX + 5) {
                conn.execute(
                    "INSERT INTO track_analysis (uri, algo, flags, head_env, tail_env, analyzed_at) VALUES (?1, ?2, 2, x'', x'', ?3)",
                    params![format!("bulk:{i}"), ALGO, i],
                )
                .unwrap();
            }
        }
        db.save_analysis("t:new", &sample(HAS_TAIL, 0.0)).unwrap();
        let n: i64 = db
            .r()
            .query_row("SELECT COUNT(*) FROM track_analysis", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, KEEP_MAX);
        assert!(
            db.analysis("t:new").unwrap().is_some(),
            "en yeni satır atılmaz"
        );
        assert!(db.analysis("bulk:0").unwrap().is_none(), "en eski atıldı");
    }

    #[test]
    fn rows_from_another_algorithm_version_are_ignored() {
        let db = Db::open_in_memory().unwrap();
        db.save_analysis("t:a", &sample(HAS_TAIL, 0.0)).unwrap();
        db.w()
            .execute("UPDATE track_analysis SET algo = ?1", [ALGO + 1])
            .unwrap();
        assert_eq!(db.analysis("t:a").unwrap(), None);
        assert!(db.load_analyses(10).unwrap().is_empty());
        // Yeniden kaydedince satır güncel sürümle yazılır.
        db.save_analysis("t:a", &sample(HAS_HEAD, 0.0)).unwrap();
        assert!(db.analysis("t:a").unwrap().is_some());
    }

    #[test]
    fn schema_step_keeps_every_older_row() {
        // 8 → 9: yeni tablo gelir, var olan hiçbir tabloya dokunulmaz.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE hidden_songs (uri TEXT NOT NULL, context TEXT NOT NULL DEFAULT '', name TEXT NOT NULL DEFAULT '',
                                        artists TEXT NOT NULL DEFAULT '', hidden_at INTEGER NOT NULL, PRIMARY KEY (uri, context));
             INSERT INTO hidden_songs (uri, hidden_at) VALUES ('t:x', 5);
             PRAGMA user_version = 8;",
        )
        .unwrap();
        let steps = [migrate::Step {
            to: 9,
            name: "track analysis",
            up: track_analysis_step,
        }];
        let report = migrate::migrate(&mut conn, &steps).unwrap();
        assert_eq!((report.from, report.to), (8, 9));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM hidden_songs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM track_analysis", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "yeni tablo boş gelir");
        // Yeniden çalıştırmak zararsız.
        migrate::migrate(&mut conn, &steps).unwrap();
    }
}
