//! Ortak bileşenler (Adım 9).
//!
//! Her etkileşimli bileşenin beş durumu var: normal, üzerine gelme, basma,
//! klavye odağı, pasif. Uygulamada durum gpui'den gelir (`Look::Live`);
//! bileşen vitrininde (`gallery.rs`) bütün durumlar yan yana görülsün diye
//! zorlanabilir.
//!
//! Bileşen hissi: "sakin ve kesin". Dururken neredeyse görünmez, dokununca
//! anında ve net tepki: ton katmanı açılır, basınca biraz daha.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, AnyView, App, Bounds, Div, ElementId, FontWeight, IntoElement, ParentElement,
    Pixels, Point, Render, Rgba, SharedString, Stateful, Styled, Window, canvas, div, point,
    prelude::*, px, rgb,
};

use super::text::line_with_tip;
use super::theme::*;
use super::tween::{HoverFill, SwitchPart, hover_fill, switch_paint};
use crate::t;

/// Bir bileşenin hangi durumda çizileceği.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// Uygulamada: durumlar fareden ve klavyeden gelir.
    Live,
    Hover,
    Pressed,
    Focus,
    Disabled,
}

impl Look {
    pub const ALL: [Look; 5] = [
        Look::Live,
        Look::Hover,
        Look::Pressed,
        Look::Focus,
        Look::Disabled,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Look::Live => "Normal",
            Look::Hover => "Üzerine gelme",
            Look::Pressed => "Basma",
            Look::Focus => "Klavye odağı",
            Look::Disabled => "Pasif",
        }
    }

    pub fn enabled(self) -> bool {
        self != Look::Disabled
    }
}

/// Klavye odağı halkası: her düğmede hep duran 2 px'lik kenar; normalde
/// saydam, odaktayken vurgu renginde. Kenar hep orada olduğu için odak
/// gelince yerleşim kaymaz. (Gölgeyle çizilen halka, zemini saydam düğmenin
/// içini de boyuyordu.)
fn ring(el: Stateful<Div>, look: Look, color: u32) -> Stateful<Div> {
    el.border_2().border_color(if look == Look::Focus {
        rgb(color).into()
    } else {
        gpui::transparent_black()
    })
}

/// Dolu zeminli düğmelerin (asıl düğme, çal düğmesi) odak halkası: düğmenin
/// dışında, araya 2 px zemin rengi boşluk bırakan 2 px'lik halka. Halka
/// düğmenin üstüne çizilince renk farkı 1,6–1,9:1'de kalıyordu; dışarıda koyu
/// zeminin üstünde ≥ 4,5:1.
fn outer_ring(el: Stateful<Div>, look: Look, accent: u32) -> Stateful<Div> {
    if look != Look::Focus {
        return el;
    }
    el.shadow(vec![
        gpui::BoxShadow {
            color: rgb(accent).into(),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(1.),
            spread_radius: px(4.),
        },
        gpui::BoxShadow {
            color: rgb(palette().bg).into(),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(1.),
            spread_radius: px(2.),
        },
    ])
}

/// Durumlar: üzerine gelince `hover`, basınca `pressed`. Odaktayken de
/// fareye tepki verir; pasifken hiç.
fn states(el: Stateful<Div>, look: Look, hover: Rgba, pressed: Rgba) -> Stateful<Div> {
    match look {
        // Canlı durumlarda renk `layer`ın katmanından gelir (140 ms geçiş,
        // bkz. `tween`); burada yalnız imleç.
        Look::Live | Look::Focus => el.cursor_pointer(),
        Look::Hover => el.bg(hover),
        Look::Pressed => el.bg(pressed),
        Look::Disabled => el,
    }
}

/// Canlı durumların üzerine gelme / basma katmanı: bileşenin İLK çocuğu
/// olarak eklenir (içeriğin altında kalsın). `rest_alpha`: bileşenin kendi
/// zemininin saydamlığı (0 = yok ya da opak). Vitrinde zorlanan durumlarda
/// (`Look::Hover` …) katman yok, renk `states`ten düz gelir.
fn layer(
    look: Look,
    hover: Rgba,
    pressed: Rgba,
    rest_alpha: f32,
    radius: f32,
) -> Option<HoverFill> {
    matches!(look, Look::Live | Look::Focus).then(|| hover_fill(hover, pressed, rest_alpha, radius))
}

