//! Ana sayfa: "Şimdi" odaklı (7 Kasım gecesi, kullanıcı geri bildirimi: "klasik
//! Spotify'a fazla benziyor, selamlama tepeye yapışık").
//!
//! Yukarıdan aşağı: bol üst boşlukla selamlama; **kahraman satırı**: solda
//! büyük "Kaldığın yerden" kartı (kapak, şarkı, sanatçı, Devam et / Duraklat ve
//! ilerleme), yanında günün saatine göre Spotify karışımlarından tek öneri
//! (üstte küçük etiket yok: eyebrow yasağı); altında az sayıda, birer açıklama satırlı bölüm: Çok
//! dinlediklerin, Senin için yeni (günlük karışımlar tek yığın kart), Listelerin.
//! Kahraman kartların zemini kapağın renginden hafifçe boyanır (koyu tavanla,
//! yazı her renkte okunur). Yalnız elimizdeki veri: Spotify'ın ana sayfa akışı
//! (önbellekten), kütüphane ve yerel çalma geçmişi.
//!
//! Hızlı erişim ızgarası kalktı: Beğenilenler ve son çalınanlar zaten kahraman
//! kartta ve "Çok dinlediklerin"de; ızgara aynı şeyi ikinci kez gösteriyordu.
//! "Albümlerin" de kalktı (Kütüphane sekmesinde, rafta ve ızgarada).

use std::time::Instant;

use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, Window, div, img, linear_color_stop,
    linear_gradient, prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{Look, Tooltip, primary_button, secondary_button},
    cover_image,
    explore::card_spec,
    framed, fuzzy,
    library::Loadable,
    pages::{Art, CardSpec, collection_card, local_hour},
    player::{Status, fmt_time},
    text::line_with_tip,
    theme::*,
};
use crate::engine::{CardKind, CollectionId, ExploreCard, Library, LibraryItem};
use crate::{t, tp};

/// Kahraman satırının yüksekliği: solda kapak 184 px (24 px iç boşluk).
const HERO_HEIGHT: f32 = 232.;
const HERO_COVER: f32 = HERO_HEIGHT - 2. * S5;
/// Önerinin kapağı ve (dar pencerede alt alta dizilince) yüksekliği.
const PICK_COVER: f32 = 136.;
const PICK_STACKED_HEIGHT: f32 = PICK_COVER + 2. * S5;
/// İçerik bu enden darsa kahramanlar alt alta dizilir.
const HERO_STACK_BELOW: f32 = 900.;
/// Öneri kartı, iki kart yan yanayken satırın bu kadarı.
const PICK_SHARE: f32 = 0.4;
/// Kahraman kartların zemini: kapak renginin zeminle karışımı (en güçlü
/// baştaki, bu parlaklık tavanına kadar kısılır) ve tavan. 0,04'te beyaz yazı
/// ≥ 11:1, ikincil (%64) ≥ 6:1 (test).
const HERO_TINT: f32 = 0.5;
const HERO_MAX_LUMINANCE: f32 = 0.04;
/// Ana sayfa bölümleri arası.
const SECTION_GAP: f32 = S7;

