//! Disk önbelleği.
//!
//!   %LOCALAPPDATA%\Openify\
//!     library.db    kütüphane, liste içerikleri ve şarkı bilgileri (SQLite)
//!     covers\       kapak resimleri (en fazla 200 MB, en eski görülen silinir)
//!     audio\        librespot'un ses önbelleği (en fazla 2 GB, şifreli)
//!
//! Açılışta kütüphane buradan anında gösterilir, Spotify'dan arka planda
//! tazelenir. Önbellek silinirse hiçbir şey kaybolmaz; her şey yeniden indirilir.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::MutexGuard,
    time::SystemTime,
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{
    conns::Conns,
    library::{
        CollectionId, Entry, Folder, ItemKind, Library, LibraryItem, LibraryMeta, LibraryNode,
        TrackRow, library_nodes,
    },
    migrate::{self, Step},
    rules::Traits,
    traits,
};

pub const AUDIO_CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const COVER_CACHE_LIMIT: u64 = 200 * 1024 * 1024;

/// Önbellek klasörü (`library.db`, `pages.db`, `covers\`, `audio\`).
/// Sınama kancası: `OPENIFY_TEST_CACHE_DIR` verilirse hepsi orada durur
/// (sınama koşusu kullanıcının gerçek `%LOCALAPPDATA%\Openify` önbelleğine
/// dokunmaz; Ayarlar'ın boyut / temizleme sayfası da aynı klasöre bakar).
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OPENIFY_TEST_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = crate::crash::test_sandbox() {
        return Some(dir.join("cache"));
    }
    Some(dirs::cache_dir()?.join("Openify"))
}

pub fn audio_dir() -> Option<PathBuf> {
    Some(cache_dir()?.join("audio"))
}

// ── Şema ve göçler ──────────────────────────────────────────────────────
//
// Sürüm `PRAGMA user_version`'da; adımları `migrate.rs` sırayla, her biri kendi
// işleminde uygular. Eskiden şema sürümü değişince tablolar silinip baştan
// kurulurdu; artık yalnız aşağıdaki eski (1–2) sürümler için ve yalnız yeniden
// indirilebilen tablolar için öyle.
//
//   1–2  Eski biçimler (kapak adresi yok; 2'ye kadar listelerin adı / şarkı
//        sayısı kök listeden gelen çevrilmemiş / sıfır değerlerdi). Artık
//        kullanıcı kalmamış olması beklenir; `baseline` bunları eskisi gibi
//        yeniden indirilebilir tabloları düşürüp 3'ün şemasına getirir.
//   3    Listeler ve şarkılar için kapak resmi adresleri (şimdiye kadarki şema).
//   4    Klasör / sabitleme / eklenme tarihi için veri modeli:
//          library_items.kind, library_items.parent  (klasör ağacı),
//          collection_tracks.added_at                (ms; NULL = bilinmiyor),
//          library_pins                              (kullanıcının sabitlemeleri).
//        Hiçbir satır silinmez, mevcut satırlar varsayılanla (liste, kökte,
//        eklenme anı bilinmiyor) kalır; eklenme anı bir sonraki tazelemede
//        Spotify'dan gelir.
//   5    Takip edilen sanatçılar: `library_artists` (sıra = en son takip edilen
//        önde; yeniden indirilebilir, `library_items` gibi her tazelemede baştan
//        yazılır ama ondan ayrı tabloda, klasör ağacı yazıcısına karışmaz).
//        Yeni tablo; hiçbir mevcut satıra dokunulmaz.
//   6    Kütüphane öğelerinin eklenme anı ve yaratıcısı (kütüphane sıralaması):
//          library_items.added_at  (ms; NULL = bilinmiyor),
//          library_items.creator   (boş = bilinmiyor).
//        İki sütun eklenir, hiçbir satır silinmez; değerler bir sonraki
//        tazelemede Spotify'dan gelir (yeniden indirilebilir veri).
//   7    Yerel çalma geçmişi (kütüphane araçları, istatistik): `play_history`.
//        Yeni tablo; hiçbir mevcut satıra dokunulmaz. KULLANICI VERİSİ: yeniden
//        indirilemez, hiçbir göç düşürmez; yalnız bu bilgisayarda durur
//        (Spotify'a gitmez), gizli oturumda yazılmaz (bkz. `tools/stats.rs`).
//   8    Yerel olarak gizlenen şarkılar (bkz. `hidden.rs`): `hidden_songs`
//        (şarkı, bağlam; bağlam '' = hiçbir yerde çalma). Yeni tablo; hiçbir
//        mevcut satıra dokunulmaz. KULLANICI VERİSİ: yeniden indirilemez, hiçbir
//        göç düşürmez; yalnız bu bilgisayarda durur (Spotify'a gitmez).
//   9    AutoMix şarkı çözümlemeleri (bkz. `automix.rs`, `output/beat.rs`):
//        `track_analysis` (şarkı başına baş / son uç tempo, sessizlik, zarf).
//        Yeni tablo; hiçbir mevcut satıra dokunulmaz. Çalarak yeniden
//        hesaplanabilir (indirme gerekmez), hiçbir göç düşürmez.
//  10    Liste açıklaması ve ortak liste işareti (kök listenin
//        `ListAttributes`'ından): `library_list_info`. Yeni tablo; hiçbir
//        mevcut satıra dokunulmaz. Yeniden indirilebilir (her tazelemede kök
//        listeden gelir).
//
// Hangi tablo hangi tür veri:
//   yeniden indirilebilir  meta, library_items, library_list_info, collections,
//                          collection_tracks, tracks
//   yeniden indirilebilir ama pahalı (istek harcar) ve ASLA düşürülmez  lyrics
//   kullanıcı durumu, ASLA düşürülmez  playback (kaldığı yer), library_pins (sabitleme),
//                                      play_history (yerel çalma geçmişi), hidden_songs (gizlenen şarkılar)
const MIGRATIONS: [Step; 8] = [
    Step {
        to: 3,
        name: "baseline",
        up: baseline,
    },
    Step {
        to: 4,
        name: "folders, pins, added_at",
        up: library_model,
    },
    Step {
        to: 5,
        name: "followed artists",
        up: followed_artists,
    },
    Step {
        to: 6,
        name: "library item stamps",
        up: item_stamps,
    },
    Step {
        to: 7,
        name: "play history",
        up: play_history,
    },
    Step {
        to: 8,
        name: "hidden songs",
        up: super::hidden::hidden_songs_step,
    },
    Step {
        to: 9,
        name: "track analysis",
        up: super::automix::track_analysis_step,
    },
    Step {
        to: 10,
        name: "playlist description and collaborative",
        up: list_info,
    },
];

/// Sürüm 3'ün şeması (`IF NOT EXISTS`: göçten önce sürümlenmiş dosyaları da devralır).
const SCHEMA_V3: &str = "
    CREATE TABLE IF NOT EXISTS meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    -- Sidebar albums and playlists, in order.
    CREATE TABLE IF NOT EXISTS library_items (
        position INTEGER PRIMARY KEY,
        id       TEXT NOT NULL,
        name     TEXT NOT NULL,
        subtitle TEXT NOT NULL,
        image    TEXT NOT NULL DEFAULT ''
    );
    -- A list's track order and its revision on Spotify.
    CREATE TABLE IF NOT EXISTS collections (
        id       TEXT PRIMARY KEY,
        revision BLOB
    );
    CREATE TABLE IF NOT EXISTS collection_tracks (
        collection TEXT NOT NULL,
        position   INTEGER NOT NULL,
        track_uri  TEXT NOT NULL,
        PRIMARY KEY (collection, position)
    );
    CREATE TABLE IF NOT EXISTS tracks (
        uri         TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        artists     TEXT NOT NULL,
        album       TEXT NOT NULL,
        duration_ms INTEGER NOT NULL,
        image       TEXT NOT NULL DEFAULT ''
    );
    -- Resume state (see resume.rs): 'state' small record, 'context' big record.
    CREATE TABLE IF NOT EXISTS playback (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    -- Lyrics (see lyrics.rs): raw response ('body') or a 'none' answer
    -- (body NULL; 'expires_at' unix s, asked again once it passes).
    -- Re-fetching costs requests: no migration ever drops this table.
    CREATE TABLE IF NOT EXISTS lyrics (
        uri        TEXT NOT NULL,
        source     TEXT NOT NULL,
        body       TEXT,
        expires_at INTEGER,
        saved_at   INTEGER NOT NULL,
        PRIMARY KEY (uri, source)
    );";

/// 0 → 3. Yeni (0) ya da göçten önce sürümlenmiş dosyayı 3'ün şemasına getirir.
///
/// 1–2 sürümlerinde şarkı / liste tabloları başka biçimdeydi: bunlar
/// YENİDEN İNDİRİLEBİLİR veridir (Spotify'dan tazelenir), düşürülür; `playback`
/// ve `lyrics` hiçbir sürümde düşürülmez.
fn baseline(tx: &Transaction, from: i64) -> rusqlite::Result<()> {
    if (1..3).contains(&from) {
        tx.execute_batch(
            "DROP TABLE IF EXISTS library_items;
             DROP TABLE IF EXISTS collections;
             DROP TABLE IF EXISTS collection_tracks;
             DROP TABLE IF EXISTS tracks;
             DROP TABLE IF EXISTS meta;",
        )?;
    }
    tx.execute_batch(SCHEMA_V3)
}

/// 3 → 4: klasör / sabitleme / eklenme tarihi modeli (veri kaybı yok).
fn library_model(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- 'playlist' (an openable list or album) | 'folder'.
         ALTER TABLE library_items ADD COLUMN kind TEXT NOT NULL DEFAULT 'playlist';
         -- Key of the containing folder (a library_items.id); NULL = at the root.
         ALTER TABLE library_items ADD COLUMN parent TEXT;
         -- When the track was added to the list, unix MILLISECONDS; NULL = unknown.
         ALTER TABLE collection_tracks ADD COLUMN added_at INTEGER;
         -- Pinned items (list / album / folder key). library_items is rewritten
         -- on every library refresh; this table is independent of it, so pins
         -- survive refreshes.
         CREATE TABLE IF NOT EXISTS library_pins (
             id        TEXT PRIMARY KEY,
             pinned_at INTEGER NOT NULL
         );",
    )
}

