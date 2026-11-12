//! Kütüphane yedeği (yalnız dışa aktarım; geri yükleme henüz yok).
//!
//! Önbellekteki (library.db) kütüphaneyi tek bir JSON dosyasına yazar: listeler
//! ve şarkı adresleri, albümler, klasörler, sabitlemeler, takip edilen
//! sanatçılar, Beğenilenler. Ağa çıkılmaz, Spotify'a bir şey yazılmaz.
//!
//! Şarkı adresi (`spotify:track:…`) asıl veridir; ad, sanatçı, albüm yalnız
//! dosya okunabilsin diye yanına konur (önbellekte yoksa adres tek başına).
//! Şarkıları hiç indirilmemiş (önbellekte olmayan) liste `tracks_cached:
//! false` ile yalnız adıyla yazılır ve özette sayılır.
//!
//! Dosya önce geçici bir adla yazılıp yerine taşınır: yarım dosya kalmaz.

use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::civil;
use crate::engine::{
    cache::Db,
    library::{CollectionId, Entry, Library, TrackRow},
};

/// Yedek biçiminin sürümü (alan eklemek sürümü artırmaz, alan çıkarmak / anlamını
/// değiştirmek artırır).
pub const FORMAT: u32 = 1;

/// Dışa aktarımın özeti (arayüz sayıları gösterir).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub path: PathBuf,
    /// Yazılan çalma listesi sayısı (Beğenilenler hariç).
    pub lists: usize,
    /// Dosyadaki toplam şarkı adresi (Beğenilenler dahil, tekrarlar sayılır).
    pub tracks: usize,
    pub albums: usize,
    pub folders: usize,
    pub artists: usize,
    pub pinned: usize,
    /// Şarkıları önbellekte olmadığı için yalnız adıyla yazılan liste.
    pub missing: usize,
}

fn track_json(entry: &Entry, info: Option<&TrackRow>) -> Value {
    let mut v = serde_json::Map::new();
    v.insert("uri".into(), json!(entry.track));
    if let Some(t) = info {
        v.insert("name".into(), json!(t.name));
        v.insert("artists".into(), json!(t.artists));
        v.insert("album".into(), json!(t.album));
        v.insert("duration_ms".into(), json!(t.duration_ms));
    }
    if let Some(at) = entry.added_at {
        v.insert("added_at".into(), json!(at));
    }
    Value::Object(v)
}

/// Saf kurucu: kütüphane, liste içerikleri (anahtar → satırlar; yoksa önbellekte
/// değil) ve şarkı bilgileri verilir, JSON ve özet döner.
pub fn build(
    now_ms: i64,
    offset_minutes: i64,
    library: &Library,
    contents: &HashMap<String, Vec<Entry>>,
    info: &HashMap<String, TrackRow>,
) -> (Value, Summary) {
    let meta = &library.meta;
    let mut summary = Summary::default();
    let tracks_of = |key: &str| -> Option<Vec<Value>> {
        contents.get(key).map(|rows| rows.iter().map(|e| track_json(e, info.get(&e.track))).collect())
    };

    let liked = tracks_of("liked");
    summary.tracks += liked.as_ref().map_or(0, Vec::len);

    let mut playlists = Vec::new();
    for p in &library.playlists {
        let key = p.id.key();
        let tracks = tracks_of(key);
        match &tracks {
            Some(t) => summary.tracks += t.len(),
            None => summary.missing += 1,
        }
        let mut v = json!({
            "uri": key,
            "name": p.name,
            "creator": meta.creators.get(key),
            "added_at": meta.added.get(key),
            "folder": meta.parents.get(key),
            "pinned": meta.pinned.iter().any(|k| k == key),
            "tracks_cached": tracks.is_some(),
        });
        if let Some(t) = tracks {
            v["tracks"] = Value::Array(t);
        }
        playlists.push(v);
    }
    summary.lists = playlists.len();

    let albums: Vec<Value> = library
        .albums
        .iter()
        .map(|a| {
            let key = a.id.key();
            json!({
                "uri": key,
                "name": a.name,
                "by": a.subtitle,
                "added_at": meta.added.get(key),
                "pinned": meta.pinned.iter().any(|k| k == key),
            })
        })
        .collect();
    summary.albums = albums.len();

    let folders: Vec<Value> = meta
        .folders
        .iter()
        .map(|f| {
            json!({
                "key": f.key,
                "name": f.name,
                "parent": f.parent,
                "pinned": meta.pinned.iter().any(|k| k == &f.key),
            })
        })
        .collect();
    summary.folders = folders.len();

    let artists: Vec<Value> = library
        .artists
        .iter()
        .map(|a| json!({ "uri": a.id.key(), "name": a.name, "pinned": meta.pinned.iter().any(|k| k == a.id.key()) }))
        .collect();
    summary.artists = artists.len();
    summary.pinned = meta.pinned.len();

    let value = json!({
        "app": "Openify",
        "kind": "library-backup",
        "format": FORMAT,
        "exported_at": now_ms,
        "exported_date": civil::date_stamp(now_ms, offset_minutes),
        "liked": { "count": library.liked_count, "tracks_cached": liked.is_some(), "tracks": liked.unwrap_or_default() },
        "playlists": playlists,
        "albums": albums,
        "folders": folders,
        "artists": artists,
        "pinned": meta.pinned,
    });
    (value, summary)
}