// ── Günün dilimi ve öneri ───────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DayPart {
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl DayPart {
    pub(super) fn of(hour: u16) -> Self {
        match hour {
            5..=11 => Self::Morning,
            12..=17 => Self::Afternoon,
            18..=22 => Self::Evening,
            _ => Self::Night,
        }
    }

    pub(super) fn greeting(self) -> &'static str {
        match self {
            Self::Morning => t!(HomeGreetMorning),
            Self::Afternoon => t!(HomeGreetAfternoon),
            Self::Evening => t!(HomeGreetEvening),
            Self::Night => t!(HomeGreetNight),
        }
    }

    fn tagline(self) -> &'static str {
        match self {
            Self::Morning => t!(HomeTagMorning),
            Self::Afternoon => t!(HomeTagAfternoon),
            Self::Evening => t!(HomeTagEvening),
            Self::Night => t!(HomeTagNight),
        }
    }

    /// Karışım adında / açıklamasında aranan sözcükler (Türkçe harfler
    /// sadeleşmiş, küçük harf): sabah enerjik, akşam sakin.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Morning => &[
                "morning",
                "sabah",
                "workout",
                "spor",
                "energy",
                "enerji",
                "upbeat",
                "happy",
                "mutlu",
                "dance",
                "feel good",
                "motivation",
            ],
            Self::Afternoon => &[
                "focus", "odak", "work", "calisma", "indie", "pop", "rock", "summer", "yaz", "hits",
            ],
            Self::Evening => &[
                "chill", "moody", "evening", "aksam", "jazz", "soul", "acoustic", "akustik",
                "lofi", "relax", "sakin", "rahat", "slow", "sunset",
            ],
            Self::Night => &[
                "sleep", "uyku", "night", "gece", "ambient", "piano", "calm", "lofi", "dream",
                "sakin", "deep",
            ],
        }
    }
}

/// Zaman önerisine aday olan kart mı: ruh hâli karışımı (haftalık keşifler,
/// radar ve tekrar listeleri zamanla ilgili değil).
fn pickable(card: &ExploreCard) -> bool {
    card.kind == CardKind::Playlist
        && !matches!(
            card.format.as_str(),
            "discover-weekly" | "release-radar" | "on-repeat" | "repeat-rewind"
        )
}

