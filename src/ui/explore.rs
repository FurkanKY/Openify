//! Gezinme sekmeleri, Keşfet, tür sayfası ve Ana sayfanın Spotify akışı
//! (Adım 12, 7 Kasım; bkz. `DESIGN.md` → "Gezinme" ve `engine::explore`).
//!
//! Veri motordan gelir (`LibraryEvent::Explore`); önbellekteki veri sekmeye
//! girilen karede çizilir, bayatsa arkadan tazelenir. Alt sayfalar (tür
//! sayfaları) yalnız tıklanınca istenir. Boş, yükleniyor ve çevrimdışı
//! durumları sayfa sayfa: bir bölümün verisi gelmediyse yalnız o bölüm
//! atlanır; hiçbir şey gelmediyse bütün sayfada durum görünümü.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui::{
    AnyElement, Context, Div, ElementId, FocusHandle, FontWeight, Window, div, prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{Look, Tooltip},
    history::{Route, Tab},
    library::Loadable,
    manage::CardTarget,
    pages::{Art, CARD_GAP, CardSpec, artist_card},
    text::line_with_tip,
    theme::*,
};
use crate::engine::{
    CardKind, CategoryList, CollectionId, Command, ExploreCard, ExploreData, ExploreKey, Feed,
    LibraryCommand, LibraryItem,
};
use crate::t;

/// Tür karosunun en dar eni ve yüksekliği.
const TILE_MIN: f32 = 184.;
const TILE_HEIGHT: f32 = 96.;
/// Tür karosunun rengi biraz koyulur: beyaz yazı her renkte okunsun.
const TILE_DARKEN: f32 = 0.22;
/// Keşfet'te "benzer sanatçılardan öneriler"de en çok sanatçı.
const SUGGESTIONS: usize = 20;
/// Sekme düğmelerinin eni (simge + ad); simge yalnız dar pencerede.
pub(super) const TABS_LABELS_FROM: f32 = 1160.;

/// Keşfet ve Ana sayfanın Spotify verisi.
#[derive(Default)]
pub struct ExploreUi {
    feeds: HashMap<ExploreKey, Arc<Feed>>,
    categories: Option<Arc<CategoryList>>,
    /// Son denemesi başarısız olan anahtarlar (hata metniyle).
    failed: HashMap<ExploreKey, String>,
    /// Keşfet'in son bölümü: önbellekteki benzer sanatçılardan, kütüphanede
    /// olmayanlar (sekmeye girilince bir kez hesaplanır).
    suggestions: Vec<crate::engine::ArtistCard>,
}

/// Bir kartı sayfa kartına çevirir. Anahtar bölüme özel: aynı liste iki
/// bölümde de geçebilir, öğe kimlikleri çakışmasın.
pub(super) fn card_spec(section: &str, c: &ExploreCard) -> CardSpec {
    let key = format!("{section}:{}", c.uri);
    match c.kind {
        CardKind::Playlist => CardSpec {
            key,
            title: c.name.clone(),
            subtitle: c.subtitle.clone(),
            art: Art::Image {
                url: c.image.clone(),
                icon: "list-music",
            },
            round: false,
            target: CardTarget::Collection {
                id: CollectionId::Playlist(c.uri.clone()),
                title: c.name.clone(),
            },
        },
        CardKind::Album => CardSpec {
            key,
            title: c.name.clone(),
            subtitle: c.subtitle.clone(),
            art: Art::Image {
                url: c.image.clone(),
                icon: "disc-3",
            },
            round: false,
            target: CardTarget::Album {
                uri: c.uri.clone(),
                item: Some(LibraryItem {
                    id: CollectionId::Album(c.uri.clone()),
                    name: c.name.clone(),
                    subtitle: c.subtitle.clone(),
                    image: c.image.clone(),
                }),
            },
        },
    }
}

/// `#rrggbb` → 0xRRGGBB.
fn parse_color(hex: &str) -> Option<u32> {
    u32::from_str_radix(hex.strip_prefix('#')?, 16)
        .ok()
        .filter(|_| hex.len() == 7)
}

/// Tür karosunun zemini (rengi koyulmuş; renk yoksa sessiz ton).
pub(super) fn tile_color(hex: &str) -> u32 {
    match parse_color(hex) {
        Some(c) => mix(c, 0x000000, TILE_DARKEN),
        None => mix(BG, 0xFFFFFF, 0.14),
    }
}

