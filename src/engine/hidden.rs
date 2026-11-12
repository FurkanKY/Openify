//! Yerel şarkı gizleme: "Bu şarkıyı gizle" (bir liste ya da albüm içinde) ve
//! "Hiçbir yerde çalma" (genel).
//!
//! Tamamen YERELDİR: Spotify hesabına hiçbir şey yazılmaz (Spotify'ın kendi
//! "bu şarkıyı listeden gizle" işlemi bilerek kullanılmıyor), kayıt yalnız bu
//! bilgisayarın `library.db` dosyasında (`hidden_songs`, şema sürümü 8)
//! durur; yedeklenmez, hesaba bağlı değildir.
//!
//! Kural (bkz. `rules.rs` ilkesi): gizli şarkı karıştırmada ve sırada
//! ATLANIR, ama kullanıcının elle seçtiği şarkı yine çalar (satıra çift
//! tıklama, "sıraya ekle", radyoda ilk şarkı olarak seçilen tohum). Gizleme
//! bağlam başınadır: "bu listede gizle" yalnız o listenin (albümün) çalma
//! sırasını etkiler; "hiçbir yerde çalma" bütün bağlamlarda geçerlidir.
//!
//! `HiddenList` saf bir dizin: kayıtlardan kurulur, sıra modeli (`queue.rs`)
//! ve arayüz (satırı soluk gösterir) aynı değeri (`Arc`) paylaşır.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Transaction;

use super::cache::Db;

/// Gizlenebilecek en çok kayıt (tablo ve ayar listesi şişmesin).
pub const HIDDEN_MAX: usize = 5000;

/// Gizlenmiş bir şarkı. `context`: gizlendiği liste / albümün anahtarı
/// (`CollectionId::key()`); `None`: hiçbir yerde çalma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenSong {
    pub uri: String,
    pub context: Option<String>,
    /// Ayarlardaki listede gösterilmek üzere kopyalanan künye.
    pub name: String,
    pub artists: String,
    /// Unix ms.
    pub hidden_at: i64,
}

impl HiddenSong {
    fn key(&self) -> (&str, &str) {
        (self.uri.as_str(), self.context.as_deref().unwrap_or(""))
    }
}

/// Gizli şarkıların dizini: en yeni önde sıralı kayıtlar + hızlı sorgu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenList {
    entries: Vec<HiddenSong>,
    everywhere: HashSet<String>,
    by_context: HashMap<String, HashSet<String>>,
}

impl HiddenList {
    /// Kayıtlardan kurar: aynı (şarkı, bağlam) tekrarı atılır, en yeni önde,
    /// en çok `HIDDEN_MAX`.
    pub fn new(mut entries: Vec<HiddenSong>) -> Self {
        entries.sort_by(|a, b| {
            b.hidden_at
                .cmp(&a.hidden_at)
                .then_with(|| a.uri.cmp(&b.uri))
        });
        let mut seen = HashSet::new();
        entries.retain(|e| {
            !e.uri.is_empty() && seen.insert((e.uri.clone(), e.context.clone().unwrap_or_default()))
        });
        entries.truncate(HIDDEN_MAX);
        let mut everywhere = HashSet::new();
        let mut by_context: HashMap<String, HashSet<String>> = HashMap::new();
        for e in &entries {
            match &e.context {
                None => {
                    everywhere.insert(e.uri.clone());
                }
                Some(ctx) => {
                    by_context
                        .entry(ctx.clone())
                        .or_default()
                        .insert(e.uri.clone());
                }
            }
        }
        Self {
            entries,
            everywhere,
            by_context,
        }
    }

    pub fn entries(&self) -> &[HiddenSong] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hiçbir yerde çalınmayacak mı?
    pub fn hidden_everywhere(&self, uri: &str) -> bool {
        self.everywhere.contains(uri)
    }

    /// Bu bağlamda (liste / albüm) gizlenmiş mi (genel gizleme sayılmaz)?
    pub fn hidden_in(&self, context: &str, uri: &str) -> bool {
        self.by_context
            .get(context)
            .is_some_and(|s| s.contains(uri))
    }

    /// Bu bağlamda çalma sırasından düşer mi (genel ya da bağlama özgü)?
    /// `context`: çalan bağlamın anahtarı; bağlamsız çalmada yalnız genel.
    pub fn hides(&self, context: Option<&str>, uri: &str) -> bool {
        self.everywhere.contains(uri) || context.is_some_and(|c| self.hidden_in(c, uri))
    }

