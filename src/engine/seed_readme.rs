//! README ekran görüntüleri için İngilizce demo kütüphane (yalnız sınamada derlenir).
//!
//! `seed.rs`'in Türkçe sınama kütüphanesinden ayrı: bütün adlar uydurma (gerçek
//! sanatçı, şarkı ya da kullanıcı adı yok), kapaklar kodla üretilmiş soyut
//! resimler (ağa çıkılmaz, telif yok). Kapaklar `<önbellek>\covers\` altına
//! yazılır, bu yüzden `OPENIFY_SEED_DIR` ile `OPENIFY_TEST_CACHE_DIR` aynı
//! klasör olmalı:
//!
//! ```text
//! $env:OPENIFY_SEED_DIR = $env:OPENIFY_TEST_CACHE_DIR = "<boş geçici klasör>"
//! cargo test --release --target-dir target/liste seed_readme_library -- --ignored
//! ```

#[cfg(test)]
mod readme {
    use std::sync::Arc;

    use crate::engine::{
        cache::Db,
        cover_path,
        explore::{
            CardKind, Category, CategoryList, ExploreCard, ExploreKey, ExploreSection, Feed,
        },
        library::{CollectionId, Entry, Folder, Library, LibraryItem, LibraryMeta, TrackRow},
        pages::{AlbumCard, AlbumPage, ArtistCard, ArtistPage, PageCache, ReleaseKind},
    };
    use crate::i18n::Lang;

    /// (albüm, sanatçı, [(şarkı, saniye)])
    const ALBUMS: [(&str, &str, [(&str, u32); 5]); 8] = [
        (
            "Glass Hours",
            "Night Harbor",
            [
                ("Glass Hours", 224),
                ("Low Light", 198),
                ("Harbor Lights", 241),
                ("Slow Signal", 207),
                ("After the Rain", 263),
            ],
        ),
        (
            "Salt & Neon",
            "Cobalt Static",
            [
                ("Salt & Neon", 187),
                ("Electric Tide", 215),
                ("Night Shift", 232),
                ("Static Bloom", 196),
                ("Afterglow Drive", 254),
            ],
        ),
        (
            "Slow Fires",
            "Ember & Ash",
            [
                ("Slow Fires", 248),
                ("Paper Moon", 203),
                ("Cinder Road", 221),
                ("Warm Hands", 189),
                ("Last Ember", 276),
            ],
        ),
        (
            "Orchard Letters",
            "Velvet Orchard",
            [
                ("Orchard Letters", 213),
                ("Dear September", 236),
                ("Honey and Rust", 192),
                ("Small Windows", 208),
                ("Quiet Harvest", 247),
            ],
        ),
        (
            "Lantern Season",
            "Paper Lanterns",
            [
                ("Lantern Season", 201),
                ("Kites at Dusk", 229),
                ("River Song", 184),
                ("Silk Road Home", 238),
                ("Soft Departures", 215),
            ],
        ),
        (
            "Hollow Roads",
            "Juniper Hollow",
            [
                ("Hollow Roads", 226),
                ("Gravel and Gold", 205),
                ("Northbound", 243),
                ("Wide Open", 191),
                ("Porch Light", 258),
            ],
        ),
        (
            "Tidelines",
            "Low Tide Club",
            [
                ("Tidelines", 194),
                ("Sea Glass", 218),
                ("Salt Air", 207),
                ("Undertow", 233),
                ("Morning Swim", 181),
            ],
        ),
        (
            "Parade of One",
            "The Quiet Parade",
            [
                ("Parade of One", 211),
                ("Marching Band Blues", 195),
                ("Paper Crown", 227),
                ("Nobody Knows", 202),
                ("Confetti Rain", 249),
            ],
        ),
    ];

    fn album_url(n: usize) -> String {
        format!("https://demo.invalid/covers/album-{n}")
    }

    fn artist_url(n: usize) -> String {
        format!("https://demo.invalid/covers/artist-{n}")
    }

