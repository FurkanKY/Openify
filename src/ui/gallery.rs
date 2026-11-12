//! Bileşen vitrini (Adım 9): her bileşen bütün durumlarıyla yan yana,
//! gpui'nin gerçek çizimiyle.
//!
//! `OPENIFY_GALLERY=1` ile açılır; liste alanının yerine geçer. Tasarım onayı
//! burada verilir, sonraki adımlarda da gerileme kontrolü için kullanılır.

use std::time::Instant;

use gpui::{AnyElement, Context, ElementId, FontWeight, Window, div, prelude::*, px, rgb};

use super::{
    Shell,
    components::{
        Look, MenuEntry, MenuItem, Tooltip, chip, explain_error, icon_button, kbd, menu,
        ghost_button, play_button, primary_button, primary_button_sized, secondary_button, state_view,
    },
    library::skeleton_rows,
    pill::{PILL_CLEARANCE, PILL_MAX_WIDTH, PillLeft, PillModel},
    player::Notice,
    shelf::{Card, CardAction, CardMeta, CardState, FolderInfo, SHELF_COVER, SHELF_NAME, STRIP_COVER},
    text::line,
    theme::*,
};
use crate::engine::{CollectionId, NoticeKind, TrackRow};

/// Vitrin açık mı (`OPENIFY_GALLERY`)?
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OPENIFY_GALLERY").is_some())
}