/// Günün saatine göre karışımlardan biri (kartın sırası), her saat için
/// değişmez:
/// 1. Spotify'ın kendi günün saatine göre listesi ("daylist") varsa o;
/// 2. adında / açıklamasında günün diliminin sözcüğü en çok geçen karışım
///    (akşam sakin, sabah enerjik); eşitlikte listede önce gelen;
/// 3. yoksa günlük karışımlardan biri (4 saatlik dilimle, `saat / 4`);
/// 4. o da yoksa ilk aday.
pub(super) fn time_pick(cards: &[&ExploreCard], hour: u16) -> Option<usize> {
    if let Some(i) = cards.iter().position(|c| c.format == "daylist") {
        return Some(i);
    }
    let keywords = DayPart::of(hour).keywords();
    let mut best: Option<(usize, usize)> = None;
    for (i, card) in cards.iter().enumerate().filter(|(_, c)| pickable(c)) {
        let hay = fuzzy::fold(&format!("{} {}", card.name, card.subtitle));
        let score = keywords.iter().filter(|k| hay.contains(*k)).count();
        if score > 0 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    if let Some((_, i)) = best {
        return Some(i);
    }
    let daily: Vec<usize> = cards
        .iter()
        .enumerate()
        .filter(|(_, c)| c.format == "daily-mix")
        .map(|(i, _)| i)
        .collect();
    if !daily.is_empty() {
        return Some(daily[(hour as usize / 4) % daily.len()]);
    }
    cards.iter().position(|c| pickable(c))
}

/// Kahraman kartın zemini: kapak rengi (`glow`) kart yüzeyiyle karışır, en
/// parlak yeri `HERO_MAX_LUMINANCE`'ı geçmez; (baştaki, sondaki) renk.
pub(super) fn hero_tint(glow: u32) -> (u32, u32) {
    let mut t = HERO_TINT;
    loop {
        let top = mix(BAR, glow, t);
        if luminance(top) <= HERO_MAX_LUMINANCE || t <= 0. {
            return (top, mix(BAR, glow, t * 0.3));
        }
        t -= 0.01;
    }
}

/// Kahraman kartın dolgusu: tonsuzsa kart yüzeyi, tonluysa üstten alta
/// koyulaşan kapak rengi.
fn hero_fill(tint: Option<u32>) -> gpui::Background {
    match tint {
        Some(glow) => {
            let (top, bottom) = hero_tint(glow);
            linear_gradient(
                160.,
                linear_color_stop(rgb(top), 0.),
                linear_color_stop(rgb(bottom), 1.),
            )
        }
        None => rgb(BAR).into(),
    }
}

/// Öneri: kart, kapağın adresi (renk için), ad, açıklama.
struct Pick {
    spec: CardSpec,
    name: String,
    subtitle: String,
    image: String,
}

impl Shell {
    /// Bu kare çalarken ana sayfadaki ilerleme çubuğu kendiliğinden ilerlesin
    /// diye listeyi saniyede bir yeniden çizer mi (bkz. `views::ListView`)?
    pub(super) fn home_hero_ticks(&self) -> bool {
        self.pages.history.current() == &super::history::Route::Home
            && self.state.is_playing()
            && self.state.track.is_some()
            && !self.all.open
    }

    /// Spotify'ın ana sayfa akışındaki bütün kartlar (bölümlere bakılmaz).
    fn feed_cards(&self) -> Vec<&ExploreCard> {
        self.home_feed()
            .map(|f| f.sections.iter().flat_map(|s| s.cards.iter()).collect())
            .unwrap_or_default()
    }

    /// Günün saatine göre öneri: akıştan; akış yoksa (çevrimdışı) kütüphanedeki
    /// Spotify listelerinden.
    fn home_pick(&self, made: &[LibraryItem], hour: u16) -> Option<Pick> {
        let feed = self.feed_cards();
        if let Some(i) = time_pick(&feed, hour) {
            let c = feed[i];
            return Some(Pick {
                spec: card_spec("home-pick", c),
                name: c.name.clone(),
                subtitle: c.subtitle.clone(),
                image: c.image.clone(),
            });
        }
        let cards: Vec<ExploreCard> = made
            .iter()
            .map(|p| ExploreCard {
                uri: p.id.key().to_string(),
                name: p.name.clone(),
                subtitle: p.subtitle.clone(),
                image: p.image.clone(),
                kind: CardKind::Playlist,
                format: String::new(),
            })
            .collect();
        let refs: Vec<&ExploreCard> = cards.iter().collect();
        let i = time_pick(&refs, hour)?;
        let item = &made[i];
        Some(Pick {
            spec: collection_card(item),
            name: item.name.clone(),
            subtitle: crate::i18n::canon::localize(&item.subtitle).into_owned(),
            image: item.image.clone(),
        })
    }

    fn spec_art(&self, spec: &CardSpec, side: f32, radius: f32) -> AnyElement {
        match &spec.art {
            Art::Image { url, icon } => {
                cover_image(&self.images, url, &spec.title, side, radius, icon)
            }
            Art::Collection { id, image } => {
                self.collection_art(id, &spec.title, image, side, radius)
            }
            Art::Stack(urls) => cover_image(
                &self.images,
                urls.first().map(String::as_str).unwrap_or_default(),
                &spec.title,
                side,
                radius,
                "list-music",
            ),
        }
    }

    /// Kapağın rengi: mozaikse ilk kapağınki.
    fn cover_tint(&self, image: &str) -> Option<u32> {
        match crate::engine::mosaic_urls(image) {
            Some(urls) => self.images.tint(urls[0]),
            None => self.images.tint(image),
        }
    }

    // ── Sayfa ───────────────────────────────────────────────────────────

    pub(super) fn home_body(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let Loadable::Loaded(library) = &self.library.library else {
            // Kütüphane yok: bağlantı durumu varsa tam sayfa durum (karşılama,
            // çevrimdışı …), yoksa gerçekten yükleniyor (boş sayfa, sonra
            // iskelet; bkz. `welcome`).
            return self.pending_page(
                self.library_gate(),
                || self.page_skeleton(false, false),
                window,
                cx,
            );
        };
        let hour = local_hour();
        let part = DayPart::of(hour);
        let margin = self.margin();
        let compact = self.compact();
        let home = crate::engine::home(library, &self.recent, 24);

        // Selamlama: tepeden bol boşluk (S7), altında tek sakin satır.
        let greeting = div()
            .px(px(margin))
            .pt(px(S7))
            .pb(px(S6))
            .flex()
            .flex_col()
            .gap(px(S2))
            .child(
                div()
                    .flex()
                    .font_family(font())
                    .type_role(if compact { T_HEADLINE } else { T_DISPLAY })
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(
                        line_with_tip("home-greeting", part.greeting().to_string())
                            .flex()
                            .min_w_0(),
                    ),
            )
            .child(
                div()
                    .type_role(T_BODY)
                    .text_color(muted())
                    .child(part.tagline()),
            );

        let pick = self.home_pick(&home.made_for_you, hour);
        let mut body = div()
            .flex()
            .flex_col()
            .child(greeting)
            .child(self.home_hero(library, &home.recent, pick.as_ref(), cx));

        // Çok dinlediklerin: Spotify'ın tekrar listeleri (varsa) ve son
        // çalınanlar.
        let feed = self.feed_cards();
        let (cols, _) = self.grid();
        let mut often: Vec<CardSpec> = feed
            .iter()
            .filter(|c| matches!(c.format.as_str(), "on-repeat" | "repeat-rewind"))
            .map(|c| card_spec("home-often", c))
            .collect();
        let mut often_ids: Vec<CollectionId> = Vec::new();
        // Hiçbir şey yüklü değilken kahraman kart son çalınan listeyi gösterir:
        // aynı liste altta tekrarlanmaz.
        let in_hero = if self.state.track.is_none() {
            home.recent.first().map(|i| i.id.clone())
        } else {
            None
        };
        for item in &home.recent {
            if often.len() >= cols {
                break;
            }
            if in_hero.as_ref() == Some(&item.id)
                || often.iter().any(|c| c.key.ends_with(item.id.key()))
            {
                continue;
            }
            often_ids.push(item.id.clone());
            often.push(collection_card(item));
        }
        body = body.children(self.home_row(
            "home#often",
            t!(HomeOftenTitle),
            t!(HomeOftenDesc),
            often.clone(),
            often,
            cx,
        ));

        // Senin için yeni: haftalık keşifler, günlük karışımlar tek yığın.
        let pick_uri = pick.as_ref().map(|p| p.spec.key.clone());
        body = body.children(self.home_new(&home.made_for_you, pick_uri.as_deref(), cx));

        // Listelerin: kendi listelerin (Spotify'ın hazırladıkları ve yukarıda
        // gösterilenler hariç).
        let mine: Vec<CardSpec> = library
            .playlists
            .iter()
            .filter(|p| {
                !home.made_for_you.iter().any(|m| m.id == p.id)
                    && !often_ids.contains(&p.id)
                    && in_hero.as_ref() != Some(&p.id)
            })
            .map(collection_card)
            .collect();
        let body = body.children(self.home_row(
            "home#lists",
            t!(HomeListsTitle),
            t!(HomeListsDesc),
            mine.clone(),
            mine,
            cx,
        ));
        // İskelet ya da durumdan sonra gelen içerik belirerek gelir.
        self.settle("home", body)
    }

    /// "Senin için yeni": akış gelmediyse kütüphanedeki Spotify listeleri,
    /// gelene dek iskelet, akış denenip başarısızsa (çevrimdışı) yalnız
    /// kütüphane. Akışta: kısa hâli haftalık keşifler, Yeni Müzik Radarı,
    /// günün zaman önerisi olmayan "daylist" ve günlük karışımlar TEK yığın
    /// kart; "Tümünü göster" hepsini tek tek açar (günlük karışımlar, tür
    /// karışımları, radyolar dahil).
    fn home_new(
        &self,
        made_for_you: &[LibraryItem],
        pick: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(feed) = self.home_feed() else {
            if !self.home_feed_failed() && self.load_may_arrive() {
                return vec![self.skeleton_section(t!(HomeNewTitle))];
            }
            let made: Vec<CardSpec> = made_for_you.iter().map(collection_card).collect();
            return self
                .home_row(
                    "home#new",
                    t!(HomeNewTitle),
                    t!(HomeNewDescMade),
                    made.clone(),
                    made,
                    cx,
                )
                .into_iter()
                .collect();
        };
        let all: Vec<&ExploreCard> = feed.sections.iter().flat_map(|s| s.cards.iter()).collect();
        let is_pick = |c: &&&ExploreCard| pick.is_some_and(|p| p == format!("home-pick:{}", c.uri));
        let by = |format: &str| {
            all.iter()
                .filter(|c| c.format == format && !is_pick(c))
                .copied()
                .collect::<Vec<_>>()
        };
        let spec = |c: &ExploreCard| card_spec("home-new", c);

        let mut short: Vec<CardSpec> = Vec::new();
        for format in ["discover-weekly", "release-radar", "daylist"] {
            short.extend(by(format).into_iter().map(spec));
        }
        let daily = by("daily-mix");
        match daily.as_slice() {
            [] => {}
            [one] => short.push(spec(one)),
            [first, ..] => {
                let mut stack = spec(first);
                stack.key = "home-new:daily-stack".into();
                stack.title = t!(HomeDailyMixes).into();
                stack.subtitle = tp!(HomeMixCount, daily.len());
                stack.art = Art::Stack(daily.iter().take(3).map(|c| c.image.clone()).collect());
                short.push(stack);
            }
        }
        // Hepsi: yığın yerine tek tek, ardından tür karışımları ve radyolar.
        let mut full: Vec<CardSpec> = Vec::new();
        for format in ["discover-weekly", "release-radar", "daylist", "daily-mix"] {
            full.extend(by(format).into_iter().map(spec));
        }
        let rest: Vec<&ExploreCard> = all
            .iter()
            .filter(|c| {
                !matches!(
                    c.format.as_str(),
                    "discover-weekly"
                        | "release-radar"
                        | "daylist"
                        | "daily-mix"
                        | "on-repeat"
                        | "repeat-rewind"
                ) && !is_pick(c)
            })
            .copied()
            .collect();
        full.extend(rest.into_iter().map(spec));
        self.home_row(
            "home#new",
            t!(HomeNewTitle),
            t!(HomeNewDescWeekly),
            short,
            full,
            cx,
        )
        .into_iter()
        .collect()
    }

    /// Bölüm: başlık, altında açıklama satırı ve bir sıra kart; `full` kartın
    /// hepsi ("Tümünü göster"). Boşsa hiç yok.
    fn home_row(
        &self,
        key: &str,
        title: &str,
        hint: &str,
        short: Vec<CardSpec>,
        full: Vec<CardSpec>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if short.is_empty() {
            return None;
        }
        let (cols, width) = self.grid();
        let expanded = self.pages.expanded.contains(key);
        let toggle = full.len() > short.len().min(cols);
        let shown: &[CardSpec] = if expanded {
            &full
        } else {
            &short[..short.len().min(cols)]
        };
        let mut grid = div().px(px(self.margin())).flex().flex_col().gap(px(S5));
        for row in shown.chunks(cols) {
            let mut line = div().flex().gap(px(super::pages::CARD_GAP));
            for card in row {
                line = line.child(self.page_card(card, width, cx));
            }
            grid = grid.child(line);
        }
        Some(
            div()
                .pb(px(SECTION_GAP))
                .child(self.section_title_hint(key, title, Some(hint), toggle, cx))
                .child(grid)
                .into_any_element(),
        )
    }

    // ── Kahramanlar ─────────────────────────────────────────────────────

    fn home_hero(
        &self,
        library: &Library,
        recent: &[LibraryItem],
        pick: Option<&Pick>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let room = self.window_width - 2. * self.margin();
        let stacked = room < HERO_STACK_BELOW || pick.is_none();
        let pick_width = ((room - S5) * PICK_SHARE).floor();
        let resume = self.home_resume(
            library,
            recent,
            if stacked {
                room
            } else {
                room - S5 - pick_width
            },
            cx,
        );
        let mut row = div()
            .px(px(self.margin()))
            .pb(px(SECTION_GAP))
            .flex()
            .gap(px(S5));
        if room < HERO_STACK_BELOW {
            row = row.flex_col().gap(px(S4));
        }
        row = row.child(resume);
        if let Some(pick) = pick {
            row = row.child(self.home_pick_card(
                pick,
                if room < HERO_STACK_BELOW {
                    room
                } else {
                    pick_width
                },
                room < HERO_STACK_BELOW,
                cx,
            ));
        }
        row.into_any_element()
    }

    /// Kaldığın yerden: çalan (ya da kaldığı yerden yüklenen) şarkı; yoksa son
    /// çalınan liste (Çal) ya da Beğenilenler.
    fn home_resume(
        &self,
        library: &Library,
        recent: &[LibraryItem],
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Instant::now();
        let accent = self.accent().text;
        let card = div()
            .w(px(width))
            .h(px(HERO_HEIGHT))
            .flex_none()
            .p(px(S5))
            .rounded(px(R_XL))
            .flex()
            .gap(px(S5));
        let texts = |name: String, second: String, tip_id: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(px(S1))
                .child(
                    div()
                        .flex()
                        .font_family(font())
                        .type_role(T_HEADLINE)
                        .font_weight(FontWeight::BOLD)
                        .text_color(text())
                        .child(
                            line_with_tip(ElementId::Name(format!("{tip_id}-name").into()), name)
                                .flex()
                                .min_w_0(),
                        ),
                )
                .child(
                    div().flex().type_role(T_TITLE).text_color(soft()).child(
                        line_with_tip(ElementId::Name(format!("{tip_id}-sub").into()), second)
                            .flex()
                            .min_w_0(),
                    ),
                )
        };

        if let Some(t) = &self.state.track {
            let cover = match self.state.cover.clone() {
                Some(image) => framed(img(image).size_full().rounded(px(R_LG)), HERO_COVER, R_LG),
                None => cover_image(
                    &self.images,
                    t.cover_url.as_deref().unwrap_or_default(),
                    &t.name,
                    HERO_COVER,
                    R_LG,
                    "music",
                ),
            };
            let (icon_name, label_text) = match self.state.status {
                Status::Playing => ("pause-fill", t!(Pause)),
                Status::Ended => ("play-fill", t!(PlayAgain)),
                _ => ("play-fill", t!(Resume)),
            };
            let progress = self.state.progress(now).clamp(0., 1.);
            let bar = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(S1))
                .child(
                    div()
                        .h(px(S1))
                        .w_full()
                        .rounded_full()
                        .bg(veil(VEIL_SELECTED))
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(progress))
                                .rounded_full()
                                .bg(rgb(accent)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .type_role(T_LABEL)
                        .tabular()
                        .text_color(muted())
                        .child(fmt_time(self.state.position_ms(now)))
                        .child(fmt_time(self.state.duration_ms())),
                );
            let controls = div()
                .flex()
                .items_center()
                .gap(px(S5))
                .child(
                    primary_button(
                        "home-resume-play",
                        icon_name,
                        label_text,
                        Look::Live,
                        accent,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                )
                .child(bar);
            let cover = div()
                .id("home-resume-cover")
                .flex_none()
                .cursor_pointer()
                .tooltip(Tooltip::text(t!(FullScreen), Some("F11")))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx)))
                .child(cover);
            return card
                .bg(hero_fill(
                    Some(self.state.accent_at(now).glow).filter(|g| *g != BG),
                ))
                .child(cover)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .justify_between()
                        .child(texts(t.name.clone(), t.artists.clone(), "home-resume"))
                        .child(controls),
                )
                .into_any_element();
        }

        // Hiçbir şey yüklü değil: son çalınan liste (yoksa Beğenilenler).
        let item = recent.first().cloned().unwrap_or_else(|| LibraryItem {
            id: CollectionId::Liked,
            name: t!(LikedSongs).into(),
            subtitle: tp!(Songs, library.liked_count),
            image: String::new(),
        });
        let spec = collection_card(&item);
        let target = spec.target.clone();
        let tint = self.cover_tint(&item.image);
        card.bg(hero_fill(tint))
            .child(
                div()
                    .flex_none()
                    .child(self.spec_art(&spec, HERO_COVER, R_LG)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(texts(
                        spec.title.clone(),
                        spec.subtitle.clone(),
                        "home-resume",
                    ))
                    .child(
                        div().flex().child(
                            primary_button(
                                "home-resume-play",
                                "play-fill",
                                t!(Play),
                                Look::Live,
                                accent,
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.play_card(target.clone(), cx)
                                }),
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// Günün saatine göre öneri kartı: kapak, ad, açıklama ve
    /// Çal; kartın kendisi listeyi açar.
    fn home_pick_card(
        &self,
        pick: &Pick,
        width: f32,
        stacked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let open = pick.spec.target.clone();
        let play = pick.spec.target.clone();
        let menu = pick.spec.target.clone();
        div()
            .id("home-pick")
            .w(px(width))
            .h(px(if stacked {
                PICK_STACKED_HEIGHT
            } else {
                HERO_HEIGHT
            }))
            .flex_none()
            .p(px(S5))
            .rounded(px(R_XL))
            .flex()
            .items_center()
            .gap(px(S5))
            .cursor_pointer()
            .bg(hero_fill(self.cover_tint(&pick.image)))
            .on_click(cx.listener(move |this, _, _, cx| this.open_card(open.clone(), cx)))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_card_menu(menu.clone(), ev.position, cx);
                }),
            )
            .child(super::tween::hover_fill(
                veil(VEIL_HOVER),
                veil(VEIL_PRESSED),
                0.,
                R_XL,
            ))
            .child(
                div()
                    .flex_none()
                    .child(self.spec_art(&pick.spec, PICK_COVER, R_LG)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S1))
                    .child(
                        div()
                            .flex()
                            .font_family(font())
                            .type_role(T_TITLE)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(text())
                            .child(
                                line_with_tip("home-pick-name", pick.name.clone())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .type_role(T_SECONDARY)
                            .text_color(muted())
                            .child(
                                line_with_tip("home-pick-sub", pick.subtitle.clone())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(
                        div().pt(px(S3)).flex().child(
                            secondary_button(
                                "home-pick-play",
                                "play-fill",
                                t!(Play),
                                Look::Live,
                                accent,
                            )
                            .block_mouse_except_scroll()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.play_card(play.clone(), cx)),
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str, subtitle: &str, format: &str) -> ExploreCard {
        ExploreCard {
            uri: format!("spotify:playlist:{name}"),
            name: name.into(),
            subtitle: subtitle.into(),
            image: String::new(),
            kind: CardKind::Playlist,
            format: format.into(),
        }
    }

    fn pick(cards: &[ExploreCard], hour: u16) -> Option<String> {
        let refs: Vec<&ExploreCard> = cards.iter().collect();
        time_pick(&refs, hour).map(|i| cards[i].name.clone())
    }

    #[test]
    fn day_parts_match_the_greeting_hours() {
        assert_eq!(DayPart::of(4), DayPart::Night);
        assert_eq!(DayPart::of(5), DayPart::Morning);
        assert_eq!(DayPart::of(11), DayPart::Morning);
        assert_eq!(DayPart::of(12), DayPart::Afternoon);
        assert_eq!(DayPart::of(17), DayPart::Afternoon);
        assert_eq!(DayPart::of(18), DayPart::Evening);
        assert_eq!(DayPart::of(22), DayPart::Evening);
        assert_eq!(DayPart::of(23), DayPart::Night);
        assert_eq!(DayPart::of(0), DayPart::Night);
    }

    #[test]
    fn spotifys_own_daylist_wins_at_any_hour() {
        let cards = [
            card("Chill Mix", "", "genre-mix"),
            card("sunny afternoon indie", "", "daylist"),
        ];
        for hour in [8, 14, 20, 2] {
            assert_eq!(pick(&cards, hour).as_deref(), Some("sunny afternoon indie"));
        }
    }

    #[test]
    fn evening_takes_calm_and_morning_takes_upbeat_by_name() {
        let cards = [
            card("Workout Hits", "Güçlü ve enerjik", "genre-mix"),
            card("Chill Akşam", "Sakin parçalar", "genre-mix"),
            card("Daily Mix 1", "Tarkan ve daha fazlası", "daily-mix"),
        ];
        assert_eq!(pick(&cards, 20).as_deref(), Some("Chill Akşam"));
        assert_eq!(pick(&cards, 7).as_deref(), Some("Workout Hits"));
    }

    #[test]
    fn without_a_keyword_a_daily_mix_is_picked_by_the_hour() {
        let cards = [
            card("Discover Weekly", "", "discover-weekly"),
            card("Daily Mix 1", "a", "daily-mix"),
            card("Daily Mix 2", "b", "daily-mix"),
            card("Daily Mix 3", "c", "daily-mix"),
        ];
        // Hep aynı saatte aynı cevap; haftalık keşifler aday değil.
        for hour in 0..24 {
            let a = pick(&cards, hour);
            assert_eq!(a, pick(&cards, hour));
            assert_ne!(a.as_deref(), Some("Discover Weekly"));
        }
        assert_eq!(pick(&cards, 18).as_deref(), Some("Daily Mix 2"));
        assert_eq!(pick(&cards, 21).as_deref(), Some("Daily Mix 3"));
        assert_eq!(pick(&cards, 8).as_deref(), Some("Daily Mix 3"));
    }

    #[test]
    fn nothing_pickable_gives_none() {
        let cards = [
            card("Discover Weekly", "", "discover-weekly"),
            card("On Repeat", "", "on-repeat"),
        ];
        assert_eq!(pick(&cards, 12), None);
        assert_eq!(pick(&[], 12), None);
    }

    /// Kahraman kartın yazısı her kapak renginde okunur: tonun en parlak
    /// yeri tavanı geçmez, beyaz yazı ≥ 10:1, ikincil (%64) ≥ 4,5:1.
    #[test]
    fn hero_tint_keeps_text_readable_on_every_hue() {
        let blend = |alpha: f32, over: u32| mix(over, TEXT, alpha);
        for h in 0..36 {
            let hue = h as f32 * 10.;
            // Doygun, parlak bir kapak rengi (en kötü durum).
            let glow = hsl(hue, 0.9, 0.55);
            let (top, bottom) = hero_tint(glow);
            for color in [top, bottom] {
                assert!(
                    luminance(color) <= HERO_MAX_LUMINANCE + 1e-3,
                    "{hue}: {:.4}",
                    luminance(color)
                );
                assert!(
                    contrast(TEXT, color) >= 10.,
                    "{hue}: {:.2}",
                    contrast(TEXT, color)
                );
                assert!(
                    contrast(blend(0.64, color), color) >= 4.5,
                    "{hue}: {:.2}",
                    contrast(blend(0.64, color), color)
                );
            }
        }
        // Renksiz kapakta (BG) ton kart yüzeyine çok yakın.
        assert_eq!(hero_tint(BAR).0, BAR);
    }

    /// Basit HSL → 0xRRGGBB (yalnız test).
    fn hsl(h: f32, s: f32, l: f32) -> u32 {
        let c = (1. - (2. * l - 1.).abs()) * s;
        let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
        let m = l - c / 2.;
        let (r, g, b) = match h as u32 {
            0..=59 => (c, x, 0.),
            60..=119 => (x, c, 0.),
            120..=179 => (0., c, x),
            180..=239 => (0., x, c),
            240..=299 => (x, 0., c),
            _ => (c, 0., x),
        };
        let ch = |v: f32| ((v + m) * 255.).round().clamp(0., 255.) as u32;
        (ch(r) << 16) | (ch(g) << 8) | ch(b)
    }
}
