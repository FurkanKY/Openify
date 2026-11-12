//! CANLI yazma sınaması (kullanıcının onayıyla, kendi hesabında, bir kez).
//!
//! Çalıştırma (internet ve kayıtlı Spotify girişi gerekir):
//!
//!   cargo test --release live_write -- --ignored --nocapture
//!
//! Kurallar:
//! - Yalnız bu sınamanın kendi oluşturduğu "Openify deneme" listesine dokunur.
//! - Beğeni sınaması yalnız o an BEĞENİLMEMİŞ bir şarkıyı beğenir ve geri alır.
//! - Temizlik, bir adım çökse bile çalışır: sınama şarkısının beğenisi geri
//!   alınır, deneme listesi kütüphaneden çıkarılır.
//! - Sonunda kök liste (sırasıyla) ve beğeni kümesi sınama öncesi
//!   görüntüyle karşılaştırılır; farklıysa sınama başarısız sayılır.
//! - İstekler seyrek (adımlar arası bekleme) ve az; yeniden deneme döngüsü yok.

use std::{
    panic::{AssertUnwindSafe, resume_unwind},
    time::{Duration, Instant},
};

use futures::FutureExt;
use librespot::core::{authentication::Credentials, config::SessionConfig};

use super::*;
use crate::engine::library::{raw_collection_uris, raw_rootlist_uris, resolve_tracks};

const TEST_NAME: &str = "Openify deneme";
const TEST_NAME_2: &str = "Openify deneme 2";
const STEP_PAUSE: Duration = Duration::from_millis(1500);

/// Bilinen, herkese açık şarkılar (kimlikler çözülmeden hiçbir yazma yapılmaz).
const CANDIDATES: &[&str] = &[
    "spotify:track:4cOdK2wGLETKBW3PvgPWqT", // Never Gonna Give You Up
    "spotify:track:3n3Ppam7vgaVa1iaRUc9Lp", // Mr. Brightside
    "spotify:track:7ouMYWpwJ422jRcDASZB7P", // Knights of Cydonia
    "spotify:track:0VjIjW4GlUZAMYd2vXMi3b", // Blinding Lights
    "spotify:track:7qiZfU4dY1lWllzX7mPBI3", // Shape of You
    "spotify:track:5ChkMS8OtdzJeqyybCc9R5", // Billie Jean
    "spotify:track:4u7EnebtmKWzUH433cf5Qv", // Bohemian Rhapsody
];

/// Sınamanın hesapta bıraktıkları (temizlik bunlara bakar).
#[derive(Default)]
struct Left {
    playlist: Mutex<Option<String>>,
    liked: Mutex<Option<String>>,
}

async fn connect() -> Session {
    let json = keyring::Entry::new("Openify", "spotify-session")
        .expect("Credential Manager")
        .get_password()
        .expect("Kayıtlı Spotify girişi yok");
    let credentials: Credentials = serde_json::from_str(&json).expect("kayıtlı giriş bozuk");
    let session = Session::new(SessionConfig::default(), None);
    session
        .connect(credentials, false)
        .await
        .expect("Spotify'a bağlanılamadı");
    session
}

async fn pause() {
    tokio::time::sleep(STEP_PAUSE).await;
}

async fn run(s: &Session, left: &Left, label: &str, op: EditOp) -> EditOutcome {
    let t = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(45), apply_now(s, &op)).await;
    let ms = t.elapsed().as_millis();
    match result {
        Ok(Ok(outcome)) => {
            println!("[{label}] BAŞARILI {outcome:?} ({ms} ms)");
            if let EditOutcome::Created { uri } = &outcome {
                *left.playlist.lock().unwrap() = Some(uri.clone());
            }
            pause().await;
            outcome
        }
        Ok(Err(e)) => {
            println!("[{label}] HATA ({ms} ms): {e:?}");
            if let Some(uri) = e.partial_create() {
                *left.playlist.lock().unwrap() = Some(uri.to_string());
            }
            panic!("[{label}] işlem başarısız: {e}");
        }
        Err(_) => panic!("[{label}] 45 sn içinde yanıt gelmedi"),
    }
}