/// Bölüm: küçük başlık ve içerik.
fn section(title: &'static str, body: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(S3))
        .child(
            div()
                .type_role(T_LABEL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(label())
                .child(title),
        )
        .child(body)
        .into_any_element()
}

/// Durum sütunlarının başlığı.
fn look_header() -> AnyElement {
    let mut row = div().flex().items_center().gap(px(S5)).child(div().w(px(120.)));
    for look in Look::ALL {
        row = row.child(
            div()
                .w(px(120.))
                .type_role(T_LABEL)
                .text_color(label())
                .child(look.name()),
        );
    }
    row.into_any_element()
}

/// Bir bileşenin beş durumu tek satırda.
fn look_row(name: &'static str, make: impl Fn(Look) -> AnyElement) -> AnyElement {
    let mut row = div().flex().items_center().gap(px(S5)).child(
        div()
            .w(px(120.))
            .type_role(T_SECONDARY)
            .text_color(muted())
            .child(name),
    );
    for look in Look::ALL {
        row = row.child(div().w(px(120.)).flex().child(make(look)));
    }
    row.into_any_element()
}

fn sample_track(n: usize) -> TrackRow {
    let (name, artists, album) = [
        ("Sür Eve", "Güneş", "Sür Eve"),
        ("Bir Pesimistin Gözyaşları", "Sagopa Kajmer", "Bir Pesimistin Gözyaşları"),
        ("Kışın arabada sohbet gibisi yok, hele radyoda eski bir şarkı varsa", "Uzun Ad Denemesi", "Çok uzun bir albüm adı: kısaltma burada denenir"),
        ("Dal", "Adamlar", "Harekete kimse mâni olamaz."),
        ("2036", "AURA", "ELEMENTAL"),
        ("Melekler Ölmez", "mor ve ötesi", "Melekler Ölmez"),
    ][n % 6];
    TrackRow {
        uri: format!("spotify:track:vitrin{n}"),
        name: name.into(),
        artists: artists.into(),
        album: album.into(),
        duration_ms: 180_000 + n as u32 * 17_000,
        image: String::new(),
    }
}

/// Gölge sınaması (`OPENIFY_GALLERY=golge`): eski sağ panel gibi yüzen,
/// gölgeli panel şarkı satırlarının üstünde. `OPENIFY_GOLGE_KATMAN=0|1|2`:
/// gölgesiz / tek / iki katman. "Şeffaf delik" sınamasının yeri (7 Kasım:
/// delikler gpui'nin değil, ekran görüntüsü betiğinin; bkz.
/// vendor/gpui/OPENIFY.md madde 7).
fn shadow_test() -> Option<u8> {
    (std::env::var("OPENIFY_GALLERY").ok()? == "golge")
        .then(|| std::env::var("OPENIFY_GOLGE_KATMAN").ok().and_then(|v| v.parse().ok()).unwrap_or(1))
}

impl Shell {
    fn shadow_gallery(&self, layers: u8, cx: &mut Context<Self>) -> AnyElement {
        let mut shadows = if layers == 0 { Vec::new() } else { float_shadow() };
        if layers == 2 {
            // DESIGN.md'deki eski ikinci katman (0 2 6 siyah %30).
            shadows.push(gpui::BoxShadow {
                color: shade(0.30).into(),
                offset: gpui::point(px(0.), px(2.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
            });
        }
        let mut rows = div().flex().flex_col();
        for i in 0..14 {
            rows = rows.child(self.track_row(3000 + i, &sample_track(i), i == 3, i == 5, false, Look::Live, cx));
        }
        let mut panel_rows = div().flex().flex_col().gap(px(S2)).p(px(S4));
        for i in 0..8 {
            panel_rows = panel_rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .child(div().size(px(40.)).rounded(px(R_SM)).bg(rgb(PLACEHOLDER)))
                    .child(div().flex_1().min_w_0().type_role(T_BODY).text_color(text()).child(line(sample_track(i).name))),
            );
        }
        // Eski sağ panelle aynı yüzey (bkz. git 5d40f4f `render_side_panel`).
        let panel = div()
            .absolute()
            .top(px(S2))
            .right(px(S4))
            .w(px(360.))
            .h(px(520.))
            .rounded(px(R_XL))
            .bg(rgb(BAR))
            .border_1()
            .border_color(rgb(BORDER))
            .shadow(shadows)
            .overflow_hidden()
            .opacity(1.)
            .occlude()
            .child(panel_rows);
        div()
            .id("gallery-golge")
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .overflow_hidden()
            .child(rows)
            .child(panel)
            .into_any_element()
    }

    pub(super) fn render_gallery(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(layers) = shadow_test() {
            return self.shadow_gallery(layers, cx);
        }
        let accent = self.accent().text;

        // Yazı ölçeği: altı rol.
        let roles = [
            ("Sayfa başlığı · 40", T_DISPLAY, true),
            ("Bölüm başlığı · 28", T_HEADLINE, true),
            ("Palet, kart · 18", T_TITLE, false),
            ("Ana metin · 14", T_BODY, false),
            ("İkincil · 13", T_SECONDARY, false),
            ("Etiket · 12", T_LABEL, false),
        ];
        let mut type_scale = div().flex().flex_col().gap(px(S3));
        for (name, role, display) in roles {
            type_scale = type_scale.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S5))
                    .child(div().w(px(160.)).type_role(T_LABEL).text_color(label()).child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .type_role(role)
                            .when(display, |el| {
                                el.font_family(font()).font_weight(FontWeight::EXTRA_BOLD)
                            })
                            .text_color(text())
                            .child(line("Beğenilenler · Sür Eve")),
                    ),
            );
        }

        // Boşluk ölçeği: dokuz basamak.
        let mut spacing = div().flex().items_end().gap(px(S4));
        for (name, step) in [
            ("2", S0),
            ("4", S1),
            ("8", S2),
            ("12", S3),
            ("16", S4),
            ("24", S5),
            ("32", S6),
            ("48", S7),
            ("64", S8),
        ] {
            spacing = spacing.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(S1))
                    .child(div().w(px(step)).h(px(step)).rounded(px(R_XS)).bg(veil(VEIL_SELECTED)))
                    .child(div().type_role(T_LABEL).tabular().text_color(label()).child(name)),
            );
        }

        // Ton katmanları.
        let mut veils = div().flex().flex_wrap().gap(px(S4));
        for (name, alpha) in [
            ("Üzerine gelme 6%", VEIL_HOVER),
            ("Seçili 10%", VEIL_SELECTED),
            ("Basma 14%", VEIL_PRESSED),
        ] {
            veils = veils.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S1))
                    .child(div().w(px(120.)).h(px(40.)).rounded(px(R_MD)).bg(veil(alpha)))
                    .child(div().type_role(T_LABEL).text_color(label()).child(name)),
            );
        }

        // Çalan satırın kapak ışığı basamakları.
        for (name, alpha) in [
            ("Çalıyor 8%", ACCENT_PLAYING),
            ("Çalıyor, üzerine 12%", ACCENT_PLAYING_HOVER),
            ("Çalıyor, seçili 16%", ACCENT_PLAYING_SELECTED),
            ("Çalıyor, basma 20%", ACCENT_PLAYING_PRESSED),
        ] {
            veils = veils.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S1))
                    .child(div().w(px(120.)).h(px(40.)).rounded(px(R_MD)).bg(accent_veil(accent, alpha)))
                    .child(div().type_role(T_LABEL).text_color(label()).child(name)),
            );
        }

        // Düğmeler.
        let buttons = div()
            .flex()
            .flex_col()
            .gap(px(S4))
            .child(look_header())
            .child(look_row("Simge düğmesi", |look| {
                icon_button(("g-icon", look as usize), "skip-forward-fill", look, accent)
                    .into_any_element()
            }))
            .child(look_row("Çal düğmesi", |look| {
                play_button(("g-play", look as usize), false, look, accent).into_any_element()
            }))
            .child(look_row("Asıl düğme", |look| {
                primary_button(("g-primary", look as usize), "play-fill", "Çal", look, accent)
                    .into_any_element()
            }))
            .child(look_row("Asıl düğme, küçük", |look| {
                primary_button_sized(("g-primary-small", look as usize), "pause-fill", "Duraklat", look, accent, true)
                    .into_any_element()
            }))
            .child(look_row("Süzgeç hapı", |look| {
                chip(("g-chip", look as usize), "Albümler", false, look, accent).into_any_element()
            }))
            .child(look_row("Süzgeç hapı, seçili", |look| {
                chip(("g-chip-on", look as usize), "Hepsi", true, look, accent).into_any_element()
            }))
            .child(look_row("Sessiz düğme", |look| {
                ghost_button(("g-ghost", look as usize), "arrow-up-down", "Son çalınan", false, look, accent).into_any_element()
            }))
            .child(look_row("Sessiz düğme, asıl", |look| {
                ghost_button(("g-ghost-strong", look as usize), "plus", "Yeni liste", true, look, accent).into_any_element()
            }));

        // Satırlar: gerçek şarkı satırı.
        let mut rows = div().flex().flex_col().gap(px(S1));
        let states: [(&str, bool, bool, Look); 6] = [
            ("Normal", false, false, Look::Live),
            ("Üzerine gelme", false, false, Look::Hover),
            ("Basma", false, false, Look::Pressed),
            ("Seçili", false, true, Look::Live),
            ("Çalıyor", true, false, Look::Live),
            ("Çalıyor + seçili", true, true, Look::Live),
        ];
        for (i, (name, playing, selected, look)) in states.into_iter().enumerate() {
            let track = sample_track(if i == 2 { 2 } else { i });
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .child(div().w(px(120.)).type_role(T_SECONDARY).text_color(muted()).child(name))
                    .child(div().flex_1().min_w_0().child(self.track_row(
                        1000 + i,
                        &track,
                        playing,
                        selected,
                        false,
                        look,
                        cx,
                    ))),
            );
        }
        // Raf kartı: bütün durumlar (raftaki ve "Tümü"deki kart aynı).
        let card = |i: usize, name: &str| Card {
            id: CollectionId::Playlist(format!("spotify:playlist:vitrin{i}")),
            name: name.into(),
            subtitle: "Takip edilen · 29 şarkı".into(),
            image: String::new(),
            meta: CardMeta::default(),
        };
        let state = |look, selected, playing| CardState { look, selected, playing };
        let shelf_states: [(&str, Card, CardState); 9] = [
            ("Normal", card(0, "Wholesome Mix"), state(Look::Live, false, false)),
            ("Üzerine gelme", card(1, "2020'ler Mix"), state(Look::Hover, false, false)),
            ("Basma", card(2, "Lights Out"), state(Look::Pressed, false, false)),
            ("Klavye odağı", card(3, "Buzuq Mix"), state(Look::Focus, false, false)),
            ("Seçili", card(4, "Kışın arabada sohbet gibisi yok"), state(Look::Live, true, false)),
            ("Çalıyor", card(5, "666"), state(Look::Live, false, true)),
            (
                "Beğenilenler, seçili + çalıyor",
                Card {
                    id: CollectionId::Liked,
                    name: "Beğenilenler".into(),
                    subtitle: String::new(),
                    image: String::new(),
                    meta: CardMeta::default(),
                },
                state(Look::Live, true, true),
            ),
            (
                "Sabitli",
                Card { meta: CardMeta { pin: Some(0), ..CardMeta::default() }, ..card(6, "Gece sürüşü") },
                state(Look::Live, false, false),
            ),
            (
                "Klasör",
                Card {
                    id: CollectionId::Playlist("spotify:start-group:vitrin:Yaz".into()),
                    name: "Yaz listeleri".into(),
                    subtitle: "5 liste".into(),
                    image: String::new(),
                    meta: CardMeta {
                        folder: Some(FolderInfo { key: "spotify:start-group:vitrin:Yaz".into(), open: false, count: 5 }),
                        ..CardMeta::default()
                    },
                },
                state(Look::Live, false, false),
            ),
        ];
        let caption = |name: &'static str| div().type_role(T_LABEL).text_color(label()).child(name);
        let mut shelf_cards = div().flex().flex_wrap().items_start().gap(px(S5));
        for (i, (name, c, st)) in shelf_states.iter().enumerate() {
            shelf_cards = shelf_cards.child(
                div()
                    .w(px(SHELF_COVER + S6))
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(self.shelf_card(
                        ElementId::Name(format!("g-shelf-{i}").into()),
                        c,
                        SHELF_COVER,
                        SHELF_NAME,
                        *st,
                        CardAction::Nothing,
                        cx,
                    ))
                    .child(caption(name)),
            );
        }
        // Şerit: liste kayınca raf 40 px kapaklara iner.
        let mut strip = div().flex().items_center().gap(px(S2));
        for (i, (_, c, st)) in shelf_states.iter().enumerate() {
            strip = strip.child(self.shelf_card(
                ElementId::Name(format!("g-strip-{i}").into()),
                c,
                STRIP_COVER,
                0.,
                *st,
                CardAction::Nothing,
                cx,
            ));
        }

        // Yüzen hap: bütün durumları.
        let now = Instant::now();
        let notice = |kind, text: &str| Notice { kind, text: text.into(), at: now };
        let model = |left, playing: bool, has_track: bool, glow: bool| PillModel {
            left,
            playing,
            has_track,
            has_next: has_track,
            start_title: (!has_track).then(|| "Çal: Beğenilenler".to_string()),
            position_ms: 46_000,
            duration_ms: 137_000,
            progress: 46. / 137.,
            volume: 0.8,
            volume_shown: 0.8,
            muted: false,
            glow: glow.then_some(self.accent().glow),
            live: false,
        };
        let track = |notice: Option<Notice>, offline: bool| PillLeft::Track {
            uri: "spotify:track:vitrin".into(),
            name: "Bulamazsın".into(),
            artists: "PERMA".into(),
            cover: None,
            notice,
            offline,
        };
        let pills: [(&str, PillModel); 8] = [
            ("Çalıyor", model(track(None, false), true, true, true)),
            ("Duraklatıldı", model(track(None, false), false, true, true)),
            ("Kaldığı yerde, çevrimdışı", model(track(None, true), false, true, true)),
            (
                "Sorun bildirimi (şarkının ikinci satırında)",
                model(track(Some(notice(NoticeKind::Problem, "Spotify bu şarkıyı vermedi. Çal'a basıp yeniden dene.")), false), false, true, true),
            ),
            (
                "Yükleniyor",
                model(PillLeft::Pending { name: "Sür Eve".into(), image: String::new(), notice: None }, false, false, false),
            ),
            ("Boşta (çal düğmesi açık listeyi başlatır)", model(PillLeft::Idle, false, false, false)),
            (
                "Boşta, süren iş",
                model(PillLeft::Notice(notice(NoticeKind::Working, "Spotify bağlantısı yenileniyor…")), false, false, false),
            ),
            (
                "Çevrimdışı",
                model(
                    PillLeft::Offline { icon: "wifi-off", title: "Çevrimdışı", detail: "Kütüphane önbellekten" },
                    false,
                    false,
                    false,
                ),
            ),
        ];
        let mut pill_states = div().flex().flex_col().gap(px(S5));
        for (i, (name, m)) in pills.iter().enumerate() {
            pill_states = pill_states.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(caption(name))
                    .child(div().id(("g-pill", i)).w(px(PILL_MAX_WIDTH)).child(self.pill(m, window, cx))),
            );
        }

        // İpucu, kısayol, menü.
        let floating = div()
            .flex()
            .items_start()
            .gap(px(S6))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S3))
                    .child(Tooltip::element("Sonraki şarkı", None))
                    .child(Tooltip::element("Çal", Some("Boşluk")))
                    .child(div().flex().child(kbd("Ctrl K"))),
            )
            .child(menu(
                "g-menu",
                vec![
                    MenuEntry::Item(MenuItem::new("Sıraya ekle").icon("list-music").shortcut("Q")),
                    MenuEntry::Item(MenuItem::new("Albüme git").icon("disc-3").look(Look::Hover)),
                    MenuEntry::Item(MenuItem::new("Sanatçıya git").icon("music")),
                    MenuEntry::Separator,
                    MenuEntry::Item(
                        MenuItem::new("Bağlantıyı kopyala").shortcut("Ctrl C").look(Look::Disabled),
                    ),
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuItem::new("Listeden çıkar").icon("x").danger()),
                ],
                accent,
            ));

        // Kapaktan gelen vurgu: zemine göre okunur hâle getirilmiş örnekler.
        let mut accents = div().flex().gap(px(S4));
        for raw in [0xF59E5B, 0x1E3A8A, 0x7F1D1D, 0x14532D, 0x4C1D95, 0x525252] {
            let fixed = ensure_contrast(raw, BG, 4.5);
            accents = accents.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S1))
                    .child(div().w(px(80.)).h(px(24.)).rounded(px(R_SM)).bg(rgb(raw)))
                    .child(div().type_role(T_BODY).font_weight(FontWeight::MEDIUM).text_color(rgb(fixed)).child("Sür Eve"))
                    .child(
                        div()
                            .type_role(T_LABEL)
                            .tabular()
                            .text_color(label())
                            .child(format!("{:.1}:1", contrast(fixed, BG))),
                    ),
            );
        }

        div()
            .id("gallery")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                div()
                    .p(px(S6))
                    // Hapın altında kalmasın.
                    .pb(px(PILL_CLEARANCE + S6))
                    .flex()
                    .flex_col()
                    .gap(px(S7))
                    .child(
                        div()
                            .font_family(font())
                            .type_role(T_HEADLINE)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child("Bileşen vitrini"),
                    )
                    .child(section("Yazı ölçeği", type_scale))
                    .child(section("Boşluk ölçeği", spacing))
                    .child(section("Ton katmanları", veils))
                    .child(section("Düğmeler", buttons))
                    .child(section("Şarkı satırı", rows))
                    .child(section("Raf kartı ve Tümü karosu (Tümü ızgarasındaki kart aynı)", shelf_cards))
                    .child(section("Şerit: kartlar ve Tümü düğmesi", strip))
                    .child(section("Yüzen hap", pill_states))
                    .child(section("İpucu, kısayol, menü", floating))
                    .child(section("Durumlar: hata, çevrimdışı, boş", {
                        let (i1, t1, m1) = explain_error("Spotify durum kodu 403");
                        let (i2, t2, m2) = explain_error("dns error: os error 11001");
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(S7))
                            .items_start()
                            .child(state_view(i1, t1, m1).child(secondary_button(
                                "g-retry",
                                "heart-fill",
                                "Beğenilenlere dön",
                                Look::Live,
                                accent,
                            )))
                            .child(state_view(i2, t2, m2).child(secondary_button(
                                "g-retry-2",
                                "refresh-cw",
                                "Yeniden dene",
                                Look::Hover,
                                accent,
                            )))
                            .child(state_view(
                                "list-music",
                                "Bu liste boş",
                                "Spotify'da bu listeye şarkı eklediğinde burada görünür.",
                            ))
                    }))
                    .child(self.now_playing_gallery(cx))
                    .child(self.now_panel_gallery(cx))
                    // Adım 12–13: sayfalar, gezinme, kütüphane yönetimi.
                    .child(self.pages_gallery(cx))
                    .child(self.manage_gallery(cx))
                    .child(section("Yükleniyor: iskelet satırlar", div().h(px(240.)).overflow_hidden().child(skeleton_rows(false))))
                    .child(section("Kapaktan gelen vurgu (okunur hâle getirilmiş)", accents)),
            )
            .into_any_element()
    }
}
