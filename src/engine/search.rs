//! Spotify'ın tamamında arama: resmi uygulamanın GraphQL araması
//! (`searchDesktop`). İstek, hash keşfi, kapı ve yeniden deneme
//! `pathfinder.rs`'te; burada yalnız değişkenler ve yanıtın çözümlenmesi.
//!
//! Kırılganlık: `sha256Hash` koda gömülmüyor. Spotify değiştirirse web
//! player'ın JavaScript paketinden yenisi otomatik bulunur
//! (`pathfinder::SEARCH_DESKTOP`) ve kalıcı kılınır; her açılışta yeniden
//! aranmaz.

use anyhow::Result;
use librespot::core::session::Session;
use serde_json::{Value, json};

use super::{
    cache::Db,
    library::TrackRow,
    pathfinder::{self, PathfinderError, Priority, SEARCH_DESKTOP},
};

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub uri: String,
    pub name: String,
    pub subtitle: String,
    /// Kapak ya da portre (en küçük ≥ 64 px); yoksa boş.
    pub image: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub tracks: Vec<TrackRow>,
    pub artists: Vec<SearchHit>,
    pub albums: Vec<SearchHit>,
    pub playlists: Vec<SearchHit>,
    /// Podcast'ler (show). Altyazı: yayıncı (yoksa boş).
    pub podcasts: Vec<SearchHit>,
    /// Podcast bölümleri. Altyazı: podcast adı (+ yıl).
    pub episodes: Vec<SearchHit>,
    /// Kullanıcı profilleri. Altyazı boş (arayüz "Profil" yazar).
    pub profiles: Vec<SearchHit>,
}

impl SearchResults {
    /// Bütün türlerdeki sonuç sayısı.
    pub fn total(&self) -> usize {
        self.tracks.len()
            + self.artists.len()
            + self.albums.len()
            + self.playlists.len()
            + self.podcasts.len()
            + self.episodes.len()
            + self.profiles.len()
    }
}

fn variables(query: &str, limit: usize) -> Value {
    json!({
        "searchTerm": query,
        "offset": 0,
        "limit": limit,
        "numberOfTopResults": 5,
        "includeAudiobooks": false,
        "includePreReleases": false,
        "includeLocalConcertsField": false,
        "includeArtistHasConcertsField": false,
    })
}

/// Arama: hash'i istemci yönetir (kayıtlı hash, eskiyse web player'dan yenisi,
/// kapı); yanıt çözülür.
pub async fn run(session: &Session, db: Option<&Db>, query: &str, limit: usize) -> Result<SearchResults, PathfinderError> {
    let value = pathfinder::query(session, db, &SEARCH_DESKTOP, variables(query, limit), Priority::Interactive).await?;
    Ok(parse_results(&value))
}

/// Geliştirme kancası: `OPENIFY_TEST_SEARCH_JSON=<dosya>` verilirse tam arama
/// sayfası Spotify'a gitmeden bu kayıtlı yanıttan dolar (ekran sınaması;
/// `fixtures/search_desktop.json` ile aynı biçim).
pub fn test_results() -> Option<SearchResults> {
    let path = std::env::var_os("OPENIFY_TEST_SEARCH_JSON")?;
    let mut results = from_json(&std::fs::read_to_string(path).ok()?)?;
    // Kayıttaki resim adresleri uydurma: ağa hiç çıkılmasın, yer tutucular görünsün.
    for hit in [&mut results.artists, &mut results.albums, &mut results.playlists, &mut results.podcasts, &mut results.episodes, &mut results.profiles]
        .into_iter()
        .flatten()
    {
        hit.image.clear();
    }
    for row in &mut results.tracks {
        row.image.clear();
    }
    Some(results)
}

/// Kayıtlı bir `searchDesktop` yanıtını çözer (testler ve geliştirme kancası).
pub fn from_json(text: &str) -> Option<SearchResults> {
    Some(parse_results(&serde_json::from_str(text).ok()?))
}