async fn read_list(s: &Session, uri: &str) -> PlaylistState {
    playlist_state(s, uri).await.expect("liste okunamadı")
}

fn short(uris: &[String], names: &HashMap<String, String>) -> String {
    uris.iter()
        .map(|u| names.get(u).cloned().unwrap_or_else(|| u.clone()))
        .collect::<Vec<_>>()
        .join(" | ")
}

use std::collections::HashMap;

async fn scenario(s: &Session, before_liked: &[String], left: &Left) {
    let liked: HashSet<&str> = before_liked.iter().map(String::as_str).collect();

    // Şarkıları çöz: yazmadan önce geçerli olduklarını gör.
    let cands: Vec<String> = CANDIDATES.iter().map(|u| u.to_string()).collect();
    let rows = resolve_tracks(s, &cands)
        .await
        .expect("şarkı bilgisi alınamadı");
    let names: HashMap<String, String> = rows
        .iter()
        .map(|r| (r.uri.clone(), format!("{} - {}", r.name, r.artists)))
        .collect();
    let valid: Vec<String> = cands
        .iter()
        .filter(|u| names.contains_key(*u))
        .cloned()
        .collect();
    println!("Geçerli sınama şarkıları: {}/{}", valid.len(), cands.len());
    assert!(valid.len() >= 5, "yeterli geçerli şarkı yok: {valid:?}");
    let (a, b, c, d) = (
        valid[0].clone(),
        valid[1].clone(),
        valid[2].clone(),
        valid[3].clone(),
    );
    // Beğeni şarkısı: şu an beğenilmemiş olan (listedeki son aday öncelikli).
    let like_track = valid
        .iter()
        .rev()
        .find(|u| !liked.contains(u.as_str()))
        .cloned()
        .expect("bütün adaylar zaten beğenilmiş");
    println!(
        "Beğeni sınaması şarkısı (şu an beğenilmemiş): {}",
        names[&like_track]
    );
    pause().await;

    let root_before = raw_rootlist_uris(s).await.unwrap();

    // 2. Oluştur: üç şarkı.
    let temp = new_temp_id();
    let EditOutcome::Created { uri: pl } = run(
        s,
        left,
        "2 CreatePlaylist",
        EditOp::CreatePlaylist {
            temp_id: temp,
            name: TEST_NAME.into(),
            uris: vec![a.clone(), b.clone(), c.clone()],
            created: None,
        },
    )
    .await
    else {
        panic!("Created beklenirdi");
    };
    println!("    yeni liste: {pl}");
    let st = read_list(s, &pl).await;
    println!(
        "    okundu: ad={:?} öğeler=[{}]",
        st.name,
        short(&st.uris, &names)
    );
    assert_eq!(st.name, TEST_NAME);
    assert_eq!(st.uris, vec![a.clone(), b.clone(), c.clone()]);
    let root_now = raw_rootlist_uris(s).await.unwrap();
    println!(
        "    kök liste: {} -> {} öğe, yeni liste {}",
        root_before.len(),
        root_now.len(),
        if root_now.first() == Some(&pl) {
            "başta"
        } else {
            "başta DEĞİL"
        }
    );
    assert_eq!(root_now.len(), root_before.len() + 1);
    assert!(root_now.contains(&pl));
    pause().await;

    // 3. Araya ekle: 1. sıraya bir şarkı.
    run(
        s,
        left,
        "3 AddToPlaylist At(1)",
        EditOp::AddToPlaylist {
            playlist: pl.clone(),
            uris: vec![d.clone()],
            position: AddPosition::At(1),
        },
    )
    .await;
    let st = read_list(s, &pl).await;
    println!("    okundu: [{}]", short(&st.uris, &names));
    assert_eq!(st.uris, vec![a.clone(), d.clone(), b.clone(), c.clone()]);
    let mut order = st.uris.clone();

    // 4. Taşıma: anlam (MOV_TO_IS_AFTER_REMOVAL) burada belli olur.
    for (label, from, len, to) in [
        ("4a Move ileri 1 öğe (0, 1 -> 2)", 0usize, 1usize, 2usize),
        ("4b Move geri 2 öğe (2, 2 -> 0)", 2, 2, 0),
        ("4c Move ileri 2 öğe (0, 2 -> 2)", 0, 2, 2),
    ] {
        let block: Vec<String> = order[from..from + len].to_vec();
        let mut predicted = order.clone();
        assert!(apply_move(&mut predicted, from, len, to));
        run(
            s,
            left,
            label,
            EditOp::MovePlaylistItems {
                playlist: pl.clone(),
                from_index: from as u32,
                length: len as u32,
                to_index: to as u32,
                uris: block,
            },
        )
        .await;
        let st = read_list(s, &pl).await;
        println!("    önce:      [{}]", short(&order, &names));
        println!("    beklenen:  [{}]", short(&predicted, &names));
        println!("    okunan:    [{}]", short(&st.uris, &names));
        assert_eq!(
            st.uris, predicted,
            "{label}: sunucudaki sıra beklenenden farklı (MOV_TO_IS_AFTER_REMOVAL yanlış olabilir)"
        );
        order = predicted;
    }

    // 5a. Adlandır.
    run(
        s,
        left,
        "5a RenamePlaylist",
        EditOp::RenamePlaylist {
            playlist: pl.clone(),
            name: TEST_NAME_2.into(),
            previous_name: Some(TEST_NAME.into()),
        },
    )
    .await;
    let st = read_list(s, &pl).await;
    println!("    okundu: ad={:?}", st.name);
    assert_eq!(st.name, TEST_NAME_2);
    assert_eq!(st.uris, order, "adlandırma öğeleri değiştirmemeli");

    // 5b. Çıkarma, anahtarla (eski yöntem): kaç kopya gidiyor? (bilgi amaçlı)
    let x = order[0].clone();
    run(
        s,
        left,
        "5b AddToPlaylist kopya (Last)",
        EditOp::AddToPlaylist {
            playlist: pl.clone(),
            uris: vec![x.clone()],
            position: AddPosition::Last,
        },
    )
    .await;
    let st = read_list(s, &pl).await;
    let copies = |v: &[String]| v.iter().filter(|u| **u == x).count();
    assert_eq!(copies(&st.uris), 2);
    println!("    kopya eklendi: [{}]", short(&st.uris, &names));
    post_playlist_changes(s, &pl, |st| {
        Some(changes_remove(&st.revision, &[x.clone()]))
    })
    .await
    .expect("anahtarla çıkarma isteği");
    pause().await;
    let st = read_list(s, &pl).await;
    println!(
        "    ANAHTARLA çıkarma sonrası: {} kopya kaldı (2'den) -> [{}]",
        copies(&st.uris),
        short(&st.uris, &names)
    );
    let key_based_removed = 2 - copies(&st.uris);
    println!("    SONUÇ: anahtarla çıkarma {key_based_removed} kopya sildi");
    order = st.uris.clone();

    // 5c. Tek satır çıkarma, sıra numarasıyla: İKİNCİ kopyayı hedefle.
    while copies(&order) < 2 {
        run(
            s,
            left,
            "5c AddToPlaylist kopya (Last)",
            EditOp::AddToPlaylist {
                playlist: pl.clone(),
                uris: vec![x.clone()],
                position: AddPosition::Last,
            },
        )
        .await;
        order = read_list(s, &pl).await.uris;
    }
    let positions: Vec<usize> = order
        .iter()
        .enumerate()
        .filter(|(_, u)| **u == x)
        .map(|(i, _)| i)
        .collect();
    let second = *positions.last().unwrap();
    let mut expected = order.clone();
    expected.remove(second);
    run(
        s,
        left,
        "5c RemoveFromPlaylist (sıra ile, ikinci kopya)",
        EditOp::RemoveFromPlaylist {
            playlist: pl.clone(),
            uris: vec![x.clone()],
            indices: vec![second as u32],
        },
    )
    .await;
    let st = read_list(s, &pl).await;
    println!("    önce:     [{}]", short(&order, &names));
    println!("    beklenen: [{}]", short(&expected, &names));
    println!("    okunan:   [{}]", short(&st.uris, &names));
    assert_eq!(st.uris, expected, "sıra ile çıkarma tam bir satır silmeli");
    order = st.uris.clone();

    // 5d. Tek satır çıkarma, sırasız (ilk kopya).
    run(
        s,
        left,
        "5d AddToPlaylist kopya (Last)",
        EditOp::AddToPlaylist {
            playlist: pl.clone(),
            uris: vec![x.clone()],
            position: AddPosition::Last,
        },
    )
    .await;
    order = {
        let st = read_list(s, &pl).await;
        assert_eq!(st.uris.len(), order.len() + 1);
        st.uris
    };
    assert_eq!(copies(&order), 2);
    let first = order.iter().position(|u| *u == x).unwrap();
    let mut expected = order.clone();
    expected.remove(first);
    run(
        s,
        left,
        "5d RemoveFromPlaylist (sırasız, ilk kopya)",
        EditOp::RemoveFromPlaylist {
            playlist: pl.clone(),
            uris: vec![x.clone()],
            indices: vec![],
        },
    )
    .await;
    let st = read_list(s, &pl).await;
    println!("    önce:     [{}]", short(&order, &names));
    println!("    beklenen: [{}]", short(&expected, &names));
    println!("    okunan:   [{}]", short(&st.uris, &names));
    assert_eq!(
        st.uris, expected,
        "sırasız çıkarma tam bir (ilk) satır silmeli"
    );

    // 6. Beğen / vazgeç.
    run(
        s,
        left,
        "6a Save",
        EditOp::Save {
            uri: like_track.clone(),
        },
    )
    .await;
    *left.liked.lock().unwrap() = Some(like_track.clone());
    let mut seen = false;
    for attempt in 1..=3 {
        let now = raw_collection_uris(s).await.unwrap();
        seen = now.contains(&like_track);
        println!(
            "    beğeni okuması {attempt}: şarkı {} ({} öğe)",
            if seen { "VAR" } else { "yok" },
            now.len()
        );
        if seen {
            break;
        }
        pause().await;
    }
    assert!(seen, "beğenilen şarkı koleksiyonda görünmüyor");
    run(
        s,
        left,
        "6b Unsave",
        EditOp::Unsave {
            uri: like_track.clone(),
        },
    )
    .await;
    let mut gone = false;
    for attempt in 1..=3 {
        let now = raw_collection_uris(s).await.unwrap();
        gone = !now.contains(&like_track);
        println!(
            "    beğeni okuması {attempt}: şarkı {} ({} öğe)",
            if gone { "YOK" } else { "hâlâ var" },
            now.len()
        );
        if gone {
            break;
        }
        pause().await;
    }
    assert!(gone, "beğenisi kaldırılan şarkı koleksiyonda duruyor");
    *left.liked.lock().unwrap() = None;

    // 7. Sil, geri getir, tekrar sil (geri getirme bilgi amaçlı).
    run(
        s,
        left,
        "7a DeletePlaylist",
        EditOp::DeletePlaylist {
            playlist: pl.clone(),
        },
    )
    .await;
    let root = raw_rootlist_uris(s).await.unwrap();
    println!(
        "    kök liste: {} öğe, silinen liste {}",
        root.len(),
        if root.contains(&pl) {
            "HÂLÂ VAR"
        } else {
            "yok"
        }
    );
    assert_eq!(
        root, root_before,
        "silmeden sonra kök liste sınama öncesine eşit olmalı"
    );

    let restore = tokio::time::timeout(
        Duration::from_secs(45),
        apply_now(
            s,
            &EditOp::RestorePlaylist {
                playlist: pl.clone(),
            },
        ),
    )
    .await;
    match restore {
        Ok(Ok(o)) => {
            println!("[7b RestorePlaylist] BAŞARILI {o:?}");
            pause().await;
            let root = raw_rootlist_uris(s).await.unwrap();
            println!(
                "    geri getirme sonrası kök liste: {} öğe, liste {}",
                root.len(),
                if root.contains(&pl) {
                    "geri geldi"
                } else {
                    "GERİ GELMEDİ"
                }
            );
            let st = read_list(s, &pl).await;
            println!(
                "    geri gelen liste: ad={:?}, {} öğe",
                st.name,
                st.uris.len()
            );
            run(
                s,
                left,
                "7c DeletePlaylist (ikinci)",
                EditOp::DeletePlaylist {
                    playlist: pl.clone(),
                },
            )
            .await;
        }
        Ok(Err(e)) => println!("[7b RestorePlaylist] HATA (bilgi): {e:?}"),
        Err(_) => println!("[7b RestorePlaylist] zaman aşımı (bilgi)"),
    }
    let root = raw_rootlist_uris(s).await.unwrap();
    assert_eq!(
        root, root_before,
        "son silmeden sonra kök liste sınama öncesine eşit olmalı"
    );
    *left.playlist.lock().unwrap() = None;
}