/// Ton katmanı durumları (zemini saydam öğeler: simge düğmesi, menü öğesi).
fn veil_states(el: Stateful<Div>, look: Look, accent: u32) -> Stateful<Div> {
    states(
        ring(el, look, accent),
        look,
        veil(VEIL_HOVER),
        veil(VEIL_PRESSED),
    )
}

/// `veil_states`in katmanı.
pub(super) fn veil_layer(look: Look, radius: f32) -> Option<HoverFill> {
    layer(look, veil(VEIL_HOVER), veil(VEIL_PRESSED), 0., radius)
}

// ── Menü konumu ─────────────────────────────────────────────────────────

/// Düğmenin ekrandaki yerini ölçen görünmez katman (düğmenin içine konur,
/// düğme `relative` olmalı) ve ölçüm hücresi. Menü hücredeki yerin altında
/// açılır (`menu_below`); sıralama düğmesindeki yöntemin aynısı. Hücre her
/// çizimde yenilenir: tıklama işleyicisi ile katman aynı kareden gelir.
pub fn anchor_probe() -> (AnyElement, Rc<Cell<Option<Bounds<Pixels>>>>) {
    let cell: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let set = cell.clone();
    let probe = canvas(move |bounds, _, _| set.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element();
    (probe, cell)
}

/// Menünün açılacağı nokta: düğmenin sol kenarı, alt kenarının `S1` altı.
/// Henüz ölçülmediyse (ilk kare) tıklanan nokta.
pub fn menu_below(anchor: Option<Bounds<Pixels>>, click: Point<Pixels>) -> Point<Pixels> {
    match anchor {
        Some(b) => point(b.origin.x, b.origin.y + b.size.height + px(S1)),
        None => click,
    }
}

// ── Düğmeler ────────────────────────────────────────────────────────────

/// Simge düğmesi (36 px, yuvarlak): önceki / sonraki, sessize al. Sadece
/// simgeden oluştuğu için her zaman ipucuyla kullanılır (`tooltip`).
pub fn icon_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    icon_button_tinted(id, icon_name, soft(), look, accent)
}

/// `icon_button`, simge rengi verilen (kayıtlı albümün kalbi vurgu renginde).
/// Simgenin rengi düğmenin `text_color`ından değil buradan gelir; pasifken
/// ton yine `disabled()`.
pub fn icon_button_tinted(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    tint: Rgba,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let color = if look.enabled() { tint } else { disabled() };
    let el = div()
        .id(id)
        .size(px(36.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .children(veil_layer(look, FULL))
        .child(icon(icon_name, 20., color));
    veil_states(el, look, accent)
}

/// Asıl eylem düğmesi: vurgu renginde, yazılı ("Çal", "Duraklat").
pub fn primary_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    label: impl Into<SharedString>,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    primary_button_sized(id, icon_name, label, look, accent, false)
}

/// Asıl düğmenin yüksekliği: normal ve küçük (kayınca küçülen liste başlığı).
/// Hap biçimli (tam yuvarlak) öğenin katman köşesi: dörtgenin yarı kısa
/// kenarına kırpılır.
pub(super) const FULL: f32 = 1000.;
pub const PRIMARY_HEIGHT: f32 = 40.;
pub const PRIMARY_HEIGHT_SMALL: f32 = 32.;

/// Asıl düğme; `small` ise 32 px yüksek, 13 px yazılı (liste başlığı
/// küçülünce düğme de küçülür).
pub fn primary_button_sized(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    label: impl Into<SharedString>,
    look: Look,
    accent: u32,
    small: bool,
) -> Stateful<Div> {
    let fill = if look.enabled() {
        accent
    } else {
        mix(accent, palette().bg, 0.6)
    };
    let el = div()
        .id(id)
        .h(px(if small {
            PRIMARY_HEIGHT_SMALL
        } else {
            PRIMARY_HEIGHT
        }))
        .pl(px(if small { S3 } else { S4 }))
        .pr(px(if small { S4 } else { S5 }))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(if small { S1 } else { S2 }))
        .bg(rgb(fill))
        .text_color(rgb(palette().bg))
        .type_role(if small { T_SECONDARY } else { T_BODY })
        .font_weight(FontWeight::SEMIBOLD)
        .children(layer(
            look,
            rgb(accent_hover(accent)),
            rgb(mix(accent, 0x000000, 0.14)),
            0.,
            FULL,
        ))
        .child(icon(
            icon_name,
            if small { 14. } else { 16. },
            rgb(palette().bg),
        ))
        .child(label.into());
    let el = outer_ring(el, look, accent);
    states(
        el,
        look,
        rgb(accent_hover(accent)),
        rgb(mix(accent, 0x000000, 0.14)),
    )
}

