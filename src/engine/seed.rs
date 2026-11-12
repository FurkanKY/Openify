//! Ekran sınaması için demo kütüphane (yalnız sınamada derlenir).
//!
//! Gerçek `%LOCALAPPDATA%\Openify` önbelleğine dokunmadan, çevrimdışı açılan
//! bir kopyanın (`OPENIFY_TEST_OFFLINE=1`) kütüphane görmesi için geçici bir
//! `library.db` yazar:
//!
//! ```text
//! $env:OPENIFY_SEED_DIR = "<boş geçici klasör>"
//! cargo test --release --target-dir target/liste seed_demo_library -- --ignored
//! # sonra uygulama: OPENIFY_TEST_CACHE_DIR=<aynı klasör>
//! ```
//!
//! Adlar uydurma; kapak yok (yer tutucular çizilir, ağa çıkılmaz). Kütüphane
//! düzeni için: iç içe klasörler, sabitlenmiş öğeler, eklenme anları
//! (şimdiye göre: "2 dk önce" … "1 yıl önce") ve Beğenilenler ile bir listenin
//! şarkıları eklenme tarihleriyle yazılır.

#[cfg(test)]
mod demo {
    use crate::engine::{
        cache::Db,
        library::{CollectionId, Entry, Folder, Library, LibraryItem, LibraryMeta, TrackRow},
    };

    fn item(id: CollectionId, name: &str, subtitle: &str) -> LibraryItem {
        LibraryItem {
            id,
            name: name.into(),
            subtitle: subtitle.into(),
            image: String::new(),
        }
    }

    fn track(n: u32, name: &str, artists: &str, album: &str, minutes: u32) -> TrackRow {
        TrackRow {
            uri: format!("spotify:track:{n:022}"),
            name: name.into(),
            artists: artists.into(),
            album: album.into(),
            duration_ms: minutes * 1000 + 61_000,
            image: String::new(),
        }
    }