/// Yanıt `pathfinder` tarafından doğrulandı (hata yok, `searchV2` var).
fn parse_results(value: &Value) -> SearchResults {
    let root = &value["data"]["searchV2"];

    let items = |key: &str| -> Vec<Value> {
        root[key]["items"].as_array().cloned().unwrap_or_default()
    };

    let tracks = items("tracksV2")
        .iter()
        .filter_map(|i| {
            let d = &i["item"]["data"];
            Some(TrackRow {
                uri: d["uri"].as_str()?.to_string(),
                name: str_of(&d["name"]),
                artists: artist_names(&d["artists"]),
                album: str_of(&d["albumOfTrack"]["name"]),
                duration_ms: d["duration"]["totalMilliseconds"].as_u64().unwrap_or(0) as u32,
                image: smallest_image(&d["albumOfTrack"]["coverArt"]["sources"]),
            })
        })
        .collect();

    let artists = items("artists")
        .iter()
        .filter_map(|i| {
            let d = &i["data"];
            Some(SearchHit {
                uri: d["uri"].as_str()?.to_string(),
                name: str_of(&d["profile"]["name"]),
                subtitle: crate::i18n::canon::ARTIST.into(),
                image: card_image(&d["visuals"]["avatarImage"]["sources"]),
            })
        })
        .collect();

    let albums = items("albumsV2")
        .iter()
        .filter_map(|i| {
            let d = &i["data"];
            let year = d["date"]["year"].as_u64().map(|y| format!(" · {y}")).unwrap_or_default();
            Some(SearchHit {
                uri: d["uri"].as_str()?.to_string(),
                name: str_of(&d["name"]),
                subtitle: format!("{}{year}", artist_names(&d["artists"])),
                image: card_image(&d["coverArt"]["sources"]),
            })
        })
        .collect();

    let playlists = items("playlists")
        .iter()
        .filter_map(|i| {
            let d = &i["data"];
            let owner = str_of(&d["ownerV2"]["data"]["name"]);
            Some(SearchHit {
                uri: d["uri"].as_str()?.to_string(),
                name: str_of(&d["name"]),
                subtitle: if owner.is_empty() { crate::i18n::canon::PLAYLIST.into() } else { owner },
                image: card_image(&d["images"]["items"][0]["sources"]),
            })
        })
        .collect();

    let podcasts = items("podcasts")
        .iter()
        .filter_map(|i| {
            let d = node(i);
            let uri = d["uri"].as_str().filter(|u| u.starts_with("spotify:show:"))?;
            Some(SearchHit {
                uri: uri.to_string(),
                name: str_of(&d["name"]),
                subtitle: str_of(&d["publisher"]["name"]),
                image: card_image(&d["coverArt"]["sources"]),
            })
        })
        .collect();

    let episodes = items("episodes")
        .iter()
        .filter_map(|i| {
            let d = node(i);
            let uri = d["uri"].as_str().filter(|u| u.starts_with("spotify:episode:"))?;
            let show = node(&d["podcastV2"]);
            let year = d["releaseDate"]["isoString"]
                .as_str()
                .and_then(|s| s.get(..4))
                .filter(|y| y.bytes().all(|b| b.is_ascii_digit()))
                .map(|y| format!(" · {y}"))
                .unwrap_or_default();
            // Bölümün kendi kapağı yoksa podcast'inki.
            let image = match card_image(&d["coverArt"]["sources"]) {
                own if !own.is_empty() => own,
                _ => card_image(&show["coverArt"]["sources"]),
            };
            Some(SearchHit {
                uri: uri.to_string(),
                name: str_of(&d["name"]),
                subtitle: format!("{}{year}", str_of(&show["name"])),
                image,
            })
        })
        .collect();

    let profiles = items("users")
        .iter()
        .filter_map(|i| {
            let d = node(i);
            let uri = d["uri"].as_str().filter(|u| u.starts_with("spotify:user:"))?;
            // Görünen ad yoksa kullanıcı adı, o da yoksa adresteki kimlik.
            let name = [&d["displayName"], &d["name"], &d["username"], &d["id"]]
                .into_iter()
                .map(str_of)
                .find(|n| !n.trim().is_empty())
                .unwrap_or_else(|| uri.trim_start_matches("spotify:user:").to_string());
            let image = ["avatar", "avatarImage", "image", "images"]
                .into_iter()
                .map(|k| avatar_sources(&d[k]))
                .find(|u| !u.is_empty())
                .unwrap_or_default();
            Some(SearchHit { uri: uri.to_string(), name, subtitle: String::new(), image })
        })
        .collect();

    SearchResults {
        tracks,
        artists,
        albums,
        playlists,
        podcasts,
        episodes,
        profiles,
    }
}