/// Süzgeç hapı ("Hepsi", "Albümler"): 32 px, yuvarlak. Kapalıyken ton
/// katmanı, açıkken (seçili süzgeç) ön plan renginde dolu.
pub fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let fill = if on {
        layer(
            look,
            rgb(mix(palette().text, accent, 0.14)),
            rgb(mix(palette().text, palette().bg, 0.22)),
            0.,
            FULL,
        )
    } else {
        layer(
            look,
            veil(VEIL_SELECTED),
            veil(VEIL_PRESSED),
            VEIL_HOVER,
            FULL,
        )
    };
    let el = div()
        .id(id)
        .h(px(32.))
        .px(px(S3))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .type_role(T_SECONDARY)
        .children(fill)
        .child(label.into());
    if on {
        let el = el
            .bg(if look.enabled() { text() } else { veil(0.24) })
            .text_color(rgb(palette().bg))
            .font_weight(FontWeight::SEMIBOLD);
        let el = outer_ring(el, look, accent);
        // Beyaz zeminde ton katmanı görünmez: çal düğmesi gibi ısınır / grileşir.
        states(
            el,
            look,
            rgb(mix(palette().text, accent, 0.14)),
            rgb(mix(palette().text, palette().bg, 0.22)),
        )
    } else {
        let el = el
            .bg(veil(VEIL_HOVER))
            .text_color(if look.enabled() { muted() } else { disabled() })
            .font_weight(FontWeight::MEDIUM);
        let el = ring(el, look, accent);
        states(el, look, veil(VEIL_SELECTED), veil(VEIL_PRESSED))
    }
}

/// Çalar hapının ortasındaki büyük çal / duraklat düğmesi (44 px, beyaz).
pub fn play_button(
    id: impl Into<ElementId>,
    playing: bool,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .size(px(44.))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(if look.enabled() { text() } else { veil(0.24) })
        .children(layer(
            look,
            rgb(mix(palette().text, accent, 0.14)),
            rgb(mix(palette().text, palette().bg, 0.22)),
            0.,
            FULL,
        ))
        .child(icon(
            if playing { "pause-fill" } else { "play-fill" },
            18.,
            rgb(palette().bg),
        ));
    // Beyaz zeminde ton katmanı görünmez: üzerine gelince kapağın rengine
    // hafifçe ısınır, basınca grileşir.
    let el = outer_ring(el, look, accent);
    states(
        el,
        look,
        rgb(mix(palette().text, accent, 0.14)),
        rgb(mix(palette().text, palette().bg, 0.22)),
    )
}

/// İkincil düğme: ince kenarlı, saydam zeminli ("Yeniden dene").
pub fn secondary_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    label: impl Into<SharedString>,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let color = if look.enabled() { text() } else { disabled() };
    let el = div()
        .id(id)
        .h(px(32.))
        .px(px(S3))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(S2))
        .border_1()
        .border_color(veil(LINE_INPUT))
        .text_color(color)
        .type_role(T_SECONDARY)
        .font_weight(FontWeight::SEMIBOLD)
        .children(veil_layer(look, FULL))
        .child(icon(icon_name, 14., color))
        .child(label.into());
    let el = if look == Look::Focus {
        el.border_color(rgb(accent))
    } else {
        el
    };
    states(el, look, veil(VEIL_HOVER), veil(VEIL_PRESSED))
}