    /// Hiçbir şey gizli değilse sıra süzgeci hiç çalışmaz.
    pub fn filters_nothing(&self) -> bool {
        self.entries.is_empty()
    }

    /// Kayıt eklenmiş yeni dizin (aynı şarkı + bağlam varsa künye / zaman yenilenir).
    pub fn with(&self, song: HiddenSong) -> Self {
        let mut entries: Vec<HiddenSong> = self
            .entries
            .iter()
            .filter(|e| e.key() != song.key())
            .cloned()
            .collect();
        entries.push(song);
        Self::new(entries)
    }

    /// Kaydı çıkarılmış yeni dizin.
    pub fn without(&self, uri: &str, context: Option<&str>) -> Self {
        let key = (uri, context.unwrap_or(""));
        Self::new(
            self.entries
                .iter()
                .filter(|e| e.key() != key)
                .cloned()
                .collect(),
        )
    }
}

// ── Tablo ───────────────────────────────────────────────────────────────

/// 7 → 8: gizlenen şarkılar (yeni tablo, veri kaybı yok).
pub(super) fn hidden_songs_step(tx: &Transaction, _from: i64) -> rusqlite::Result<()> {
    tx.execute_batch(
        "-- Locally hidden songs. context = '' means hidden everywhere, otherwise the key of the
         -- playlist / album (CollectionId::key) the song is hidden in. Never sent to Spotify.
         CREATE TABLE IF NOT EXISTS hidden_songs (
             uri       TEXT NOT NULL,
             context   TEXT NOT NULL DEFAULT '',
             name      TEXT NOT NULL DEFAULT '',
             artists   TEXT NOT NULL DEFAULT '',
             hidden_at INTEGER NOT NULL,
             PRIMARY KEY (uri, context)
         );",
    )
}