/// Önbellekten okuyup `path`e yazar.
pub fn export(db: &Db, path: &Path, now_ms: i64, offset_minutes: i64) -> Result<Summary> {
    let library = db.load_library()?.context("Kütüphane önbellekte yok")?;
    let mut keys: Vec<String> = vec!["liked".into()];
    keys.extend(library.playlists.iter().map(|p| p.id.key().to_string()));
    let mut contents: HashMap<String, Vec<Entry>> = HashMap::new();
    let mut uris: BTreeSet<String> = BTreeSet::new();
    for key in keys {
        let Some(id) = CollectionId::from_key(&key) else { continue };
        if let Some(rows) = db.collection_entries(&id)? {
            uris.extend(rows.iter().map(|e| e.track.clone()));
            contents.insert(key, rows);
        }
    }
    let uris: Vec<String> = uris.into_iter().collect();
    let info = db.tracks(&uris)?;
    let (value, mut summary) = build(now_ms, offset_minutes, &library, &contents, &info);
    write_json(path, &value)?;
    summary.path = path.to_path_buf();
    Ok(summary)
}

/// Geçici dosyaya yaz, yerine taşı; üst klasör yoksa oluşturulur.
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("Klasör açılamadı: {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&tmp, bytes).with_context(|| format!("Yazılamadı: {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("Yerine taşınamadı: {}", path.display()))?;
    Ok(())
}

/// Önerilen dosya adı: `openify-backup-2026-11-11.json`.
pub fn suggested_name(now_ms: i64, offset_minutes: i64) -> String {
    format!("openify-backup-{}.json", civil::date_stamp(now_ms, offset_minutes))
}