/// Sessiz düğme (7 Kasım gecesi, kütüphane araç satırı): kenarsız, zemini
/// saydam, simge ve soluk yazı; üzerine gelince ton katmanı. `strong`: yazı
/// ön plan renginde (satırdaki asıl eylem, "Yeni liste").
pub fn ghost_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    label: impl Into<SharedString>,
    strong: bool,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let color = match (look.enabled(), strong) {
        (false, _) => disabled(),
        (true, true) => text(),
        (true, false) => muted(),
    };
    let el = div()
        .id(id)
        .h(px(32.))
        .px(px(S3))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(S2))
        .text_color(color)
        .type_role(T_SECONDARY)
        .font_weight(FontWeight::MEDIUM)
        .children(veil_layer(look, FULL))
        .child(icon(icon_name, 14., color))
        .child(label.into());
    let el = if look == Look::Focus {
        el.border_1().border_color(rgb(accent))
    } else {
        el
    };
    states(el, look, veil(VEIL_HOVER), veil(VEIL_PRESSED))
}

// ── Ayar denetimleri (Adım 15) ───────────────────────────────────────────

/// Anahtar (açık / kapalı): 44 × 24, yuvarlak. Kapalıyken oluk renginde ve
/// soluk düğme; açıkken vurgu renginde ve koyu düğme (çal düğmesi gibi:
/// vurgunun üstünde koyu).
pub fn switch(id: impl Into<ElementId>, on: bool, look: Look, accent: u32) -> Stateful<Div> {
    // Oluk ve topuz rengi iki durum için; geçişi `SwitchPaint` çizer.
    let colors = |on: bool| match (look.enabled(), on) {
        (false, _) => (veil(0.10), disabled()),
        (true, true) => (rgb(accent), rgb(palette().bg)),
        (true, false) => (rgb(palette().track), soft()),
    };
    let ((track_off, knob_off), (track_on, knob_on)) = (colors(false), colors(true));
    let (hover, pressed) = if on {
        (rgb(accent_hover(accent)), rgb(mix(accent, 0x000000, 0.14)))
    } else {
        (veil(VEIL_SELECTED), veil(VEIL_PRESSED))
    };
    let el = div()
        .id(id)
        .w(px(44.))
        .h(px(24.))
        .flex_shrink_0()
        .p(px(S0))
        .rounded_full()
        .relative()
        .child(switch_paint(
            on,
            SwitchPart::Track {
                off: track_off.into(),
                on: track_on.into(),
            },
        ))
        .children(layer(look, hover, pressed, 0., FULL))
        .child(switch_paint(
            on,
            SwitchPart::Knob {
                off: knob_off.into(),
                on: knob_on.into(),
                size: 20.,
                inset: S0,
            },
        ));
    let el = outer_ring(el, look, accent);
    states(el, look, hover, pressed)
}

/// Birkaç seçenekli düğmenin (ses kalitesi, animasyon) çerçevesi: koyu,
/// ince kenarlı hap; içine `segment`ler konur.
pub fn segmented_track() -> Div {
    div()
        .flex_shrink_0()
        .p(px(S0))
        .rounded_full()
        .flex()
        .items_center()
        .bg(shade(SHADE_INPUT))
        .border_1()
        .border_color(veil(LINE_INPUT))
}

/// Seçenek düğmesi: seçili olan ön plan renginde dolu (süzgeç hapı gibi),
/// diğerleri soluk yazı, üzerine gelince ton katmanı.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .h(px(28.))
        .px(px(S3))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .type_role(T_SECONDARY);
    if on {
        let el = el
            .bg(if look.enabled() { text() } else { veil(0.24) })
            .text_color(rgb(palette().bg))
            .font_weight(FontWeight::SEMIBOLD)
            .child(label.into());
        outer_ring(el, look, accent)
    } else {
        let el = el
            .text_color(if look.enabled() { muted() } else { disabled() })
            .font_weight(FontWeight::MEDIUM)
            .children(veil_layer(look, FULL))
            .child(label.into());
        states(
            ring(el, look, accent),
            look,
            veil(VEIL_HOVER),
            veil(VEIL_PRESSED),
        )
    }
}