impl Db {
    /// Bütün kayıtlar, en yeni önde.
    pub fn hidden_songs(&self) -> Result<Vec<HiddenSong>> {
        let conn = self.r();
        let mut stmt = conn.prepare_cached(
            "SELECT uri, context, name, artists, hidden_at FROM hidden_songs ORDER BY hidden_at DESC, uri LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([HIDDEN_MAX as i64], |r| {
                let context: String = r.get(1)?;
                Ok(HiddenSong {
                    uri: r.get(0)?,
                    context: (!context.is_empty()).then_some(context),
                    name: r.get(2)?,
                    artists: r.get(3)?,
                    hidden_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Şarkıyı gizler (aynı şarkı + bağlam varsa künyesi ve zamanı yenilenir).
    pub fn hide_song(&self, song: &HiddenSong) -> Result<()> {
        let conn = self.w();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM hidden_songs", [], |r| r.get(0))?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM hidden_songs WHERE uri = ?1 AND context = ?2)",
            rusqlite::params![song.uri, song.context.as_deref().unwrap_or("")],
            |r| r.get(0),
        )?;
        if !exists && count >= HIDDEN_MAX as i64 {
            anyhow::bail!("hidden songs list is full ({HIDDEN_MAX})");
        }
        conn.execute(
            "INSERT INTO hidden_songs (uri, context, name, artists, hidden_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (uri, context) DO UPDATE SET name = excluded.name, artists = excluded.artists,
                                                      hidden_at = excluded.hidden_at",
            rusqlite::params![song.uri, song.context.as_deref().unwrap_or(""), song.name, song.artists, song.hidden_at],
        )?;
        Ok(())
    }

    /// Gizlemeyi kaldırır; kaydı vardı mı?
    pub fn show_song(&self, uri: &str, context: Option<&str>) -> Result<bool> {
        let n = self.w().execute(
            "DELETE FROM hidden_songs WHERE uri = ?1 AND context = ?2",
            rusqlite::params![uri, context.unwrap_or("")],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(uri: &str, context: Option<&str>, at: i64) -> HiddenSong {
        HiddenSong {
            uri: uri.into(),
            context: context.map(Into::into),
            name: format!("ad {uri}"),
            artists: "sanatçı".into(),
            hidden_at: at,
        }
    }

    #[test]
    fn global_hides_everywhere_and_context_hides_only_there() {
        let list = HiddenList::new(vec![
            song("t:a", None, 1),
            song("t:b", Some("spotify:playlist:x"), 2),
        ]);
        // Genel: her bağlamda ve bağlamsız.
        assert!(list.hides(None, "t:a"));
        assert!(list.hides(Some("spotify:playlist:x"), "t:a"));
        assert!(list.hides(Some("liked"), "t:a"));
        // Bağlama özgü: yalnız o bağlamda.
        assert!(list.hides(Some("spotify:playlist:x"), "t:b"));
        assert!(!list.hides(Some("spotify:playlist:y"), "t:b"));
        assert!(!list.hides(None, "t:b"));
        assert!(!list.hides(Some("spotify:playlist:x"), "t:c"));
        assert!(list.hidden_everywhere("t:a") && !list.hidden_everywhere("t:b"));
        assert!(
            list.hidden_in("spotify:playlist:x", "t:b")
                && !list.hidden_in("spotify:playlist:x", "t:a")
        );
    }

    #[test]
    fn newest_first_and_duplicates_collapse() {
        let list = HiddenList::new(vec![
            song("t:a", None, 5),
            song("t:b", None, 9),
            song("t:a", None, 7),
            song("t:a", Some("c"), 1),
            song("", None, 1),
        ]);
        let order: Vec<(&str, Option<&str>)> = list
            .entries()
            .iter()
            .map(|e| (e.uri.as_str(), e.context.as_deref()))
            .collect();
        // Aynı (şarkı, bağlam) bir kez; boş adres atılır; en yeni önde.
        assert_eq!(order, [("t:b", None), ("t:a", None), ("t:a", Some("c"))]);
        assert_eq!(list.entries()[1].hidden_at, 7);
    }

    #[test]
    fn with_and_without_edit_one_entry() {
        let list = HiddenList::default()
            .with(song("t:a", None, 1))
            .with(song("t:a", Some("c"), 2));
        assert_eq!(list.len(), 2);
        let list = list.without("t:a", None);
        assert!(!list.hidden_everywhere("t:a") && list.hidden_in("c", "t:a"));
        let list = list.without("t:a", Some("c"));
        assert!(list.is_empty() && list.filters_nothing());
        // Olmayan kaydı çıkarmak zararsız.
        assert!(list.without("t:z", None).is_empty());
    }

    #[test]
    fn table_round_trip_and_update() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.hidden_songs().unwrap().is_empty());
        db.hide_song(&song("t:a", None, 10)).unwrap();
        db.hide_song(&song("t:a", Some("spotify:album:z"), 20))
            .unwrap();
        db.hide_song(&song("t:b", None, 30)).unwrap();
        let all = db.hidden_songs().unwrap();
        assert_eq!(
            all.iter().map(|e| e.uri.as_str()).collect::<Vec<_>>(),
            ["t:b", "t:a", "t:a"]
        );
        assert_eq!(all[1].context.as_deref(), Some("spotify:album:z"));
        // Yeniden gizlemek yineleme açmaz, zamanı yeniler.
        db.hide_song(&HiddenSong {
            hidden_at: 99,
            name: "yeni".into(),
            ..song("t:a", None, 0)
        })
        .unwrap();
        let all = db.hidden_songs().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            (all[0].uri.as_str(), all[0].hidden_at, all[0].name.as_str()),
            ("t:a", 99, "yeni")
        );
        // Göster: yalnız o kayıt gider.
        assert!(db.show_song("t:a", None).unwrap());
        assert!(!db.show_song("t:a", None).unwrap(), "ikinci kez yok");
        let left = HiddenList::new(db.hidden_songs().unwrap());
        assert!(
            !left.hidden_everywhere("t:a")
                && left.hidden_in("spotify:album:z", "t:a")
                && left.hidden_everywhere("t:b")
        );
    }

    #[test]
    fn table_refuses_new_entries_when_full_but_updates_existing() {
        let db = Db::open_in_memory().unwrap();
        {
            let conn = db.w();
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..HIDDEN_MAX {
                tx.execute(
                    "INSERT INTO hidden_songs (uri, context, name, artists, hidden_at) VALUES (?1, '', '', '', ?2)",
                    rusqlite::params![format!("t:{i}"), i as i64],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        assert!(db.hide_song(&song("t:yeni", None, 1)).is_err());
        assert!(
            db.hide_song(&song("t:5", None, 7)).is_ok(),
            "var olan kayıt yenilenir"
        );
    }
}
