//! SQLite şema göçü çalıştırıcısı.
//!
//! Üç veritabanı (`library.db`, `pages.db`, `edits.db`) da aynı çalıştırıcıyı
//! kullanır. Sürüm `PRAGMA user_version`'da durur; her veritabanı kendi sıralı
//! adım listesini (`Step`) verir:
//!
//!   - Adımlar sürüm sırasıyla (`to` artan) uygulanır; her adım KENDİ
//!     işleminde (`BEGIN IMMEDIATE`) çalışır ve `user_version` aynı işlemde
//!     yükselir: adım yarıda kesilirse (hata, çökme, güç kesintisi) o adımın
//!     hiçbir izi kalmaz, veritabanı bir önceki sağlam sürümde durur.
//!   - Sürümü adımlardan yeni olan dosya (uygulama geri alınmış) AÇILMAZ ve
//!     hiçbir şeyine dokunulmaz (`MigrateError::TooNew`): yeni sürümün verisini
//!     eski kodla bozmak ya da silmek yerine bu veritabanı o oturumda yok sayılır.
//!   - Hiçbir adım kullanıcı verisini silmez. Yalnız yeniden indirilebilen veri
//!     (önbellek) silinebilir ve silen adım bunu açıkça söyler (bkz. `cache.rs`).
//!   - İlk adım (`baseline`) göçten önce sürümlenmemiş ya da sürümlenip eski
//!     şemayla kalmış dosyaları da devralır: `IF NOT EXISTS` ve sütun yoklaması
//!     ile kendiliğinden tekrarlanabilir olmalıdır.

use std::fmt;

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior};

/// Tek bir şema adımı.
pub struct Step {
    /// Adım uygulanınca `user_version` bu olur (adımlar arasında artan).
    pub to: i64,
    /// Günlük için kısa ad.
    pub name: &'static str,
    /// Adımın işi; ikinci değer adımdan ÖNCEKİ sürümdür (eski sürüme göre
    /// davranması gereken baseline adımı için). `user_version` çalıştırıcı
    /// tarafından yazılır, adım yazmaz.
    pub up: fn(&Transaction, i64) -> rusqlite::Result<()>,
}

/// Göçün özeti.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub from: i64,
    pub to: i64,
    /// Bu çağrıda uygulanan adımların adları.
    pub applied: Vec<&'static str>,
}

/// Dosyanın sürümü bu kodun bildiği en yeni sürümden büyük.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrateError {
    TooNew { found: i64, supported: i64 },
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooNew { found, supported } => write!(
                f,
                "database version {found} is newer than this app supports ({supported}); a newer Openify wrote it"
            ),
        }
    }
}

impl std::error::Error for MigrateError {}

pub fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
}

/// Bağlantıdaki veritabanını `steps`in sonuna kadar yükseltir.
pub fn migrate(conn: &mut Connection, steps: &[Step]) -> Result<Report> {
    debug_assert!(
        steps.windows(2).all(|w| w[0].to < w[1].to) && steps.first().is_none_or(|s| s.to > 0),
        "göç adımları sürüme göre artan olmalı"
    );
    let latest = steps.last().map_or(0, |s| s.to);
    let from = user_version(conn)?;
    if from > latest {
        return Err(MigrateError::TooNew { found: from, supported: latest }.into());
    }
    let mut applied = Vec::new();
    for step in steps.iter().filter(|s| s.to > from) {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .with_context(|| format!("göç \"{}\" başlatılamadı", step.name))?;
        // Kilit alındıktan sonra yeniden oku: aynı dosyayı başka bir süreç de
        // yükseltiyor olabilir (ilk adımı o yapmış olabilir).
        let now = user_version(&tx)?;
        if now >= step.to {
            continue;
        }
        (step.up)(&tx, now).with_context(|| format!("göç \"{}\" (→ {}) başarısız", step.name, step.to))?;
        tx.pragma_update(None, "user_version", step.to)?;
        tx.commit().with_context(|| format!("göç \"{}\" kaydedilemedi", step.name))?;
        applied.push(step.name);
    }
    if !applied.is_empty() {
        log::info!("veritabanı göçü: sürüm {from} → {latest} ({})", applied.join(", "));
    }
    Ok(Report { from, to: latest, applied })
}