/// Seçim alanı (çıkış aygıtı): arama kutusu gibi koyu, kenarlı; yazı ve
/// aşağı ok. Açılan liste çağıranın (menü paneli).
pub fn select_field(
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    open: bool,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let color = if look.enabled() { text() } else { disabled() };
    let el = div()
        .id(id)
        .h(px(32.))
        .px(px(S3))
        .rounded(px(R_LG))
        .flex()
        .items_center()
        .gap(px(S2))
        .bg(shade(SHADE_INPUT))
        .border_1()
        .border_color(if open || look == Look::Focus {
            rgb(accent)
        } else {
            veil(LINE_INPUT)
        })
        .type_role(T_SECONDARY)
        .text_color(color)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(line_with_tip("select-value", value.into()).flex().min_w_0()),
        )
        .child(icon(
            "chevron-down",
            14.,
            if look.enabled() { muted() } else { disabled() },
        ));
    if look.enabled() {
        el.cursor_pointer().hover(|s| {
            s.bg(shade(SHADE_INPUT_HOVER)).border_color(if open {
                rgb(accent)
            } else {
                veil(LINE_HOVER)
            })
        })
    } else {
        el
    }
}

// ── Durumlar ────────────────────────────────────────────────────────────

/// Boş, hata ya da çevrimdışı durumu: sessiz bir karoda simge, başlık ve
/// ne olduğunu / ne yapılabileceğini söyleyen açıklama. Eylem (ör. "Yeniden
/// dene") çağıran tarafından eklenir.
pub fn state_view(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(S3))
        .max_w(px(360.))
        .child(
            div()
                .size(px(S7))
                .rounded(px(R_LG))
                .bg(rgb(palette().placeholder))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icon_name, 22., label())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(S1))
                .child(
                    div()
                        .type_role(T_BODY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text())
                        .child(title.into()),
                )
                .child(
                    div()
                        .type_role(T_SECONDARY)
                        .text_color(muted())
                        .text_center()
                        .child(message.into()),
                ),
        )
}

/// Tam sayfa durumun sütun eni (karşılama, çevrimdışı, Premium, motor durdu).
const FULL_STATE_WIDTH: f32 = 440.;

/// Tam sayfa durumun üstündeki sessiz simge karosu (`full_state`'in `mark`'ı):
/// 64 px, 12 px köşe, yer tutucu zemini, ortada soluk simge. Karşılamada
/// bunun yerine uygulama işareti konur.
pub fn state_mark(icon_name: &'static str) -> AnyElement {
    div()
        .size(px(S8))
        .rounded(px(R_XL))
        .bg(rgb(palette().placeholder))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(icon_name, 28., label()))
        .into_any_element()
}

/// Tam sayfa durum (karşılama, çevrimdışı, Premium gerekli, motor durdu,
/// "uzun sürdü"): ortada tek sütun. Yukarıdan aşağı: işaret, başlık (Headline),
/// açıklama (Body, soluk), eylemler (ilki asıl düğme) ve altında tek satır not.
/// Etiket üstü / kicker yok. Sayfanın ortasına koymak çağıranın işi.
pub fn full_state(
    mark: AnyElement,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    actions: Vec<AnyElement>,
    note: Option<SharedString>,
) -> Div {
    let words = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(S2))
        .child(
            div()
                .font_family(font())
                .type_role(T_HEADLINE)
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(text())
                .text_center()
                .child(title.into()),
        )
        .child(
            div()
                .type_role(T_BODY)
                .text_color(muted())
                .text_center()
                .child(body.into()),
        );
    let steps = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(S4))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(S3))
                .children(actions),
        )
        .children(note.map(|n| {
            div()
                .type_role(T_SECONDARY)
                .text_color(label())
                .text_center()
                .child(n)
        }));
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(S5))
        .max_w(px(FULL_STATE_WIDTH))
        .child(mark)
        .child(words)
        .child(steps)
}

