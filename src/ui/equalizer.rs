//! Ekolayzer bölümü (Ayarlar → "Ekolayzer", Ses'in hemen altında).
//!
//! - Aç / kapa, 10 dikey sürgü (31,5 Hz … 16 kHz, ±12 dB, 0,5 dB adım) ve ön
//!   kazanç sürgüsü: fareyle sürüklenir, çift tıkla sıfırlanır, klavyede
//!   Tab durağıdır (↑ ↓ 0,5 dB, PgUp / PgDn 3 dB, ← → komşu sürgü, Delete /
//!   Backspace / 0 sıfırlar). Her değişiklik anında motora gider
//!   (`Command::SetEq`, motor 8 blokta süzer: tıkırtı yok); diske sürükleme
//!   bitince / tuşta yazılır. Herhangi bir değişiklik ekolayzeri açar:
//!   duyulmayan bir ayar bırakmamak için.
//! - Hazır ayarlar (Düz, Bas artır, Tiz artır, Vokal, Akustik) ve "Sıfırla".
//! - Kulaklık profili (OPRA, CC BY-SA 4.0): arama kutusuna ilk dokunuşta veri
//!   indirilir (bkz. `engine::opra`; önbellekten hemen, ETag ile arkadan
//!   güncellenir). Seçilen profil 10 banda uydurulur (`engine::equalizer::fit`);
//!   profil seçiliyken "Ölçüm: <yaratıcı> · OPRA (CC BY-SA 4.0)" satırı gösterilir.
//!   Sürgüyü elle oynatınca profil bırakılır (artık o profil değil).
//!   Bellekteki katalog (~10 MB) Ayarlar'dan çıkınca atılır.

use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{
    AnyElement, Bounds, ClickEvent, Context, ElementId, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, Point, SharedString, Window, canvas, div, prelude::*, px, rgb,
};

use super::{
    Drag, Shell,
    components::{Look, ghost_button, secondary_button},
    history::Route,
    settings::{row_text, setting_row},
    text::line_with_tip,
    theme::*,
};
use crate::{
    engine::{
        Command,
        equalizer::{self, BANDS, PREAMP_RANGE_DB, PRESETS, RANGE_DB, STEP_DB},
        opra::{self, Catalog, Hit},
    },
    settings::{EqProfile, Settings},
    t, tf,
};

/// Sürgünün tıklanabilir yüksekliği ve tutamaç çapı (px).
const TRACK_H: f32 = 148.;
const THUMB: f32 = 14.;
/// Sürgü sütununun tıklanabilir eni.
const HIT_W: f32 = 28.;
/// Aramada gösterilen en çok satır.
const RESULT_ROWS: usize = 6;
/// Arama kutusu eni.
const SEARCH_W: f32 = 240.;
/// PgUp / PgDn adımı (dB).
const PAGE_STEP_DB: f32 = 3.0;
/// Sürgü dizisinde ön kazancın sırası (bantlardan sonra).
const PRE: usize = BANDS;
/// Bant etiketleri (Hz; "k" = bin).
const LABELS: [&str; BANDS] = [
    "32", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];

/// Profil verisinin durumu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Load {
    Idle,
    Loading,
    Failed,
}

/// Ekolayzer bölümünün ekran durumu (`SettingsUi::eq`).
pub struct EqUi {
    /// Sürgülerin ekrandaki yerleri (fare konumunu değere çevirmek için);
    /// sonuncusu ön kazanç.
    bounds: Vec<Rc<Cell<Option<Bounds<Pixels>>>>>,
    catalog: Option<Arc<Catalog>>,
    load: Load,
    /// Her Ayarlar girişinde artar: eski iş parçacığının geç gelen yanıtı yok sayılır.
    epoch: u64,
    query: String,
    hits: Vec<Hit>,
    cursor: usize,
    /// Sınama kancası (`OPENIFY_TEST_EQ`): katalog gelince seçilecek satır.
    test_pick: Option<usize>,
}

enum Msg {
    Ready(u64, Arc<Catalog>),
    Failed(u64),
}

