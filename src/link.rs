//! Spotify şarkı bağlantılarını çözümler.

use anyhow::{Context, Result, bail};
use librespot::core::{SpotifyUri, spotify_id::SpotifyId};

/// Şu biçimlerin hepsini kabul eder:
///   https://open.spotify.com/track/ID?si=...
///   https://open.spotify.com/intl-tr/track/ID
///   spotify:track:ID
///   ID
pub fn parse_track(input: &str) -> Result<SpotifyUri> {
    let input = input.trim();

    if input.starts_with("spotify:") {
        let uri = SpotifyUri::from_uri(input).context("Geçersiz Spotify URI")?;
        return match uri {
            SpotifyUri::Track { .. } => Ok(uri),
            _ => bail!("Bu bir şarkı bağlantısı değil (şimdilik sadece şarkılar destekleniyor)"),
        };
    }

    let id = if input.contains("open.spotify.com") {
        let path = input.split('?').next().unwrap_or(input);
        let mut segments = path.split('/');
        segments
            .position(|s| s == "track")
            .and_then(|_| segments.next())
            .context("Bu bir şarkı bağlantısı değil (şimdilik sadece şarkılar destekleniyor)")?
    } else {
        input
    };

    let id = SpotifyId::from_base62(id).context("Geçersiz şarkı ID'si")?;
    Ok(SpotifyUri::Track { id })
}

/// "Spotify'da aç" için açılacak adres: yalnız tanınan türde, kimliği harf /
/// rakamdan oluşan `spotify:<tür>:<kimlik>` (kabuğa verilen metin başka bir şey
/// taşıyamaz). Başka her şey (geçici liste, "liked", bozuk adres) için `None`.
pub fn app_target(uri: &str) -> Option<String> {
    let mut parts = uri.split(':');
    let (Some("spotify"), Some(kind), Some(id), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let known = matches!(kind, "track" | "album" | "artist" | "playlist" | "show" | "episode" | "user");
    // Kullanıcı kimliği `.`, `_` ve `-` de taşıyabilir (kabuk için zararsız).
    let plain = |b: u8| b.is_ascii_alphanumeric() || (kind == "user" && matches!(b, b'.' | b'_' | b'-'));
    (known && !id.is_empty() && id.bytes().all(plain)).then(|| uri.to_string())
}

/// Windows'ta Spotify masaüstü uygulaması `spotify:` bağlantılarını
/// karşılıyor mu (`HKCR\spotify`'ın "URL Protocol" değeri)? Salt okunur.
/// Windows dışında ve sınama kopyasında (`OPENIFY_TEST_DISPLAY`) `false`.
pub fn spotify_app_registered() -> bool {
    #[cfg(windows)]
    {
        use windows::{
            Win32::{
                Foundation::ERROR_SUCCESS,
                System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW},
            },
            core::HSTRING,
        };
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
            return false;
        }
        let subkey = HSTRING::from("spotify");
        let value = HSTRING::from("URL Protocol");
        let mut size = 0u32;
        // SAFETY: yalnız boyut sorgusu; okuma arabelleği verilmiyor.
        let probe = unsafe {
            RegGetValueW(HKEY_CLASSES_ROOT, &subkey, &value, RRF_RT_REG_SZ, None, None, Some(&mut size))
        };
        probe == ERROR_SUCCESS
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "2WUy2Uywcj5cP0IXQagO3z";

    #[test]
    fn app_targets_are_only_plain_spotify_uris() {
        for kind in ["track", "album", "artist", "playlist", "show", "episode", "user"] {
            let uri = format!("spotify:{kind}:{ID}");
            assert_eq!(app_target(&uri).as_deref(), Some(uri.as_str()));
        }
        // Kullanıcı kimliğinde nokta, alt çizgi ve tire olabilir; başka tür kimlikte olamaz.
        assert_eq!(app_target("spotify:user:ay_se.k-1").as_deref(), Some("spotify:user:ay_se.k-1"));
        assert_eq!(app_target("spotify:track:a.b"), None);
        assert_eq!(app_target("spotify:user:a&calc"), None);
        for bad in [
            "liked".to_string(),
            "local:playlist:1a-2".to_string(),
            "spotify:user:x:playlist:y".to_string(),
            format!("spotify:track:{ID}:extra"),
            "spotify:track:".to_string(),
            "spotify:track:a b".to_string(),
            "spotify:track:a&calc".to_string(),
            "spotify:page:x".to_string(),
            String::new(),
        ] {
            assert_eq!(app_target(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn parses_all_formats() {
        for input in [
            format!("https://open.spotify.com/track/{ID}?si=abc123"),
            format!("https://open.spotify.com/intl-tr/track/{ID}"),
            format!("spotify:track:{ID}"),
            ID.to_string(),
        ] {
            assert!(parse_track(&input).is_ok(), "{input}");
        }
    }

    #[test]
    fn rejects_non_tracks() {
        assert!(parse_track(&format!("https://open.spotify.com/album/{ID}")).is_err());
        assert!(parse_track(&format!("spotify:album:{ID}")).is_err());
        assert!(parse_track("merhaba").is_err());
    }
}