/// Ham hata metnini kullanıcının anlayacağı başlık ve açıklamaya çevirir.
/// (Önceden "Alınamadı: {e}" ile teknik metin olduğu gibi gösteriliyordu.)
/// Spotify erişim vermedi mi (403)? Yeniden denemek bir şey değiştirmez.
pub fn is_forbidden(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("403") || e.contains("forbidden")
}

pub fn explain_error(error: &str) -> (&'static str, &'static str, &'static str) {
    let e = error.to_lowercase();
    // Adım 17: motor giriş / Premium gerektiğini bu öneklerle bildirir
    // (`engine::login::LinkState::library_error`; yazılar `crate::msg`).
    if e.starts_with("auth:") {
        ("user", t!(LibNeedsLoginTitle), t!(LibNeedsLoginBody))
    } else if e.starts_with("premium:") {
        (
            "circle-alert",
            t!(PillNeedsPremiumTitle),
            t!(LibNeedsPremiumBody),
        )
    } else if is_forbidden(&e) {
        ("circle-alert", t!(ErrForbiddenTitle), t!(ErrForbiddenBody))
    } else if e.contains("dns")
        || e.contains("connect")
        || e.contains("timed out")
        || e.contains("timeout")
        || e.contains("os error")
        || e.contains("network")
    {
        ("wifi-off", t!(ErrOfflineTitle), t!(ErrOfflineBody))
    } else if e.contains("429") || e.contains("rate") {
        ("circle-alert", t!(ErrBusyTitle), t!(ErrBusyBody))
    } else {
        ("circle-alert", t!(ErrFailedTitle), t!(ErrFailedBody))
    }
}

// ── Küçük parçalar ──────────────────────────────────────────────────────

/// Kısayol etiketi ("Ctrl K").
pub fn kbd(keys: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .px(px(S2))
        .rounded(px(R_SM))
        .bg(veil(VEIL_SELECTED))
        .type_role(T_LABEL)
        .tabular()
        .text_color(muted())
        .child(keys.into())
}

// ── İpucu ───────────────────────────────────────────────────────────────

/// Sadece simgeden oluşan düğmelerin ipucu: ne yaptığı ve kısayolu.
pub struct Tooltip {
    text: SharedString,
    shortcut: Option<SharedString>,
}

impl Tooltip {
    /// `.tooltip(...)` için kurucu.
    pub fn text(
        text: impl Into<SharedString>,
        shortcut: Option<&'static str>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |_, cx| {
            cx.new(|_| Tooltip {
                text: text.clone(),
                shortcut: shortcut.map(SharedString::from),
            })
            .into()
        }
    }

    /// Vitrin için: ipucunun kendisi, bir düğmeye bağlı olmadan.
    pub fn element(text: impl Into<SharedString>, shortcut: Option<&'static str>) -> Div {
        tooltip_panel(text.into(), shortcut.map(SharedString::from))
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        // gpui ipucunu fareye göre yerleştiriyor; biraz aşağı it ki imleç
        // yazının üstüne gelmesin.
        div()
            .pt(px(S2))
            .child(tooltip_panel(self.text.clone(), self.shortcut.clone()))
    }
}

fn tooltip_panel(content: SharedString, shortcut: Option<SharedString>) -> Div {
    div()
        .font_family(UI_FONT)
        .flex()
        .items_center()
        .gap(px(S2))
        .px(px(S2))
        .py(px(S1))
        .rounded(px(R_MD))
        .bg(rgb(palette().bar))
        .border_1()
        .border_color(rgb(palette().border))
        .shadow(float_shadow())
        .type_role(T_LABEL)
        .text_color(text())
        .child(content)
        .children(shortcut.map(kbd))
}

// ── Menü ────────────────────────────────────────────────────────────────