/// Tablo → sütun adları (sırasıyla), `sqlite_master`'dan: göç testlerinde
/// "her yoldan aynı şemaya varılıyor mu" denetimi için.
#[cfg(test)]
pub fn schema_columns(conn: &Connection) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    tables
        .into_iter()
        .map(|t| {
            let mut stmt = conn.prepare(&format!("SELECT name FROM pragma_table_info('{t}') ORDER BY cid")).unwrap();
            let cols = stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
            (t, cols)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_a(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE a (x INTEGER); INSERT INTO a VALUES (1);")
    }
    fn add_b(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE b (y INTEGER); INSERT INTO a VALUES (2);")
    }
    fn add_column(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
        tx.execute_batch("ALTER TABLE a ADD COLUMN z TEXT;")
    }
    fn fail_after_write(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE half (q INTEGER); INSERT INTO a VALUES (99);")?;
        tx.execute_batch("THIS IS NOT SQL")
    }

    const STEPS: [Step; 3] = [
        Step { to: 1, name: "a", up: create_a },
        Step { to: 2, name: "b", up: add_b },
        Step { to: 4, name: "z", up: add_column },
    ];

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn fresh_database_runs_every_step_in_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        let report = migrate(&mut conn, &STEPS).unwrap();
        assert_eq!(report, Report { from: 0, to: 4, applied: vec!["a", "b", "z"] });
        assert_eq!(user_version(&conn).unwrap(), 4);
        assert_eq!(count(&conn, "a"), 2);
    }

    #[test]
    fn every_older_version_reaches_the_latest_and_keeps_its_rows() {
        // Sürüm numaraları aralıklı olabilir (3 yok): 0, 1, 2 hepsi 4'e varır.
        for (start, done) in [(0, 0), (1, 1), (2, 2)] {
            let mut conn = Connection::open_in_memory().unwrap();
            migrate(&mut conn, &STEPS[..done]).unwrap();
            assert_eq!(user_version(&conn).unwrap(), start);
            let report = migrate(&mut conn, &STEPS).unwrap();
            assert_eq!((report.from, report.to), (start, 4));
            assert_eq!(user_version(&conn).unwrap(), 4);
            // Önceki adımların yazdığı satırlar durur, yinelenmez.
            assert_eq!(count(&conn, "a"), 2, "başlangıç sürümü {start}");
        }
    }

    #[test]
    fn running_again_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &STEPS).unwrap();
        let again = migrate(&mut conn, &STEPS).unwrap();
        assert_eq!(again, Report { from: 4, to: 4, applied: vec![] });
        assert_eq!(count(&conn, "a"), 2);
    }

    #[test]
    fn a_failing_step_rolls_back_alone_and_keeps_earlier_steps() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps = [
            Step { to: 1, name: "a", up: create_a },
            Step { to: 2, name: "bozuk", up: fail_after_write },
        ];
        let err = migrate(&mut conn, &steps).unwrap_err();
        assert!(format!("{err:#}").contains("bozuk"), "{err:#}");
        // Birinci adım kalıcı, ikincinin yarım yazdığı hiçbir şey yok.
        assert_eq!(user_version(&conn).unwrap(), 1);
        assert_eq!(count(&conn, "a"), 1);
        let half: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'half'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(half, 0);
        // Düzeltilmiş adımla yeniden denenince kaldığı yerden sürer.
        let fixed = [Step { to: 1, name: "a", up: create_a }, Step { to: 2, name: "b", up: add_b }];
        migrate(&mut conn, &fixed).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 2);
    }

    #[test]
    fn a_newer_database_is_refused_and_left_untouched() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &STEPS).unwrap();
        conn.pragma_update(None, "user_version", 9).unwrap();
        let err = migrate(&mut conn, &STEPS).unwrap_err();
        assert_eq!(err.downcast_ref::<MigrateError>(), Some(&MigrateError::TooNew { found: 9, supported: 4 }));
        assert_eq!(user_version(&conn).unwrap(), 9);
        assert_eq!(count(&conn, "a"), 2);
    }

    #[test]
    fn a_step_that_another_process_already_ran_is_skipped() {
        // İki süreç aynı anda açarsa ikincisi kilidi alınca sürümü ilerlemiş görür.
        let dir = std::env::temp_dir().join(format!("openify-migrate-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        let mut first = Connection::open(&path).unwrap();
        let mut second = Connection::open(&path).unwrap();
        migrate(&mut first, &STEPS).unwrap();
        // `second` açıldığında dosya zaten güncel: hiçbir adım yeniden çalışmaz.
        let report = migrate(&mut second, &STEPS).unwrap();
        assert!(report.applied.is_empty());
        assert_eq!(count(&second, "a"), 2);
        drop((first, second));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
