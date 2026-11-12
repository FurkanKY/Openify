//! Keşfet ve Ana sayfa verisi (Adım 12, 7 Kasım): Spotify'ın iç GraphQL
//! sorguları; istek, hash tablosu / keşfi, kapı ve yeniden deneme
//! `pathfinder.rs`'te (benzer sanatçılar ve aramayla ortak istemci).
//!
//! Sorgular (7 Kasım 2026'da canlı denendi, bkz. `fixtures/explore_*.json`):
//!   - `home`: ana sayfa akışı. Daily Mix, Discover Weekly, Yeni Müzik Radarı,
//!     On Repeat, daylist, tür karışımları, radyolar buradan (sana özel).
//!   - `browseAll`: tür ve ruh hâli karoları (ad, renk).
//!   - `browsePage`: bir karonun sayfası (bölüm bölüm liste / albüm). Aynı
//!     sorgu "Yeni Çıkanlar" ve "Listeler" (Top 50 Türkiye / Global) sayfaları
//!     için de kullanılır (sabit sayfa adresleri).
//!
//! Spotify'ı boğmamak için (`related.rs`'in kuralları, kapı `pathfinder`'da ortak):
//!   - yalnız ekrana gelen sayfa için istenir; alt sayfalar (tür sayfaları)
//!     tıklanınca, önceden alma yok;
//!   - aynı anahtar için aynı anda tek istek (`wire.rs`);
//!   - önbellek (`pages.db`, `explore:<dil>:*`): ana sayfa akışı 6 sa, tür karoları
//!     24 sa, yeni çıkanlar / listeler 6 sa, tür sayfası 24 sa taze; bayat
//!     olan 7 güne kadar gösterilir (arkadan tazelenir);
//!   - 2 sn'de en çok bir istek (benzer sanatçılar ve aramayla ortak kapı), 429'da
//!     Retry-After kadar istek yok, yeniden deneme yok, üst üste iki 429'da
//!     bu oturumda Spotify yolu kapalı (devre kesici).
//! Hata ekranda söylenmez: bayat önbellek varsa o, yoksa sayfa boş / çevrimdışı
//! durumunu gösterir.

use std::{collections::HashSet, sync::Arc};

use anyhow::{Result, bail};
use librespot::core::session::Session;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::{self, Lang};

use super::{
    cache::Db,
    pages::{Kind, PageCache, now_ms},
    pathfinder::{self, BROWSE_ALL, BROWSE_PAGE, HOME, Priority},
    search::{card_image, str_of},
};

const HOUR_MS: i64 = 3600 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// Bayat veri bu kadar süre gösterilir; daha eskisi gösterilmez.
pub const STALE_MAX_MS: i64 = 7 * DAY_MS;

/// Ana sayfa ve tür sayfası bölümlerinde en çok kart.
const MAX_CARDS: usize = 24;
/// Spotify'ın tür sayfası adresleri (`browseAll` listesindeki sabit kimlikler).
const NEW_RELEASES_PAGE: &str = "spotify:page:0JQ5DAqbMKFz6FAsUtgAab";
const CHARTS_PAGE: &str = "spotify:page:0JQ5DAudkNjCgYMM0TZXDw";
/// Ana sayfa sorgusu `timeZone` ister (günün saatine göre "daylist" vb.).
const TIME_ZONE: &str = "Europe/Istanbul";
const INTEGRATION: &str = "INTEGRATION_WEB_PLAYER";

// ── Türler ──────────────────────────────────────────────────────────────

/// İstenebilen bir veri parçası.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExploreKey {
    /// Ana sayfa akışı (sana özel karışımlar).
    Home,
    /// Tür ve ruh hâli karoları.
    Categories,
    NewReleases,
    /// Trend listeler (Top 50 Türkiye / Global, ...).
    Charts,
    /// Bir türün sayfası (`spotify:page:<id>`).
    Category(String),
}