/// Bölüm öğesinin verisi: Spotify bazı bölümlerde `{ "data": … }`, bazılarında
/// `{ "item": { "data": … } }` (şarkılar) sarar; ikisi de çözülür.
fn node(item: &Value) -> &Value {
    if item["item"]["data"].is_object() {
        &item["item"]["data"]
    } else if item["data"].is_object() {
        &item["data"]
    } else {
        item
    }
}

/// Profil resmi: `{ "sources": […] }` ya da `{ "items": [ { "sources": […] } ] }`.
fn avatar_sources(v: &Value) -> String {
    let direct = card_image(&v["sources"]);
    if !direct.is_empty() {
        return direct;
    }
    card_image(&v["items"][0]["sources"])
}

/// `[{url, width, height}, ...]` içinden en küçük (ama 64 px'ten küçük olmayan) resim.
fn smallest_image(sources: &Value) -> String {
    let mut list: Vec<(u64, &str)> = sources
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| Some((s["width"].as_u64().unwrap_or(0), s["url"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|(w, _)| *w);
    list.iter()
        .find(|(w, _)| *w >= 64)
        .or(list.last())
        .map(|(_, u)| u.to_string())
        .unwrap_or_default()
}

/// Kart için resim: 300 px civarı (yoksa en büyüğü). Kartlar 96–160 px
/// çiziliyor; 64 px'lik küçük resim bulanık kalırdı.
pub(super) fn card_image(sources: &Value) -> String {
    let mut list: Vec<(u64, &str)> = sources
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| Some((s["width"].as_u64().unwrap_or(0), s["url"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|(w, _)| *w);
    list.iter()
        .find(|(w, _)| *w >= 240)
        .or(list.last())
        .map(|(_, u)| u.to_string())
        .unwrap_or_default()
}

pub(super) fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn artist_names(artists: &Value) -> String {
    artists["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["profile"]["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_response() {
        let value = json!({ "data": { "searchV2": {
            "tracksV2": { "items": [ { "item": { "data": {
                "uri": "spotify:track:1", "name": "Bring Me To Life",
                "artists": { "items": [ { "profile": { "name": "Evanescence" } } ] },
                "albumOfTrack": { "name": "Fallen" },
                "duration": { "totalMilliseconds": 235893 } } } } ] },
            "artists": { "items": [ { "data": { "uri": "spotify:artist:a", "profile": { "name": "Evanescence" } } } ] },
            "albumsV2": { "items": [ { "data": { "uri": "spotify:album:b", "name": "Fallen",
                "artists": { "items": [ { "profile": { "name": "Evanescence" } } ] }, "date": { "year": 2003 } } } ] },
            "playlists": { "items": [ { "data": { "uri": "spotify:playlist:c", "name": "goth rock",
                "ownerV2": { "data": { "name": "ror.exe" } } } } ] }
        } } });
        let r = parse_results(&value);
        assert_eq!(r.tracks[0].name, "Bring Me To Life");
        assert_eq!(r.tracks[0].artists, "Evanescence");
        assert_eq!(r.tracks[0].duration_ms, 235893);
        assert_eq!(r.artists[0].name, "Evanescence");
        assert_eq!(r.albums[0].subtitle, "Evanescence · 2003");
        assert_eq!(r.playlists[0].subtitle, "ror.exe");
        // Eski biçimli yanıtta (podcast, profil yok) yeni listeler boş kalır.
        assert!(r.podcasts.is_empty() && r.episodes.is_empty() && r.profiles.is_empty());
        assert_eq!(r.total(), 4);
    }

    fn fixture() -> SearchResults {
        let value: Value = serde_json::from_str(include_str!("fixtures/search_desktop.json")).unwrap();
        parse_results(&value)
    }

    #[test]
    fn fixture_keeps_the_existing_kinds() {
        let r = fixture();
        // Adresi olmayan şarkı atlanır.
        assert_eq!(r.tracks.len(), 2);
        assert_eq!(r.tracks[1].artists, "Mavi Orman, Kuzey");
        assert_eq!(r.artists.len(), 2);
        assert_eq!(r.albums[0].subtitle, "Mavi Orman · 2021");
        assert_eq!(r.albums[1].subtitle, "Kuzey", "yılsız albüm yalnız sanatçıyı yazar");
        assert_eq!(r.playlists[0].subtitle, "Openify Test");
        assert_eq!(r.playlists[1].subtitle, crate::i18n::canon::PLAYLIST, "sahibi yoksa kanonik 'Liste'");
    }

    #[test]
    fn parses_podcasts() {
        let r = fixture();
        // Adresi olmayan podcast atlanır.
        assert_eq!(r.podcasts.len(), 2);
        assert_eq!(r.podcasts[0].uri, "spotify:show:00000000000000000000s1");
        assert_eq!(r.podcasts[0].name, "Yavaş Sabah");
        assert_eq!(r.podcasts[0].subtitle, "Sakin Yayınlar");
        assert!(r.podcasts[0].image.ends_with("f68ds1"), "kart için ~300 px resim: {}", r.podcasts[0].image);
        // Kapağı ve yayıncısı olmayan podcast boş alanlarla yine listelenir.
        assert_eq!(r.podcasts[1].subtitle, "");
        assert_eq!(r.podcasts[1].image, "");
    }

    #[test]
    fn parses_episodes_with_show_name_and_year() {
        let r = fixture();
        assert_eq!(r.episodes.len(), 2);
        assert_eq!(r.episodes[0].name, "Sessizliğin müziği");
        assert_eq!(r.episodes[0].subtitle, "Yavaş Sabah · 2024");
        assert!(r.episodes[0].image.ends_with("ba8ae1"), "kendi kapağı önce");
        // Bozuk tarih yılı yazmaz; kapak yoksa podcast'in kapağı.
        assert_eq!(r.episodes[1].subtitle, "Yavaş Sabah");
        assert!(r.episodes[1].image.ends_with("ba8as1"));
    }

    #[test]
    fn parses_profiles_with_fallback_names() {
        let r = fixture();
        // Adresi olmayan kullanıcı atlanır.
        assert_eq!(r.profiles.len(), 2);
        assert_eq!(r.profiles[0].name, "Selin K.");
        assert_eq!(r.profiles[0].uri, "spotify:user:selin.k");
        assert_eq!(r.profiles[0].subtitle, "", "tür adını arayüz yazar");
        assert!(r.profiles[0].image.ends_with("3b82u1"));
        // Görünen ad yoksa kullanıcı adı; avatar null ise resim boş.
        assert_eq!(r.profiles[1].name, "12345678");
        assert_eq!(r.profiles[1].image, "");
        assert_eq!(r.total(), 2 + 2 + 2 + 2 + 2 + 2 + 2);
    }

    #[test]
    fn items_are_found_with_or_without_the_item_wrapper() {
        let wrapped = json!({ "item": { "data": { "uri": "spotify:show:a" } } });
        let plain = json!({ "data": { "uri": "spotify:show:b" } });
        let bare = json!({ "uri": "spotify:show:c" });
        assert_eq!(node(&wrapped)["uri"], "spotify:show:a");
        assert_eq!(node(&plain)["uri"], "spotify:show:b");
        assert_eq!(node(&bare)["uri"], "spotify:show:c");
        assert_eq!(avatar_sources(&json!({ "items": [ { "sources": [ { "url": "u", "width": 300 } ] } ] })), "u");
    }
}