/// 4 → 5: takip edilen sanatçılar (yeni tablo, veri kaybı yok).
fn followed_artists(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- Followed artists, newest follow first (position). Re-fetchable.
         CREATE TABLE IF NOT EXISTS library_artists (
             position INTEGER PRIMARY KEY,
             uri      TEXT NOT NULL,
             name     TEXT NOT NULL,
             image    TEXT NOT NULL DEFAULT ''
         );",
    )
}

/// 5 → 6: kütüphane öğelerinin eklenme anı ve yaratıcısı (veri kaybı yok).
fn item_stamps(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- When the item was added to the library, unix MILLISECONDS; NULL = unknown.
         ALTER TABLE library_items ADD COLUMN added_at INTEGER;
         -- Creator (owner user name for playlists); empty = unknown.
         ALTER TABLE library_items ADD COLUMN creator TEXT NOT NULL DEFAULT '';",
    )
}

/// 6 → 7: yerel çalma geçmişi (yeni tablo, veri kaybı yok).
fn play_history(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- One row per counted play (>= 30 s or half the track), unix MILLISECONDS.
         -- Name / artists / album are copied in: the history outlives cache refreshes.
         CREATE TABLE IF NOT EXISTS play_history (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             played_at   INTEGER NOT NULL,
             uri         TEXT NOT NULL,
             name        TEXT NOT NULL,
             artists     TEXT NOT NULL,
             album       TEXT NOT NULL,
             duration_ms INTEGER NOT NULL,
             listened_ms INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS play_history_at ON play_history (played_at);",
    )
}

/// 9 → 10: liste açıklaması ve ortak liste işareti (yeni tablo, veri kaybı yok).
fn list_info(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- Playlist description and collaborative flag, from the root list.
         -- Rewritten on every library refresh (re-fetchable).
         CREATE TABLE IF NOT EXISTS library_list_info (
             id            TEXT PRIMARY KEY,
             description   TEXT NOT NULL DEFAULT '',
             collaborative INTEGER NOT NULL DEFAULT 0
         );",
    )
}

/// Şarkı sözü tablosunda tutulacak en çok satır (ham yanıt ~5–20 KB).
const LYRICS_KEEP: i64 = 4000;

/// `tracks` sorgusunda bir seferde sorulan adres sayısı.
const TRACK_QUERY_CHUNK: usize = 500;

/// `lyrics` tablosundaki bir satır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsRow {
    /// Ham yanıt; `None` = "bu kaynakta yok".
    pub body: Option<String>,
    /// Unix saniye; `None` = süresiz (bulunan sözler).
    pub expires_at: Option<i64>,
}

/// Kütüphane önbelleği (`library.db`).
///
/// Bağlantılar ayrı: tek YAZICI (WAL) ve salt-okunur küçük bir OKUMA havuzu
/// (bkz. `conns.rs`): büyük bir yazma (kütüphane tazelemesi, şarkı bilgisi
/// doldurma) sürerken açılış / liste / arama okumaları gecikmez.
pub struct Db {
    conns: Conns,
}

fn init_writer(mut conn: Connection) -> Result<Connection> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    migrate::migrate(&mut conn, &MIGRATIONS).context("Önbellek şeması yükseltilemedi")?;
    Ok(conn)
}

impl Db {
    pub fn open_default() -> Result<Self> {
        let dir = cache_dir().context("Önbellek klasörü bulunamadı")?;
        fs::create_dir_all(&dir)?;
        Self::open(&dir.join("library.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        // Önce yazıcı: şemayı kurar / yükseltir, WAL dosyalarını oluşturur;
        // okuma bağlantıları ondan sonra açılır.
        let writer = init_writer(Connection::open(path).context("Önbellek veritabanı açılamadı")?)?;
        Ok(Self {
            conns: Conns::file(path, writer),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            conns: Conns::memory(init_writer(Connection::open_in_memory()?)?),
        })
    }

    /// Yazıcı bağlantı (tek, sıralı).
    pub(super) fn w(&self) -> MutexGuard<'_, Connection> {
        self.conns.w()
    }

    /// Okuma bağlantısı; yazıcıyı beklemez.
    pub(super) fn r(&self) -> MutexGuard<'_, Connection> {
        self.conns.r()
    }

    // ── Kütüphane ───────────────────────────────────────────────────────

    pub fn load_library(&self) -> Result<Option<Library>> {
        let conn = self.r();
        let liked_count: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'liked_count'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(liked_count) = liked_count else {
            return Ok(None);
        };

        let mut library = Library {
            liked_count: liked_count.parse().unwrap_or(0),
            ..Default::default()
        };
        // Düz listeler (albümler, listeler) klasör satırlarını atlar; klasörler
        // `meta`ya gider: kökteki sırasıyla, `at` = kendisinden önceki liste sayısı.
        let mut stmt = conn.prepare(
            "SELECT id, kind, parent, name, subtitle, image, added_at, creator FROM library_items ORDER BY position",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, kind, parent, name, subtitle, image, added_at, creator) in rows {
            if ItemKind::from_tag(&kind) == ItemKind::Folder {
                let at = library.playlists.len();
                library.meta.folders.push(Folder {
                    key,
                    name,
                    parent,
                    at,
                });
                continue;
            }
            let Some(id) = CollectionId::from_key(&key) else {
                continue;
            };
            if let Some(parent) = parent {
                library.meta.parents.insert(key.clone(), parent);
            }
            if let Some(added_at) = added_at {
                library.meta.added.insert(key.clone(), added_at);
            }
            if !creator.is_empty() {
                library.meta.creators.insert(key, creator);
            }
            let item = LibraryItem {
                id,
                name,
                subtitle,
                image,
            };
            match item.id {
                CollectionId::Album(_) => library.albums.push(item),
                _ => library.playlists.push(item),
            }
        }
        library.artists = Self::read_artists(&conn)?;
        library.meta.pinned = Self::read_pins(&conn)?;
        Self::read_list_info(&conn, &mut library.meta)?;
        Ok(Some(library))
    }