impl EqUi {
    pub fn new() -> Self {
        Self {
            bounds: (0..=BANDS).map(|_| Rc::new(Cell::new(None))).collect(),
            catalog: None,
            load: Load::Idle,
            epoch: 0,
            query: String::new(),
            hits: Vec::new(),
            cursor: 0,
            test_pick: None,
        }
    }

    /// Ayarlar'dan çıkınca: katalog ve arama atılır, geç gelen yanıt yok sayılır.
    pub fn leave(&mut self) {
        self.catalog = None;
        self.load = Load::Idle;
        self.epoch += 1;
        self.query.clear();
        self.hits.clear();
        self.cursor = 0;
    }

    fn refresh_hits(&mut self) {
        self.hits = match &self.catalog {
            Some(c) => c.search(&self.query, RESULT_ROWS),
            None => Vec::new(),
        };
        self.cursor = 0;
    }
}

/// dB okunur biçimde: "+3,5", "−2", "0" (Türkçede ondalık virgül).
pub fn format_db(v: f32) -> String {
    let v = (v * 10.0).round() / 10.0;
    if v == 0.0 {
        return "0".into();
    }
    let sign = if v > 0.0 { '+' } else { '−' };
    let a = v.abs();
    let body = if (a - a.round()).abs() < 0.001 {
        format!("{a:.0}")
    } else {
        format!("{a:.1}")
    };
    crate::i18n::localize_decimal(&format!("{sign}{body}"))
}

/// Sürgü konumundan (y) dB: üst uç +12, alt uç −12, 0,5 dB'e oturur.
fn value_at(b: Bounds<Pixels>, y: Pixels) -> f32 {
    let travel = (f32::from(b.size.height) - THUMB).max(1.);
    let frac = ((f32::from(y - b.origin.y) - THUMB / 2.) / travel).clamp(0., 1.);
    equalizer::snap_db(RANGE_DB - frac * 2. * RANGE_DB)
}

// ── Eylemler ────────────────────────────────────────────────────────────

impl Shell {
    /// Motora güncel ekolayzer ayarını yollar (beklemez; motor tutar ve çıkışa verir).
    pub(super) fn eq_send(&self) {
        let s = self.prefs.eq_setting();
        self.engine.send(Command::SetEq {
            enabled: s.enabled,
            gains_db: s.gains_db,
            preamp_db: s.preamp_db,
        });
    }

    /// Ayarlar'dan çıkıldı (bkz. `pages::navigate`).
    pub(super) fn eq_leave(&mut self) {
        self.sui.eq.leave();
    }

    /// Ayarlar'a girilince: sınama kancası (`OPENIFY_TEST_EQ="search=hd 6;pick=0"`,
    /// yalnız sınama koşusunda) aramayı kendiliğinden açar.
    pub(super) fn eq_enter(&mut self, cx: &mut Context<Self>) {
        if !crate::crash::is_test_run() {
            return;
        }
        let Ok(spec) = std::env::var("OPENIFY_TEST_EQ") else {
            return;
        };
        for part in spec.split(';') {
            match part.split_once('=') {
                Some(("search", q)) => {
                    self.sui.eq.query = q.to_string();
                    self.eq_ensure_catalog(cx);
                }
                Some(("pick", n)) => self.sui.eq.test_pick = n.trim().parse().ok(),
                _ => {}
            }
        }
    }

    /// Bir sürgünün değerini (0..10 bant, 10 ön kazanç) ayarlar. Değer
    /// değiştiyse ekolayzer açılır, bant ise profil bırakılır; motora gider.
    /// Diske yazmak çağıranın işi (sürüklemede bırakınca).
    fn eq_set(&mut self, i: usize, v: f32, cx: &mut Context<Self>) -> bool {
        let v = equalizer::snap_db(v);
        let p = &mut self.prefs;
        p.eq_gains.resize(BANDS, 0.0);
        let old = if i < BANDS {
            p.eq_gains[i]
        } else {
            p.eq_preamp
        };
        if (old - v).abs() < 0.001 {
            return false;
        }
        if i < BANDS {
            p.eq_gains[i] = v;
            p.eq_profile = None;
        } else {
            p.eq_preamp = v.clamp(-PREAMP_RANGE_DB, PREAMP_RANGE_DB);
        }
        p.eq_enabled = true;
        self.eq_send();
        cx.notify();
        true
    }