/// Tür sayfasının üstündeki ışımanın rengi: türün rengi, ışıma tavanını
/// (`GLOW_MAX_LUMINANCE`'ın birkaç katı: tür sayfasında renk daha belirgin)
/// aşmayacak kadar zeminle karışık.
pub(super) fn category_glow(hex: &str) -> Option<u32> {
    let color = parse_color(hex)?;
    let mut t = 0.62;
    loop {
        let top = mix(BG, color, t);
        if luminance(top) <= 3. * GLOW_MAX_LUMINANCE || t <= 0.1 {
            return Some(top);
        }
        t -= 0.02;
    }
}

impl Shell {
    // ── Sekmeler ────────────────────────────────────────────────────────

    pub(super) fn go_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let route = match tab {
            Tab::Home => Route::Home,
            Tab::Explore => Route::Explore,
            Tab::Library => Route::Library,
        };
        self.navigate(route, cx);
    }

    /// Başlık çubuğundaki üç sekme ("Ana sayfa · Keşfet · Kütüphane").
    pub(super) fn render_tabs(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let labels = f32::from(window.viewport_size().width) >= TABS_LABELS_FROM;
        let active = self.pages.history.current().tab();
        let mut row = div().flex_none().h_full().flex().items_center().gap(px(S1));
        for (tab, icon_name, name, keys, stop) in [
            (Tab::Home, "house", t!(NavHome), "Ctrl 1", super::tab::HOME),
            (
                Tab::Explore,
                "compass",
                t!(NavExplore),
                "Ctrl 2",
                super::tab::EXPLORE,
            ),
            (
                Tab::Library,
                "library",
                t!(NavLibrary),
                "Ctrl 3",
                super::tab::LIBRARY,
            ),
        ] {
            let (handle, look) = self.stop_look(format!("tab-{name}"), stop, true, window, cx);
            row = row.child(self.tab_button(
                tab,
                icon_name,
                name,
                keys,
                labels,
                active == Some(tab),
                (handle, look),
                cx,
            ));
        }
        row.into_any_element()
    }

    fn tab_button(
        &self,
        tab: Tab,
        icon_name: &'static str,
        name: &'static str,
        keys: &'static str,
        labels: bool,
        on: bool,
        (handle, look): (Option<FocusHandle>, Look),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = if on { text() } else { muted() };
        // Seçili sekme: ton katmanı zemin; üzerine gelince bir basamak açılır.
        // Seçili olmayan: zemin yok, üzerine gelince 140 ms'de ton katmanı.
        let fill = if on {
            super::tween::hover_fill(veil(VEIL_PRESSED), veil(VEIL_PRESSED), VEIL_SELECTED, R_LG)
        } else {
            super::tween::hover_fill(veil(VEIL_HOVER), veil(VEIL_PRESSED), 0., R_LG)
        };
        div()
            .id(ElementId::Name(format!("tab-{name}").into()))
            .h(px(32.))
            .flex_none()
            .px(px(if labels { S3 } else { S2 }))
            .rounded(px(R_LG))
            .flex()
            .items_center()
            .gap(px(S2))
            .cursor_pointer()
            // Klavye odağı: hep duran 2 px'lik kenar, odakta vurgu renginde.
            .border_2()
            .border_color(if look == Look::Focus {
                rgb(self.accent().text).into()
            } else {
                gpui::transparent_black()
            })
            .when_some(handle, |el, h| el.track_focus(&h))
            .type_role(T_SECONDARY)
            .font_weight(if on {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(color)
            .when(on, |el| el.bg(veil(VEIL_SELECTED)))
            .when(!on, |el| el.hover(|s| s.text_color(text())))
            .tooltip(Tooltip::text(name, Some(keys)))
            .on_click(cx.listener(move |this, _, _, cx| this.go_tab(tab, cx)))
            .child(fill)
            .child(icon(icon_name, 16., color))
            .when(labels, |el| el.child(name))
            .into_any_element()
    }

    // ── Veri ────────────────────────────────────────────────────────────

    /// Anahtarların verisini iste: önbellekteki hemen alınır (bu karede
    /// çizilir), taze değilse (ya da hiç yoksa) motordan istenir. Başarısız
    /// önceki denemenin hatası temizlenir (yükleniyor görünür).
    pub(super) fn ask_explore(&mut self, keys: &[ExploreKey]) {
        // Önbellek okuması UI iş parçacığında yapılmaz: motor önbellekteki veriyi
        // hemen olay olarak yollar, taze değilse arkadan da ister.
        for key in keys {
            if !self.pages.explore.has(key) {
                self.pages.explore.failed.remove(key);
            }
            self.engine
                .send(Command::Library(LibraryCommand::OpenExplore(key.clone())));
        }
    }

    fn store_explore(&mut self, key: ExploreKey, data: ExploreData) {
        self.pages.explore.failed.remove(&key);
        match data {
            ExploreData::Feed(feed) => {
                self.pages.explore.feeds.insert(key, feed);
            }
            ExploreData::Categories(list) => self.pages.explore.categories = Some(list),
        }
    }

    pub(super) fn on_explore(&mut self, key: ExploreKey, data: ExploreData) {
        self.store_explore(key, data);
    }

    pub(super) fn on_explore_failed(&mut self, key: ExploreKey, error: String) {
        // Gösterilen (bayat) veri hata yüzünden silinmez; yoksa hata kalır.
        if !self.pages.explore.has(&key) {
            self.pages.explore.failed.insert(key, error);
        }
    }

    /// Sekmeye girilirken: Keşfet'in istediği her şey.
    pub(super) fn enter_explore(&mut self, cx: &mut Context<Self>) {
        self.ask_explore(&[
            ExploreKey::NewReleases,
            ExploreKey::Charts,
            ExploreKey::Categories,
        ]);
        self.refresh_suggestions(cx);
    }

    /// Kütüphanedeki albümlerin sanatçıları: önerilerden çıkar.
    fn known_artists(&self) -> HashSet<String> {
        let Loadable::Loaded(library) = &self.library.library else {
            return HashSet::new();
        };
        library
            .albums
            .iter()
            .flat_map(|a| a.subtitle.split(", "))
            .map(|n| n.trim().to_lowercase())
            .filter(|n| !n.is_empty())
            .collect()
    }

    /// Öneriler önbellekteki benzer sanatçı listelerinden hesaplanır (çok
    /// satır okur, çözümler): arka plan yürütücüsünde, sonuç geldikçe gösterilir.
    fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let known = self.known_artists();
        cx.spawn(async move |this, cx| {
            let list = cx
                .background_executor()
                .spawn(async move { crate::engine::suggested_artists(&known, SUGGESTIONS) })
                .await;
            let _ = this.update(cx, |shell, cx| {
                shell.pages.explore.suggestions = list;
                cx.notify();
            });
        })
        .detach();
    }

    // ── Görünüm ─────────────────────────────────────────────────────────

    /// Bölüm başlığı (kart bölümlerinin başlığıyla aynı, "Tümünü göster"siz).
    fn section_heading(&self, title: &str) -> Div {
        div()
            .px(px(self.margin()))
            .pb(px(S3))
            .h(px(S7))
            .flex()
            .items_end()
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(title.to_string()),
            )
    }

    /// Yükleniyor: bölüm başlığı ve bir sıra iskelet kart (yerleşim zıplamaz).
    pub(super) fn skeleton_section(&self, title: &str) -> AnyElement {
        let bar = |w: f32, h: f32| {
            div()
                .w(px(w))
                .h(px(h))
                .rounded(px(R_XS))
                .bg(veil(VEIL_HOVER))
        };
        let (cols, width) = self.grid();
        let mut cards = div().px(px(self.margin())).flex().gap(px(CARD_GAP));
        for _ in 0..cols {
            cards = cards.child(
                div()
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(div().size(px(width)).rounded(px(R_LG)).bg(veil(VEIL_HOVER)))
                    .child(bar(width * 0.7, T_LABEL.size))
                    .child(bar(width * 0.4, T_LABEL.size)),
            );
        }
        div()
            .pb(px(S6))
            .child(self.section_heading(title))
            .child(cards)
            .into_any_element()
    }

    fn cards_of<'a>(
        &self,
        section: &str,
        cards: impl IntoIterator<Item = &'a ExploreCard>,
    ) -> Vec<CardSpec> {
        cards.into_iter().map(|c| card_spec(section, c)).collect()
    }

    /// Ana sayfa akışı (Spotify'ın sana özel listeleri), geldiyse.
    pub(super) fn home_feed(&self) -> Option<&Arc<Feed>> {
        self.pages.explore.feeds.get(&ExploreKey::Home)
    }

    /// Ana sayfa akışı denendi ve başarısız mı (iskelet yerine hiçbir şey)?
    pub(super) fn home_feed_failed(&self) -> bool {
        self.pages.explore.failed.contains_key(&ExploreKey::Home)
    }

    /// Keşfet sayfası: yeni çıkanlar → trend listeler → tür ve ruh hâli →
    /// benzer sanatçılardan öneriler.
    pub(super) fn explore_body(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let ex = &self.pages.explore;
        let keys = [
            ExploreKey::NewReleases,
            ExploreKey::Charts,
            ExploreKey::Categories,
        ];
        let new = ex.feeds.get(&ExploreKey::NewReleases);
        let charts = ex.feeds.get(&ExploreKey::Charts);
        let cats = ex.categories.as_ref();
        // Hiçbir bölümün verisi yok: bağlantı durumu (giriş, çevrimdışı, Premium,
        // motor, uzun sürdü) bütün sayfada durum olarak; hepsi başarısızsa hata.
        if new.is_none() && charts.is_none() && cats.is_none() {
            let all_failed = keys.iter().all(|k| ex.failed.contains_key(k));
            if let Some(gate) = self.gate_for(None, all_failed) {
                return self.gate_page(&gate, window, cx);
            }
            if all_failed {
                let error = ex.failed.get(&keys[0]).cloned().unwrap_or_default();
                return self.page_error(&error, Route::Explore, window, cx);
            }
        }
        // İskelet yalnız gerçekten yüklenirken (veri gelebilir, yavaş değil) ve
        // sayfa açılır açılmaz değil: önbellekteki veri birkaç karede gelir,
        // iskelet çakıp silinmesin (bkz. `welcome`).
        let skeleton = self.load_may_arrive() && self.page_skeleton_due("explore", window);
        let loading = |k: &ExploreKey| !ex.failed.contains_key(k) && skeleton;
        let meta = div().child(t!(ExploreMeta)).into_any_element();
        let mut body =
            div()
                .flex()
                .flex_col()
                .child(self.page_header(None, t!(NavExplore), meta, Vec::new()));

        match new {
            Some(feed) => {
                let cards = self.cards_of("new", feed.new_releases());
                body = body.children(self.card_section(
                    "explore#new".into(),
                    t!(ExploreNew),
                    cards,
                    cx,
                ));
            }
            None if loading(&ExploreKey::NewReleases) => {
                body = body.child(self.skeleton_section(t!(ExploreNew)))
            }
            None => {}
        }
        match charts {
            Some(feed) => {
                if let Some(section) = feed.featured() {
                    let cards = self.cards_of("chart", &section.cards);
                    body = body.children(self.card_section(
                        "explore#charts".into(),
                        t!(ExploreCharts),
                        cards,
                        cx,
                    ));
                }
            }
            None if loading(&ExploreKey::Charts) => {
                body = body.child(self.skeleton_section(t!(ExploreCharts)))
            }
            None => {}
        }
        match cats {
            Some(list) => {
                body = body.child(
                    div()
                        .pb(px(S6))
                        .child(self.section_heading(t!(ExploreGenres)))
                        .child(self.category_tiles(list, cx)),
                );
            }
            None if loading(&ExploreKey::Categories) => body = body.child(self.tiles_skeleton()),
            None => {}
        }
        let people: Vec<CardSpec> = ex.suggestions.iter().map(artist_card).collect();
        let body = body.children(self.card_section(
            "explore#similar".into(),
            t!(ExploreSimilar),
            people,
            cx,
        ));
        // Gelen veri iskelet ya da durumdan sonraysa belirerek gelir.
        if skeleton {
            self.mark_shown();
        }
        self.settle("explore", body)
    }

    /// Keşfet'te hiçbir bölümün verisi yokken bağlantı durumundan gösterilecek
    /// durum (bkz. `welcome`); yok demek veri geliyor ya da başka bir hata var.
    pub(super) fn explore_gate(&self) -> Option<super::welcome::Gate> {
        let ex = &self.pages.explore;
        let keys = [
            ExploreKey::NewReleases,
            ExploreKey::Charts,
            ExploreKey::Categories,
        ];
        if keys.iter().any(|k| ex.has(k)) {
            return None;
        }
        self.gate_for(None, keys.iter().all(|k| ex.failed.contains_key(k)))
    }

    /// Bağlantı (yeniden) kurulunca açık sayfanın verisini yeniden iste: motor
    /// bağlantı yokken sorulanları çevrimdışı olunca atıyor, sayfa veriyi hiç
    /// almıyordu (Ana sayfa "Senin için yeni" iskeleti, boş Keşfet).
    pub(super) fn reask_explore(&mut self) {
        let want: Vec<ExploreKey> = match self.pages.history.current() {
            Route::Home => vec![ExploreKey::Home],
            Route::Explore => vec![
                ExploreKey::NewReleases,
                ExploreKey::Charts,
                ExploreKey::Categories,
            ],
            Route::Category(uri) => vec![ExploreKey::Category(uri.clone())],
            _ => return,
        };
        let missing: Vec<ExploreKey> = want
            .into_iter()
            .filter(|k| !self.pages.explore.has(k))
            .collect();
        if !missing.is_empty() {
            self.ask_explore(&missing);
        }
    }

    fn tile_grid(&self) -> (usize, f32) {
        let room = (self.window_width - 2. * self.margin()).max(TILE_MIN);
        let cols = (((room + CARD_GAP) / (TILE_MIN + CARD_GAP)).floor() as usize).max(2);
        (cols, (room - CARD_GAP * (cols as f32 - 1.)) / cols as f32)
    }

    fn tiles_skeleton(&self) -> AnyElement {
        let (cols, width) = self.tile_grid();
        let mut grid = div()
            .px(px(self.margin()))
            .flex()
            .flex_col()
            .gap(px(CARD_GAP));
        for _ in 0..2 {
            let mut line = div().flex().gap(px(CARD_GAP));
            for _ in 0..cols {
                line = line.child(
                    div()
                        .w(px(width))
                        .h(px(TILE_HEIGHT))
                        .rounded(px(R_LG))
                        .bg(veil(VEIL_HOVER)),
                );
            }
            grid = grid.child(line);
        }
        div()
            .pb(px(S6))
            .child(self.section_heading(t!(ExploreGenres)))
            .child(grid)
            .into_any_element()
    }

    /// Tür ve ruh hâli karoları: her biri kendi renginde, adı büyük yazılı.
    fn category_tiles(&self, list: &CategoryList, cx: &mut Context<Self>) -> AnyElement {
        let (cols, width) = self.tile_grid();
        let mut grid = div()
            .px(px(self.margin()))
            .flex()
            .flex_col()
            .gap(px(CARD_GAP));
        for row in list.items.chunks(cols) {
            let mut line = div().flex().gap(px(CARD_GAP));
            for cat in row {
                let uri = cat.uri.clone();
                let color = tile_color(&cat.color);
                line = line.child(
                    div()
                        .id(ElementId::Name(format!("cat-{}", cat.uri).into()))
                        .relative()
                        .w(px(width))
                        .h(px(TILE_HEIGHT))
                        .flex_none()
                        .overflow_hidden()
                        .rounded(px(R_LG))
                        .bg(rgb(color))
                        .cursor_pointer()
                        .tooltip(Tooltip::text(cat.name.clone(), None))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.navigate(Route::Category(uri.clone()), cx)
                        }))
                        .child(super::tween::hover_fill(
                            veil(VEIL_SELECTED),
                            veil(VEIL_PRESSED),
                            0.,
                            R_LG,
                        ))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .p(px(S4))
                                .flex()
                                .font_family(font())
                                .type_role(T_TITLE)
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .text_color(text())
                                .child(
                                    line_with_tip(
                                        ElementId::Name(format!("catn-{}", cat.uri).into()),
                                        cat.name.clone(),
                                    )
                                    .flex()
                                    .min_w_0(),
                                ),
                        ),
                );
            }
            grid = grid.child(line);
        }
        grid.into_any_element()
    }

    /// Bir türün sayfası: başlıkta türün rengi (pencere ışıması, bkz.
    /// `Shell::render_glow`), altında türün listeleri bölüm bölüm.
    pub(super) fn category_body(&self, uri: &str, window: &Window, cx: &mut Context<Self>) -> Div {
        let ex = &self.pages.explore;
        let key = ExploreKey::Category(uri.to_string());
        let Some(feed) = ex.feeds.get(&key) else {
            return match ex.failed.get(&key) {
                Some(error) => self.page_error(error, Route::Category(uri.to_string()), window, cx),
                None => self.page_skeleton_plain(true),
            };
        };
        let title = if feed.title.is_empty() {
            self.category_name(uri)
        } else {
            feed.title.clone()
        };
        let meta = div().child(t!(ExploreGenres)).into_any_element();
        let mut body =
            div()
                .flex()
                .flex_col()
                .child(self.page_header(None, &title, meta, Vec::new()));
        for (i, section) in feed.sections.iter().enumerate() {
            let cards = self.cards_of(&format!("cat{i}"), &section.cards);
            body = body.children(self.card_section(
                format!("cat:{uri}#{i}"),
                &section.title,
                cards,
                cx,
            ));
        }
        body
    }

    /// Karolardaki adı (sayfa henüz gelmediyse başlık için).
    fn category_name(&self, uri: &str) -> String {
        self.pages
            .explore
            .categories
            .as_ref()
            .and_then(|l| l.items.iter().find(|c| c.uri == uri))
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    /// Tür sayfası açıkken pencere ışımasının rengi (yoksa kapak ışıması).
    pub(super) fn category_glow_color(&self) -> Option<u32> {
        let Route::Category(uri) = self.pages.history.current() else {
            return None;
        };
        let feed = self
            .pages
            .explore
            .feeds
            .get(&ExploreKey::Category(uri.clone()))?;
        let hex = if feed.color.is_empty() {
            self.pages
                .explore
                .categories
                .as_ref()?
                .items
                .iter()
                .find(|c| &c.uri == uri)?
                .color
                .clone()
        } else {
            feed.color.clone()
        };
        category_glow(&hex)
    }
}

