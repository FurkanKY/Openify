//! Şarkı özellikleri (açık söz işareti, popülerlik) deposu.
//!
//! `TrackRow` bu bilgiyi taşımaz (her yerde elle kurulan bir yapı; alan
//! eklemek önbellek şemasını ve arama / sayfa kodunu birlikte oynatırdı).
//! Özellikler Spotify'ın şarkı üst verisiyle gelir (`library::fetch_track_batch`
//! şarkıyı ayrıştırırken `note` ile bırakır); `Db::save_tracks` şarkı satırlarını
//! yazarken bekleyenleri alıp `track_traits` tablosuna geçirir. Böylece
//! `fetch_tracks`'ın işareti ve çağıranlar değişmez.
//!
//! Bu bilgi YENİDEN İNDİRİLEBİLİR: tablo yoksa ya da şarkı için kayıt yoksa
//! "bilinmiyor" (açık söz değil, popülerlik 0) sayılır; `engine` açık sözlüleri
//! atla açıkken eksikleri arka planda tamamlar.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use super::rules::Traits;

/// Bekleyen kayıtların üst sınırı (yazılamayan bir veritabanında şişmesin).
const PENDING_MAX: usize = 20_000;

fn pending() -> &'static Mutex<HashMap<String, Traits>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Traits>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// Spotify'dan yeni gelen bir şarkının özelliği (veritabanına yazılmayı bekler).
pub fn note(uri: &str, traits: Traits) {
    let Ok(mut map) = pending().lock() else {
        return;
    };
    if map.len() >= PENDING_MAX {
        map.clear();
    }
    map.insert(uri.to_string(), traits);
}

/// `uris` için bekleyenleri alır (bekleyenlerden düşer).
pub fn take<'a>(uris: impl IntoIterator<Item = &'a str>) -> Vec<(String, Traits)> {
    let Ok(mut map) = pending().lock() else {
        return Vec::new();
    };
    if map.is_empty() {
        return Vec::new();
    }
    uris.into_iter()
        .filter_map(|u| map.remove_entry(u))
        .collect()
}

/// Spotify'ın ham değerlerinden: popülerlik 0–100'e sıkıştırılır.
pub fn from_spotify(explicit: bool, popularity: i32) -> Traits {
    Traits {
        explicit,
        popularity: popularity.clamp(0, 100) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noted_traits_are_taken_once_and_only_for_the_asked_uris() {
        // Ortak (süreç çapında) depo: bu testin adreslerine özgü anahtarlar.
        note("test:traits:a", from_spotify(true, 55));
        note("test:traits:b", from_spotify(false, 300));
        let got = take(["test:traits:a", "test:traits:zzz"]);
        assert_eq!(
            got,
            vec![(
                "test:traits:a".to_string(),
                Traits {
                    explicit: true,
                    popularity: 55
                }
            )]
        );
        assert!(take(["test:traits:a"]).is_empty(), "ikinci kez verilmez");
        let b = take(["test:traits:b"]);
        assert_eq!(b[0].1.popularity, 100, "popülerlik 100'e sıkışır");
    }

    #[test]
    fn popularity_is_clamped() {
        assert_eq!(from_spotify(false, -5).popularity, 0);
        assert_eq!(
            from_spotify(true, 42),
            Traits {
                explicit: true,
                popularity: 42
            }
        );
    }

    #[test]
    fn the_unknown_default_is_clean_and_unpopular() {
        assert_eq!(
            Traits::default(),
            Traits {
                explicit: false,
                popularity: 0
            }
        );
    }
}
