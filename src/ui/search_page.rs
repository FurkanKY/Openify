//! Tam arama sayfasının çipleri ve podcast / bölüm / profil kartları.
//!
//! Çipler (Hepsi · Şarkılar · Sanatçılar · Albümler · Listeler · Podcast'ler ·
//! Profiller) sayfanın başlığının altında tek sıra; seçilen tür o türün bütün
//! sonuçlarını açar, "Hepsi" karışık görünümdür (her tür bir sıra). Çipler
//! Tab durağıdır (sayfa duraklarının arasında, çizim sırasıyla) ve ← / →
//! aynı gruptaki komşuya geçer (`seg:search:<tür>`, bkz. `Shell::segment_step`).
//!
//! Podcast, bölüm ve profil için uygulamada sayfa yok: kart tıklanınca
//! Spotify'ın web sayfası (`open.spotify.com`) açılır ("Spotify'da aç").

use gpui::{AnyElement, Context, Div, ElementId, FontWeight, SharedString, Window, div, prelude::*, px};

use super::{
    Shell,
    components::{Tooltip, chip},
    cover_image,
    pages::{CARD_GAP, CardSpec},
    text::line_with_tip,
    theme::*,
};
use crate::engine::{SearchHit, SearchResults};
use crate::{t, tf};

/// Çip: hangi tür gösteriliyor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SearchKind {
    #[default]
    All,
    Songs,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Profiles,
}

/// Sonuç bölümleri (bir çip birden çok bölüm gösterebilir: Podcast'ler +
/// Bölümler).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Songs,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Episodes,
    Profiles,
}

/// Bir bölüm seçili çipte nasıl görünür.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Show {
    /// Hiç görünmez.
    Hidden,
    /// Kısa hâl: bir sıra (şarkıda 5 satır) ve "Tümünü göster".
    Few,
    /// Bölümün bütün sonuçları.
    Full,
}

impl SearchKind {
    /// Çiplerin soldan sağa sırası.
    pub const ALL: [SearchKind; 7] = [
        SearchKind::All,
        SearchKind::Songs,
        SearchKind::Artists,
        SearchKind::Albums,
        SearchKind::Playlists,
        SearchKind::Podcasts,
        SearchKind::Profiles,
    ];

    /// Çipin yazısı.
    pub fn label(self) -> &'static str {
        match self {
            SearchKind::All => t!(SearchChipAll),
            SearchKind::Songs => t!(SectionSongs),
            SearchKind::Artists => t!(SectionArtists),
            SearchKind::Albums => t!(KindAlbums),
            SearchKind::Playlists => t!(KindLists),
            SearchKind::Podcasts => t!(SearchChipPodcasts),
            SearchKind::Profiles => t!(SearchChipProfiles),
        }
    }

    /// Tab durağı anahtarının son parçası.
    fn id(self) -> &'static str {
        match self {
            SearchKind::All => "all",
            SearchKind::Songs => "songs",
            SearchKind::Artists => "artists",
            SearchKind::Albums => "albums",
            SearchKind::Playlists => "playlists",
            SearchKind::Podcasts => "podcasts",
            SearchKind::Profiles => "profiles",
        }
    }

    /// Bu çipte `section` nasıl görünür: "Hepsi" her türden bir sıra
    /// gösterir (podcast bölümleri hariç: onlar yalnız Podcast'ler çipinde),
    /// türün kendi çipi yalnız o türü tam açar. Podcast'ler çipi
    /// podcast'leri ve bölümlerini birlikte açar.
    pub fn show(self, section: Section) -> Show {
        use Section as S;
        match (self, section) {
            (SearchKind::All, S::Episodes) => Show::Hidden,
            (SearchKind::All, _) => Show::Few,
            (SearchKind::Songs, S::Songs)
            | (SearchKind::Artists, S::Artists)
            | (SearchKind::Albums, S::Albums)
            | (SearchKind::Playlists, S::Playlists)
            | (SearchKind::Podcasts, S::Podcasts | S::Episodes)
            | (SearchKind::Profiles, S::Profiles) => Show::Full,
            _ => Show::Hidden,
        }
    }

    /// Bu çipte kaç sonuç görünür (boş mu diye).
    pub fn count(self, r: &SearchResults) -> usize {
        let lens = [
            (Section::Songs, r.tracks.len()),
            (Section::Artists, r.artists.len()),
            (Section::Albums, r.albums.len()),
            (Section::Playlists, r.playlists.len()),
            (Section::Podcasts, r.podcasts.len()),
            (Section::Episodes, r.episodes.len()),
            (Section::Profiles, r.profiles.len()),
        ];
        lens.into_iter().filter(|(s, _)| self.show(*s) != Show::Hidden).map(|(_, n)| n).sum()
    }

    /// Bölümlerin başlığı görünsün mü (birden çok bölüm varken)?
    pub fn titled(self) -> bool {
        matches!(self, SearchKind::All | SearchKind::Podcasts)
    }
}

