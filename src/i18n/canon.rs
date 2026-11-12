//! Kalıcı verideki "kanonik" altyazılar.
//!
//! Kütüphane öğelerinin altyazısı (`LibraryItem::subtitle`: "29 şarkı", "Takip edilen · 29
//! şarkı", "Spotify · 50 şarkı") motorda üretilir ve SQLite önbelleğinde saklanır; değişmeyen
//! listeler (aynı revizyon) önbellekten olduğu gibi yeniden kullanılır. Yazı o anki dilde
//! üretilseydi dil değişince eski dilde kalırdı. Bu yüzden saklanan biçim DİLDEN BAĞIMSIZ,
//! tek bir kanonik biçimdir (eski önbelleklerle uyumlu olsun diye Türkçe biçim) ve
//! ekranda gösterilirken `localize` ile o anki dile çevrilir. Biçimi üreten (`songs`,
//! `followed`, `spotify`) ve ayrıştıran (`is_*`, `song_count`, `localize`) kod burada, tek yerde.
//!
//! Bu dosyadaki Türkçe yazılar kullanıcıya gösterilmez; gösterilen her şey `t!` / `tp!` ile gelir.

use std::borrow::Cow;

use crate::{t, tp};

const SONGS_SUFFIX: &str = " şarkı";
const FOLLOWED_PREFIX: &str = "Takip edilen · ";
const SPOTIFY_PREFIX: &str = "Spotify · ";

/// Sanatçı arama sonucunun kanonik altyazısı.
pub const ARTIST: &str = "Sanatçı";
/// Liste arama sonucunun (sahibi bilinmeyen) kanonik altyazısı.
pub const PLAYLIST: &str = "Liste";

/// "29 şarkı"
pub fn songs(n: usize) -> String {
    format!("{n}{SONGS_SUFFIX}")
}

/// "Takip edilen · 29 şarkı": kullanıcının sahibi olmadığı liste.
pub fn followed(n: usize) -> String {
    format!("{FOLLOWED_PREFIX}{}", songs(n))
}

/// "Spotify · 50 şarkı": Spotify'ın kendi listesi.
pub fn spotify(n: usize) -> String {
    format!("{SPOTIFY_PREFIX}{}", songs(n))
}

/// Altyazı Spotify'ın kendi listesine mi ait?
pub fn is_spotify(raw: &str) -> bool {
    raw.starts_with("Spotify")
}

/// Altyazı takip edilen (başkasının) listeye mi ait?
pub fn is_followed(raw: &str) -> bool {
    raw.starts_with("Takip edilen")
}

/// Kanonik altyazının sonundaki şarkı sayısı ("12 şarkı", "Spotify · 50 şarkı" → 50).
pub fn song_count(raw: &str) -> Option<usize> {
    let body = raw.strip_suffix(SONGS_SUFFIX)?;
    let digits = body.rsplit(' ').next()?;
    digits.trim().parse().ok()
}

/// Yalnız "12 şarkı" biçimi (öneksiz): kullanıcının kendi listesi (bkz. `manage::is_own`).
pub fn plain_song_count(raw: &str) -> Option<usize> {
    raw.strip_suffix(SONGS_SUFFIX)?.trim().parse().ok()
}

/// Kanonik altyazıyı o anki dile çevirir; tanınmayan yazı (albüm sanatçıları gibi)
/// olduğu gibi döner.
pub fn localize(raw: &str) -> Cow<'_, str> {
    match raw {
        ARTIST => return Cow::Borrowed(t!(Artist)),
        PLAYLIST => return Cow::Borrowed(t!(Playlist)),
        _ => {}
    }
    let Some(n) = song_count(raw) else {
        return Cow::Borrowed(raw);
    };
    if raw == songs(n) {
        Cow::Owned(tp!(Songs, n))
    } else if raw == followed(n) {
        Cow::Owned(format!("{} · {}", t!(CanonFollowed), tp!(Songs, n)))
    } else if raw == spotify(n) {
        Cow::Owned(format!("Spotify · {}", tp!(Songs, n)))
    } else {
        Cow::Borrowed(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, with_lang};

    #[test]
    fn canonical_forms_round_trip() {
        assert_eq!(songs(1), "1 şarkı");
        assert_eq!(followed(12), "Takip edilen · 12 şarkı");
        assert_eq!(spotify(50), "Spotify · 50 şarkı");
        assert_eq!(song_count("12 şarkı"), Some(12));
        assert_eq!(song_count("Spotify · 50 şarkı"), Some(50));
        assert_eq!(song_count("Sür Eve"), None);
        assert!(is_spotify(&spotify(3)) && !is_spotify(&songs(3)));
        assert!(is_followed(&followed(3)) && !is_followed(&songs(3)));
    }

    #[test]
    fn localize_follows_the_current_language() {
        with_lang(Lang::En, || {
            assert_eq!(localize("1 şarkı"), "1 song");
            assert_eq!(localize("29 şarkı"), "29 songs");
            assert_eq!(localize("Takip edilen · 29 şarkı"), "Followed · 29 songs");
            assert_eq!(
                localize("Spotify · 1.234 şarkı"),
                "Spotify · 1.234 şarkı",
                "gruplanmış sayı kanonik değil: olduğu gibi kalır"
            );
            assert_eq!(localize("Spotify · 1234 şarkı"), "Spotify · 1,234 songs");
            assert_eq!(localize("Sanatçı"), "Artist");
            assert_eq!(localize("Liste"), "Playlist");
            assert_eq!(localize("Tarkan, Sezen Aksu"), "Tarkan, Sezen Aksu");
        });
        with_lang(Lang::Tr, || {
            assert_eq!(localize("29 şarkı"), "29 şarkı");
            assert_eq!(
                localize("Takip edilen · 29 şarkı"),
                "Takip edilen · 29 şarkı"
            );
            assert_eq!(localize("Sanatçı"), "Sanatçı");
        });
    }
}
