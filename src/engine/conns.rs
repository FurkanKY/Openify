//! SQLite bağlantı düzeni: tek yazıcı + salt-okunur küçük bir okuma havuzu.
//!
//! `library.db` ve `pages.db` (bkz. `cache.rs`, `pages.rs`) kullanır. Eskiden
//! tek bağlantı tek kilidin arkasındaydı: kütüphane tazelemesi gibi uzun bir
//! yazma sürerken açılış, liste ve sayfa okumaları onu beklerdi. WAL kipinde
//! okuyucu yazıcıyı, yazıcı okuyucuyu beklemez; bu yüzden okumalar kendi
//! (salt-okunur) bağlantılarından geçer:
//!
//!   - `w()`: tek yazıcı; yazmalar sıralanır (SQLite zaten tek yazıcıya izin verir).
//!   - `r()`: okuma bağlantısı; boş olan ilki alınır, hepsi meşgulse sırası
//!     gelenin kilidinde beklenir. Yazıcıyı HİÇ beklemez.
//!
//! Dosyasız (bellek içi, yalnız testler) veritabanı bağlantılar arasında
//! paylaşılamaz: orada okumalar da yazıcıdan geçer.

use std::{
    path::Path,
    sync::{
        Mutex, MutexGuard, TryLockError,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

/// Okuma bağlantısı sayısı.
const READERS: usize = 2;
/// Okuma bağlantısının sayfa önbelleği (KiB): bağlantı başına bellek payı küçük kalsın.
const READER_CACHE_KIB: i64 = 1024;

pub struct Conns {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

fn open_reader(path: &Path) -> Result<Mutex<Connection>> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
    )
    .context("Okuma bağlantısı açılamadı")?;
    conn.pragma_update(None, "cache_size", -READER_CACHE_KIB)?;
    Ok(Mutex::new(conn))
}

impl Conns {
    /// `writer`: `path`teki dosyaya bağlı, şeması hazır (göçleri yapılmış) yazıcı.
    /// Okuma bağlantıları ondan SONRA açılır (WAL dosyaları ve şema hazır olsun);
    /// açılamazlarsa okumalar yazıcıdan geçer (yavaşlar, bozulmaz).
    pub fn file(path: &Path, writer: Connection) -> Self {
        let readers = (0..READERS).map(|_| open_reader(path)).collect::<Result<Vec<_>>>().unwrap_or_else(|e| {
            log::warn!("{}: okuma bağlantıları açılamadı, okumalar yazıcıdan geçecek: {e:#}", path.display());
            Vec::new()
        });
        Self { writer: Mutex::new(writer), readers, next_reader: AtomicUsize::new(0) }
    }

    /// Bellek içi veritabanı: tek bağlantı.
    #[cfg(test)]
    pub fn memory(writer: Connection) -> Self {
        Self { writer: Mutex::new(writer), readers: Vec::new(), next_reader: AtomicUsize::new(0) }
    }

    /// Yazıcı bağlantı (tek, sıralı).
    pub fn w(&self) -> MutexGuard<'_, Connection> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Okuma bağlantısı (dönüşümlü başlayarak ilk boş olan).
    pub fn r(&self) -> MutexGuard<'_, Connection> {
        let n = self.readers.len();
        if n == 0 {
            return self.w();
        }
        let start = self.next_reader.fetch_add(1, Ordering::Relaxed) % n;
        for i in 0..n {
            match self.readers[(start + i) % n].try_lock() {
                Ok(guard) => return guard,
                Err(TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(TryLockError::WouldBlock) => {}
            }
        }
        self.readers[start].lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(test)]
    pub fn reader_count(&self) -> usize {
        self.readers.len()
    }
}