    /// Sürükleme: fare konumuna göre değer.
    pub(super) fn eq_drag_to(&mut self, i: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(b) = self.sui.eq.bounds.get(i).and_then(|c| c.get()) else {
            return;
        };
        let v = value_at(b, position.y);
        self.eq_set(i, v, cx);
    }

    /// Sürükleme bitti: ayar diske yazılır.
    pub(super) fn eq_commit(&mut self) {
        self.save_settings();
    }

    /// Klavye ve yazı: odaktaki sürgü ya da arama kutusu. `true`: tuş işlendi.
    pub(super) fn eq_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(self.pages.history.current(), Route::Settings) {
            return false;
        }
        let Some((focus, _)) = self.nav.extra_focused(window) else {
            return false;
        };
        let ks = &event.keystroke;
        if ks.modifiers.alt || ks.modifiers.platform {
            return false;
        }
        let key = ks.key.as_str();
        if focus == "eq:search" {
            return self.eq_search_key(event, cx);
        }
        // Ctrl'li tuşlar (ses, sıra) sürgüde de genel kısayollara gider.
        if ks.modifiers.control {
            return false;
        }
        let slot = if focus == "eq:pre" {
            PRE
        } else if let Some(i) = focus
            .strip_prefix("eq:band:")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|i| *i < BANDS)
        {
            i
        } else {
            return false;
        };
        let now = if slot < BANDS {
            self.prefs.eq_gains.get(slot).copied().unwrap_or(0.0)
        } else {
            self.prefs.eq_preamp
        };
        let target = match key {
            "up" => now + STEP_DB,
            "down" => now - STEP_DB,
            "pageup" => now + PAGE_STEP_DB,
            "pagedown" => now - PAGE_STEP_DB,
            "delete" | "backspace" | "0" => 0.0,
            "left" => {
                window.focus_prev();
                cx.notify();
                return true;
            }
            "right" => {
                window.focus_next();
                cx.notify();
                return true;
            }
            _ => return false,
        };
        if self.eq_set(slot, target, cx) {
            self.eq_commit();
        }
        true
    }

    fn eq_search_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        let before = self.sui.eq.query.clone();
        if ks.modifiers.control {
            match key {
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                        let line = text.lines().next().unwrap_or_default();
                        self.sui.eq.query.push_str(line);
                    }
                }
                "backspace" => {
                    let q = &mut self.sui.eq.query;
                    let trimmed = q.trim_end().len();
                    let cut = q[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                    q.truncate(cut);
                }
                _ => return false,
            }
        } else {
            match key {
                "backspace" => {
                    self.sui.eq.query.pop();
                }
                "space" => self.sui.eq.query.push(' '),
                "escape" => {
                    if self.sui.eq.query.is_empty() {
                        return false;
                    }
                    self.sui.eq.query.clear();
                }
                "down" => {
                    self.sui.eq.cursor =
                        (self.sui.eq.cursor + 1).min(self.sui.eq.hits.len().saturating_sub(1))
                }
                "up" => self.sui.eq.cursor = self.sui.eq.cursor.saturating_sub(1),
                "enter" => {
                    if let Some(hit) = self.sui.eq.hits.get(self.sui.eq.cursor).copied() {
                        self.eq_pick(hit, cx);
                    }
                }
                "tab" => return false,
                _ => match ks
                    .key_char
                    .as_deref()
                    .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                {
                    Some(ch) => self.sui.eq.query.push_str(ch),
                    None => return false,
                },
            }
        }
        if self.sui.eq.query.chars().count() > 60 {
            self.sui.eq.query = self.sui.eq.query.chars().take(60).collect();
        }
        if self.sui.eq.query != before {
            self.eq_ensure_catalog(cx);
            self.sui.eq.refresh_hits();
        }
        cx.notify();
        true
    }

    /// Arama kutusuna ilk dokunuş: OPRA verisi önbellekten (varsa) gelir, arkadan
    /// güncellenir; yoksa indirilir. Zaten yüklüyse ya da sürüyorsa bir şey yapmaz.
    pub(super) fn eq_ensure_catalog(&mut self, cx: &mut Context<Self>) {
        let eq = &mut self.sui.eq;
        if eq.catalog.is_some() || eq.load == Load::Loading {
            return;
        }
        eq.load = Load::Loading;
        let epoch = eq.epoch;
        let (tx, rx) = async_channel::unbounded::<Msg>();
        let source = opra::Source::from_env();
        std::thread::Builder::new()
            .name("openify-opra".into())
            .spawn(move || {
                let Some(dir) = opra::cache_dir() else {
                    let _ = tx.send_blocking(Msg::Failed(epoch));
                    return;
                };
                let ready = tx.clone();
                let result = opra::load_blocking(&dir, source, move |catalog| {
                    let _ = ready.send_blocking(Msg::Ready(epoch, Arc::new(catalog)));
                });
                if let Err(e) = result {
                    log::warn!("Kulaklık profilleri alınamadı: {e:?}");
                    let _ = tx.send_blocking(Msg::Failed(epoch));
                }
            })
            .ok();
        cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                let alive = this.update(cx, |shell, cx| shell.eq_on_msg(msg, cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn eq_on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let epoch = self.sui.eq.epoch;
        match msg {
            Msg::Ready(e, catalog) if e == epoch => {
                self.sui.eq.catalog = Some(catalog);
                self.sui.eq.load = Load::Idle;
                self.sui.eq.refresh_hits();
                if let Some(n) = self.sui.eq.test_pick.take()
                    && let Some(hit) = self.sui.eq.hits.get(n).copied()
                {
                    self.eq_pick(hit, cx);
                }
            }
            Msg::Failed(e) if e == epoch && self.sui.eq.catalog.is_none() => {
                self.sui.eq.load = Load::Failed
            }
            _ => {}
        }
        cx.notify();
    }

    /// Bir profili seç: 10 banda uydurulur, ekolayzer açılır, ayar yazılır.
    fn eq_pick(&mut self, hit: Hit, cx: &mut Context<Self>) {
        let Some(catalog) = self.sui.eq.catalog.clone() else {
            return;
        };
        let Some(product) = catalog.products.get(hit.product) else {
            return;
        };
        let Some(profile) = product.profiles.get(hit.profile) else {
            return;
        };
        let fit = equalizer::fit(&profile.bands);
        self.prefs.eq_gains = fit.gains_db.to_vec();
        self.prefs.eq_enabled = true;
        self.prefs.eq_profile = Some(EqProfile {
            id: profile.id.clone(),
            name: product.title(),
            author: profile.author.clone(),
            details: profile.details.clone(),
            deviation_db: fit.rms_db,
        });
        self.sui.eq.query.clear();
        self.sui.eq.hits.clear();
        self.eq_send();
        self.save_settings();
        cx.notify();
    }

    fn eq_clear_profile(&mut self, cx: &mut Context<Self>) {
        self.prefs.eq_profile = None;
        self.save_settings();
        cx.notify();
    }

    fn eq_reset(&mut self, cx: &mut Context<Self>) {
        self.prefs.eq_gains = vec![0.0; BANDS];
        self.prefs.eq_preamp = 0.0;
        self.prefs.eq_profile = None;
        self.eq_send();
        self.save_settings();
        cx.notify();
    }
}

// ── Görünüm ─────────────────────────────────────────────────────────────

impl Shell {
    /// Bir dikey sürgü sütunu: değer yazısı, sürgü, etiket. `slot`: 0..10 bant, 10 ön kazanç.
    fn eq_slider(
        &self,
        slot: usize,
        value: f32,
        label: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let key = if slot == PRE {
            "eq:pre".to_string()
        } else {
            format!("eq:band:{slot}")
        };
        let (stop, look) = self.stop_look(key, super::tab::PAGE, true, window, cx);
        let focused = look == Look::Focus;
        let on = self.prefs.eq_enabled;
        let bounds = self.sui.eq.bounds[slot].clone();
        let focus_handle = stop.clone();

        let travel = TRACK_H - THUMB;
        let center = THUMB / 2. + travel / 2.;
        let thumb_y = THUMB / 2. + travel * (1. - (value + RANGE_DB) / (2. * RANGE_DB));
        let (fill_top, fill_h) = if value >= 0. {
            (thumb_y, center - thumb_y)
        } else {
            (center, thumb_y - center)
        };
        let fill_color = if on {
            rgb(accent)
        } else {
            rgb(mix(palette().text, palette().bg, 0.62))
        };
        let value_color = if focused {
            rgb(accent)
        } else if value == 0. {
            label_color()
        } else {
            text()
        };

        let track = div()
            .id(ElementId::Name(format!("eq-slider-{slot}").into()))
            .relative()
            .w(px(HIT_W))
            .h(px(TRACK_H))
            .flex_none()
            .cursor_pointer()
            .when_some(stop.clone(), |el, h| {
                el.child(self.focus_probe(&h)).track_focus(&h)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    if let Some(h) = &focus_handle {
                        window.focus(h);
                    }
                    if ev.click_count >= 2 {
                        if this.eq_set(slot, 0., cx) {
                            this.eq_commit();
                        }
                        return;
                    }
                    this.dragging = Some(Drag::Eq(slot));
                    this.eq_drag_to(slot, ev.position, cx);
                }),
            )
            .child(
                canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            // Oluk.
            .child(
                div()
                    .absolute()
                    .left(px((HIT_W - 4.) / 2.))
                    .top(px(THUMB / 2.))
                    .w(px(4.))
                    .h(px(travel))
                    .rounded(px(R_XS))
                    .bg(rgb(palette().track)),
            )
            // Sıfır çizgisi.
            .child(
                div()
                    .absolute()
                    .left(px((HIT_W - 12.) / 2.))
                    .top(px(center))
                    .w(px(12.))
                    .h(px(1.))
                    .bg(veil(LINE_HOVER)),
            )
            // Sıfırdan değere dolgu.
            .child(
                div()
                    .absolute()
                    .left(px((HIT_W - 4.) / 2.))
                    .top(px(fill_top))
                    .w(px(4.))
                    .h(px(fill_h.max(0.)))
                    .rounded(px(R_XS))
                    .bg(fill_color),
            )
            // Tutamaç.
            .child(
                div()
                    .absolute()
                    .left(px((HIT_W - THUMB) / 2.))
                    .top(px(thumb_y - THUMB / 2.))
                    .size(px(THUMB))
                    .rounded_full()
                    .bg(rgb(0xFFFFFF))
                    .shadow(pill_shadow())
                    .when(focused, |el| el.border_2().border_color(rgb(accent))),
            );

        div()
            .flex_1()
            .min_w(px(HIT_W))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(S1))
            .child(
                div()
                    .h(px(S4))
                    .type_role(T_LABEL)
                    .tabular()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(value_color)
                    .child(format_db(value)),
            )
            .child(track)
            .child(
                div()
                    .type_role(T_LABEL)
                    .tabular()
                    .text_color(muted())
                    .child(label),
            )
            .into_any_element()
    }

    /// Sürgüler: ön kazanç solda, ince bir çizgiyle bantlardan ayrı.
    fn eq_sliders(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let gains = {
            let mut g = self.prefs.eq_gains.clone();
            g.resize(BANDS, 0.0);
            g
        };
        let mut row = div()
            .flex()
            .items_start()
            .gap(px(S1))
            .opacity(if self.prefs.eq_enabled { 1. } else { 0.62 });
        row = row.child(div().w(px(S8)).flex_none().flex().child(self.eq_slider(
            PRE,
            self.prefs.eq_preamp,
            t!(EqPreamp),
            window,
            cx,
        )));
        row = row.child(
            div()
                .w(px(1.))
                .h(px(TRACK_H))
                .mt(px(S5))
                .flex_none()
                .bg(veil(LINE))
                .mx(px(S1)),
        );
        for (i, gain) in gains.iter().enumerate() {
            row = row.child(self.eq_slider(i, *gain, LABELS[i], window, cx));
        }
        div()
            .py(px(S3))
            .border_t_1()
            .border_color(veil(LINE))
            .child(row)
            .into_any_element()
    }

    /// Arama kutusu: Tab durağı, yazı `eq_key`'te.
    fn eq_search_field(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (stop, look) = self.stop_look("eq:search", super::tab::PAGE, true, window, cx);
        let focused = look == Look::Focus;
        let query = self.sui.eq.query.clone();
        let empty = query.is_empty();
        div()
            .id("eq-search")
            .relative()
            .w(px(SEARCH_W))
            .max_w_full()
            .h(px(32.))
            .px(px(S3))
            .rounded(px(R_LG))
            .flex()
            .items_center()
            .gap(px(S2))
            .bg(shade(SHADE_INPUT))
            .border_1()
            .border_color(if focused {
                rgb(accent)
            } else {
                veil(LINE_INPUT)
            })
            .type_role(T_SECONDARY)
            .text_color(text())
            .when_some(stop, |el, h| {
                let click = h.clone();
                el.child(self.focus_probe(&h))
                    .track_focus(&h)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&click);
                        this.keyboard_focus = true;
                        this.eq_ensure_catalog(cx);
                        cx.notify();
                    }))
            })
            .child(icon("search", 14., muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .when(empty, |el| {
                        el.text_color(label_color()).child(t!(EqSearchPlaceholder))
                    })
                    .when(!empty, |el| {
                        el.child(line_with_tip("eq-search-text", query).flex().min_w_0())
                            .child(div().when(focused, |el| {
                                el.ml(px(S0)).w(px(S0)).h(px(S4)).bg(rgb(accent))
                            }))
                    }),
            )
            .into_any_element()
    }

    /// Arama sonuçları ya da durum yazısı (yükleniyor / hata / sonuç yok).
    fn eq_results(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let eq = &self.sui.eq;
        let accent = self.accent().text;
        let note = |text: SharedString| {
            div()
                .pt(px(S2))
                .type_role(T_SECONDARY)
                .text_color(label_color())
                .child(text)
                .into_any_element()
        };
        if eq.load == Load::Failed {
            let (stop, look) = self.stop_look("eq:retry", super::tab::PAGE, true, window, cx);
            let retry = secondary_button("eq-retry", "refresh-cw", t!(EqRetry), look, accent)
                .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                    this.leave_button(ev, window);
                    this.sui.eq.load = Load::Idle;
                    this.eq_ensure_catalog(cx);
                    cx.notify();
                }));
            return Some(
                div()
                    .pt(px(S2))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .flex_wrap()
                    .child(
                        div()
                            .type_role(T_SECONDARY)
                            .text_color(label_color())
                            .child(t!(EqSearchFailed)),
                    )
                    .child(retry)
                    .into_any_element(),
            );
        }
        if eq.query.trim().is_empty() {
            return None;
        }
        let Some(catalog) = &eq.catalog else {
            return Some(note(t!(EqSearchLoading).into()));
        };
        if eq.hits.is_empty() {
            return Some(note(tf!(EqSearchNone, eq.query.trim()).into()));
        }
        let search_focused = self
            .nav
            .extra_focused(window)
            .is_some_and(|(k, _)| k == "eq:search");
        let mut list = div()
            .mt(px(S2))
            .p(px(S1))
            .rounded(px(R_LG))
            .bg(shade(SHADE_INPUT))
            .border_1()
            .border_color(veil(LINE_INPUT))
            .flex()
            .flex_col();
        for (i, hit) in eq.hits.iter().enumerate() {
            let Some(product) = catalog.products.get(hit.product) else {
                continue;
            };
            let Some(profile) = product.profiles.get(hit.profile) else {
                continue;
            };
            let sub = [profile.author.as_str(), profile.details.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" · ");
            let hit = *hit;
            let selected = search_focused && i == eq.cursor;
            list = list.child(
                div()
                    .id(ElementId::Name(format!("eq-hit-{i}").into()))
                    .relative()
                    .min_h(px(S7))
                    .px(px(S3))
                    .py(px(S1))
                    .rounded(px(R_MD))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(veil(VEIL_HOVER)))
                    .child(super::tween::hover_fill(
                        veil(VEIL_HOVER),
                        veil(VEIL_PRESSED),
                        0.,
                        R_MD,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.eq_pick(hit, cx)))
                    .child(
                        div()
                            .flex()
                            .type_role(T_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text())
                            .child(
                                line_with_tip(("eq-hit-title", i), product.title())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .type_role(T_SECONDARY)
                            .text_color(muted())
                            .child(line_with_tip(("eq-hit-sub", i), sub).flex().min_w_0()),
                    ),
            );
        }
        let mut out = div().flex().flex_col().child(list);
        if eq.hits.len() >= RESULT_ROWS {
            out = out.child(note(tf!(EqSearchMore, RESULT_ROWS).into()));
        }
        Some(out.into_any_element())
    }

    /// Seçili profil: ad, atıf satırı (CC BY-SA), yaklaştırma sapması, kaldır.
    fn eq_selected(
        &self,
        profile: &EqProfile,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let (stop, look) = self.stop_look("eq:profile-clear", super::tab::PAGE, true, window, cx);
        let remove = ghost_button(
            "eq-profile-clear",
            "x",
            t!(EqProfileRemove),
            false,
            look,
            accent,
        )
        .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
        .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
            this.leave_button(ev, window);
            this.eq_clear_profile(cx)
        }));
        let credit = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(S1))
            .type_role(T_SECONDARY)
            .text_color(muted())
            .child(tf!(EqProfileMeasurement, profile.author))
            .child("·")
            .child(
                div()
                    .id("eq-opra-link")
                    .cursor_pointer()
                    .text_color(soft())
                    .hover(|s| s.text_color(text()))
                    .child("OPRA (CC BY-SA 4.0)")
                    .on_click(cx.listener(|_, _, _, cx| {
                        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_none() {
                            cx.open_url(opra::HOME_URL);
                        }
                    })),
            );
        let details = (!profile.details.is_empty()).then(|| {
            div()
                .flex()
                .type_role(T_SECONDARY)
                .text_color(label_color())
                .child(
                    line_with_tip("eq-profile-details", profile.details.clone())
                        .flex()
                        .min_w_0(),
                )
        });
        let deviation = (profile.deviation_db > 0.0).then(|| {
            div()
                .type_role(T_SECONDARY)
                .text_color(label_color())
                .child(tf!(
                    EqProfileFit,
                    crate::i18n::localize_decimal(&format!("{:.1}", profile.deviation_db))
                ))
        });
        div()
            .mt(px(S3))
            .p(px(S3))
            .rounded(px(R_LG))
            .bg(shade(SHADE_INPUT))
            .border_1()
            .border_color(veil(LINE_INPUT))
            .flex()
            .items_start()
            .gap(px(S3))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S0))
                    .child(
                        div()
                            .flex()
                            .type_role(T_BODY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(text())
                            .child(
                                line_with_tip("eq-profile-name", profile.name.clone())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(credit)
                    .children(details)
                    .children(deviation),
            )
            .child(remove)
            .into_any_element()
    }

    /// "Ekolayzer" bölümü.
    pub(super) fn eq_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let p = &self.prefs;
        let on = self.switch_control(
            "eq-enabled",
            p.eq_enabled,
            |p, v| p.eq_enabled = v,
            window,
            cx,
        );

        // Hazır ayarlar + Sıfırla.
        let current = if p.eq_profile.is_some() {
            ""
        } else {
            equalizer::preset_of(&p.eq_setting().gains_db).unwrap_or("")
        };
        let presets = self.choice_control(
            "eq-preset",
            &[
                ("flat", t!(EqPresetFlat)),
                ("bass", t!(EqPresetBass)),
                ("treble", t!(EqPresetTreble)),
                ("vocal", t!(EqPresetVocal)),
                ("acoustic", t!(EqPresetAcoustic)),
            ],
            current,
            |p: &mut Settings, k| {
                if let Some((_, g)) = PRESETS.iter().find(|(key, _)| *key == k) {
                    p.eq_gains = g.to_vec();
                    p.eq_enabled = true;
                    p.eq_profile = None;
                }
            },
            window,
            cx,
        );
        let (reset_stop, reset_look) =
            self.stop_look("eq:reset", super::tab::PAGE, true, window, cx);
        let reset = secondary_button("eq-reset", "refresh-cw", t!(EqReset), reset_look, accent)
            .when_some(reset_stop, |el, h| {
                el.child(self.focus_probe(&h)).track_focus(&h)
            })
            .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                this.leave_button(ev, window);
                this.eq_reset(cx)
            }));
        let preset_row = div()
            .py(px(S3))
            .border_t_1()
            .border_color(veil(LINE))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(S3))
            .flex_wrap()
            .child(presets)
            .child(reset)
            .into_any_element();

        // Kulaklık profili: açıklama + arama; altında sonuçlar ve seçili profil.
        let mut profile_block = div()
            .py(px(S3))
            .border_t_1()
            .border_color(veil(LINE))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S5))
                    .flex_wrap()
                    .child(
                        row_text(t!(EqHeadphoneTitle), t!(EqHeadphoneDesc), None)
                            .min_w(px(S8 * 3.)),
                    )
                    .child(self.eq_search_field(window, cx)),
            );
        if let Some(results) = self.eq_results(window, cx) {
            profile_block = profile_block.child(results);
        }
        if let Some(profile) = &p.eq_profile {
            profile_block = profile_block.child(self.eq_selected(profile, window, cx));
        }
        profile_block = profile_block.child(
            div()
                .pt(px(S3))
                .type_role(T_LABEL)
                .text_color(label_color())
                .child(t!(EqOpraBlurb)),
        );

        self.settings_section(
            t!(SectionEqualizer),
            vec![
                setting_row(true, row_text(t!(EqEnable), t!(EqEnableDesc), None), on),
                self.eq_sliders(window, cx),
                preset_row,
                profile_block.into_any_element(),
            ],
        )
    }
}