/// Hata ya da çökme olsa da çalışır: sınamanın hesapta bıraktıklarını geri alır.
async fn cleanup(s: &Session, left: &Left) {
    let liked = left.liked.lock().unwrap().take();
    if let Some(uri) = liked {
        println!("TEMİZLİK: sınama şarkısının beğenisi kaldırılıyor ({uri})");
        for attempt in 1..=3 {
            match apply_now(s, &EditOp::Unsave { uri: uri.clone() }).await {
                Ok(_) => break,
                Err(e) => println!("    temizlik beğeni denemesi {attempt} başarısız: {e:?}"),
            }
            pause().await;
        }
    }
    let playlist = left.playlist.lock().unwrap().take();
    if let Some(pl) = playlist {
        // Yalnız hâlâ kütüphanedeyse ve bu sınamanın oluşturduğu liste ise.
        let in_root = raw_rootlist_uris(s)
            .await
            .map(|r| r.contains(&pl))
            .unwrap_or(true);
        if in_root {
            println!("TEMİZLİK: deneme listesi kütüphaneden çıkarılıyor ({pl})");
            for attempt in 1..=3 {
                match apply_now(
                    s,
                    &EditOp::DeletePlaylist {
                        playlist: pl.clone(),
                    },
                )
                .await
                {
                    Ok(_) => break,
                    Err(e) => println!("    temizlik liste denemesi {attempt} başarısız: {e:?}"),
                }
                pause().await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Spotify hesabına canlı yazar; yalnız elle: cargo test --release live_write -- --ignored --nocapture"]
async fn live_write_roundtrip() {
    let s = connect().await;
    println!("Bağlandı: kullanıcı {}", s.username());

    let root_before = raw_rootlist_uris(&s).await.expect("kök liste");
    pause().await;
    let liked_before = raw_collection_uris(&s).await.expect("beğeniler");
    let mut sorted_before = liked_before.clone();
    sorted_before.sort();
    println!(
        "ÖNCE: kök liste {} öğe, koleksiyon {} öğe (şarkı: {})",
        root_before.len(),
        liked_before.len(),
        liked_before
            .iter()
            .filter(|u| u.starts_with("spotify:track:"))
            .count()
    );
    pause().await;

    let left = Left::default();
    let result = AssertUnwindSafe(scenario(&s, &liked_before, &left))
        .catch_unwind()
        .await;
    cleanup(&s, &left).await;
    pause().await;

    let root_after = raw_rootlist_uris(&s).await.expect("kök liste (sonra)");
    pause().await;
    let liked_after = raw_collection_uris(&s).await.expect("beğeniler (sonra)");
    let mut sorted_after = liked_after.clone();
    sorted_after.sort();
    println!(
        "SONRA: kök liste {} öğe, koleksiyon {} öğe",
        root_after.len(),
        liked_after.len()
    );
    println!(
        "KÖK LİSTE (sıra dahil) özdeş: {}",
        root_after == root_before
    );
    println!(
        "BEĞENİLER: sayı {} = {}, küme özdeş: {}",
        liked_before.len(),
        liked_after.len(),
        sorted_after == sorted_before
    );
    let diff_added: Vec<&String> = sorted_after
        .iter()
        .filter(|u| !sorted_before.contains(u))
        .collect();
    let diff_removed: Vec<&String> = sorted_before
        .iter()
        .filter(|u| !sorted_after.contains(u))
        .collect();
    println!("    eklenen: {diff_added:?}, eksilen: {diff_removed:?}");

    if let Err(panic) = result {
        resume_unwind(panic);
    }
    assert_eq!(root_after, root_before, "kök liste değişmiş");
    assert_eq!(sorted_after, sorted_before, "beğeniler değişmiş");
}

/// Teşhis: oluşturma isteğinin ad ve şarkıları gerçekten kaydedip kaydetmediğini
/// ve okumanın doğru çalışıp çalışmadığını gösterir (yalnız elle).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Spotify hesabına canlı yazar; yalnız elle"]
async fn live_diag_create() {
    let s = connect().await;
    let root_before = raw_rootlist_uris(&s).await.expect("kök liste");
    pause().await;
    // Salt okuma: okuyucu çalışıyor mu? İlk liste.
    if let Some(first) = root_before
        .iter()
        .find(|u| u.starts_with("spotify:playlist:"))
    {
        let st = playlist_state(&s, first)
            .await
            .expect("ilk liste okunamadı");
        println!(
            "OKUYUCU SINAMASI (salt okuma) {first}: ad={:?} öğe={}",
            st.name,
            st.uris.len()
        );
    }
    pause().await;

    let left = Left::default();
    let result = AssertUnwindSafe(async {
        let uris: Vec<String> = CANDIDATES.iter().take(3).map(|u| u.to_string()).collect();
        let pl = create_playlist(&s, TEST_NAME, &uris)
            .await
            .expect("oluşturma");
        println!("oluşturuldu: {pl}");
        pause().await;
        post_rootlist_changes(&s, |rev| rootlist_add(rev, &pl))
            .await
            .expect("kök listeye ekleme");
        *left.playlist.lock().unwrap() = Some(pl.clone());
        pause().await;
        for i in 0..2 {
            let id = playlist_id(&pl).unwrap();
            let bytes = s
                .spclient()
                .request(
                    &http::Method::GET,
                    &format!("/playlist/v2/playlist/{id}"),
                    None,
                    None,
                )
                .await
                .expect("okuma");
            let list = SelectedListContent::parse_from_bytes(&bytes).unwrap();
            println!(
                "okuma {i}: {} bayt, length={}, ad={:?}, öğe={}, owner={:?}, rev={} bayt",
                bytes.len(),
                list.length(),
                list.attributes.name(),
                list.contents.items.len(),
                list.owner_username(),
                list.revision().len()
            );
            pause().await;
        }
        // Kök listedeki dekorlu girdi.
        let bytes = s.spclient().get_rootlist(0, Some(1000)).await.unwrap();
        let root = SelectedListContent::parse_from_bytes(&bytes).unwrap();
        if let Some(ix) = root.contents.items.iter().position(|i| i.uri() == pl) {
            let m = &root.contents.meta_items[ix];
            println!(
                "kökte: sıra {ix}, ad={:?}, uzunluk={}, durum={}",
                m.attributes.name(),
                m.length(),
                m.status_code()
            );
        }
    })
    .catch_unwind()
    .await;
    cleanup(&s, &left).await;
    pause().await;
    let root_after = raw_rootlist_uris(&s).await.expect("kök liste (sonra)");
    println!("kök liste özdeş: {}", root_after == root_before);
    if let Err(p) = result {
        resume_unwind(p);
    }
    assert_eq!(root_after, root_before);
}

/// CANLI takip sınaması (yalnız elle, kullanıcının onayıyla, kendi hesabında):
/// okuma tarafı (`artist` kümesi) çalışıyor mu, `Follow` / `Unfollow` yazması
/// kümeye işleniyor mu? Yalnız o an TAKİP EDİLMEYEN bilinen bir sanatçıyı
/// takip eder ve bırakır; sonunda küme sınama öncesiyle özdeş olmalı, çökse
/// bile temizlik sanatçıyı bırakır. Seyrek istek, yeniden deneme döngüsü yok.
///
///   cargo test --release live_follow -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Spotify hesabına canlı yazar; yalnız elle: cargo test --release live_follow -- --ignored --nocapture"]
async fn live_follow() {
    use crate::engine::library::raw_artist_uris;
    const CANDIDATES: &[&str] = &[
        "spotify:artist:0gxyHStUsqpMadRV0Di1Qt", // Rick Astley
        "spotify:artist:1dfeR4HaWDbWqFHLkxsg1d", // Queen
        "spotify:artist:3WrFJ7ztbogyGnTHbHJFl2", // The Beatles
    ];
    let s = connect().await;
    let before = raw_artist_uris(&s)
        .await
        .expect("takip edilenler okunamadı");
    println!(
        "ÖNCE: {} takip edilen sanatçı (okuma tarafı çalışıyor)",
        before.len()
    );
    pause().await;
    let Some(pick) = CANDIDATES.iter().find(|u| !before.iter().any(|b| b == *u)) else {
        println!("Bütün adaylar zaten takipte: yazma sınanmadı (salt okuma geçti).");
        return;
    };
    let pick = pick.to_string();
    let result = AssertUnwindSafe(async {
        run_follow(&s, "Follow", EditOp::Follow { uri: pick.clone() }).await;
        let mid = raw_artist_uris(&s).await.unwrap();
        println!("takipten sonra kümede: {}", mid.contains(&pick));
        assert!(mid.contains(&pick), "takip kümeye işlenmedi");
        pause().await;
        run_follow(&s, "Unfollow", EditOp::Unfollow { uri: pick.clone() }).await;
        let after = raw_artist_uris(&s).await.unwrap();
        println!("bırakınca kümede: {}", after.contains(&pick));
        assert!(!after.contains(&pick), "bırakma kümeye işlenmedi");
    })
    .catch_unwind()
    .await;
    // Temizlik: çökse bile sınama sanatçısı bırakılır.
    let _ = apply_now(&s, &EditOp::Unfollow { uri: pick.clone() }).await;
    pause().await;
    let mut sorted_before = before.clone();
    sorted_before.sort();
    let mut sorted_after = raw_artist_uris(&s).await.expect("takip edilenler (sonra)");
    sorted_after.sort();
    println!("KÜME özdeş: {}", sorted_after == sorted_before);
    if let Err(p) = result {
        resume_unwind(p);
    }
    assert_eq!(
        sorted_after, sorted_before,
        "takip edilen sanatçılar değişmiş"
    );
}

async fn run_follow(s: &Session, label: &str, op: EditOp) {
    let t = Instant::now();
    let r = tokio::time::timeout(Duration::from_secs(45), apply_now(s, &op)).await;
    match r {
        Ok(Ok(o)) => println!("[{label}] BAŞARILI {o:?} ({} ms)", t.elapsed().as_millis()),
        Ok(Err(e)) => panic!("[{label}] işlem başarısız: {e:?}"),
        Err(_) => panic!("[{label}] 45 sn içinde yanıt gelmedi"),
    }
    pause().await;
}