/// Menü öğesi (sağ tık menüsü, tepsi menüsü).
pub struct MenuItem {
    pub icon: Option<&'static str>,
    pub label: SharedString,
    pub shortcut: Option<&'static str>,
    pub danger: bool,
    pub look: Look,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        MenuItem {
            icon: None,
            label: label.into(),
            shortcut: None,
            danger: false,
            look: Look::Live,
        }
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn shortcut(mut self, keys: &'static str) -> Self {
        self.shortcut = Some(keys);
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }
}

pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// Yüzen menü paneli. Öğe yüksekliği 32 px, genişlik içeriğe göre (en az
/// 200 px). Tehlikeli işlemler (silme) en altta, kırmızı yazıyla.
pub fn menu(id: impl Into<ElementId>, entries: Vec<MenuEntry>, accent: u32) -> Stateful<Div> {
    let mut panel = menu_panel(id);
    for (i, entry) in entries.into_iter().enumerate() {
        panel = panel.child(match entry {
            MenuEntry::Separator => menu_separator().into_any_element(),
            MenuEntry::Item(item) => menu_item(i, item, accent).into_any_element(),
        });
    }
    panel
}

/// Menünün paneli (öğeler çağıranın: tepsi menüsü öğelere tıklama bağlar).
pub fn menu_panel(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .font_family(UI_FONT)
        .min_w(px(MENU_MIN_WIDTH))
        .p(px(S1))
        .flex()
        .flex_col()
        .rounded(px(R_XL))
        .bg(rgb(palette().bar))
        .border_1()
        .border_color(rgb(palette().border))
        .shadow(float_shadow())
}

/// Menü panelinin en dar hâli ve öğe yüksekliği.
pub const MENU_MIN_WIDTH: f32 = 200.;
pub const MENU_ITEM_HEIGHT: f32 = 32.;
/// Ayırıcının kapladığı yükseklik (1 px çizgi, üstünde ve altında S1).
pub const MENU_SEPARATOR_HEIGHT: f32 = 1. + 2. * S1;

pub fn menu_separator() -> Div {
    div().my(px(S1)).mx(px(S2)).h(px(1.)).bg(veil(LINE))
}

pub fn menu_item(i: usize, item: MenuItem, accent: u32) -> Stateful<Div> {
    let color = match (item.look.enabled(), item.danger) {
        (false, _) => disabled(),
        (true, true) => rgb(palette().danger_text),
        (true, false) => text(),
    };
    let el = div()
        .id(i)
        .h(px(MENU_ITEM_HEIGHT))
        .px(px(S2))
        .rounded(px(R_MD))
        .flex()
        .items_center()
        .gap(px(S3))
        .type_role(T_BODY)
        .children(veil_layer(item.look, R_MD))
        .text_color(color)
        .child(
            div()
                .size(px(16.))
                .flex_shrink_0()
                .children(item.icon.map(|name| icon(name, 16., color))),
        )
        // Uzun liste ya da sanatçı adı (alt menü) kısalırsa tamamı ipucunda.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(line_with_tip("label", item.label).flex().min_w_0()),
        )
        .children(item.shortcut.map(|keys| {
            div()
                .type_role(T_LABEL)
                .tabular()
                .text_color(label())
                .child(keys)
        }));
    veil_states(el, item.look, accent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    #[test]
    fn menu_opens_below_the_button_and_not_over_it() {
        let button = Bounds {
            origin: point(px(300.), px(180.)),
            size: size(px(36.), px(36.)),
        };
        let click = point(px(320.), px(200.));
        let at = menu_below(Some(button), click);
        assert_eq!(at, point(px(300.), px(180. + 36. + S1)));
        // Menünün üst kenarı düğmenin alt kenarından aşağıda: düğmeyi örtmez.
        assert!(at.y > button.origin.y + button.size.height);
        // Ölçüm yoksa tıklanan noktaya düşer.
        assert_eq!(menu_below(None, click), click);
    }

    #[test]
    fn tinted_icon_button_keeps_its_size() {
        // Vurgulu kalp ile düz simge düğmesi aynı kutu: yalnız renk değişir.
        let _ = icon_button_tinted("t", "heart-fill", rgb(0xff0000), Look::Live, 0xff0000);
        let _ = icon_button("t", "heart", Look::Disabled, 0xff0000);
    }
}