/// Kaydet iletişim kutusu çıkmazsa ya da açılamazsa kullanılan yer:
/// Belgeler\Openify yedekleri\<öneri>.
pub fn fallback_path(now_ms: i64, offset_minutes: i64) -> PathBuf {
    let base = dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir);
    base.join("Openify yedekleri").join(suggested_name(now_ms, offset_minutes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::library::{Folder, LibraryItem, LibraryMeta};

    fn pl(n: u32) -> String {
        format!("spotify:playlist:{n:022}")
    }

    fn row(n: u32) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{n:022}"),
            name: format!("Şarkı {n}"),
            artists: "Sanatçı".into(),
            album: "Albüm".into(),
            duration_ms: 1000 * n,
            image: String::new(),
        }
    }

    fn item(id: CollectionId, name: &str) -> LibraryItem {
        LibraryItem { id, name: name.into(), subtitle: String::new(), image: String::new() }
    }

    fn library() -> Library {
        let folder = "spotify:start-group:abc:Yol".to_string();
        Library {
            liked_count: 2,
            albums: vec![item(CollectionId::Album("spotify:album:a".into()), "Albüm")],
            playlists: vec![item(CollectionId::Playlist(pl(1)), "Bir"), item(CollectionId::Playlist(pl(2)), "İki")],
            artists: vec![crate::engine::library::artist_item("spotify:artist:a", "Mavi Orman", "")],
            meta: LibraryMeta {
                folders: vec![Folder { key: folder.clone(), name: "Yol".into(), parent: None, at: 1 }],
                parents: [(pl(2), folder)].into(),
                added: [(pl(1), 1_700_000_000_000)].into(),
                creators: [(pl(1), "furka".to_string())].into(),
                pinned: vec![pl(1), "liked".into()],
            },
        }
    }

    #[test]
    fn json_has_every_part_of_the_library() {
        let lib = library();
        let contents: HashMap<String, Vec<Entry>> = [
            ("liked".to_string(), vec![Entry::untimed(row(1).uri), Entry { track: row(2).uri, added_at: Some(5) }]),
            (pl(1), vec![Entry::untimed(row(1).uri), Entry::untimed(row(3).uri)]),
        ]
        .into();
        let info: HashMap<String, TrackRow> = [row(1), row(2)].into_iter().map(|r| (r.uri.clone(), r)).collect();
        let (v, s) = build(1_794_400_496_000, 0, &lib, &contents, &info);
        assert_eq!(
            (s.lists, s.tracks, s.albums, s.folders, s.artists, s.pinned, s.missing),
            (2, 4, 1, 1, 1, 2, 1)
        );
        assert_eq!(v["app"], "Openify");
        assert_eq!(v["format"], FORMAT);
        assert_eq!(v["exported_date"], "2026-11-11");
        // Beğenilenler: adresler, bilinen şarkıda ad, eklenme anı.
        assert_eq!(v["liked"]["tracks"][1]["added_at"], 5);
        assert_eq!(v["liked"]["tracks"][0]["name"], "Şarkı 1");
        // Liste: sahip, eklenme, sabit; klasörde olmayan null.
        let p1 = &v["playlists"][0];
        assert_eq!((p1["creator"].as_str(), p1["pinned"].as_bool(), p1["tracks_cached"].as_bool()), (Some("furka"), Some(true), Some(true)));
        assert_eq!(p1["added_at"], 1_700_000_000_000i64);
        assert!(p1["folder"].is_null());
        // Önbellekte bilgisi olmayan şarkı yalnız adresiyle.
        assert_eq!(p1["tracks"][1], json!({ "uri": row(3).uri }));
        // Şarkıları olmayan liste yalnız adıyla; klasörde.
        let p2 = &v["playlists"][1];
        assert_eq!(p2["tracks_cached"], false);
        assert!(p2.get("tracks").is_none());
        assert_eq!(p2["folder"], "spotify:start-group:abc:Yol");
        assert_eq!(v["folders"][0]["name"], "Yol");
        assert_eq!(v["artists"][0]["uri"], "spotify:artist:a");
        assert_eq!(v["pinned"], json!([pl(1), "liked"]));
        assert_eq!(v["albums"][0]["uri"], "spotify:album:a");
    }

    #[test]
    fn export_reads_the_cache_and_writes_a_file_atomically() {
        let dir = std::env::temp_dir().join(format!("openify-backup-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let db = Db::open_in_memory().unwrap();
        db.save_library(&library()).unwrap();
        db.save_tracks(&[row(1), row(2)]).unwrap();
        db.save_collection_entries(&CollectionId::Liked, Some(b"1"), &[Entry::untimed(row(2).uri)]).unwrap();
        db.save_collection_entries(
            &CollectionId::Playlist(pl(1)),
            Some(b"1"),
            &[Entry::untimed(row(1).uri), Entry::untimed(row(2).uri)],
        )
        .unwrap();
        // Üst klasör yok: oluşturulur.
        let path = dir.join("alt").join("yedek.json");
        let s = export(&db, &path, 1_794_400_496_000, 0).unwrap();
        assert_eq!((s.lists, s.tracks, s.missing, s.path.clone()), (2, 3, 1, path.clone()));
        let text = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["playlists"][0]["tracks"][0]["name"], "Şarkı 1");
        assert!(!path.with_extension("json.tmp").exists(), "geçici dosya kalmaz");
        // İkinci yazım dosyanın üstüne yazar.
        export(&db, &path, 1_794_400_496_000, 0).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_without_a_cached_library_fails_cleanly() {
        let db = Db::open_in_memory().unwrap();
        let path = std::env::temp_dir().join("openify-backup-yok.json");
        assert!(export(&db, &path, 0, 0).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn names_and_fallback_location() {
        assert_eq!(suggested_name(1_794_400_496_000, 0), "openify-backup-2026-11-11.json");
        let p = fallback_path(1_794_400_496_000, 0);
        assert!(p.ends_with("Openify yedekleri/openify-backup-2026-11-11.json") || p.ends_with("Openify yedekleri\\openify-backup-2026-11-11.json"), "{p:?}");
    }

    /// Risk ölçümü: 100 liste x 200 şarkı (20 bin adres) yedeği. Çıta: < 2 sn.
    #[test]
    fn a_twenty_thousand_track_backup_is_quick() {
        let db = Db::open_in_memory().unwrap();
        let mut lib = Library { liked_count: 0, ..Default::default() };
        let rows: Vec<TrackRow> = (0..2000).map(row).collect();
        db.save_tracks(&rows).unwrap();
        for p in 0..100u32 {
            lib.playlists.push(item(CollectionId::Playlist(pl(p)), &format!("Liste {p}")));
        }
        db.save_library(&lib).unwrap();
        for p in 0..100u32 {
            let entries: Vec<Entry> = (0..200u32).map(|i| Entry::untimed(row((p * 7 + i) % 2000).uri)).collect();
            db.save_collection_entries(&CollectionId::Playlist(pl(p)), Some(b"1"), &entries).unwrap();
        }
        let path = std::env::temp_dir().join(format!("openify-backup-perf-{}.json", std::process::id()));
        let t = std::time::Instant::now();
        let s = export(&db, &path, 1_794_400_496_000, 0).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.;
        let bytes = std::fs::metadata(&path).unwrap().len();
        let _ = std::fs::remove_file(&path);
        println!("ölçüm: 100 liste / 20000 adres yedek {ms:.1} ms, {:.1} MB", bytes as f64 / 1e6);
        assert_eq!((s.lists, s.tracks), (100, 20_000));
        assert!(ms < 2000., "{ms} ms");
    }
}