/// Uygulamada sayfası olmayan sonuç: karta tıklayınca Spotify'ın web sayfası
/// açılır.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtCard {
    pub uri: String,
    pub title: String,
    pub subtitle: String,
    pub image: String,
    pub round: bool,
    pub icon: &'static str,
}

/// Podcast'ler (kare kapak).
pub fn podcast_cards(hits: &[SearchHit]) -> Vec<ExtCard> {
    hits.iter()
        .map(|h| ExtCard {
            uri: h.uri.clone(),
            title: h.name.clone(),
            subtitle: if h.subtitle.is_empty() { t!(SearchKindPodcast).into() } else { h.subtitle.clone() },
            image: h.image.clone(),
            round: false,
            icon: "mic-vocal",
        })
        .collect()
}

/// Podcast bölümleri (kare kapak).
pub fn episode_cards(hits: &[SearchHit]) -> Vec<ExtCard> {
    hits.iter()
        .map(|h| ExtCard {
            uri: h.uri.clone(),
            title: h.name.clone(),
            subtitle: if h.subtitle.is_empty() { t!(SearchKindEpisode).into() } else { h.subtitle.clone() },
            image: h.image.clone(),
            round: false,
            icon: "mic-vocal",
        })
        .collect()
}

/// Kullanıcı profilleri (yuvarlak portre, sanatçılar gibi).
pub fn profile_cards(hits: &[SearchHit]) -> Vec<ExtCard> {
    hits.iter()
        .map(|h| ExtCard {
            uri: h.uri.clone(),
            title: h.name.clone(),
            subtitle: t!(SearchKindProfile).into(),
            image: h.image.clone(),
            round: true,
            icon: "user",
        })
        .collect()
}

/// `spotify:show:ID` → `https://open.spotify.com/show/ID`. Yalnız bilinen
/// türler ve tek kimlikli adresler (`spotify:user:x:playlist:y` gibi
/// uzun adresler ve yerel dosyalar `None`); kimlik yüzde-kodlanır.
pub fn web_url(uri: &str) -> Option<String> {
    let mut parts = uri.split(':');
    if parts.next()? != "spotify" {
        return None;
    }
    let kind = parts.next()?;
    let id = parts.next()?;
    if parts.next().is_some() || id.is_empty() {
        return None;
    }
    if !matches!(kind, "show" | "episode" | "user" | "artist" | "album" | "playlist" | "track") {
        return None;
    }
    let mut url = format!("https://open.spotify.com/{kind}/");
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    Some(url)
}