fn label_color() -> gpui::Rgba {
    label()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_reads_naturally() {
        crate::i18n::with_lang(crate::i18n::Lang::En, || {
            assert_eq!(format_db(0.0), "0");
            assert_eq!(format_db(3.5), "+3.5");
            assert_eq!(format_db(-2.0), "−2");
            assert_eq!(format_db(12.0), "+12");
            assert_eq!(format_db(-0.04), "0", "yuvarlanınca sıfır");
        });
        crate::i18n::with_lang(crate::i18n::Lang::Tr, || {
            assert_eq!(format_db(-6.5), "−6,5");
            assert_eq!(format_db(0.5), "+0,5");
        });
    }

    /// Fare konumu → dB: üst uç +12, alt uç −12, orta 0; 0,5 dB'e oturur;
    /// sınır dışı sınırlanır.
    #[test]
    fn slider_position_maps_to_a_snapped_value() {
        use gpui::{point, size};
        let b = Bounds {
            origin: point(px(0.), px(100.)),
            size: size(px(HIT_W), px(TRACK_H)),
        };
        let at = |y: f32| value_at(b, px(y));
        assert_eq!(at(100. + THUMB / 2.), RANGE_DB);
        assert_eq!(at(100. + TRACK_H - THUMB / 2.), -RANGE_DB);
        assert_eq!(at(100. + TRACK_H / 2.), 0.0);
        assert_eq!(at(0.), RANGE_DB, "üstün dışı");
        assert_eq!(at(1000.), -RANGE_DB, "altın dışı");
        for y in 100..250 {
            let v = at(y as f32);
            assert!(
                ((v / STEP_DB).round() * STEP_DB - v).abs() < 1e-6,
                "{y}: {v}"
            );
        }
    }

    #[test]
    fn labels_cover_every_band() {
        assert_eq!(LABELS.len(), BANDS);
    }
}