    /// Liste açıklamaları ve ortak liste işaretleri (`library_list_info`).
    fn read_list_info(conn: &Connection, meta: &mut LibraryMeta) -> Result<()> {
        let mut stmt =
            conn.prepare("SELECT id, description, collaborative FROM library_list_info")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)? != 0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, description, collaborative) in rows {
            if !description.is_empty() {
                meta.descriptions.insert(id.clone(), description);
            }
            if collaborative {
                meta.collaborative.insert(id);
            }
        }
        Ok(())
    }

    /// `read_list_info`in yazıcısı: tabloyu baştan yazar.
    fn save_list_info(tx: &Transaction, meta: &LibraryMeta) -> Result<()> {
        tx.execute("DELETE FROM library_list_info", [])?;
        let mut insert = tx.prepare(
            "INSERT INTO library_list_info (id, description, collaborative) VALUES (?1, ?2, ?3)",
        )?;
        for (id, description) in &meta.descriptions {
            insert.execute(params![
                id,
                description,
                meta.collaborative.contains(id) as i64
            ])?;
        }
        for id in meta.collaborative.iter() {
            if !meta.descriptions.contains_key(id) {
                insert.execute(params![id, "", 1])?;
            }
        }
        Ok(())
    }

    fn read_pins(conn: &Connection) -> Result<Vec<String>> {
        let mut stmt = conn.prepare("SELECT id FROM library_pins ORDER BY pinned_at, id")?;
        let keys = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(keys)
    }

    /// Takip edilen sanatçılar, en son takip edilen önde.
    fn read_artists(conn: &Connection) -> Result<Vec<LibraryItem>> {
        let mut stmt =
            conn.prepare("SELECT uri, name, image FROM library_artists ORDER BY position")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(uri, name, image)| super::library::artist_item(&uri, &name, &image))
            .collect())
    }

    /// Takip edilen sanatçıları baştan yazar (sıra = verilen sıra).
    fn save_artists(tx: &Transaction, artists: &[LibraryItem]) -> Result<()> {
        tx.execute("DELETE FROM library_artists", [])?;
        let mut insert = tx.prepare(
            "INSERT INTO library_artists (position, uri, name, image) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (i, a) in artists.iter().enumerate() {
            insert.execute(params![i as i64, a.id.key(), a.name, a.image])?;
        }
        Ok(())
    }

    /// Kütüphaneyi yazar: albümler, listeler ve (`library.meta`da varsa)
    /// klasörler, parent / eklenme anı / yaratıcı ile (`library_nodes`).
    /// Sabitlemelere (`library_pins`) dokunmaz: onlar `meta.pinned`dan değil
    /// `set_pinned`dan yazılır.
    pub fn save_library(&self, library: &Library) -> Result<()> {
        let nodes = library_nodes(library);
        self.save_library_nodes(&nodes, library.liked_count)?;
        // Takip edilen sanatçılar ayrı tabloda: klasör ağacı yazıcısı onlara dokunmaz.
        let mut conn = self.w();
        let tx = conn.transaction()?;
        Self::save_artists(&tx, &library.artists)?;
        Self::save_list_info(&tx, &library.meta)?;
        tx.commit()?;
        Ok(())
    }

    /// Kenar çubuğu ağacı: klasörler ve listeler, kenar çubuğundaki sırayla.
    #[allow(dead_code)] // yalnız testlerde kullanılıyor (arayüz `load_library`nin `meta`sını okur)
    pub fn load_library_nodes(&self) -> Result<Vec<LibraryNode>> {
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT id, kind, parent, name, subtitle, image, added_at, creator FROM library_items ORDER BY position",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(LibraryNode {
                    key: r.get(0)?,
                    kind: ItemKind::from_tag(&r.get::<_, String>(1)?),
                    parent: r.get(2)?,
                    name: r.get(3)?,
                    subtitle: r.get(4)?,
                    image: r.get(5)?,
                    added_at: r.get(6)?,
                    creator: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Ağacı baştan yazar (sıra = verilen sıra) ve beğenilen sayısını günceller.
    /// Sabitlemeler (`library_pins`) ayrı tabloda: bu yazma onlara dokunmaz.
    pub fn save_library_nodes(&self, nodes: &[LibraryNode], liked_count: usize) -> Result<()> {
        let mut conn = self.w();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM library_items", [])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO library_items (position, id, name, subtitle, image, kind, parent, added_at, creator)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for (i, n) in nodes.iter().enumerate() {
                insert.execute(params![
                    i as i64,
                    n.key,
                    n.name,
                    n.subtitle,
                    n.image,
                    n.kind.tag(),
                    n.parent,
                    n.added_at,
                    n.creator
                ])?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('liked_count', ?1)",
            [liked_count.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Bir liste / albüm / klasörü sabitle ya da sabitlemeyi kaldır. Zaten
    /// sabitliyse ilk sabitlenme anı korunur. `now_ms`: unix milisaniye.
    pub fn set_pinned(&self, key: &str, pinned: bool, now_ms: i64) -> Result<()> {
        let conn = self.w();
        if pinned {
            conn.execute(
                "INSERT OR IGNORE INTO library_pins (id, pinned_at) VALUES (?1, ?2)",
                params![key, now_ms],
            )?;
        } else {
            conn.execute("DELETE FROM library_pins WHERE id = ?1", [key])?;
        }
        Ok(())
    }

    /// Sabitlenenlerin anahtarları, sabitlenme sırasıyla (en eski önce).
    pub fn pinned_keys(&self) -> Result<Vec<String>> {
        Self::read_pins(&self.r())
    }

    /// Yeni bir sabitleme için zaman damgası: `now_ms`, ama mevcut en yeni
    /// sabitlemeden kesinlikle sonra (aynı milisaniyedeki iki sabitleme
    /// sırasını korusun).
    pub fn next_pin_time(&self, now_ms: i64) -> Result<i64> {
        let latest: Option<i64> =
            self.w()
                .query_row("SELECT MAX(pinned_at) FROM library_pins", [], |r| r.get(0))?;
        Ok(latest.map_or(now_ms, |l| now_ms.max(l + 1)))
    }

    // ── Liste içerikleri ────────────────────────────────────────────────

    pub fn revision(&self, id: &CollectionId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .r()
            .query_row(
                "SELECT revision FROM collections WHERE id = ?1",
                [id.key()],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Listenin şarkı sırası; hiç kaydedilmediyse `None`.
    pub fn collection_uris(&self, id: &CollectionId) -> Result<Option<Vec<String>>> {
        let conn = self.r();
        let known: Option<i64> = conn
            .query_row("SELECT 1 FROM collections WHERE id = ?1", [id.key()], |r| {
                r.get(0)
            })
            .optional()?;
        if known.is_none() {
            return Ok(None);
        }
        let mut stmt = conn.prepare(
            "SELECT track_uri FROM collection_tracks WHERE collection = ?1 ORDER BY position",
        )?;
        let uris = stmt
            .query_map([id.key()], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(Some(uris))
    }

    /// Listenin satırları (şarkı + eklenme anı), sırayla; hiç kaydedilmediyse `None`.
    pub fn collection_entries(&self, id: &CollectionId) -> Result<Option<Vec<Entry>>> {
        let conn = self.r();
        let known: Option<i64> = conn
            .query_row("SELECT 1 FROM collections WHERE id = ?1", [id.key()], |r| {
                r.get(0)
            })
            .optional()?;
        if known.is_none() {
            return Ok(None);
        }
        let mut stmt = conn.prepare(
            "SELECT track_uri, added_at FROM collection_tracks WHERE collection = ?1 ORDER BY position",
        )?;
        let entries = stmt
            .query_map([id.key()], |r| {
                Ok(Entry {
                    track: r.get(0)?,
                    added_at: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<Entry>>>()?;
        Ok(Some(entries))
    }

    /// Listenin ilk şarkılarının farklı albüm kapakları (en çok `n`, liste
    /// sırasıyla): kapağı olmayan listede 2x2 mozaik için. Yalnız önbellekten
    /// (listenin şarkıları hiç kaydedilmediyse boş).
    pub fn cover_samples(&self, id: &CollectionId, n: usize) -> Result<Vec<String>> {
        let conn = self.r();
        let mut stmt = conn.prepare_cached(
            "SELECT t.image FROM collection_tracks c JOIN tracks t ON t.uri = c.track_uri
             WHERE c.collection = ?1 AND t.image != ''
             GROUP BY t.image ORDER BY MIN(c.position) LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![id.key(), n as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    /// Listenin şarkı sırasını yazar; eklenme anı bilinmez (`NULL`), ama bu
    /// listede zaten kayıtlı bir şarkının eklenme anı korunur (düzenleme
    /// yamaları listeyi bu yoldan baştan yazar; eldeki tarihler kaybolmasın).
    pub fn save_collection(
        &self,
        id: &CollectionId,
        revision: Option<&[u8]>,
        uris: &[String],
    ) -> Result<()> {
        let entries: Vec<Entry> = uris.iter().map(Entry::untimed).collect();
        self.save_collection_entries(id, revision, &entries)
    }

    /// Listenin satırlarını (şarkı + eklenme anı) yazar. `added_at: None` olan
    /// satır, aynı listede aynı şarkı için önceden kayıtlı bir eklenme anı
    /// varsa onu korur; verilen değer her zaman önceliklidir.
    pub fn save_collection_entries(
        &self,
        id: &CollectionId,
        revision: Option<&[u8]>,
        entries: &[Entry],
    ) -> Result<()> {
        let mut conn = self.w();
        let tx = conn.transaction()?;
        let mut known: HashMap<String, i64> = HashMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT track_uri, added_at FROM collection_tracks
                 WHERE collection = ?1 AND added_at IS NOT NULL ORDER BY position",
            )?;
            for row in stmt.query_map([id.key()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })? {
                let (uri, at) = row?;
                known.entry(uri).or_insert(at);
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO collections (id, revision) VALUES (?1, ?2)",
            params![id.key(), revision],
        )?;
        tx.execute(
            "DELETE FROM collection_tracks WHERE collection = ?1",
            [id.key()],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO collection_tracks (collection, position, track_uri, added_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (i, entry) in entries.iter().enumerate() {
                let added_at = entry.added_at.or_else(|| known.get(&entry.track).copied());
                insert.execute(params![id.key(), i as i64, entry.track, added_at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ── Şarkı bilgileri ─────────────────────────────────────────────────

    pub fn tracks(&self, uris: &[String]) -> Result<HashMap<String, TrackRow>> {
        let conn = self.r();
        let mut found = HashMap::with_capacity(uris.len());
        // Tek tek (binlerce kez) sormak yerine parça parça `IN (...)`.
        for chunk in uris.chunks(TRACK_QUERY_CHUNK) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT uri, name, artists, album, duration_ms, image FROM tracks WHERE uri IN ({marks})"
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk), |r| {
                Ok(TrackRow {
                    uri: r.get(0)?,
                    name: r.get(1)?,
                    artists: r.get(2)?,
                    album: r.get(3)?,
                    duration_ms: r.get(4)?,
                    image: r.get(5)?,
                    plays: None,
                })
            })?;
            for row in rows {
                let row = row?;
                found.insert(row.uri.clone(), row);
            }
        }
        Ok(found)
    }

    pub fn save_tracks(&self, rows: &[TrackRow]) -> Result<()> {
        let mut conn = self.w();
        let tx = conn.transaction()?;
        {
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO tracks (uri, name, artists, album, duration_ms, image)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for t in rows {
                insert.execute(params![
                    t.uri,
                    t.name,
                    t.artists,
                    t.album,
                    t.duration_ms,
                    t.image
                ])?;
            }
        }
        tx.commit()?;
        drop(conn);
        // Bu şarkılarla birlikte gelen özellikler (açık söz, popülerlik; bkz. `traits.rs`).
        let noted = traits::take(rows.iter().map(|t| t.uri.as_str()));
        if !noted.is_empty()
            && let Err(e) = self.save_track_traits(&noted)
        {
            log::warn!("Şarkı özellikleri yazılamadı: {e:#}");
        }
        Ok(())
    }

    // ── Şarkı özellikleri (açık söz, popülerlik) ────────────────────────
    //
    // Şema sürümüne bağlı değil: tablo ilk yazılışta `IF NOT EXISTS` ile
    // kurulur; yoksa okuma "bilinmiyor" döner. Yeniden indirilebilir veri.

    /// `uris` için bilinen özellikler (kaydı olmayanlar sonuçta yok).
    pub fn track_traits(&self, uris: &[String]) -> Result<HashMap<String, Traits>> {
        let conn = self.r();
        let mut found = HashMap::with_capacity(uris.len());
        for chunk in uris.chunks(TRACK_QUERY_CHUNK) {
            let marks = vec!["?"; chunk.len()].join(",");
            let query = format!(
                "SELECT uri, explicit, popularity FROM track_traits WHERE uri IN ({marks})"
            );
            let mut stmt = match conn.prepare_cached(&query) {
                Ok(stmt) => stmt,
                // Tablo henüz kurulmadı: hiçbir özellik bilinmiyor.
                Err(e) if e.to_string().contains("no such table") => return Ok(found),
                Err(e) => return Err(e.into()),
            };
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Traits {
                        explicit: r.get::<_, i64>(1)? != 0,
                        popularity: r.get::<_, i64>(2)?.clamp(0, 100) as u8,
                    },
                ))
            })?;
            for row in rows {
                let (uri, traits) = row?;
                found.insert(uri, traits);
            }
        }
        Ok(found)
    }

    pub fn save_track_traits(&self, rows: &[(String, Traits)]) -> Result<()> {
        let mut conn = self.w();
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS track_traits (
                 uri        TEXT PRIMARY KEY,
                 explicit   INTEGER NOT NULL,
                 popularity INTEGER NOT NULL
             );",
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO track_traits (uri, explicit, popularity) VALUES (?1, ?2, ?3)",
            )?;
            for (uri, t) in rows {
                insert.execute(params![uri, t.explicit as i64, t.popularity as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Kütüphanedeki bütün listelerde geçen, bilgisi önbellekte olan
    /// şarkılar (yerel arama dizini için; her şarkı bir kez).
    pub fn all_library_tracks(&self) -> Result<Vec<TrackRow>> {
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT t.uri, t.name, t.artists, t.album, t.duration_ms, t.image FROM tracks t
             WHERE t.uri IN (SELECT track_uri FROM collection_tracks)",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TrackRow {
                    uri: r.get(0)?,
                    name: r.get(1)?,
                    artists: r.get(2)?,
                    album: r.get(3)?,
                    duration_ms: r.get(4)?,
                    image: r.get(5)?,
                    plays: None,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .r()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.w().execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }

    // ── Kaldığı yerden sürme ────────────────────────────────────────────

    /// Çalma durumunu yazar. `context` yoksa önceki büyük kayıt korunur.
    /// Tek işlemde: ikisi hiçbir zaman yarım yazılmaz.
    pub fn save_playback(&self, state: &str, context: Option<&str>) -> Result<()> {
        let mut conn = self.w();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO playback (key, value) VALUES ('state', ?1)",
            [state],
        )?;
        if let Some(context) = context {
            tx.execute(
                "INSERT OR REPLACE INTO playback (key, value) VALUES ('context', ?1)",
                [context],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Kayıtlı çalma durumu (küçük kayıt, büyük kayıt); hiç yazılmadıysa `None`.
    pub fn load_playback(&self) -> Result<Option<(String, Option<String>)>> {
        let conn = self.r();
        let get = |key: &str| -> Result<Option<String>> {
            Ok(conn
                .query_row("SELECT value FROM playback WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?)
        };
        let Some(state) = get("state")? else {
            return Ok(None);
        };
        Ok(Some((state, get("context")?)))
    }

    // ── Şarkı sözleri ───────────────────────────────────────────────────

    /// Bir şarkının bir kaynaktan (`spotify` / `lrclib`) kayıtlı cevabı.
    /// Süresi dolmuş 'yok' kayıtları da döner; karar çağıranın (`now`).
    pub fn lyrics_row(&self, uri: &str, source: &str) -> Result<Option<LyricsRow>> {
        Ok(self
            .r()
            .query_row(
                "SELECT body, expires_at FROM lyrics WHERE uri = ?1 AND source = ?2",
                [uri, source],
                |r| {
                    Ok(LyricsRow {
                        body: r.get(0)?,
                        expires_at: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Cevabı yazar (`body: None` = bu kaynakta söz yok, `expires_at`e kadar).
    /// Tablo en çok `LYRICS_KEEP` satır tutar, en eskiler silinir.
    pub fn save_lyrics(
        &self,
        uri: &str,
        source: &str,
        body: Option<&str>,
        expires_at: Option<i64>,
        now: i64,
    ) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT OR REPLACE INTO lyrics (uri, source, body, expires_at, saved_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uri, source, body, expires_at, now],
        )?;
        conn.execute(
            "DELETE FROM lyrics WHERE rowid IN
                 (SELECT rowid FROM lyrics ORDER BY saved_at DESC, rowid DESC LIMIT -1 OFFSET ?1)",
            [LYRICS_KEEP],
        )?;
        Ok(())
    }

    /// Listelerde geçen ama bilgisi henüz indirilmemiş şarkılar.
    pub fn missing_tracks(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT ct.track_uri FROM collection_tracks ct
             LEFT JOIN tracks t ON t.uri = ct.track_uri
             WHERE t.uri IS NULL AND ct.track_uri LIKE 'spotify:track:%'
             LIMIT ?1",
        )?;
        let uris = stmt
            .query_map([limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(uris)
    }
}

// ── Kapak önbelleği ─────────────────────────────────────────────────────

fn cover_dir() -> Option<PathBuf> {
    Some(cache_dir()?.join("covers"))
}

/// Diskteki bir kapağın en uzun kenarı en çok bu kadar olur. Kapaklar en
/// büyük yerinde (sayfa başlığı) 128 mantıksal px, 2x ekranda 256 px
/// gösterilir; gpui resmi dosyanın tam çözünürlüğünde çözüp bellekte hiç
/// bırakmadığı için fazlası yalnız bellek yer. Çalan şarkının kapağı çubukta
/// 56 px gösterilir, 256 ona da yeter.
const COVER_MAX_SIDE: u32 = 256;
const COVER_JPEG_QUALITY: u8 = 88;

/// Resim adresinin hash'i (FNV-1a). Spotify'ın adresleri çok çeşitli
/// (`.../image/<id>`, `.../en`, `evening_default.jpg`); dosya adı adresin
/// tamamından türetiliyor.
fn cover_hash(url: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Resim adresinin diskteki yeri: `covers\<adresin FNV-1a özeti>.c2`.
/// (`.c2`: küçültülmüş kapak; eski sürümün tam boy `.img` dosyaları için
/// bkz. `migrate_legacy_cover`.)
pub fn cover_path(url: &str) -> Option<PathBuf> {
    if url.is_empty() {
        return None;
    }
    Some(cover_dir()?.join(format!("{:016x}.c2", cover_hash(url))))
}

/// Eski sürümün tam çözünürlüklü kapak dosyası (`<özet>.img`).
fn legacy_cover_path(url: &str) -> Option<PathBuf> {
    if url.is_empty() {
        return None;
    }
    Some(cover_dir()?.join(format!("{:016x}.img", cover_hash(url))))
}

/// Kapağı diske yazılacak boyuta küçültür: en uzun kenar `COVER_MAX_SIDE`'ı
/// aşıyorsa en-boy oranı korunarak (Lanczos3) küçültülür ve JPEG olarak
/// yeniden kodlanır. Zaten küçükse (ör. 64 px'lik şarkı küçük resimleri) ya
/// da çözülemiyorsa baytlar olduğu gibi döner. Çözme ve küçültme birkaç ms
/// sürer: arayüz iş parçacığında çağrılmamalı.
pub fn shrink_cover(bytes: &[u8]) -> Vec<u8> {
    let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()
    else {
        return bytes.to_vec();
    };
    // Önce yalnız başlıktan boyut: küçük resim hiç çözülmez.
    match reader.into_dimensions() {
        Ok((w, h)) if w.max(h) > COVER_MAX_SIDE => {}
        _ => return bytes.to_vec(),
    }
    let Ok(image) = image::load_from_memory(bytes) else {
        return bytes.to_vec();
    };
    let small = image
        .resize(
            COVER_MAX_SIDE,
            COVER_MAX_SIDE,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgb8();
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, COVER_JPEG_QUALITY);
    match small.write_with_encoder(encoder) {
        Ok(()) => out,
        Err(_) => bytes.to_vec(),
    }
}

/// Eski sürümden kalma tam boy kapağı (`.img`) küçültüp yeni yerine (`.c2`)
/// yazar, eskisini siler. Ağa çıkmadan eski önbelleği yeniden kullanır; yalnız
/// gerçekten gösterilen kapaklar (istenince) dönüştürülür. Yeni dosya
/// yazıldıysa baytları döner.
fn migrate_legacy_cover(url: &str) -> Option<Vec<u8>> {
    let legacy = legacy_cover_path(url)?;
    let bytes = fs::read(&legacy).ok()?;
    let small = shrink_cover(&bytes);
    write_cover(url, &small);
    // Yeni dosya yazılamadıysa eskisini silme.
    if cover_path(url).is_some_and(|p| p.exists()) {
        let _ = fs::remove_file(&legacy);
        Some(small)
    } else {
        None
    }
}

/// Kapak diskte (küçültülmüş halde) var mı? Yoksa ve eski sürümün tam boy
/// dosyası varsa onu dönüştürür. Engellemeli (dosya okur): arka planda çağır.
pub fn ensure_cover(url: &str) -> bool {
    cover_path(url).is_some_and(|p| p.exists()) || migrate_legacy_cover(url).is_some()
}

pub fn read_cover(url: &str) -> Option<Vec<u8>> {
    let path = cover_path(url)?;
    let bytes = fs::read(&path).ok().or_else(|| migrate_legacy_cover(url))?;
    // "Son görülme" zamanını güncelle: temizlikte en eski görülenler silinir.
    if let Ok(file) = fs::File::options().write(true).open(&path) {
        let _ = file.set_modified(SystemTime::now());
    }
    Some(bytes)
}

pub fn write_cover(url: &str, bytes: &[u8]) {
    let Some(path) = cover_path(url) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// İndirilen kapağı küçültüp diske yazar; yazılan (küçültülmüş) baytları
/// döner. Engellemeli: arka planda çağır.
pub fn store_cover(url: &str, bytes: &[u8]) -> Vec<u8> {
    let small = shrink_cover(bytes);
    write_cover(url, &small);
    small
}

/// Kapak klasörü sınırı aşıyorsa en uzun süredir görülmeyenleri siler.
pub fn prune_covers() {
    let Some(dir) = cover_dir() else { return };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= COVER_CACHE_LIMIT {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, size, path) in files {
        if total <= COVER_CACHE_LIMIT {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

// ── Önbellek boyutu ve temizleme (Ayarlar → Depolama) ───────────────────

/// Temizlenince silinen alt klasörler: şarkı sesleri ve kapaklar, ikisi de
/// yeniden indirilir. `library.db` (bekleyen yazmalar, kaldığı yer, sözler)
/// ve giriş bilgisi asla silinmez.
pub const CLEARABLE: [&str; 2] = ["audio", "covers"];

/// Ayarlar sayfasının gösterdiği / temizlediği klasör. Sınama kopyasında
/// (`OPENIFY_TEST_DISPLAY`) gerçek önbelleğe dokunulmaz: yalnız
/// `OPENIFY_TEST_CACHE_DIR` verilirse o klasör kullanılır; verilmezse
/// boyut gerçek klasörden okunur ama temizleme kapalıdır.
pub fn settings_cache_dir() -> Option<PathBuf> {
    cache_dir()
}

/// Temizleme bu çalışmada izinli mi (sınama kopyasında yalnız sahte klasörle)?
pub fn clearing_allowed() -> bool {
    std::env::var_os("OPENIFY_TEST_DISPLAY").is_none()
        || std::env::var_os("OPENIFY_TEST_CACHE_DIR").is_some()
}

/// Klasörün (alt klasörleriyle) toplam boyutu, bayt. Okunamayanlar sayılmaz.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// `root` altındaki bu alt klasörlerin içini siler (klasörlerin kendisi
/// kalır; açık dosyalar atlanır). Boşalttığı bayt sayısını döner.
pub fn clear_subdirs(root: &Path, names: &[&str]) -> u64 {
    let mut freed = 0;
    for name in names {
        let dir = root.join(name);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let size = if path.is_dir() {
                dir_size(&path)
            } else {
                entry.metadata().map_or(0, |m| m.len())
            };
            let removed = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            if removed.is_ok() {
                freed += size;
            }
        }
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uri: &str) -> TrackRow {
        TrackRow {
            uri: uri.into(),
            name: format!("{uri} adı"),
            artists: "Sanatçı".into(),
            album: "Albüm".into(),
            duration_ms: 180_000,
            image: String::new(),
            plays: None,
        }
    }

    #[test]
    fn library_roundtrip_keeps_order() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.load_library().unwrap().is_none());

        let library = Library {
            liked_count: 322,
            albums: vec![LibraryItem {
                id: CollectionId::Album("spotify:album:a".into()),
                name: "Albüm".into(),
                subtitle: "Sanatçı".into(),
                image: "https://i.scdn.co/image/a".into(),
            }],
            playlists: vec![
                LibraryItem {
                    id: CollectionId::Playlist("spotify:playlist:b".into()),
                    name: "B".into(),
                    subtitle: "Spotify · 50 şarkı".into(),
                    image: String::new(),
                },
                LibraryItem {
                    id: CollectionId::Playlist("spotify:playlist:c".into()),
                    name: "C".into(),
                    subtitle: "3 şarkı".into(),
                    image: String::new(),
                },
            ],
            artists: vec![
                super::super::library::artist_item(
                    "spotify:artist:y",
                    "Yeni takip",
                    "https://i.scdn.co/image/y",
                ),
                super::super::library::artist_item("spotify:artist:e", "Eski takip", ""),
            ],
            ..Default::default()
        };
        db.save_library(&library).unwrap();
        let back = db.load_library().unwrap().unwrap();
        assert_eq!(back.liked_count, 322);
        assert_eq!(back.albums.len(), 1);
        assert_eq!(back.albums[0].image, "https://i.scdn.co/image/a");
        let names: Vec<_> = back.playlists.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["B", "C"]);
        // Takip edilen sanatçılar: sıra (en son takip edilen önde), ad, portre, kanonik altyazı.
        let artists: Vec<_> = back
            .artists
            .iter()
            .map(|a| (a.id.key(), a.name.as_str(), a.image.as_str()))
            .collect();
        assert_eq!(
            artists,
            [
                (
                    "spotify:artist:y",
                    "Yeni takip",
                    "https://i.scdn.co/image/y"
                ),
                ("spotify:artist:e", "Eski takip", "")
            ]
        );
        assert!(
            back.artists
                .iter()
                .all(|a| a.subtitle == crate::i18n::canon::ARTIST)
        );
        // Sanatçılar ayrı tabloda: kütüphanenin geri kalanı değişince kaybolmazlar,
        // sanatçıları boşaltmak da listelere dokunmaz.
        let mut changed = back.clone();
        changed.playlists.pop();
        db.save_library(&changed).unwrap();
        let again = db.load_library().unwrap().unwrap();
        assert_eq!((again.playlists.len(), again.artists.len()), (1, 2));
        let mut emptied = again.clone();
        emptied.artists.clear();
        db.save_library(&emptied).unwrap();
        let last = db.load_library().unwrap().unwrap();
        assert_eq!((last.playlists.len(), last.artists.len()), (1, 0));
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        let mut out = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
        img.write_with_encoder(enc).unwrap();
        out
    }

    #[test]
    fn shrink_cover_limits_long_side_and_keeps_aspect() {
        let small = shrink_cover(&jpeg(640, 400));
        let (w, h) = image::load_from_memory(&small)
            .unwrap()
            .to_rgb8()
            .dimensions();
        assert_eq!(w, COVER_MAX_SIDE);
        assert_eq!(h, 160);

        // Dikey resim: yükseklik sınırlanır.
        let tall = shrink_cover(&jpeg(300, 600));
        let (w, h) = image::load_from_memory(&tall)
            .unwrap()
            .to_rgb8()
            .dimensions();
        assert_eq!(h, COVER_MAX_SIDE);
        assert_eq!(w, 128);
    }

    #[test]
    fn shrink_cover_leaves_small_and_broken_images_alone() {
        let thumb = jpeg(64, 64);
        assert_eq!(shrink_cover(&thumb), thumb);
        let exact = jpeg(COVER_MAX_SIDE, COVER_MAX_SIDE);
        assert_eq!(shrink_cover(&exact), exact);
        assert_eq!(shrink_cover(b"bu bir resim degil"), b"bu bir resim degil");
    }

    #[test]
    fn collections_and_missing_tracks() {
        let db = Db::open_in_memory().unwrap();
        let id = CollectionId::Playlist("spotify:playlist:x".into());
        assert_eq!(db.collection_uris(&id).unwrap(), None);

        let uris: Vec<String> = ["spotify:track:1", "spotify:track:2", "spotify:track:3"]
            .map(String::from)
            .to_vec();
        db.save_collection(&id, Some(b"rev1"), &uris).unwrap();
        assert_eq!(db.collection_uris(&id).unwrap().unwrap(), uris);
        assert_eq!(db.revision(&id).unwrap().unwrap(), b"rev1");

        db.save_tracks(&[row("spotify:track:2")]).unwrap();
        let mut missing = db.missing_tracks(10).unwrap();
        missing.sort();
        assert_eq!(missing, ["spotify:track:1", "spotify:track:3"]);

        let found = db.tracks(&uris).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found["spotify:track:2"].name, "spotify:track:2 adı");

        // Liste değişince eski sıra tamamen yenisiyle değişir.
        db.save_collection(&id, Some(b"rev2"), &uris[..1]).unwrap();
        assert_eq!(db.collection_uris(&id).unwrap().unwrap(), uris[..1]);
    }

    #[test]
    fn lyrics_rows_roundtrip_and_prune() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.lyrics_row("spotify:track:a", "spotify").unwrap(), None);
        db.save_lyrics("spotify:track:a", "spotify", Some("{}"), None, 10)
            .unwrap();
        db.save_lyrics("spotify:track:a", "lrclib", None, Some(99), 11)
            .unwrap();
        let hit = db
            .lyrics_row("spotify:track:a", "spotify")
            .unwrap()
            .unwrap();
        assert_eq!((hit.body.as_deref(), hit.expires_at), (Some("{}"), None));
        let none = db.lyrics_row("spotify:track:a", "lrclib").unwrap().unwrap();
        assert_eq!((none.body, none.expires_at), (None, Some(99)));
        // Üzerine yazma.
        db.save_lyrics("spotify:track:a", "lrclib", Some("x"), None, 12)
            .unwrap();
        assert_eq!(
            db.lyrics_row("spotify:track:a", "lrclib")
                .unwrap()
                .unwrap()
                .body
                .as_deref(),
            Some("x")
        );
        // Sınır aşılınca en eskiler silinir, yenisi kalır.
        for i in 0..(LYRICS_KEEP + 5) {
            db.save_lyrics(
                &format!("spotify:track:{i}"),
                "spotify",
                Some("b"),
                None,
                100 + i,
            )
            .unwrap();
        }
        let n: i64 = db
            .w()
            .query_row("SELECT COUNT(*) FROM lyrics", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, LYRICS_KEEP);
        assert!(
            db.lyrics_row("spotify:track:a", "spotify")
                .unwrap()
                .is_none()
        );
        assert!(
            db.lyrics_row(&format!("spotify:track:{}", LYRICS_KEEP + 4), "spotify")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn cover_path_is_stable_and_stays_in_cover_dir() {
        let a = cover_path("https://i.scdn.co/image/ab67616d0000b273abc").unwrap();
        let b = cover_path("https://i.scdn.co/image/ab67616d0000b273abc").unwrap();
        assert_eq!(a, b);
        assert!(a.parent().unwrap().ends_with("covers"));
        let c = cover_path("https://seed-mix-image.spotifycdn.com/v6/img/../../etc/en").unwrap();
        assert!(c.parent().unwrap().ends_with("covers"));
        assert_ne!(a, c);
        assert!(cover_path("").is_none());
    }

    /// Sahte önbellek klasörü (gerçek `%LOCALAPPDATA%\Openify`'a dokunmaz).
    fn fake_cache(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("openify-cache-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("audio").join("ab")).unwrap();
        fs::create_dir_all(root.join("covers")).unwrap();
        fs::write(root.join("audio").join("ab").join("x"), vec![0u8; 1000]).unwrap();
        fs::write(root.join("audio").join("y"), vec![0u8; 500]).unwrap();
        fs::write(root.join("covers").join("c.c2"), vec![0u8; 200]).unwrap();
        fs::write(root.join("library.db"), vec![0u8; 300]).unwrap();
        root
    }

    #[test]
    fn dir_size_counts_nested_files() {
        let root = fake_cache("size");
        assert_eq!(dir_size(&root), 2000);
        assert_eq!(dir_size(&root.join("yok")), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clearing_empties_audio_and_covers_but_keeps_the_database() {
        let root = fake_cache("clear");
        let freed = clear_subdirs(&root, &CLEARABLE);
        assert_eq!(freed, 1700);
        assert_eq!(dir_size(&root), 300, "yalnız library.db kalır");
        assert!(
            root.join("audio").is_dir() && root.join("covers").is_dir(),
            "klasörlerin kendisi kalır"
        );
        assert!(root.join("library.db").is_file());
        // İkinci temizlik zararsız.
        assert_eq!(clear_subdirs(&root, &CLEARABLE), 0);
        let _ = fs::remove_dir_all(&root);
    }

    // ── Göç, okuma bağlantısı, klasör / sabitleme / eklenme anı modeli ──

    use std::sync::Arc;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openify-db-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn version(db: &Db) -> i64 {
        migrate::user_version(&db.w()).unwrap()
    }

    fn pl(n: u32) -> CollectionId {
        CollectionId::Playlist(format!("spotify:playlist:{n}"))
    }

    /// Eski kodun yazdığı dosya (sürüm 1–2: kapak sütunu yok; 3: şimdiye kadarki
    /// şema), içinde her tabloda veriyle.
    fn legacy_file(path: &Path, version: i64) {
        let conn = Connection::open(path).unwrap();
        let image = if version >= 3 {
            ", image TEXT NOT NULL DEFAULT ''"
        } else {
            ""
        };
        conn.execute_batch(&format!(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE library_items (position INTEGER PRIMARY KEY, id TEXT NOT NULL, name TEXT NOT NULL, subtitle TEXT NOT NULL{image});
             CREATE TABLE collections (id TEXT PRIMARY KEY, revision BLOB);
             CREATE TABLE collection_tracks (collection TEXT NOT NULL, position INTEGER NOT NULL, track_uri TEXT NOT NULL, PRIMARY KEY (collection, position));
             CREATE TABLE tracks (uri TEXT PRIMARY KEY, name TEXT NOT NULL, artists TEXT NOT NULL, album TEXT NOT NULL, duration_ms INTEGER NOT NULL{image});
             CREATE TABLE playback (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE lyrics (uri TEXT NOT NULL, source TEXT NOT NULL, body TEXT, expires_at INTEGER, saved_at INTEGER NOT NULL, PRIMARY KEY (uri, source));
             INSERT INTO meta VALUES ('liked_count', '7');
             INSERT INTO library_items (position, id, name, subtitle) VALUES (0, 'spotify:playlist:1', 'Eski', '3 şarkı');
             INSERT INTO collections VALUES ('spotify:playlist:1', x'0102');
             INSERT INTO collection_tracks VALUES ('spotify:playlist:1', 0, 'spotify:track:a'), ('spotify:playlist:1', 1, 'spotify:track:b');
             INSERT INTO tracks (uri, name, artists, album, duration_ms) VALUES ('spotify:track:a', 'A', 'X', 'Y', 1000);
             INSERT INTO playback VALUES ('state', 'durum'), ('context', 'büyük');
             INSERT INTO lyrics VALUES ('spotify:track:a', 'spotify', '{{}}', NULL, 5);
             PRAGMA user_version = {version};"
        ))
        .unwrap();
    }

    #[test]
    fn every_older_version_reaches_the_latest_and_the_same_schema() {
        let dir = temp_dir("migrate-all");
        let fresh = Db::open(&dir.join("fresh.db")).unwrap();
        assert_eq!(version(&fresh), 10);
        let want = migrate::schema_columns(&fresh.w());
        assert_eq!(
            want["library_items"],
            [
                "position", "id", "name", "subtitle", "image", "kind", "parent", "added_at",
                "creator"
            ]
        );
        assert_eq!(
            want["collection_tracks"],
            ["collection", "position", "track_uri", "added_at"]
        );
        assert!(want.contains_key("library_pins"));
        assert_eq!(
            want["library_artists"],
            ["position", "uri", "name", "image"]
        );
        assert_eq!(
            want["play_history"],
            [
                "id",
                "played_at",
                "uri",
                "name",
                "artists",
                "album",
                "duration_ms",
                "listened_ms"
            ]
        );
        assert_eq!(
            want["hidden_songs"],
            ["uri", "context", "name", "artists", "hidden_at"]
        );
        assert_eq!(want["track_analysis"][..3], ["uri", "algo", "flags"]);
        assert_eq!(
            want["library_list_info"],
            ["id", "description", "collaborative"]
        );
        for v in [1, 2, 3] {
            let path = dir.join(format!("v{v}.db"));
            legacy_file(&path, v);
            let db = Db::open(&path).unwrap();
            assert_eq!(version(&db), 10, "sürüm {v}");
            assert_eq!(
                migrate::schema_columns(&db.w()),
                want,
                "sürüm {v} aynı şemaya varmalı"
            );
            // Kullanıcı durumu ve pahalı veri HİÇBİR sürümde silinmez.
            assert_eq!(
                db.load_playback().unwrap(),
                Some(("durum".to_string(), Some("büyük".to_string()))),
                "sürüm {v}: kaldığı yer"
            );
            assert_eq!(
                db.lyrics_row("spotify:track:a", "spotify")
                    .unwrap()
                    .unwrap()
                    .body
                    .as_deref(),
                Some("{}")
            );
            if v == 3 {
                // Şimdiki sürüm: yeniden indirilebilir veri de aynen durur.
                let lib = db.load_library().unwrap().unwrap();
                assert_eq!(
                    (
                        lib.liked_count,
                        lib.playlists.len(),
                        lib.playlists[0].name.as_str()
                    ),
                    (7, 1, "Eski")
                );
                let entries = db.collection_entries(&pl(1)).unwrap().unwrap();
                assert_eq!(
                    entries,
                    [
                        Entry::untimed("spotify:track:a"),
                        Entry::untimed("spotify:track:b")
                    ]
                );
                assert_eq!(db.revision(&pl(1)).unwrap().unwrap(), [1, 2]);
                assert_eq!(db.tracks(&["spotify:track:a".into()]).unwrap().len(), 1);
            } else {
                // 1–2: yalnız yeniden indirilebilir tablolar düşürülüp yeniden kuruldu.
                assert!(db.load_library().unwrap().is_none(), "sürüm {v}");
                assert!(db.collection_uris(&pl(1)).unwrap().is_none());
                assert!(db.tracks(&["spotify:track:a".into()]).unwrap().is_empty());
            }
            // İkinci açılış hiçbir şeyi değiştirmez.
            drop(db);
            let again = Db::open(&path).unwrap();
            assert_eq!(version(&again), 10);
            assert_eq!(again.load_playback().unwrap().unwrap().0, "durum");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Şimdiki (sürüm 4) dosya, kullanıcının sabitlemeleri ve eklenme anlarıyla:
    /// 5'e göçte hiçbir satır değişmez, `library_artists` boş gelir, sonra yazılıp okunur.
    #[test]
    fn current_version_4_file_gains_the_artists_table_and_loses_nothing() {
        let dir = temp_dir("migrate-v4-artists");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        {
            // 4'e kadar eski kodun yaptığı gibi göç et, kullanıcı verisi ekle.
            let mut conn = Connection::open(&path).unwrap();
            migrate::migrate(&mut conn, &MIGRATIONS[..2]).unwrap();
            assert_eq!(migrate::user_version(&conn).unwrap(), 4);
            assert!(!migrate::schema_columns(&conn).contains_key("library_artists"));
            conn.execute_batch(
                "INSERT INTO library_pins (id, pinned_at) VALUES ('spotify:playlist:1', 77);
                 UPDATE collection_tracks SET added_at = 1700000000000 WHERE track_uri = 'spotify:track:a';",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 10);
        // Mevcut veri aynen: liste, sabitleme, eklenme anı, kaldığı yer, sözler.
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (
                lib.liked_count,
                lib.playlists[0].name.as_str(),
                lib.artists.len()
            ),
            (7, "Eski", 0)
        );
        assert_eq!(db.pinned_keys().unwrap(), ["spotify:playlist:1"]);
        let entries = db.collection_entries(&pl(1)).unwrap().unwrap();
        assert_eq!(
            entries[0],
            Entry {
                track: "spotify:track:a".into(),
                added_at: Some(1_700_000_000_000)
            }
        );
        assert_eq!(db.load_playback().unwrap().unwrap().0, "durum");
        assert!(
            db.lyrics_row("spotify:track:a", "spotify")
                .unwrap()
                .is_some()
        );
        // Yeni tablo çalışıyor ve sonraki açılış değiştirmiyor.
        let mut with_artists = lib.clone();
        with_artists.artists = vec![super::super::library::artist_item(
            "spotify:artist:a",
            "A",
            "i",
        )];
        db.save_library(&with_artists).unwrap();
        drop(db);
        let again = Db::open(&path).unwrap();
        assert_eq!(version(&again), 10);
        assert_eq!(again.load_library().unwrap().unwrap().artists.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_database_from_a_newer_app_is_refused_and_left_alone() {
        let dir = temp_dir("too-new");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 11)
            .unwrap();
        let err = Db::open(&path).err().expect("daha yeni sürüm açılmamalı");
        assert!(format!("{err:#}").contains("version 11"), "{err:#}");
        // Dosyaya dokunulmadı: sürüm ve veri aynı.
        let raw = Connection::open(&path).unwrap();
        assert_eq!(migrate::user_version(&raw).unwrap(), 11);
        let n: i64 = raw
            .query_row("SELECT COUNT(*) FROM collection_tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        let cols = migrate::schema_columns(&raw);
        assert_eq!(
            cols["collection_tracks"],
            ["collection", "position", "track_uri"]
        );
        drop(raw);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_roundtrip_and_plain_saves_keep_known_added_at() {
        let db = Db::open_in_memory().unwrap();
        let id = pl(1);
        assert_eq!(db.collection_entries(&id).unwrap(), None);
        let entries = vec![
            Entry {
                track: "spotify:track:1".into(),
                added_at: Some(1_700_000_000_000),
            },
            Entry::untimed("spotify:track:2"),
            Entry {
                track: "spotify:track:3".into(),
                added_at: Some(1_600_000_000_000),
            },
        ];
        db.save_collection_entries(&id, Some(b"r1"), &entries)
            .unwrap();
        assert_eq!(db.collection_entries(&id).unwrap().unwrap(), entries);
        // Eski arayüz (yalnız adresler) aynı sırada sonuç verir.
        assert_eq!(
            db.collection_uris(&id).unwrap().unwrap(),
            ["spotify:track:1", "spotify:track:2", "spotify:track:3"]
        );

        // Düzenleme yaması listeyi adreslerle baştan yazar: bilinen tarihler
        // korunur, yeni gelen şarkının tarihi yok, silinenin tarihi gider.
        let patched: Vec<String> = ["spotify:track:9", "spotify:track:3", "spotify:track:1"]
            .map(String::from)
            .to_vec();
        db.save_collection(&id, Some(b"r2"), &patched).unwrap();
        assert_eq!(
            db.collection_entries(&id).unwrap().unwrap(),
            [
                Entry::untimed("spotify:track:9"),
                Entry {
                    track: "spotify:track:3".into(),
                    added_at: Some(1_600_000_000_000)
                },
                Entry {
                    track: "spotify:track:1".into(),
                    added_at: Some(1_700_000_000_000)
                },
            ]
        );
        assert_eq!(db.revision(&id).unwrap().unwrap(), b"r2");

        // Verilen tarih her zaman eskisini geçer; bir başka listeninki karışmaz.
        db.save_collection_entries(
            &id,
            None,
            &[Entry {
                track: "spotify:track:1".into(),
                added_at: Some(5),
            }],
        )
        .unwrap();
        assert_eq!(
            db.collection_entries(&id).unwrap().unwrap()[0].added_at,
            Some(5)
        );
        db.save_collection(&pl(2), None, &["spotify:track:1".to_string()])
            .unwrap();
        assert_eq!(
            db.collection_entries(&pl(2)).unwrap().unwrap()[0].added_at,
            None
        );
    }

    #[test]
    fn folder_tree_roundtrips_and_the_flat_library_ignores_folders() {
        let db = Db::open_in_memory().unwrap();
        let node = |key: &str, kind, parent: Option<&str>, name: &str| LibraryNode {
            key: key.into(),
            kind,
            parent: parent.map(String::from),
            name: name.into(),
            subtitle: String::new(),
            image: String::new(),
            added_at: None,
            creator: String::new(),
        };
        let nodes = vec![
            node("spotify:album:a", ItemKind::Playlist, None, "Albüm"),
            node(
                "spotify:start-group:f1:Klasör",
                ItemKind::Folder,
                None,
                "Klasör",
            ),
            node(
                "spotify:playlist:1",
                ItemKind::Playlist,
                Some("spotify:start-group:f1:Klasör"),
                "İçeride",
            ),
            node("spotify:playlist:2", ItemKind::Playlist, None, "Kökte"),
        ];
        db.save_library_nodes(&nodes, 11).unwrap();
        assert_eq!(db.load_library_nodes().unwrap(), nodes);
        // Düz görünüm: klasör yok, klasördeki liste düzleşmiş (eskisi gibi).
        let flat = db.load_library().unwrap().unwrap();
        assert_eq!(flat.liked_count, 11);
        assert_eq!(flat.albums.len(), 1);
        let names: Vec<_> = flat.playlists.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["İçeride", "Kökte"]);
    }

    /// Klasörler, parent'lar, eklenme anı, yaratıcı ve sabitlemeler `Library`nin
    /// `meta`sı olarak yazılıp okunur; bir sonraki yazma klasörsüzse klasör satırları kalkar.
    #[test]
    fn library_meta_roundtrips_folders_stamps_and_pins() {
        use super::super::library::{Folder, LibraryMeta, folder_id};
        let db = Db::open_in_memory().unwrap();
        let list = |n: u32, name: &str| LibraryItem {
            id: pl(n),
            name: name.into(),
            subtitle: "3 şarkı".into(),
            image: String::new(),
        };
        let f1 = "spotify:start-group:f1:Yaz".to_string();
        let f2 = "spotify:start-group:f2:Yol".to_string();
        let mut library = Library {
            liked_count: 5,
            albums: vec![LibraryItem {
                id: CollectionId::Album("spotify:album:a".into()),
                name: "Albüm".into(),
                subtitle: "X".into(),
                image: String::new(),
            }],
            // Kökte: L1; Yaz klasöründe: L2, (Yol klasöründe: L3), L4; kökte L5; sondaki boş klasör.
            playlists: vec![
                list(1, "L1"),
                list(2, "L2"),
                list(3, "L3"),
                list(4, "L4"),
                list(5, "L5"),
            ],
            meta: LibraryMeta {
                folders: vec![
                    Folder {
                        key: f1.clone(),
                        name: "Yaz".into(),
                        parent: None,
                        at: 1,
                    },
                    Folder {
                        key: f2.clone(),
                        name: "Yol".into(),
                        parent: Some(f1.clone()),
                        at: 2,
                    },
                    Folder {
                        key: "spotify:start-group:f3:Boş".into(),
                        name: "Boş".into(),
                        parent: None,
                        at: 5,
                    },
                ],
                parents: [
                    (pl(2).key().to_string(), f1.clone()),
                    (pl(3).key().to_string(), f2.clone()),
                    (pl(4).key().to_string(), f1.clone()),
                ]
                .into(),
                added: [
                    ("spotify:album:a".to_string(), 5_000),
                    (pl(3).key().to_string(), 9_000),
                ]
                .into(),
                creators: [(pl(3).key().to_string(), "ayse".to_string())].into(),
                descriptions: [
                    (pl(2).key().to_string(), "Yaz akşamları".to_string()),
                    (pl(5).key().to_string(), "İki satır\nve devamı".to_string()),
                ]
                .into(),
                collaborative: [pl(4).key().to_string()].into(),
                pinned: Vec::new(),
            },
            ..Default::default()
        };
        db.save_library(&library).unwrap();
        db.set_pinned(pl(4).key(), true, 20).unwrap();
        db.set_pinned("liked", true, 10).unwrap();
        library.meta.pinned = vec!["liked".into(), pl(4).key().to_string()];
        let back = db.load_library().unwrap().unwrap();
        assert_eq!(back, library, "her şey aynen geri gelmeli");
        assert_eq!(
            folder_id(&back.meta.folders[0].key),
            "spotify:start-group:f1"
        );
        // Düz görünüm hâlâ klasörsüz sıra: albüm + listeler sırasıyla.
        let names: Vec<_> = back.playlists.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["L1", "L2", "L3", "L4", "L5"]);
        // Klasörsüz yazma klasör satırlarını ve parent'ları siler, sabitlemeye dokunmaz.
        library.meta = LibraryMeta::default();
        db.save_library(&library).unwrap();
        let flat = db.load_library().unwrap().unwrap();
        assert!(flat.meta.folders.is_empty() && flat.meta.parents.is_empty());
        assert_eq!(flat.meta.pinned, ["liked", pl(4).key()]);
    }

    /// 5'teki bir dosya 6'ya göçte hiçbir şey kaybetmez; yeni sütunlar boş gelir.
    #[test]
    fn version_5_file_gains_item_stamps_and_loses_nothing() {
        let dir = temp_dir("migrate-v5-stamps");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::migrate(&mut conn, &MIGRATIONS[..3]).unwrap();
            assert_eq!(migrate::user_version(&conn).unwrap(), 5);
            assert_eq!(
                migrate::schema_columns(&conn)["library_items"],
                [
                    "position", "id", "name", "subtitle", "image", "kind", "parent"
                ]
            );
            conn.execute_batch(
                "INSERT INTO library_pins (id, pinned_at) VALUES ('spotify:playlist:1', 77);
                 INSERT INTO library_artists (position, uri, name) VALUES (0, 'spotify:artist:a', 'A');",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 10);
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (lib.playlists[0].name.as_str(), lib.artists.len()),
            ("Eski", 1)
        );
        assert!(
            lib.meta.added.is_empty()
                && lib.meta.creators.is_empty()
                && lib.meta.folders.is_empty()
        );
        assert_eq!(lib.meta.pinned, ["spotify:playlist:1"]);
        assert_eq!(db.collection_entries(&pl(1)).unwrap().unwrap().len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 6'daki bir dosya 7'ye göçte hiçbir şey kaybetmez; çalma geçmişi boş
    /// gelir, yazılıp okunur, sonraki açılış ve tazeleme ona dokunmaz.
    #[test]
    fn version_6_file_gains_play_history_and_loses_nothing() {
        let dir = temp_dir("migrate-v6-history");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::migrate(&mut conn, &MIGRATIONS[..4]).unwrap();
            assert_eq!(migrate::user_version(&conn).unwrap(), 6);
            assert!(!migrate::schema_columns(&conn).contains_key("play_history"));
            conn.execute_batch(
                "INSERT INTO library_pins (id, pinned_at) VALUES ('spotify:playlist:1', 77);
                 INSERT INTO library_artists (position, uri, name) VALUES (0, 'spotify:artist:a', 'A');
                 UPDATE library_items SET added_at = 1700000000000, creator = 'furka';",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 10);
        // Mevcut veri aynen.
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (lib.playlists[0].name.as_str(), lib.artists.len()),
            ("Eski", 1)
        );
        assert_eq!(lib.meta.added["spotify:playlist:1"], 1_700_000_000_000);
        assert_eq!(lib.meta.pinned, ["spotify:playlist:1"]);
        assert_eq!(db.collection_entries(&pl(1)).unwrap().unwrap().len(), 2);
        assert_eq!(db.load_playback().unwrap().unwrap().0, "durum");
        assert!(
            db.lyrics_row("spotify:track:a", "spotify")
                .unwrap()
                .is_some()
        );
        // Yeni tablo boş, yazılabilir.
        let n: i64 = db
            .r()
            .query_row("SELECT COUNT(*) FROM play_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        db.w()
            .execute(
                "INSERT INTO play_history (played_at, uri, name, artists, album, duration_ms, listened_ms) VALUES (1, 'u', 'n', 'a', 'b', 2, 3)",
                [],
            )
            .unwrap();
        // Kütüphane baştan yazılsa da (tazeleme) geçmiş durur.
        db.save_library(&lib).unwrap();
        db.save_library_nodes(&[], 1).unwrap();
        drop(db);
        let again = Db::open(&path).unwrap();
        assert_eq!(version(&again), 10);
        let n: i64 = again
            .r()
            .query_row("SELECT COUNT(*) FROM play_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            n, 1,
            "çalma geçmişi tazelemede ve yeniden açılışta silinmez"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 7'deki bir dosya 8'e göçte hiçbir şey kaybetmez; gizlenen şarkılar tablosu
    /// boş gelir, yazılıp okunur, sonraki açılış ve tazeleme ona dokunmaz.
    #[test]
    fn version_7_file_gains_hidden_songs_and_loses_nothing() {
        let dir = temp_dir("migrate-v7-hidden");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::migrate(&mut conn, &MIGRATIONS[..5]).unwrap();
            assert_eq!(migrate::user_version(&conn).unwrap(), 7);
            assert!(!migrate::schema_columns(&conn).contains_key("hidden_songs"));
            conn.execute_batch(
                "INSERT INTO library_pins (id, pinned_at) VALUES ('spotify:playlist:1', 77);
                 INSERT INTO play_history (played_at, uri, name, artists, album, duration_ms, listened_ms)
                     VALUES (5, 'u', 'n', 'a', 'b', 2, 3);",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 10);
        // Mevcut veri aynen: kütüphane, sabitleme, çalma geçmişi, kaldığı yer, sözler.
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (lib.playlists[0].name.as_str(), lib.meta.pinned.clone()),
            ("Eski", vec!["spotify:playlist:1".to_string()])
        );
        let plays: i64 = db
            .r()
            .query_row("SELECT COUNT(*) FROM play_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(plays, 1);
        assert_eq!(db.load_playback().unwrap().unwrap().0, "durum");
        assert!(
            db.lyrics_row("spotify:track:a", "spotify")
                .unwrap()
                .is_some()
        );
        // Yeni tablo boş, yazılabilir.
        assert!(db.hidden_songs().unwrap().is_empty());
        db.hide_song(&crate::engine::hidden::HiddenSong {
            uri: "spotify:track:a".into(),
            context: Some("spotify:playlist:1".into()),
            name: "A".into(),
            artists: "X".into(),
            hidden_at: 9,
        })
        .unwrap();
        // Kütüphane baştan yazılsa da (tazeleme) gizlenenler durur.
        db.save_library(&lib).unwrap();
        db.save_library_nodes(&[], 1).unwrap();
        drop(db);
        let again = Db::open(&path).unwrap();
        assert_eq!(version(&again), 10);
        let kept = again.hidden_songs().unwrap();
        assert_eq!(
            kept.len(),
            1,
            "gizlenen şarkılar tazelemede ve yeniden açılışta silinmez"
        );
        assert_eq!(kept[0].context.as_deref(), Some("spotify:playlist:1"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 8'deki bir dosya 9'a göçte hiçbir şey kaybetmez; AutoMix çözümleme tablosu boş
    /// gelir, yazılıp okunur, kütüphane tazelemesi ve yeniden açılış ona dokunmaz.
    #[test]
    fn version_8_file_gains_track_analysis_and_loses_nothing() {
        use crate::engine::output::{Analysis, HAS_TAIL};
        let dir = temp_dir("migrate-v8-analysis");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::migrate(&mut conn, &MIGRATIONS[..6]).unwrap();
            assert_eq!(migrate::user_version(&conn).unwrap(), 8);
            assert!(!migrate::schema_columns(&conn).contains_key("track_analysis"));
            conn.execute_batch(
                "INSERT INTO library_pins (id, pinned_at) VALUES ('spotify:playlist:1', 77);
                 INSERT INTO hidden_songs (uri, context, hidden_at) VALUES ('spotify:track:a', 'spotify:playlist:1', 9);",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 10);
        // Mevcut veri aynen: kütüphane, sabitleme, gizlenenler, kaldığı yer, sözler.
        let lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (lib.playlists[0].name.as_str(), lib.meta.pinned.clone()),
            ("Eski", vec!["spotify:playlist:1".to_string()])
        );
        assert_eq!(db.hidden_songs().unwrap().len(), 1);
        assert_eq!(db.load_playback().unwrap().unwrap().0, "durum");
        assert!(
            db.lyrics_row("spotify:track:a", "spotify")
                .unwrap()
                .is_some()
        );
        // Yeni tablo boş, yazılabilir.
        assert!(db.load_analyses(10).unwrap().is_empty());
        let mut a = Analysis::EMPTY;
        a.flags = HAS_TAIL;
        a.tail_ms = 900;
        db.save_analysis("spotify:track:a", &a).unwrap();
        // Kütüphane baştan yazılsa da (tazeleme) çözümlemeler durur.
        db.save_library(&lib).unwrap();
        db.save_library_nodes(&[], 1).unwrap();
        drop(db);
        let again = Db::open(&path).unwrap();
        assert_eq!(version(&again), 10);
        assert_eq!(
            again.analysis("spotify:track:a").unwrap().unwrap().tail_ms,
            900
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 9'daki bir dosya 10'a göçte hiçbir şey kaybetmez; liste açıklaması ve
    /// ortak liste tablosu boş gelir, yazılıp okunur.
    #[test]
    fn version_9_file_gains_list_info_and_loses_nothing() {
        let dir = temp_dir("migrate-v9-list-info");
        let path = dir.join("library.db");
        legacy_file(&path, 3);
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::migrate(&mut conn, &MIGRATIONS[..7]).unwrap();
            assert_eq!(migrate::user_version(&conn).unwrap(), 9);
            assert!(!migrate::schema_columns(&conn).contains_key("library_list_info"));
            conn.execute_batch(
                "INSERT INTO library_pins (id, pinned_at) VALUES ('spotify:playlist:1', 77);
                 INSERT INTO play_history (played_at, uri, name, artists, album, duration_ms, listened_ms)
                     VALUES (5, 'spotify:track:a', 'A', 'X', 'Y', 1000, 500);",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(version(&db), 10);
        // Mevcut veri aynen: kütüphane, sabitleme, çalma geçmişi, kaldığı yer, sözler.
        let mut lib = db.load_library().unwrap().unwrap();
        assert_eq!(
            (lib.playlists[0].name.as_str(), lib.meta.pinned.clone()),
            ("Eski", vec!["spotify:playlist:1".to_string()])
        );
        assert!(lib.meta.descriptions.is_empty() && lib.meta.collaborative.is_empty());
        assert_eq!(db.load_playback().unwrap().unwrap().0, "durum");
        assert!(
            db.lyrics_row("spotify:track:a", "spotify")
                .unwrap()
                .is_some()
        );
        // Yeni tablo yazılıp okunur; boş açıklama / kapalı ortaklık yazılmaz.
        lib.meta
            .descriptions
            .insert("spotify:playlist:1".into(), "Eski liste".into());
        lib.meta.collaborative.insert("spotify:playlist:1".into());
        db.save_library(&lib).unwrap();
        drop(db);
        let again = Db::open(&path).unwrap();
        assert_eq!(version(&again), 10);
        let back = again.load_library().unwrap().unwrap();
        assert_eq!(
            back.meta
                .descriptions
                .get("spotify:playlist:1")
                .map(String::as_str),
            Some("Eski liste")
        );
        assert!(back.meta.collaborative.contains("spotify:playlist:1"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pins_survive_library_rewrites_and_keep_their_order() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.pinned_keys().unwrap().is_empty());
        db.set_pinned("spotify:playlist:2", true, 200).unwrap();
        db.set_pinned("spotify:playlist:1", true, 100).unwrap();
        // Yeniden sabitlemek ilk anı korur (sıra bozulmaz).
        db.set_pinned("spotify:playlist:2", true, 900).unwrap();
        assert_eq!(
            db.pinned_keys().unwrap(),
            ["spotify:playlist:1", "spotify:playlist:2"]
        );
        // Kütüphane (düz ve ağaç yazıcıları) baştan yazılsa da sabitleme durur,
        // listenin o an kütüphanede olmaması da (alınamayan liste) onu silmez.
        let lib = Library {
            liked_count: 1,
            ..Default::default()
        };
        db.save_library(&lib).unwrap();
        db.save_library_nodes(&[], 1).unwrap();
        assert_eq!(db.pinned_keys().unwrap().len(), 2);
        db.set_pinned("spotify:playlist:1", false, 1).unwrap();
        db.set_pinned("spotify:playlist:yok", false, 1).unwrap();
        assert_eq!(db.pinned_keys().unwrap(), ["spotify:playlist:2"]);
    }

    /// İki sabitleme aynı milisaniyede olsa da sırası belli: yeni zaman her
    /// zaman en yenisinden sonra.
    #[test]
    fn pin_times_never_tie() {
        let db = Db::open_in_memory().unwrap();
        let a = db.next_pin_time(1000).unwrap();
        assert_eq!(a, 1000);
        db.set_pinned("liked", true, a).unwrap();
        let b = db.next_pin_time(1000).unwrap();
        assert_eq!(b, 1001);
        db.set_pinned("spotify:playlist:1", true, b).unwrap();
        assert_eq!(db.next_pin_time(5).unwrap(), 1002, "saat geriye kaysa da");
        assert_eq!(db.next_pin_time(9000).unwrap(), 9000);
        assert_eq!(db.pinned_keys().unwrap(), ["liked", "spotify:playlist:1"]);
    }

    #[test]
    fn tracks_lookup_handles_many_duplicates_and_missing_uris() {
        let db = Db::open_in_memory().unwrap();
        let rows: Vec<TrackRow> = (0..1203)
            .map(|i| row(&format!("spotify:track:{i}")))
            .collect();
        db.save_tracks(&rows).unwrap();
        let mut ask: Vec<String> = (0..1500).map(|i| format!("spotify:track:{i}")).collect();
        ask.push("spotify:track:7".into());
        ask.push("spotify:track:7".into());
        let found = db.tracks(&ask).unwrap();
        assert_eq!(found.len(), 1203);
        assert_eq!(found["spotify:track:1202"].name, "spotify:track:1202 adı");
        assert!(!found.contains_key("spotify:track:1203"));
        assert!(db.tracks(&[]).unwrap().is_empty());
    }

    #[test]
    fn reads_use_their_own_connections_and_never_wait_for_the_writer() {
        let dir = temp_dir("readers");
        let db = Arc::new(Db::open(&dir.join("library.db")).unwrap());
        assert_eq!(db.conns.reader_count(), 2);
        let lib = Library {
            liked_count: 3,
            ..Default::default()
        };
        db.save_library(&lib).unwrap();
        db.save_playback("s", None).unwrap();
        // Okuma bağlantısı yazıcının işlediğini görür.
        assert_eq!(db.load_library().unwrap().unwrap().liked_count, 3);
        assert_eq!(db.load_playback().unwrap().unwrap().0, "s");
        assert!(
            db.r().is_readonly(rusqlite::MAIN_DB).unwrap(),
            "okuma bağlantısı salt-okunur"
        );

        // Yazıcı açık bir yazma işleminde takılıyken okumalar beklemeden döner
        // ve işlenmemiş yazmayı görmez.
        let writer = db.w();
        writer
            .execute_batch("BEGIN IMMEDIATE; INSERT INTO meta VALUES ('beklemede', '1');")
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = Arc::clone(&db);
        std::thread::spawn(move || {
            let liked = reader.load_library().unwrap().map(|l| l.liked_count);
            let pending = reader.meta("beklemede").unwrap();
            let _ = tx.send((liked, pending));
        });
        let (liked, pending) = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("okuma yazıcıyı beklememeli");
        assert_eq!((liked, pending), (Some(3), None));
        writer.execute_batch("COMMIT").unwrap();
        drop(writer);
        assert_eq!(db.meta("beklemede").unwrap().as_deref(), Some("1"));

        // Birkaç iş parçacığı aynı anda okuyunca birbirini engellemez.
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let db = Arc::clone(&db);
                std::thread::spawn(move || (0..50).all(|_| db.load_library().unwrap().is_some()))
            })
            .collect();
        assert!(handles.into_iter().all(|h| h.join().unwrap()));
        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }
}