impl ExploreUi {
    /// Tür ve ruh hâli karoları (önbellekten ya da Keşfet'e girilince gelen);
    /// henüz yoksa `None` (arama yüzeyi tür çiplerini göstermez).
    pub fn categories(&self) -> Option<&Arc<CategoryList>> {
        self.categories.as_ref()
    }

    /// Anahtarın verisi elde mi?
    pub fn has(&self, key: &ExploreKey) -> bool {
        match key {
            ExploreKey::Categories => self.categories.is_some(),
            _ => self.feeds.contains_key(key),
        }
    }

    /// Bir sayfanın verisi elde mi (iskelet değil içerik çizilecek mi)?
    pub fn ready(&self, key: &ExploreKey) -> bool {
        self.has(key) || self.failed.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spotify'ın tür karolarında gördüğümüz bütün renkler (7 Kasım 2026,
    /// `browseAll`): beyaz yazı karoda ≥ 4,5:1 olmalı.
    const TILE_COLORS: &[&str] = &[
        "#dc148c", "#006450", "#283ea3", "#777777", "#1e3264", "#e61e32", "#8c1932", "#b02897",
        "#8d67ab", "#477d95", "#503750", "#af2896", "#e1118c", "#2d46b9", "#e91429", "#d84000",
        "#ba5d07", "#b06239", "#5179a1", "#148a08", "#a56752", "#0d73ec", "#7d4b32", "#608108",
        "#e13300", "#e8115b", "#27856a", "#537aa1", "#0d72ed",
    ];

    fn contrast(a: u32, b: u32) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn tile_text_is_readable_on_every_category_color() {
        for hex in TILE_COLORS {
            let bg = tile_color(hex);
            let c = contrast(TEXT, bg);
            assert!(c >= 4.5, "{hex}: {c:.2}:1");
        }
        // Rengi olmayan (ya da bozuk) karo sessiz tona düşer.
        assert_eq!(tile_color(""), tile_color("pembe"));
        assert!(contrast(TEXT, tile_color("")) >= 4.5);
    }

    #[test]
    fn category_glow_stays_dark_enough_for_text() {
        for hex in TILE_COLORS {
            let glow = category_glow(hex).unwrap();
            assert!(luminance(glow) <= 3. * GLOW_MAX_LUMINANCE + 1e-3, "{hex}");
            // Başlığın ve listelerin yazısı ışımanın üstünde okunur.
            assert!(
                contrast(TEXT, glow) >= 7.,
                "{hex}: {:.2}",
                contrast(TEXT, glow)
            );
        }
        assert!(category_glow("").is_none());
    }

    #[test]
    fn colors_parse_strictly() {
        assert_eq!(parse_color("#2d46b9"), Some(0x2d46b9));
        assert_eq!(parse_color("2d46b9"), None);
        assert_eq!(parse_color("#2d46"), None);
    }
}