    // ── Soyut kapak üretimi ─────────────────────────────────────────────

    /// HSL → RGB (0..1).
    fn hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
        let h = h.rem_euclid(360.0) / 60.0;
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        [r + m, g + m, b + m]
    }

    fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
        let t = t.clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    }

    fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// 256 px'lik soyut kapak: iki ton eğim + stile göre şekiller. `style`
    /// 0 güneş, 1 halkalar, 2 çapraz bantlar, 3 kesişen daireler, 4 dalga,
    /// 5 yarım ay, 6 ızgara noktaları, 7 ufuk.
    fn cover_jpeg(seed: usize, round: bool) -> Vec<u8> {
        const N: u32 = 256;
        let hue = [
            8.0, 205.0, 28.0, 340.0, 168.0, 262.0, 188.0, 48.0, 312.0, 128.0,
        ][seed % 10];
        let a = hsl(hue, 0.62, 0.36);
        let b = hsl(hue + 38.0, 0.72, 0.14);
        let accent = hsl(hue + 150.0 + (seed as f32 * 11.0) % 40.0, 0.78, 0.66);
        let soft = hsl(hue + 18.0, 0.55, 0.78);
        let style = seed % 8;
        let mut img = image::RgbImage::new(N, N);
        for (px, py, pixel) in img.enumerate_pixels_mut() {
            let (x, y) = (px as f32 / N as f32, py as f32 / N as f32);
            let mut col = mix(a, b, (x * 0.55 + y * 0.85).clamp(0.0, 1.0));
            let (cx, cy) = (x - 0.5, y - 0.5);
            let d = (cx * cx + cy * cy).sqrt();
            match style {
                0 => {
                    let sun = (x - 0.5).powi(2) + (y - 0.62).powi(2);
                    col = mix(col, accent, 1.0 - smooth(0.17, 0.175, sun.sqrt()));
                    col = mix(col, b, smooth(0.62, 0.625, y) * 0.9);
                }
                1 => {
                    let rings = ((d * 22.0).sin() * 0.5 + 0.5) * (1.0 - smooth(0.2, 0.62, d));
                    col = mix(col, accent, rings * 0.8);
                }
                2 => {
                    let band = ((x + y) * 9.0).fract();
                    col = mix(
                        col,
                        soft,
                        smooth(0.0, 0.04, band) * (1.0 - smooth(0.34, 0.38, band)) * 0.55,
                    );
                    col = mix(col, accent, 1.0 - smooth(0.2, 0.205, d));
                }
                3 => {
                    let d1 = ((x - 0.40).powi(2) + (y - 0.46).powi(2)).sqrt();
                    let d2 = ((x - 0.60).powi(2) + (y - 0.54).powi(2)).sqrt();
                    col = mix(col, accent, (1.0 - smooth(0.25, 0.255, d1)) * 0.85);
                    col = mix(col, soft, (1.0 - smooth(0.25, 0.255, d2)) * 0.6);
                }
                4 => {
                    for k in 0..4 {
                        let wave = 0.40 + k as f32 * 0.14 + (x * 7.0 + k as f32).sin() * 0.05;
                        col = mix(
                            col,
                            mix(accent, soft, k as f32 / 3.0),
                            (1.0 - smooth(0.0, 0.012, (y - wave).abs())) * 0.9,
                        );
                    }
                }
                5 => {
                    let moon = ((x - 0.5).powi(2) + (y - 0.45).powi(2)).sqrt();
                    let bite = ((x - 0.60).powi(2) + (y - 0.40).powi(2)).sqrt();
                    let m = (1.0 - smooth(0.24, 0.245, moon)) * smooth(0.21, 0.215, bite);
                    col = mix(col, accent, m);
                }
                6 => {
                    let (gx, gy) = ((x * 8.0).fract() - 0.5, (y * 8.0).fract() - 0.5);
                    let dot = (gx * gx + gy * gy).sqrt();
                    let big = 0.18 + 0.2 * (1.0 - smooth(0.0, 0.7, d));
                    col = mix(col, accent, (1.0 - smooth(big, big + 0.04, dot)) * 0.8);
                }
                _ => {
                    col = mix(col, soft, smooth(0.52, 0.56, y) * 0.35);
                    col = mix(
                        col,
                        accent,
                        (1.0 - smooth(0.0, 0.01, (y - 0.54).abs())) * 0.9,
                    );
                    let sun = ((x - 0.5).powi(2) + (y - 0.4).powi(2)).sqrt();
                    col = mix(col, accent, (1.0 - smooth(0.12, 0.125, sun)) * 0.9);
                }
            }
            if round {
                // Portre: yüz yerine sade yarım gövde silueti (yuvarlak kırpılacak).
                let head = (x - 0.5).powi(2) + ((y - 0.38) * 1.05).powi(2);
                let body = (x - 0.5).powi(2) / 0.09 + ((y - 1.0) / 0.5).powi(2);
                let sil = hsl(hue, 0.35, 0.10);
                col = mix(col, sil, 1.0 - smooth(0.012, 0.0125, head));
                col = mix(col, sil, 1.0 - smooth(0.99, 1.0, body));
            }
            // İnce dokuyu bandlaşmayı kırmak için: sabit, belirleyici gürültü.
            let n = (((px * 73856093) ^ (py * 19349663) ^ (seed as u32 * 83492791)) % 255) as f32
                / 255.0;
            let g = (n - 0.5) * 0.018;
            *pixel = image::Rgb([
                ((col[0] + g).clamp(0.0, 1.0) * 255.0) as u8,
                ((col[1] + g).clamp(0.0, 1.0) * 255.0) as u8,
                ((col[2] + g).clamp(0.0, 1.0) * 255.0) as u8,
            ]);
        }
        let mut out = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
        enc.encode_image(&img).unwrap();
        out
    }

    fn write_cover(url: &str, bytes: &[u8]) {
        let path = cover_path(url).expect("OPENIFY_TEST_CACHE_DIR verilmedi");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Sayfa kartları için ek kapaklar (Keşfet, sanatçı sayfası).
    const EXTRA: usize = 36;

    fn extra_url(n: usize) -> String {
        format!("https://demo.invalid/covers/extra-{}", n % EXTRA)
    }

    fn artist_uri(a: usize) -> String {
        format!("spotify:artist:{:022}", a + 1)
    }

    fn album_uri(a: usize) -> String {
        format!("spotify:album:{:022}", a + 1)
    }

    fn track_uri(album: usize, n: usize) -> String {
        format!("spotify:track:{:022}", album * 10 + n + 1)
    }

    #[test]
    #[ignore = "README ekran görüntüleri için elle çalıştırılır (OPENIFY_SEED_DIR + OPENIFY_TEST_CACHE_DIR)"]
    fn seed_readme_library() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("OPENIFY_SEED_DIR").expect("OPENIFY_SEED_DIR verilmedi"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            std::env::var_os("OPENIFY_TEST_CACHE_DIR").map(std::path::PathBuf::from),
            Some(dir.clone()),
            "OPENIFY_TEST_CACHE_DIR, OPENIFY_SEED_DIR ile aynı olmalı (kapaklar önbelleğe yazılır)"
        );
        for (i, _) in ALBUMS.iter().enumerate() {
            write_cover(&album_url(i), &cover_jpeg(i * 3 + 1, false));
            write_cover(&artist_url(i), &cover_jpeg(i * 3 + 2, true));
        }
        for n in 0..EXTRA {
            write_cover(&extra_url(n), &cover_jpeg(n * 5 + 7, false));
        }

        let playlist_key = |n: u32| format!("spotify:playlist:{n:022}");
        let album_key = |n: usize| format!("spotify:album:{:022}", n + 1);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let ago = |minutes: i64| now - minutes * 60_000;
        const HOUR: i64 = 60;
        const DAY: i64 = 24 * HOUR;

        // Bütün şarkılar (albüm × 5), kapak = albüm kapağı.
        let mut rows: Vec<TrackRow> = Vec::new();
        for (a, (album, artist, tracks)) in ALBUMS.iter().enumerate() {
            for (n, (name, secs)) in tracks.iter().enumerate() {
                rows.push(TrackRow {
                    uri: track_uri(a, n),
                    name: (*name).into(),
                    artists: (*artist).into(),
                    album: (*album).into(),
                    duration_ms: secs * 1000,
                    image: album_url(a),
                });
            }
        }
        // Aynı şarkıların sürümleri (Araçlar, yinelenen bulucu): Remastered, Live, Radio Edit, Mix.
        let version =
            |n: usize, name: &str, album: &str, from: usize, secs: u32, cover: usize| TrackRow {
                uri: format!("spotify:track:{:022}", 900 + n),
                name: name.into(),
                artists: ALBUMS[from].1.into(),
                album: album.into(),
                duration_ms: secs * 1000,
                image: album_url(cover),
            };
        rows.push(version(
            1,
            "Glass Hours (Remastered)",
            "Glass Hours (Deluxe)",
            0,
            226,
            0,
        ));
        rows.push(version(
            2,
            "Salt & Neon (Live)",
            "Live at the Pier",
            1,
            203,
            1,
        ));
        rows.push(version(
            3,
            "Night Shift - Radio Edit",
            "Salt & Neon",
            1,
            189,
            1,
        ));
        rows.push(version(
            4,
            "Low Light (2024 Mix)",
            "Glass Hours (Deluxe)",
            0,
            201,
            0,
        ));
        let base = ALBUMS.len() * 5; // sürümlerin başlangıç sırası
        let by = |album: usize, n: usize| &rows[album * 5 + n];

        let playlist_defs: [(u32, &str, usize, &str); 10] = [
            (1, "Night Drive", 26, "demo"),
            (2, "Road Trip", 31, "demo"),
            (3, "Sunday Morning", 18, "demo"),
            (4, "Workout", 34, "demo"),
            (5, "Deep Work", 22, "demo"),
            (6, "Discover Weekly", 30, "spotify"),
            (7, "Daily Mix 1", 50, "spotify"),
            (8, "Rainy Day", 15, "demo"),
            (9, "Old Favorites", 9, "demo"),
            (10, "Big Mix", 120, "demo"),
        ];
        let playlists: Vec<LibraryItem> = playlist_defs
            .iter()
            .map(|(n, name, count, owner)| LibraryItem {
                id: CollectionId::Playlist(playlist_key(*n)),
                name: (*name).into(),
                // Kanonik (Türkçe) biçim: arayüz dile göre çevirir (`i18n::canon`).
                subtitle: if *owner == "spotify" {
                    crate::i18n::canon::spotify(*count)
                } else {
                    crate::i18n::canon::songs(*count)
                },
                image: String::new(),
            })
            .collect();
        let travel = "spotify:start-group:0a1b2c3d4e5f6a7b:Travel".to_string();
        let focus = "spotify:start-group:2c3d4e5f6a7b8c9d:Focus".to_string();
        let meta = LibraryMeta {
            folders: vec![
                Folder {
                    key: travel.clone(),
                    name: "Travel".into(),
                    parent: None,
                    at: 1,
                },
                Folder {
                    key: focus.clone(),
                    name: "Focus".into(),
                    parent: None,
                    at: 4,
                },
            ],
            parents: [
                (playlist_key(2), travel.clone()),
                (playlist_key(3), travel.clone()),
                (playlist_key(5), focus.clone()),
            ]
            .into(),
            added: playlist_defs
                .iter()
                .map(|(n, ..)| (playlist_key(*n), ago((*n as i64 * 17 + 3) * DAY)))
                .chain((0..ALBUMS.len()).map(|a| (album_key(a), ago((a as i64 * 23 + 4) * DAY))))
                .collect(),
            creators: playlist_defs
                .iter()
                .map(|(n, _, _, owner)| (playlist_key(*n), (*owner).to_string()))
                .collect(),
            descriptions: Default::default(),
            collaborative: Default::default(),
            pinned: Vec::new(),
        };
        let library = Library {
            liked_count: 214,
            albums: [0usize, 1, 3, 4, 5, 7]
                .iter()
                .map(|&a| LibraryItem {
                    id: CollectionId::Album(album_key(a)),
                    name: ALBUMS[a].0.into(),
                    subtitle: ALBUMS[a].1.into(),
                    image: album_url(a),
                })
                .collect(),
            playlists,
            artists: (0..ALBUMS.len())
                .map(|a| {
                    crate::engine::library::artist_item(
                        &format!("spotify:artist:{:022}", a + 1),
                        ALBUMS[a].1,
                        &artist_url(a),
                    )
                })
                .collect(),
            meta,
        };
        let db = Db::open(&dir.join("library.db")).unwrap();
        db.save_library(&library).unwrap();
        db.set_pinned(&playlist_key(1), true, 1).unwrap();
        db.set_pinned(&album_key(0), true, 2).unwrap();
        db.save_tracks(&rows).unwrap();

        // Beğenilenler: 28 şarkı, eklenme anları yakından uzağa.
        let liked_ages = [
            2,
            40,
            5 * HOUR,
            DAY,
            2 * DAY,
            3 * DAY,
            6 * DAY,
            9 * DAY,
            14 * DAY,
            20 * DAY,
        ];
        let liked: Vec<Entry> = (0..28)
            .map(|i| {
                let row = &rows[(i * 7 + 3) % rows.len()];
                let age = liked_ages
                    .get(i)
                    .copied()
                    .unwrap_or((30 + i as i64 * 11) * DAY);
                Entry {
                    track: row.uri.clone(),
                    added_at: Some(ago(age)),
                }
            })
            .collect();
        db.save_collection_entries(&CollectionId::Liked, Some(b"1"), &liked)
            .unwrap();
        // Listeler: her biri farklı bir kaydırmayla şarkı alır.
        for (k, (n, _, count, _)) in playlist_defs.iter().enumerate() {
            let entries: Vec<Entry> = (0..*count)
                .map(|i| {
                    let row = &rows[(i * (k + 3) + k * 5) % (rows.len() - 4)];
                    Entry {
                        track: row.uri.clone(),
                        added_at: Some(ago((2 + k as i64) * DAY + i as i64 * 600)),
                    }
                })
                .collect();
            db.save_collection_entries(
                &CollectionId::Playlist(playlist_key(*n)),
                Some(b"1"),
                &entries,
            )
            .unwrap();
        }
        // "Night Drive": bilerek yinelenenli ve sürümlü (aynı şarkı iki kez, Remastered / Live / Radio Edit).
        let at = |m: i64| Some(ago(m));
        let night_picks: Vec<(&TrackRow, Option<i64>)> = vec![
            (by(0, 0), at(3 * DAY)),
            (by(1, 2), at(3 * DAY)),
            (&rows[base], at(2 * DAY)),
            (by(0, 1), at(2 * DAY)),
            (by(0, 0), at(DAY)),
            (by(2, 0), at(DAY)),
            (&rows[base + 1], at(5 * HOUR)),
            (by(1, 0), at(5 * HOUR)),
            (by(1, 2), at(HOUR)),
            (&rows[base + 2], at(HOUR)),
            (by(3, 1), at(40)),
            (&rows[base + 3], at(25)),
            (by(4, 0), at(10)),
            (by(5, 2), at(5)),
            (by(6, 1), at(4)),
            (by(7, 4), at(3)),
            (by(2, 3), at(2)),
            (by(3, 4), at(2)),
            (by(5, 0), at(2)),
            (by(0, 3), at(2)),
            (by(1, 4), at(2)),
            (by(4, 2), at(2)),
            (by(6, 3), at(2)),
            (by(7, 1), at(2)),
            (by(2, 4), at(2)),
            (by(3, 0), at(2)),
        ];
        let night: Vec<Entry> = night_picks
            .iter()
            .map(|(r, added)| Entry {
                track: r.uri.clone(),
                added_at: *added,
            })
            .collect();
        db.save_collection_entries(&CollectionId::Playlist(playlist_key(1)), Some(b"1"), &night)
            .unwrap();
        // "Old Favorites": Night Drive ile ortak şarkılar (Araçlar, birleştir).
        let old: Vec<Entry> = [
            (0usize, 0usize),
            (1, 2),
            (0, 1),
            (2, 0),
            (3, 1),
            (4, 0),
            (5, 2),
            (6, 1),
            (7, 4),
        ]
        .iter()
        .enumerate()
        .map(|(n, (a, t))| Entry {
            track: by(*a, *t).uri.clone(),
            added_at: Some(ago((200 + 30 * n as i64) * DAY)),
        })
        .collect();
        db.save_collection_entries(&CollectionId::Playlist(playlist_key(9)), Some(b"1"), &old)
            .unwrap();
        // "Big Mix": 120 satır, 10 şarkı 12'şer kez (toplu işlem onayı için).
        let big: Vec<Entry> = (0..120)
            .map(|n| Entry {
                track: rows[(n * 7) % 10].uri.clone(),
                added_at: Some(ago(10 + n as i64 * 450)),
            })
            .collect();
        db.save_collection_entries(&CollectionId::Playlist(playlist_key(10)), Some(b"1"), &big)
            .unwrap();
        // Albüm şarkıları (albüm sayfası ve çevrimdışı açılış için).
        for a in 0..ALBUMS.len() {
            let entries: Vec<Entry> = (0..5)
                .map(|n| Entry::untimed(by(a, n).uri.clone()))
                .collect();
            db.save_collection_entries(&CollectionId::Album(album_key(a)), Some(b"1"), &entries)
                .unwrap();
        }

        seed_pages(&dir, &rows);

        // Yerel çalma geçmişi (İstatistik): son 9 ayda 420 çalma; akşamlar yoğun.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move |m: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % m
        };
        for n in 0..420u64 {
            let pick = (next(100) * next(100) / 100 * rows.len() as u64 / 100) as usize;
            let r = &rows[pick.min(rows.len() - 1)];
            let days = next(270) * next(270) / 270;
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

    /// Sayfa önbelleği (`pages.db`): sanatçı / albüm sayfaları ve Keşfet / Ana sayfa
    /// akışları taze yazılır; çevrimdışı açılışta da dolu görünürler.
    fn seed_pages(dir: &std::path::Path, rows: &[TrackRow]) {
        let cache = PageCache::open(&dir.join("pages.db")).unwrap();
        let now = crate::engine::pages::now_ms();

        let album_card = |a: usize| AlbumCard {
            uri: album_uri(a),
            name: ALBUMS[a].0.into(),
            artists: ALBUMS[a].1.into(),
            year: 2019 + (a as u16 % 6),
            kind: ReleaseKind::Album,
            image: album_url(a),
        };
        let extra_card =
            |n: usize, name: &str, artists: &str, year: u16, kind: ReleaseKind| AlbumCard {
                uri: format!("spotify:album:{:022}", 100 + n),
                name: name.into(),
                artists: artists.into(),
                year,
                kind,
                image: extra_url(n),
            };
        let artist_card = |a: usize| ArtistCard {
            uri: artist_uri(a),
            name: ALBUMS[a].1.into(),
            image: artist_url(a),
        };
        let earlier = [
            "First Light",
            "Second Skin",
            "Small Hours",
            "Open Water",
            "Quiet Rooms",
            "Long Way Round",
            "Pale Gold",
            "Soft Focus",
        ];
        let single_names = ["Midnight Bus", "Neon Rain", "Ash Trails", "Late Frost"];
        let bios = [
            "Night Harbor makes slow, late-night synth pop built around warm analog textures and quiet vocals. Their debut grew out of a bedroom studio and a borrowed drum machine.",
            "Cobalt Static blends glitchy electronics with soft, sing-along hooks. Live shows are mostly lights and cables.",
            "Ember & Ash write unhurried folk songs about small towns and long winters, recorded close to the microphone.",
            "Velvet Orchard is a duo from the countryside making hushed, harmony-heavy pop with piano and field recordings.",
            "Paper Lanterns pair dreamy guitars with found sounds from night markets and train stations.",
            "Juniper Hollow plays dusty, open-road Americana with a rhythm section that never rushes.",
            "Low Tide Club is a four-piece surf-tinged band from a cold coastline, loud on stage and gentle on record.",
            "The Quiet Parade turns brass, handclaps and home recordings into small, joyful songs.",
        ];
        for (a, (album, artist, _)) in ALBUMS.iter().enumerate() {
            let tracks: Vec<TrackRow> = (0..5).map(|n| rows[a * 5 + n].clone()).collect();
            let album_page = Arc::new(AlbumPage {
                uri: album_uri(a),
                name: (*album).into(),
                artists: vec![artist_card(a)],
                year: 2019 + (a as u16 % 6),
                kind: ReleaseKind::Album,
                label: "Demo Records".into(),
                image: album_url(a),
                total_ms: tracks.iter().map(|t| t.duration_ms as u64).sum(),
                tracks,
            });
            cache
                .put(&format!("album:{}", album_uri(a)), &album_page, now)
                .unwrap();
            let mut top: Vec<TrackRow> = (0..5).map(|n| rows[a * 5 + n].clone()).collect();
            top.rotate_left(a % 3);
            let artist_page = Arc::new(ArtistPage {
                uri: artist_uri(a),
                name: (*artist).into(),
                image: artist_url(a),
                popularity: 62 + (a as u8 * 4) % 30,
                bio: bios[a].into(),
                top_tracks: top,
                albums: vec![
                    album_card(a),
                    extra_card(
                        a * 2,
                        earlier[a],
                        artist,
                        2017 + (a as u16 % 3),
                        ReleaseKind::Album,
                    ),
                    extra_card(
                        a * 2 + 1,
                        &format!("{} II", earlier[(a + 3) % 8]),
                        artist,
                        2015,
                        ReleaseKind::Album,
                    ),
                ],
                singles: single_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        extra_card(
                            16 + a + i,
                            name,
                            artist,
                            2022 + i as u16 % 3,
                            ReleaseKind::Single,
                        )
                    })
                    .collect(),
                compilations: vec![extra_card(
                    24 + a,
                    "Early Recordings",
                    artist,
                    2018,
                    ReleaseKind::Compilation,
                )],
                appears_on: vec![extra_card(
                    28 + a % 8,
                    "Songs for Late Evenings",
                    "Various Artists",
                    2023,
                    ReleaseKind::Compilation,
                )],
                related: (1..=5)
                    .map(|k| artist_card((a + k) % ALBUMS.len()))
                    .collect(),
            });
            cache
                .put(&format!("artist:{}", artist_uri(a)), &artist_page, now)
                .unwrap();
        }

        // Keşfet ve Ana sayfa akışları.
        let playlist_card = |n: usize, name: &str, subtitle: &str, format: &str| ExploreCard {
            uri: format!("spotify:playlist:{:022}", 200 + n),
            name: name.into(),
            subtitle: subtitle.into(),
            image: extra_url(n + 3),
            kind: CardKind::Playlist,
            format: format.into(),
        };
        let album_ex = |a: usize| ExploreCard {
            uri: album_uri(a),
            name: ALBUMS[a].0.into(),
            subtitle: ALBUMS[a].1.into(),
            image: album_url(a),
            kind: CardKind::Album,
            format: String::new(),
        };
        let home = Feed {
            title: String::new(),
            color: String::new(),
            sections: vec![ExploreSection {
                title: "Made for you".into(),
                cards: vec![
                    playlist_card(
                        0,
                        "Daily Mix 1",
                        "Night Harbor, Cobalt Static and more",
                        "daily-mix",
                    ),
                    playlist_card(
                        1,
                        "Daily Mix 2",
                        "Velvet Orchard, Paper Lanterns and more",
                        "daily-mix",
                    ),
                    playlist_card(
                        2,
                        "Daily Mix 3",
                        "Juniper Hollow, Low Tide Club and more",
                        "daily-mix",
                    ),
                    playlist_card(
                        3,
                        "Daily Mix 4",
                        "Ember & Ash, The Quiet Parade and more",
                        "daily-mix",
                    ),
                    playlist_card(
                        4,
                        "Discover Weekly",
                        "New music picked for you, every Monday",
                        "discover-weekly",
                    ),
                    playlist_card(
                        5,
                        "Release Radar",
                        "New releases from artists you follow",
                        "release-radar",
                    ),
                    playlist_card(
                        6,
                        "On Repeat",
                        "The songs you keep coming back to",
                        "on-repeat",
                    ),
                    playlist_card(
                        7,
                        "Repeat Rewind",
                        "Songs you loved, again",
                        "repeat-rewind",
                    ),
                    playlist_card(
                        8,
                        "slow evening, soft synths",
                        "Warm, unhurried electronic pop for after dark",
                        "daylist",
                    ),
                    playlist_card(9, "Chill Mix", "Gentle songs to unwind to", "genre-mix"),
                ],
            }],
        };
        cache
            .put(&ExploreKey::Home.cache_key(Lang::En), &Arc::new(home), now)
            .unwrap();
        let colors = [
            "#8d67ab", "#ba5d07", "#e61e32", "#1e3264", "#477d95", "#dc148c", "#509bf5", "#e8115b",
            "#148a08", "#27856a", "#af2896", "#006450",
        ];
        let names = [
            "Pop", "Hip-Hop", "Rock", "Indie", "Chill", "Dance", "Jazz", "Soul", "Folk", "Focus",
            "Mood", "Sleep",
        ];
        let categories = CategoryList {
            items: names
                .iter()
                .zip(colors)
                .enumerate()
                .map(|(i, (name, color))| Category {
                    uri: format!("spotify:page:0JQ5DA{i:016}"),
                    name: (*name).into(),
                    color: color.into(),
                })
                .collect(),
        };
        cache
            .put(
                &ExploreKey::Categories.cache_key(Lang::En),
                &Arc::new(categories),
                now,
            )
            .unwrap();
        let releases = Feed {
            title: String::new(),
            color: String::new(),
            sections: vec![ExploreSection {
                title: "New releases".into(),
                cards: (0..8).map(album_ex).collect(),
            }],
        };
        cache
            .put(
                &ExploreKey::NewReleases.cache_key(Lang::En),
                &Arc::new(releases),
                now,
            )
            .unwrap();
        let chart = |n: usize, name: &str, subtitle: &str| ExploreCard {
            uri: format!("spotify:playlist:{:022}", 300 + n),
            name: name.into(),
            subtitle: subtitle.into(),
            image: extra_url(n + 11),
            kind: CardKind::Playlist,
            format: "chart".into(),
        };
        let charts = Feed {
            title: String::new(),
            color: String::new(),
            sections: vec![ExploreSection {
                title: "Charts".into(),
                cards: vec![
                    chart(0, "Top 50: Worldwide", "The most played songs right now"),
                    chart(1, "Top 50: Local", "Your country's most played songs"),
                    chart(2, "Weekly Top Songs", "This week's biggest hits"),
                    chart(3, "Viral 50", "Songs everyone is sharing"),
                    chart(4, "Top Albums", "The albums on repeat"),
                ],
            }],
        };
        cache
            .put(
                &ExploreKey::Charts.cache_key(Lang::En),
                &Arc::new(charts),
                now,
            )
            .unwrap();
    }
}