impl ExploreKey {
    /// Önbellek anahtarı dile bağlıdır (`explore:<dil>:...`): Spotify liste adlarını,
    /// bölüm başlıklarını ve açıklamaları `accept-language`'a göre yerelleştirir;
    /// arayüz dili değişince önceki dilin sayfası gösterilmez, yenisi istenir.
    pub(crate) fn cache_key(&self, lang: Lang) -> String {
        let lang = lang.code();
        match self {
            Self::Home => format!("explore:{lang}:home"),
            Self::Categories => format!("explore:{lang}:categories"),
            Self::NewReleases => format!("explore:{lang}:new-releases"),
            Self::Charts => format!("explore:{lang}:charts"),
            Self::Category(uri) => format!("explore:{lang}:category:{uri}"),
        }
    }

    /// Bu kadar süre taze: yeniden istenmez.
    pub fn fresh_ms(&self) -> i64 {
        match self {
            Self::Home | Self::NewReleases | Self::Charts => 6 * HOUR_MS,
            Self::Categories | Self::Category(_) => 24 * HOUR_MS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardKind {
    Playlist,
    Album,
}

/// Bir kart: Spotify'dan gelen liste ya da albüm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploreCard {
    pub uri: String,
    pub name: String,
    /// Açıklama (listede) ya da sanatçılar (albümde); yoksa boş.
    pub subtitle: String,
    /// Kapak (~300 px); yoksa boş.
    pub image: String,
    pub kind: CardKind,
    /// Spotify'ın liste biçimi (`daily-mix`, `chart`, ...); albümde boş.
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploreSection {
    pub title: String,
    pub cards: Vec<ExploreCard>,
}

/// Bölümlerden oluşan sayfa: ana sayfa akışı, yeni çıkanlar, listeler, tür
/// sayfası.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    /// Sayfa başlığı (tür sayfasında türün adı); yoksa boş.
    pub title: String,
    /// Sayfanın rengi (`#rrggbb`); yoksa boş.
    pub color: String,
    pub sections: Vec<ExploreSection>,
}

impl Feed {
    /// Keşfet'in "Trend listeler" satırı: listeler sayfasının öne çıkan
    /// bölümü (Top 50 Türkiye / Global, haftalık listeler). Sayfa onlarca
    /// ülke listesi de içeriyor; öne çıkan bölüm hep en kısa olan (tek
    /// kartlıklar, ör. müzik videosu listesi, sayılmaz).
    pub fn featured(&self) -> Option<&ExploreSection> {
        self.sections
            .iter()
            .filter(|s| s.cards.len() >= 2)
            .min_by_key(|s| s.cards.len())
    }

    /// Keşfet'in "Yeni çıkanlar" satırı: sayfadaki bütün kartlar (editörün
    /// yeni çıkanlar listesi önde, sonra albümler); sana özel Yeni Müzik
    /// Radarı burada değil (Ana sayfa'da).
    pub fn new_releases(&self) -> Vec<&ExploreCard> {
        let mut seen = HashSet::new();
        self.sections
            .iter()
            .flat_map(|s| &s.cards)
            .filter(|c| c.format != "release-radar" && seen.insert(c.uri.as_str()))
            .take(MAX_CARDS)
            .collect()
    }
}

/// Bir tür ya da ruh hâli karosu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    /// `spotify:page:<id>`
    pub uri: String,
    pub name: String,
    /// `#rrggbb`; yoksa boş.
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryList {
    pub items: Vec<Category>,
}

/// Önbellekteki / gelen veri (arayüze giden).
#[derive(Debug, Clone)]
pub enum ExploreData {
    Feed(Arc<Feed>),
    Categories(Arc<CategoryList>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Göster, isteme.
    Fresh,
    /// Göster, arkadan tazele.
    Stale,
    /// Gösterme, iste.
    Expired,
}

/// Önbellekteki verinin tazeliği (saf: testlenir).
pub fn freshness(key: &ExploreKey, age_ms: i64) -> Freshness {
    if age_ms < key.fresh_ms() {
        Freshness::Fresh
    } else if age_ms < STALE_MAX_MS {
        Freshness::Stale
    } else {
        Freshness::Expired
    }
}

/// Önbellekteki veri ve tazeliği; yoksa ya da 7 günden eskiyse `None`.
pub fn cached(cache: &PageCache, key: &ExploreKey) -> Option<(ExploreData, Freshness)> {
    cached_at(cache, key, now_ms(), i18n::lang())
}

fn cached_at(
    cache: &PageCache,
    key: &ExploreKey,
    now: i64,
    lang: Lang,
) -> Option<(ExploreData, Freshness)> {
    let ck = key.cache_key(lang);
    let (data, age) = if matches!(key, ExploreKey::Categories) {
        let hit = cache.get::<CategoryList>(Kind::Explore, &ck, now).ok()??;
        (ExploreData::Categories(hit.page), hit.age_ms)
    } else {
        let hit = cache.get::<Feed>(Kind::Explore, &ck, now).ok()??;
        (ExploreData::Feed(hit.page), hit.age_ms)
    };
    match freshness(key, age) {
        Freshness::Expired => None,
        state => Some((data, state)),
    }
}

fn store(cache: &PageCache, key: &ExploreKey, data: &ExploreData, lang: Lang) -> Result<()> {
    let ck = key.cache_key(lang);
    match data {
        ExploreData::Feed(f) => cache.put(&ck, f, now_ms())?,
        ExploreData::Categories(c) => cache.put(&ck, c, now_ms())?,
    };
    Ok(())
}

// ── Ayrıştırma ──────────────────────────────────────────────────────────

/// `<a href=...>` gibi etiketleri ve HTML karakter kodlarını temizler.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

/// Bir bölüm öğesinin `content`'i → kart. Liste ve albüm dışı (podcast, bölüm,
/// "bulunamadı") `None`.
fn parse_card(content: &Value) -> Option<ExploreCard> {
    let d = &content["data"];
    match d["__typename"].as_str()? {
        "Playlist" => {
            let uri = d["uri"].as_str()?;
            let name = str_of(&d["name"]).trim().to_string();
            if !uri.starts_with("spotify:playlist:") || name.is_empty() {
                return None;
            }
            let mut subtitle = plain_text(d["description"].as_str().unwrap_or_default());
            if subtitle.is_empty() {
                subtitle = str_of(&d["ownerV2"]["data"]["name"]);
            }
            Some(ExploreCard {
                uri: uri.to_string(),
                name,
                subtitle,
                image: card_image(&d["images"]["items"][0]["sources"]),
                kind: CardKind::Playlist,
                format: str_of(&d["format"]),
            })
        }
        "Album" => {
            let uri = d["uri"].as_str()?;
            let name = str_of(&d["name"]).trim().to_string();
            if !uri.starts_with("spotify:album:") || name.is_empty() {
                return None;
            }
            Some(ExploreCard {
                uri: uri.to_string(),
                name,
                subtitle: artist_names(&d["artists"]),
                image: card_image(&d["coverArt"]["sources"]),
                kind: CardKind::Album,
                format: String::new(),
            })
        }
        _ => None,
    }
}

/// Bölümün kartları (tekrarsız, bozuk / bulunamayan öğeler atlanır).
fn section_cards(section: &Value) -> Vec<ExploreCard> {
    let mut seen = HashSet::new();
    section["sectionItems"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| parse_card(&item["content"]))
        .filter(|c| seen.insert(c.uri.clone()))
        .take(MAX_CARDS)
        .collect()
}

/// GraphQL hatası var ve veri yok mu? Hata metni.
fn graphql_error(value: &Value) -> Option<String> {
    value["errors"][0]["message"].as_str().map(String::from)
}

/// Ana sayfa akışındaki sana özel listelerin bölümü: Spotify'ın liste
/// biçimine göre (başlıklar İngilizce / Türkçe gelebilir, biçim değişmez).
fn home_group(format: &str) -> Option<usize> {
    match format {
        "daily-mix" | "discover-weekly" | "release-radar" | "on-repeat" | "repeat-rewind"
        | "daylist" => Some(0),
        "topic-mix" | "artist-mix-reader" | "descripto" | "genre-mix" => Some(1),
        "inspiredby-mix" => Some(2),
        _ => None,
    }
}

const HOME_GROUP_TITLES: [&str; 3] = ["Sana özel", "Karışımların", "Radyolar"];

/// Sana özel bölümündeki sıra: günlük karışımlar, haftalıklar, sonra gerisi.
fn home_rank(format: &str) -> u8 {
    match format {
        "daily-mix" => 0,
        "discover-weekly" => 1,
        "release-radar" => 2,
        "on-repeat" => 3,
        "repeat-rewind" => 4,
        _ => 5,
    }
}

/// `data.home.sectionContainer.sections.items[]` → üç bölüm (sana özel,
/// karışımların, radyolar). Spotify'ın kendi bölüm başlıkları kullanılmaz:
/// kullanıcı adı taşıyorlar ve dile göre değişiyorlar; ayrıca aynı liste
/// birkaç bölümde birden geçiyor (tekrarsızlaştırılır).
pub fn parse_home(value: &Value) -> Result<Feed> {
    let root = &value["data"]["home"];
    let Some(sections) = root["sectionContainer"]["sections"]["items"].as_array() else {
        bail!(
            "{}",
            graphql_error(value).unwrap_or_else(|| "ana sayfa akışı boş".into())
        );
    };
    let mut groups: [Vec<ExploreCard>; 3] = Default::default();
    let mut seen = HashSet::new();
    for section in sections {
        for item in section["sectionItems"]["items"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(card) = parse_card(&item["content"]) else {
                continue;
            };
            let Some(group) = home_group(&card.format) else {
                continue;
            };
            if seen.insert(card.uri.clone()) {
                groups[group].push(card);
            }
        }
    }
    groups[0].sort_by_key(|c| home_rank(&c.format));
    let sections: Vec<ExploreSection> = groups
        .into_iter()
        .zip(HOME_GROUP_TITLES)
        .filter(|(cards, _)| !cards.is_empty())
        .map(|(mut cards, title)| {
            cards.truncate(MAX_CARDS);
            ExploreSection {
                title: title.into(),
                cards,
            }
        })
        .collect();
    if sections.is_empty() {
        bail!("ana sayfa akışında sana özel liste yok");
    }
    Ok(Feed {
        title: String::new(),
        color: String::new(),
        sections,
    })
}

/// Karo olarak gösterilmeyen sayfalar: müzik dışı olanlar ve Keşfet'te
/// kendi bölümü olanlar (yeni çıkanlar, listeler, sana özel).
const HIDDEN_PAGES: &[&str] = &[
    "0JQ5DAqbMKFSi39LMRT0Cy", // Müzik (üst başlık)
    "0JQ5DArNBzkmxXHCqFLx2J", // Podcast'ler
    "0JQ5DAqbMKFETqK4t8f1n3", // Sesli kitaplar
    "0JQ5IMCbQBLl5gxKLgufp8", // Eğitim
    "0JQ5IMCbQBLjfX9OdDrA5X", // Belgesel
    "0JQ5IMCbQBLyUJhSxhaPc6", // Komedi
    "0JQ5DAt0tbjZptfcdMSKl3", // Senin için Hazırlandı (Ana sayfa'da)
    "0JQ5DAqbMKFz6FAsUtgAab", // Yeni Çıkanlar (Keşfet'te kendi bölümü)
    "0JQ5DAudkNjCgYMM0TZXDw", // Listeler (Keşfet'te "Trend listeler")
    "0JQ5DAqbMKFQIL0AXnG5AK", // Popüler
];

/// `data.browseStart.sections.items[0].sectionItems.items[]` → karolar.
/// Sayfa olmayanlar (xlink özellik kartları) ve gizlenenler atlanır.
pub fn parse_categories(value: &Value) -> Result<CategoryList> {
    let Some(sections) = value["data"]["browseStart"]["sections"]["items"].as_array() else {
        bail!(
            "{}",
            graphql_error(value).unwrap_or_else(|| "tür listesi boş".into())
        );
    };
    let mut seen = HashSet::new();
    let items: Vec<Category> = sections
        .iter()
        .flat_map(|s| s["sectionItems"]["items"].as_array().into_iter().flatten())
        .filter_map(|item| {
            let uri = item["uri"].as_str()?;
            let id = uri.strip_prefix("spotify:page:")?;
            if HIDDEN_PAGES.contains(&id) || !seen.insert(uri.to_string()) {
                return None;
            }
            let card = &item["content"]["data"]["data"]["cardRepresentation"];
            let name = str_of(&card["title"]["transformedLabel"])
                .trim()
                .to_string();
            if name.is_empty() {
                return None;
            }
            Some(Category {
                uri: uri.to_string(),
                name,
                color: valid_color(card["backgroundColor"]["hex"].as_str()),
            })
        })
        .collect();
    if items.is_empty() {
        bail!("tür listesi boş");
    }
    Ok(CategoryList { items })
}

/// `#rrggbb` ise aynısı, değilse boş.
fn valid_color(hex: Option<&str>) -> String {
    match hex {
        Some(h)
            if h.len() == 7
                && h.starts_with('#')
                && h[1..].chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            h.to_lowercase()
        }
        _ => String::new(),
    }
}

/// `data.browse` (tür sayfası, yeni çıkanlar, listeler) → bölümler. Kart
/// çıkmayan bölüm atlanır.
pub fn parse_browse(value: &Value) -> Result<Feed> {
    let b = &value["data"]["browse"];
    if b["__typename"].as_str() != Some("BrowseSectionContainer") {
        bail!(
            "{}",
            graphql_error(value).unwrap_or_else(|| "sayfa bulunamadı".into())
        );
    }
    let sections: Vec<ExploreSection> = b["sections"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let cards = section_cards(s);
            let title = str_of(&s["data"]["title"]["transformedLabel"])
                .trim()
                .to_string();
            (!cards.is_empty()).then_some(ExploreSection { title, cards })
        })
        .collect();
    if sections.is_empty() {
        bail!("sayfada gösterilecek bir şey yok");
    }
    Ok(Feed {
        title: str_of(&b["header"]["title"]["transformedLabel"])
            .trim()
            .to_string(),
        color: valid_color(b["header"]["color"]["hex"].as_str()),
        sections,
    })
}

// ── İstek ───────────────────────────────────────────────────────────────

fn browse_variables(uri: &str) -> Value {
    json!({
        "pagePagination": { "offset": 0, "limit": 10 },
        "sectionPagination": { "offset": 0, "limit": 10 },
        "uri": uri,
        "browseEndUserIntegration": INTEGRATION,
        "includeEpisodeContentRatingsV2": false,
    })
}

fn home_variables() -> Value {
    json!({
        "includeHomeChapterVideoCards": false,
        "homeEndUserIntegration": INTEGRATION,
        "timeZone": TIME_ZONE,
        "sp_t": "",
        "facet": "",
        "sectionItemsLimit": 10,
        "includeEpisodeContentRatingsV2": false,
    })
}

fn categories_variables() -> Value {
    json!({
        "pagePagination": { "offset": 0, "limit": 10 },
        "sectionPagination": { "offset": 0, "limit": 99 },
        "browseEndUserIntegration": INTEGRATION,
    })
}

/// Bir sorguyu pathfinder istemcisinden geçirip gönderir (kapı, eski hash'te
/// web player'dan yenisi, yeniden deneme orada). Hata önbelleğe yazılmaz.
async fn fetch(
    session: &Session,
    db: Option<&Db>,
    op: &pathfinder::Operation,
    variables: Value,
) -> Result<Value> {
    Ok(pathfinder::query(session, db, op, variables, Priority::Background).await?)
}

/// Veriyi indirir ve önbelleğe yazar. Hata önbelleğe yazılmaz.
pub async fn load(
    session: &Session,
    db: Option<&Db>,
    cache: &PageCache,
    key: &ExploreKey,
) -> Result<ExploreData> {
    let started = std::time::Instant::now();
    // İstek başlarken etkin dil: yanıt bu dilde istenir ve bu dilin anahtarına yazılır.
    let lang = i18n::lang();
    let data = match key {
        ExploreKey::Home => {
            let v = fetch(session, db, &HOME, home_variables()).await?;
            ExploreData::Feed(Arc::new(parse_home(&v)?))
        }
        ExploreKey::Categories => {
            let v = fetch(session, db, &BROWSE_ALL, categories_variables()).await?;
            ExploreData::Categories(Arc::new(parse_categories(&v)?))
        }
        ExploreKey::NewReleases => {
            let v = fetch(
                session,
                db,
                &BROWSE_PAGE,
                browse_variables(NEW_RELEASES_PAGE),
            )
            .await?;
            ExploreData::Feed(Arc::new(parse_browse(&v)?))
        }
        ExploreKey::Charts => {
            let v = fetch(session, db, &BROWSE_PAGE, browse_variables(CHARTS_PAGE)).await?;
            ExploreData::Feed(Arc::new(parse_browse(&v)?))
        }
        ExploreKey::Category(uri) => {
            if !uri.starts_with("spotify:page:") {
                bail!("geçersiz tür adresi");
            }
            let v = fetch(session, db, &BROWSE_PAGE, browse_variables(uri)).await?;
            ExploreData::Feed(Arc::new(parse_browse(&v)?))
        }
    };
    store(cache, key, &data, lang)?;
    log::debug!("keşfet {key:?}: {} ms", started.elapsed().as_millis());
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(s: &str) -> Value {
        serde_json::from_str(s).expect("fixture JSON")
    }

    fn home() -> Value {
        fixture(include_str!("fixtures/explore_home.json"))
    }
    fn categories() -> Value {
        fixture(include_str!("fixtures/explore_categories.json"))
    }
    fn category_page() -> Value {
        fixture(include_str!("fixtures/explore_category_page.json"))
    }
    fn charts() -> Value {
        fixture(include_str!("fixtures/explore_charts.json"))
    }
    fn new_releases() -> Value {
        fixture(include_str!("fixtures/explore_new_releases.json"))
    }

    #[test]
    fn home_groups_personal_mixes_by_format_without_duplicates() {
        let feed = parse_home(&home()).unwrap();
        let titles: Vec<&str> = feed.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles[0], "Sana özel");
        let first = &feed.sections[0].cards;
        // Günlük karışımlar önde, sonra Discover Weekly, sonra Radar.
        assert!(
            first.first().is_some_and(|c| c.format == "daily-mix"),
            "{first:?}"
        );
        let ranks: Vec<u8> = first.iter().map(|c| home_rank(&c.format)).collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
        let mut all: Vec<&str> = feed
            .sections
            .iter()
            .flat_map(|s| s.cards.iter().map(|c| c.uri.as_str()))
            .collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "aynı liste iki kez");
        assert!(
            feed.sections
                .iter()
                .flat_map(|s| &s.cards)
                .all(|c| c.kind == CardKind::Playlist)
        );
    }

    #[test]
    fn home_cards_have_clean_text_and_no_username() {
        let feed = parse_home(&home()).unwrap();
        for c in feed.sections.iter().flat_map(|s| &s.cards) {
            assert!(
                !c.subtitle.contains('<') && !c.subtitle.contains("&#"),
                "{}",
                c.subtitle
            );
            assert!(!c.name.is_empty());
        }
        assert!(!serde_json::to_string(&feed).unwrap().contains("furkan"));
    }

    #[test]
    fn home_without_personal_lists_is_an_error() {
        let v =
            json!({ "data": { "home": { "sectionContainer": { "sections": { "items": [] } } } } });
        assert!(parse_home(&v).is_err());
        assert!(
            parse_home(&json!({ "errors": [ { "message": "boom" } ] }))
                .unwrap_err()
                .to_string()
                .contains("boom")
        );
    }

    #[test]
    fn categories_keep_colors_and_drop_non_music() {
        let list = parse_categories(&categories()).unwrap();
        let names: Vec<&str> = list.items.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"Hip Hop") && names.contains(&"Rock"),
            "{names:?}"
        );
        for hidden in [
            "Podcast'ler",
            "Sesli kitaplar",
            "Müzik",
            "Yeni Çıkanlar",
            "Listeler",
            "Komedi",
        ] {
            assert!(!names.contains(&hidden), "{hidden} gizlenmeliydi");
        }
        let hiphop = list.items.iter().find(|c| c.name == "Hip Hop").unwrap();
        assert_eq!(hiphop.color, "#2d46b9");
        assert_eq!(hiphop.uri, "spotify:page:0JQ5DAqbMKFQ00XGBls6ym");
        assert!(
            list.items
                .iter()
                .all(|c| c.uri.starts_with("spotify:page:") && c.color.len() == 7)
        );
    }

    #[test]
    fn category_page_has_header_color_and_skips_not_found_items() {
        let feed = parse_browse(&category_page()).unwrap();
        assert_eq!(feed.title, "Hip Hop");
        assert_eq!(feed.color, "#8d67ab");
        assert!(feed.sections.len() >= 3);
        let first = &feed.sections[0];
        // Fixture: 5 öğenin 4.sü "NotFound": kart olmamalı (4 kart kalır).
        assert_eq!(
            first.cards.len(),
            4,
            "{:?}",
            first.cards.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        assert!(
            feed.sections
                .iter()
                .flat_map(|s| &s.cards)
                .all(|c| c.uri.starts_with("spotify:"))
        );
        // Albüm bölümü: sanatçı adı alt başlıkta.
        let albums = feed
            .sections
            .iter()
            .find(|s| s.cards[0].kind == CardKind::Album)
            .unwrap();
        assert!(
            !albums.cards[0].subtitle.is_empty() || albums.cards[0].image.starts_with("https://")
        );
    }

    #[test]
    fn charts_have_a_featured_row_with_top_50_turkey() {
        let feed = parse_browse(&charts()).unwrap();
        let featured = feed
            .sections
            .iter()
            .find(|s| s.title == "Öne Çıkan Listeler")
            .expect("öne çıkan listeler");
        assert!(
            featured.cards.iter().any(|c| c.name.contains("Türkiye")),
            "{:?}",
            featured.cards
        );
        assert!(
            featured
                .cards
                .iter()
                .all(|c| c.format == "chart" && c.image.starts_with("https://"))
        );
    }

    #[test]
    fn featured_charts_row_is_the_short_section_and_new_releases_skip_the_radar() {
        let charts = parse_browse(&charts()).unwrap();
        let featured = charts.featured().unwrap();
        assert!(featured.title.contains("Öne Çıkan"), "{}", featured.title);
        assert!(featured.cards.iter().any(|c| c.name.contains("Türkiye")));
        let fresh = parse_browse(&new_releases()).unwrap();
        let cards = fresh.new_releases();
        assert!(cards.iter().all(|c| c.format != "release-radar"));
        assert_eq!(
            cards.first().map(|c| c.kind),
            Some(CardKind::Playlist),
            "editörün listesi önde"
        );
        assert!(cards.iter().filter(|c| c.kind == CardKind::Album).count() >= 8);
    }

    #[test]
    fn new_releases_hold_albums_with_artist_names() {
        let feed = parse_browse(&new_releases()).unwrap();
        let albums: Vec<&ExploreCard> = feed
            .sections
            .iter()
            .flat_map(|s| &s.cards)
            .filter(|c| c.kind == CardKind::Album)
            .collect();
        assert!(albums.len() >= 8);
        assert!(
            albums
                .iter()
                .all(|a| !a.subtitle.is_empty() && a.image.starts_with("https://i.scdn.co/"))
        );
        // Albüm kapağı ~300 px'lik olanı seçilir (640 değil).
        assert!(
            albums[0].image.ends_with("1e02ae49c87dcf0b8e27f27303c5")
                || albums[0].image.contains("00001e02"),
            "{}",
            albums[0].image
        );
    }

    #[test]
    fn browse_errors_are_errors_not_empty_pages() {
        assert!(
            parse_browse(&json!({ "data": { "browse": { "__typename": "NotFound" } } })).is_err()
        );
        assert!(
            parse_browse(&json!({ "errors": [ { "message": "PersistedQueryNotFound" } ] }))
                .is_err()
        );
    }

    #[test]
    fn plain_text_strips_tags_and_entities() {
        assert_eq!(
            plain_text(
                "Quiet music for you. Also try <a href=\"spotify:playlist:x\">Quiet Mix</a>"
            ),
            "Quiet music for you. Also try Quiet Mix"
        );
        assert_eq!(plain_text("Here&#x27;s  a &amp; b"), "Here's a & b");
    }

    #[test]
    fn freshness_follows_per_key_rules() {
        let h = |n: i64| n * HOUR_MS;
        assert_eq!(freshness(&ExploreKey::Home, h(5)), Freshness::Fresh);
        assert_eq!(freshness(&ExploreKey::Home, h(7)), Freshness::Stale);
        assert_eq!(freshness(&ExploreKey::Categories, h(20)), Freshness::Fresh);
        assert_eq!(freshness(&ExploreKey::Categories, h(25)), Freshness::Stale);
        assert_eq!(
            freshness(&ExploreKey::Category("spotify:page:x".into()), h(23)),
            Freshness::Fresh
        );
        assert_eq!(
            freshness(&ExploreKey::NewReleases, 6 * DAY_MS),
            Freshness::Stale
        );
        assert_eq!(
            freshness(&ExploreKey::Charts, 8 * DAY_MS),
            Freshness::Expired
        );
    }

    #[test]
    fn cache_round_trip_uses_per_key_type() {
        let cache = PageCache::open_in_memory().unwrap();
        let feed = Arc::new(parse_browse(&charts()).unwrap());
        store(
            &cache,
            &ExploreKey::Charts,
            &ExploreData::Feed(feed.clone()),
            Lang::Tr,
        )
        .unwrap();
        let list = Arc::new(parse_categories(&categories()).unwrap());
        store(
            &cache,
            &ExploreKey::Categories,
            &ExploreData::Categories(list.clone()),
            Lang::Tr,
        )
        .unwrap();
        let (data, state) =
            cached_at(&cache, &ExploreKey::Charts, now_ms() + HOUR_MS, Lang::Tr).unwrap();
        assert_eq!(state, Freshness::Fresh);
        assert!(matches!(data, ExploreData::Feed(f) if *f == *feed));
        let (data, _) = cached_at(
            &cache,
            &ExploreKey::Categories,
            now_ms() + 30 * HOUR_MS,
            Lang::Tr,
        )
        .unwrap();
        assert!(matches!(data, ExploreData::Categories(c) if *c == *list));
        // 7 günden eski: gösterilmez.
        assert!(cached_at(&cache, &ExploreKey::Charts, now_ms() + 8 * DAY_MS, Lang::Tr).is_none());
        // Hiç yazılmamış anahtar.
        assert!(cached_at(&cache, &ExploreKey::Home, now_ms(), Lang::Tr).is_none());
    }

    #[test]
    fn cache_is_separate_per_interface_language() {
        let cache = PageCache::open_in_memory().unwrap();
        let feed = Arc::new(parse_browse(&charts()).unwrap());
        store(
            &cache,
            &ExploreKey::Charts,
            &ExploreData::Feed(feed),
            Lang::Tr,
        )
        .unwrap();
        let now = now_ms() + HOUR_MS;
        assert!(cached_at(&cache, &ExploreKey::Charts, now, Lang::Tr).is_some());
        // Dil değişince Türkçe sayfa İngilizce arayüzde gösterilmez.
        assert!(cached_at(&cache, &ExploreKey::Charts, now, Lang::En).is_none());
        assert_ne!(
            ExploreKey::Home.cache_key(Lang::En),
            ExploreKey::Home.cache_key(Lang::Tr)
        );
        assert_ne!(
            ExploreKey::Category("spotify:page:x".into()).cache_key(Lang::En),
            ExploreKey::Category("spotify:page:x".into()).cache_key(Lang::Tr)
        );
    }
}