impl Shell {
    /// Çip sırası: sayfa başlığının altında, sola hizalı, dar pencerede alta sarar.
    pub(super) fn search_chips(&self, kind: SearchKind, window: &Window, cx: &mut Context<Self>) -> Div {
        let accent = self.accent().text;
        let mut row = div().px(px(self.margin())).pb(px(S5)).flex().flex_wrap().items_center().gap(px(S2));
        for k in SearchKind::ALL {
            // ← / → aynı grupta gezer (`seg:` anahtarı).
            let (stop, look) = self.stop_look(format!("seg:search:{}", k.id()), super::tab::PAGE, true, window, cx);
            row = row.child(
                chip(ElementId::Name(format!("search-kind-{}", k.id()).into()), k.label(), kind == k, look, accent)
                    .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_search_kind(k, cx))),
            );
        }
        row
    }

    fn set_search_kind(&mut self, kind: SearchKind, cx: &mut Context<Self>) {
        if let Some(search) = self.pages.search.as_mut()
            && search.kind != kind
        {
            search.kind = kind;
            cx.notify();
        }
    }

    /// Sanatçı / albüm / liste bölümü: "Hepsi"de başlıklı bir sıra ve
    /// "Tümünü göster", türün kendi çipinde başlıksız tam ızgara.
    pub(super) fn kind_section(
        &self,
        kind: SearchKind,
        section: Section,
        key: String,
        title: &str,
        cards: Vec<CardSpec>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match kind.show(section) {
            Show::Hidden => None,
            Show::Few => self.card_section(key, title, cards, cx),
            Show::Full if cards.is_empty() => None,
            Show::Full => Some(self.card_grid(&cards, cx).into_any_element()),
        }
    }

    /// Seçili çipte hiçbir sonuç yoksa: sessiz tek satır.
    pub(super) fn search_empty_kind(&self, kind: SearchKind) -> Div {
        div()
            .px(px(self.margin()))
            .pt(px(S3))
            .type_role(T_BODY)
            .text_color(muted())
            .child(tf!(SearchChipEmpty, crate::i18n::lower(kind.label())))
    }

    /// Kart ızgarası: her pencere eninde tam sütun (`Shell::grid`), son sıra
    /// soldan dolar. Bölüm başlığı yok (çip adı yeter).
    pub(super) fn card_grid(&self, cards: &[CardSpec], cx: &mut Context<Self>) -> Div {
        let (_, width) = self.grid();
        self.grid_of(cards.iter().map(|c| self.page_card(c, width, cx)).collect())
    }

    fn grid_of(&self, cards: Vec<AnyElement>) -> Div {
        let (cols, _) = self.grid();
        let mut grid = div().px(px(self.margin())).pb(px(S6)).flex().flex_col().gap(px(S5));
        let mut line = div().flex().gap(px(CARD_GAP));
        let mut n = 0;
        for card in cards {
            line = line.child(card);
            n += 1;
            if n == cols {
                grid = grid.child(std::mem::replace(&mut line, div().flex().gap(px(CARD_GAP))));
                n = 0;
            }
        }
        if n > 0 {
            grid = grid.child(line);
        }
        grid
    }

    /// Spotify'da açılan kartlardan bölüm. `show == Few`: bir sıra ve
    /// "Tümünü göster"; `Full`: hepsi. Başlık `title` verilirse.
    pub(super) fn ext_section(
        &self,
        key: String,
        title: Option<&str>,
        cards: Vec<ExtCard>,
        show: Show,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if cards.is_empty() || show == Show::Hidden {
            return None;
        }
        let (cols, width) = self.grid();
        let expanded = show == Show::Full || self.pages.expanded.contains(&key);
        let shown = if expanded { cards.len() } else { cards.len().min(cols) };
        let built: Vec<AnyElement> = cards[..shown].iter().map(|c| self.ext_card(c, width, cx)).collect();
        let toggle = show == Show::Few && cards.len() > cols;
        Some(
            div()
                .when_some(title, |el, t| el.child(self.section_title(&key, t, toggle, cx)))
                .child(self.grid_of(built))
                .into_any_element(),
        )
    }

    /// `page_card` ile aynı görünüm; tıklayınca Spotify'ın web sayfası.
    fn ext_card(&self, card: &ExtCard, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let radius = if card.round { width / 2. } else { R_LG };
        let uri = card.uri.clone();
        let id = card.uri.clone();
        div()
            .id(ElementId::Name(format!("card-{id}").into()))
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(S2))
            .cursor_pointer()
            .tooltip(Tooltip::text(t!(SearchOpenOnSpotify), None))
            .on_click(cx.listener(move |this, _, _, _| this.open_on_spotify(&uri)))
            .child(
                div()
                    .relative()
                    .size(px(width))
                    .child(cover_image(&self.images, &card.image, &card.title, width, radius, card.icon))
                    // Kapağın üstünde ton katmanı, 140 ms geçişle (`tween`).
                    .child(super::tween::hover_fill(veil(VEIL_HOVER), veil(VEIL_PRESSED), 0., radius)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S0))
                    .child(
                        div()
                            .flex()
                            .type_role(T_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text())
                            .child(line_with_tip(ElementId::Name(format!("ct-{id}").into()), card.title.clone()).flex().min_w_0()),
                    )
                    .child(
                        div()
                            .flex()
                            .type_role(T_SECONDARY)
                            .text_color(muted())
                            .child(line_with_tip(ElementId::Name(format!("cs-{id}").into()), SharedString::from(card.subtitle.clone())).flex().min_w_0()),
                    ),
            )
            .into_any_element()
    }

    /// "Spotify'da aç": adresin web sayfasını varsayılan tarayıcıda açar.
    /// Sınama kopyalarında (`OPENIFY_TEST_DISPLAY`) hiçbir şey açılmaz.
    fn open_on_spotify(&self, uri: &str) {
        let Some(url) = web_url(uri) else { return };
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
            log::debug!("Spotify'da aç (sınamada açılmadı): {url}");
            return;
        }
        let _ = std::process::Command::new("explorer").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, with_lang};

    fn fixture() -> SearchResults {
        crate::engine::search_from_json(include_str!("../engine/fixtures/search_desktop.json")).unwrap()
    }

    #[test]
    fn chips_are_in_spotify_order_and_start_with_all() {
        assert_eq!(SearchKind::default(), SearchKind::All);
        assert_eq!(SearchKind::ALL[0], SearchKind::All);
        assert_eq!(SearchKind::ALL.len(), 7);
        let ids: std::collections::HashSet<_> = SearchKind::ALL.iter().map(|k| k.id()).collect();
        assert_eq!(ids.len(), 7, "her çipin Tab anahtarı ayrı");
    }

    #[test]
    fn all_shows_a_row_of_each_kind_but_not_episodes() {
        for section in [Section::Songs, Section::Artists, Section::Albums, Section::Playlists, Section::Podcasts, Section::Profiles] {
            assert_eq!(SearchKind::All.show(section), Show::Few, "{section:?}");
        }
        assert_eq!(SearchKind::All.show(Section::Episodes), Show::Hidden);
    }

    #[test]
    fn each_chip_opens_only_its_own_kind_in_full() {
        use Section::*;
        let sections = [Songs, Artists, Albums, Playlists, Podcasts, Episodes, Profiles];
        let want = [
            (SearchKind::Songs, vec![Songs]),
            (SearchKind::Artists, vec![Artists]),
            (SearchKind::Albums, vec![Albums]),
            (SearchKind::Playlists, vec![Playlists]),
            (SearchKind::Podcasts, vec![Podcasts, Episodes]),
            (SearchKind::Profiles, vec![Profiles]),
        ];
        for (kind, full) in want {
            for s in sections {
                let expected = if full.contains(&s) { Show::Full } else { Show::Hidden };
                assert_eq!(kind.show(s), expected, "{kind:?} / {s:?}");
            }
        }
    }

    #[test]
    fn counts_follow_the_chip() {
        let r = fixture();
        assert_eq!(SearchKind::Songs.count(&r), 2);
        assert_eq!(SearchKind::Artists.count(&r), 2);
        assert_eq!(SearchKind::Podcasts.count(&r), 4, "podcast'ler + bölümler");
        assert_eq!(SearchKind::Profiles.count(&r), 2);
        // Hepsi: bölümler hariç her tür.
        assert_eq!(SearchKind::All.count(&r), r.total() - r.episodes.len());
        // Boş sonuçta her çip boş.
        let empty = SearchResults::default();
        assert!(SearchKind::ALL.iter().all(|k| k.count(&empty) == 0));
    }

    #[test]
    fn only_multi_section_views_have_titles() {
        assert!(SearchKind::All.titled() && SearchKind::Podcasts.titled());
        assert!(!SearchKind::Songs.titled() && !SearchKind::Profiles.titled());
    }

    #[test]
    fn external_cards_have_the_right_shape_and_labels() {
        let r = fixture();
        with_lang(Lang::En, || {
            let shows = podcast_cards(&r.podcasts);
            assert!(shows.iter().all(|c| !c.round), "podcast kapakları kare");
            assert_eq!(shows[0].subtitle, "Sakin Yayınlar");
            assert_eq!(shows[1].subtitle, "Podcast", "yayıncı yoksa tür adı");
            let people = profile_cards(&r.profiles);
            assert!(people.iter().all(|c| c.round && c.subtitle == "Profile"), "profiller yuvarlak");
            let eps = episode_cards(&r.episodes);
            assert_eq!(eps[0].subtitle, "Yavaş Sabah · 2024");
        });
        with_lang(Lang::Tr, || {
            assert_eq!(profile_cards(&r.profiles)[0].subtitle, "Profil");
        });
    }

    #[test]
    fn web_urls_are_built_from_spotify_uris() {
        assert_eq!(web_url("spotify:show:4rOoJ6Egrf8K2IrywzwOMk").as_deref(), Some("https://open.spotify.com/show/4rOoJ6Egrf8K2IrywzwOMk"));
        assert_eq!(web_url("spotify:episode:abc").as_deref(), Some("https://open.spotify.com/episode/abc"));
        assert_eq!(web_url("spotify:user:selin.k").as_deref(), Some("https://open.spotify.com/user/selin.k"));
        // Kimlikteki özel karakterler yüzde-kodlanır, komut satırına sızmaz.
        assert_eq!(web_url("spotify:user:a b&c").as_deref(), Some("https://open.spotify.com/user/a%20b%26c"));
        assert_eq!(web_url("spotify:user:ş").as_deref(), Some("https://open.spotify.com/user/%C5%9F"));
        // Bilinmeyen tür, uzun adres, boş kimlik, başka şema, yerel dosya.
        assert_eq!(web_url("spotify:genre:rock"), None);
        assert_eq!(web_url("spotify:user:x:playlist:y"), None);
        assert_eq!(web_url("spotify:show:"), None);
        assert_eq!(web_url("https://evil.example/x"), None);
        assert_eq!(web_url("spotify:local:a:b:c:1"), None);
        assert_eq!(web_url(""), None);
    }
}