    #[test]
    #[ignore = "ekran sınaması için elle çalıştırılır (OPENIFY_SEED_DIR)"]
    fn seed_demo_library() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("OPENIFY_SEED_DIR").expect("OPENIFY_SEED_DIR verilmedi"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let playlist_key = |n: u32| format!("spotify:playlist:{n:022}");
        let playlist = |n: u32, name: &str, subtitle: &str| {
            item(CollectionId::Playlist(playlist_key(n)), name, subtitle)
        };
        let album = |n: u32, name: &str, subtitle: &str| {
            item(
                CollectionId::Album(format!("spotify:album:{n:022}")),
                name,
                subtitle,
            )
        };
        let artist = |n: u32, name: &str| {
            crate::engine::library::artist_item(&format!("spotify:artist:{n:022}"), name, "")
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let ago = |minutes: i64| now - minutes * 60_000;
        const HOUR: i64 = 60;
        const DAY: i64 = 24 * HOUR;

        // Kök listenin sırası: Gece sürüşü | [Yol: Yol şarkıları, Pazar sabahı, [Spor: Egzersiz]]
        // | [Çalışma: Odaklanma] | Discover Weekly | Daily Mix 1.
        let playlists = vec![
            playlist(1, "Gece sürüşü", "42 şarkı"),
            playlist(2, "Yol şarkıları", "Takip edilen · 58 şarkı"),
            playlist(3, "Pazar sabahı", "19 şarkı"),
            playlist(4, "Egzersiz", "64 şarkı"),
            playlist(5, "Odaklanma", "31 şarkı"),
            playlist(6, "Discover Weekly", "Spotify · 30 şarkı"),
            playlist(7, "Daily Mix 1", "Spotify · 50 şarkı"),
            // Kütüphane araçları sınaması: yinelenenli, küçük ve büyük (≥ 50 değişiklik) listeler.
            playlist(8, "Eski favoriler", "9 şarkı"),
            playlist(9, "Büyük karışık", "120 şarkı"),
        ];
        let yol = "spotify:start-group:0a1b2c3d4e5f6a7b:Yol".to_string();
        let spor = "spotify:start-group:1b2c3d4e5f6a7b8c:Spor".to_string();
        let calisma = "spotify:start-group:2c3d4e5f6a7b8c9d:Çalışma".to_string();
        let meta = LibraryMeta {
            folders: vec![
                Folder {
                    key: yol.clone(),
                    name: "Yol".into(),
                    parent: None,
                    at: 1,
                },
                Folder {
                    key: spor.clone(),
                    name: "Spor".into(),
                    parent: Some(yol.clone()),
                    at: 3,
                },
                Folder {
                    key: calisma.clone(),
                    name: "Çalışma".into(),
                    parent: None,
                    at: 4,
                },
            ],
            parents: [
                (playlist_key(2), yol.clone()),
                (playlist_key(3), yol.clone()),
                (playlist_key(4), spor.clone()),
                (playlist_key(5), calisma.clone()),
            ]
            .into(),
            added: [
                (playlist_key(1), ago(3 * DAY)),
                (playlist_key(2), ago(40 * DAY)),
                (playlist_key(3), ago(9 * DAY)),
                (playlist_key(4), ago(200 * DAY)),
                (playlist_key(5), ago(5 * HOUR)),
                (playlist_key(6), ago(400 * DAY)),
                (playlist_key(7), ago(120 * DAY)),
                (playlist_key(8), ago(500 * DAY)),
                (playlist_key(9), ago(30 * DAY)),
                (
                    "spotify:album:0000000000000000000001".to_string(),
                    ago(20 * DAY),
                ),
                ("spotify:album:0000000000000000000002".to_string(), ago(2)),
                (
                    "spotify:album:0000000000000000000003".to_string(),
                    ago(300 * DAY),
                ),
                (
                    "spotify:album:0000000000000000000004".to_string(),
                    ago(60 * DAY),
                ),
            ]
            .into(),
            creators: [
                (playlist_key(1), "furka".to_string()),
                (playlist_key(2), "ayse".to_string()),
                (playlist_key(3), "furka".to_string()),
                (playlist_key(4), "furka".to_string()),
                (playlist_key(5), "furka".to_string()),
                (playlist_key(6), "spotify".to_string()),
                (playlist_key(7), "spotify".to_string()),
                (playlist_key(8), "furka".to_string()),
                (playlist_key(9), "furka".to_string()),
            ]
            .into(),
            pinned: Vec::new(),
        };
        let library = Library {
            liked_count: 214,
            albums: vec![
                album(1, "Uzun Yol", "Mavi Orman"),
                album(2, "Kuzey Işıkları", "Kuzey"),
                album(3, "Fallen", "Evanescence"),
                album(4, "Sessiz Ev", "Deniz Aydın"),
            ],
            playlists,
            artists: vec![
                artist(1, "Mavi Orman"),
                artist(2, "Deniz Aydın"),
                artist(3, "Kuzey"),
            ],
            meta,
        };
        let db = Db::open(&dir.join("library.db")).unwrap();
        db.save_library(&library).unwrap();
        // Sabitlenenler: Odaklanma (klasördeki liste) ve bir albüm.
        db.set_pinned(&playlist_key(5), true, 1).unwrap();
        db.set_pinned("spotify:album:0000000000000000000002", true, 2)
            .unwrap();

        // Şarkılar ve eklenme anları: Beğenilenler ile "Gece sürüşü".
        let rows = [
            track(1, "Sessiz Gece", "Mavi Orman", "Uzun Yol", 12),
            track(2, "Kuzeye Doğru", "Kuzey", "Kuzey Işıkları", 24),
            track(3, "Bring Me to Life", "Evanescence", "Fallen", 3),
            track(4, "Eski Radyo", "Deniz Aydın", "Sessiz Ev", 41),
            track(5, "Sabah Sisi", "Mavi Orman", "Uzun Yol", 8),
            track(6, "Ayna", "Deniz Aydın", "Sessiz Ev", 55),
            track(7, "Aurora", "Kuzey", "Kuzey Işıkları", 17),
            track(8, "My Immortal", "Evanescence", "Fallen", 30),
            track(9, "Yağmur Sonrası", "Mavi Orman", "Uzun Yol", 21),
            track(10, "Limanda", "Deniz Aydın", "Sessiz Ev", 4),
        ];
        // Aynı şarkıların sürümleri (yinelenen bulucu): Remaster, Live, Radio Edit.
        let versions = [
            track(
                11,
                "Sessiz Gece - 2011 Remaster",
                "Mavi Orman",
                "Uzun Yol (Deluxe)",
                12,
            ),
            track(
                12,
                "Bring Me to Life (Live)",
                "Evanescence",
                "Anywhere but Home",
                40,
            ),
            track(13, "Ayna - Radio Edit", "Deniz Aydın", "Sessiz Ev", 55),
            track(14, "Aurora", "Kuzey", "Kuzey Işıkları (Yeniden)", 18),
        ];
        let mut saved = rows.to_vec();
        saved.extend(versions.iter().cloned());
        db.save_tracks(&saved).unwrap();
        let ages = [
            2,
            5 * HOUR,
            DAY,
            3 * DAY,
            9 * DAY,
            20 * DAY,
            45 * DAY,
            200 * DAY,
            400 * DAY,
            800 * DAY,
        ];
        let entries: Vec<Entry> = rows
            .iter()
            .zip(ages)
            .map(|(r, a)| Entry {
                track: r.uri.clone(),
                added_at: Some(ago(a)),
            })
            .collect();
        db.save_collection_entries(&CollectionId::Liked, Some(b"1"), &entries)
            .unwrap();
        // "Gece sürüşü": aynı şarkı iki kez (Sessiz Gece, Ayna) ve sürümler (Remaster, Live, Radio Edit).
        let at = |m: i64| Some(ago(m));
        let night_rows = [
            (&rows[0], at(3 * DAY)),
            (&rows[3], at(3 * DAY)),
            (&versions[0], at(2 * DAY)),
            (&rows[5], at(2 * DAY)),
            (&rows[0], at(DAY)),
            (&rows[2], at(DAY)),
            (&versions[1], at(5 * HOUR)),
            (&rows[7], at(5 * HOUR)),
            (&rows[5], at(HOUR)),
            (&versions[2], at(HOUR)),
            (&rows[8], at(10)),
            (&versions[3], at(5)),
            (&rows[6], at(2)),
        ];
        let night: Vec<Entry> = night_rows
            .iter()
            .map(|(r, added)| Entry {
                track: r.uri.clone(),
                added_at: *added,
            })
            .collect();
        db.save_collection_entries(&CollectionId::Playlist(playlist_key(1)), Some(b"1"), &night)
            .unwrap();
        // "Eski favoriler": ikinci küçük liste (birleştirme için); Gece sürüşü ile ortak şarkılar var.
        let old: Vec<Entry> = [0usize, 2, 4, 5, 7, 9, 3, 1, 8]
            .iter()
            .enumerate()
            .map(|(n, i)| Entry {
                track: rows[*i].uri.clone(),
                added_at: Some(ago((200 + 30 * n as i64) * DAY)),
            })
            .collect();
        db.save_collection_entries(&CollectionId::Playlist(playlist_key(8)), Some(b"1"), &old)
            .unwrap();
        // "Büyük karışık": 120 satır, 10 şarkı 12'şer kez (≥ 50 değişiklik: toplu işlem onayı).
        let big: Vec<Entry> = (0..120)
            .map(|n| Entry {
                track: rows[(n * 7) % 10].uri.clone(),
                added_at: Some(ago(10 + n as i64 * 450)),
            })
            .collect();
        db.save_collection_entries(&CollectionId::Playlist(playlist_key(9)), Some(b"1"), &big)
            .unwrap();

        // Gizlenen şarkılar (radyo / gizleme sınaması): "Gece sürüşü"nde iki şarkı bu listede,
        // biri hiçbir yerde çalınmayacak (Beğenilenler'de de soluk görünür).
        let hide = |row: &TrackRow, context: Option<String>, minutes: i64| {
            db.hide_song(&crate::engine::HiddenSong {
                uri: row.uri.clone(),
                context,
                name: row.name.clone(),
                artists: row.artists.clone(),
                hidden_at: ago(minutes),
            })
            .unwrap();
        };
        hide(&rows[3], Some(playlist_key(1)), 30);
        hide(&versions[2], Some(playlist_key(1)), 20);
        hide(&rows[7], None, 10);

        // Yerel çalma geçmişi (istatistik): son 8 ayda ~180 çalma; akşamlar yoğun, bazı şarkılar çok.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |m: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % m
        };
        let all: Vec<&TrackRow> = saved.iter().collect();
        for n in 0..180u64 {
            // Ağırlık: ilk şarkılar daha sık.
            let pick = (next(100) * next(100) / 100 * all.len() as u64 / 100) as usize;
            let r = all[pick.min(all.len() - 1)];
            // Gün: bugünden 240 güne kadar (yakın günler daha sık); saat: akşam ağırlıklı.
            let days = next(240) * next(240) / 240;
            let hour = [
                8u64, 9, 13, 17, 18, 19, 20, 20, 21, 21, 21, 22, 22, 23, 0, 12,
            ][next(16) as usize];
            let minute = next(60);
            let day_ms = 24 * 60 * 60_000i64;
            let off = crate::engine::tools::civil::local_offset_minutes() * 60_000;
            let midnight = (now + off).div_euclid(day_ms) * day_ms - off;
            let played_at =
                midnight - days as i64 * day_ms + (hour * 60 + minute) as i64 * 60_000 - n as i64;
            let listened = (r.duration_ms as u64).min(30_000 + next(r.duration_ms as u64));
            db.record_play(&crate::engine::tools::stats::PlayRecord {
                uri: r.uri.clone(),
                name: r.name.clone(),
                artists: r.artists.clone(),
                album: r.album.clone(),
                duration_ms: r.duration_ms,
                listened_ms: listened,
                played_at: played_at.min(now),
            })
            .unwrap();
        }
    }
}
