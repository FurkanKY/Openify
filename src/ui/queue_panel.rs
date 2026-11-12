//! Sağdaki yüzen panel (Adım 11 / 14): sıra ve sözler.
//!
//! Panel içeriğin üstünde yüzer (çubuk yüzeyi, kenar, 12 px köşe, yüzen
//! gölge; yerleşikken daha sakin gölge, bkz. `render_side_panel`), başlık
//! çubuğunun altından hapın üstüne kadar uzanır; geniş
//! pencerede hap ile yan yana sığarsa pencerenin altına kadar. Açılınca
//! sağdan kayarak gelir (220 ms, en çok 60 kare/sn).
//!
//! Sıra: "Şimdi çalıyor / Sıradaki / Sıradaki: <liste>" bölümleri, 56 px
//! satırlar. Satır sürüklenip bırakılabilir: bırakılacak yer kapak ışığı
//! renginde bir çizgiyle önceden görünür, listenin kenarına gelince liste
//! kendiliğinden kayar. Değişiklik ekrana motorun cevabını beklemeden, aynı
//! karede yansır (`now_playing::apply_move`). Klavye: ↑ / ↓ seç, Alt+↑ / ↓
//! taşı, Del çıkar, Enter çal. Tek tık seçer, çift tık çalar (listeyle aynı).

use std::time::Duration;

use gpui::{
    AnyElement, AnyView, ClickEvent, Context, DragMoveEvent, FontWeight, ListSizingBehavior,
    StyleRefinement, Window, div, prelude::*, px, rgb, uniform_list,
};

use super::{
    Shell, TITLEBAR_HEIGHT,
    components::{Look, Tooltip, chip, icon_button, state_view},
    cover_image,
    library::ROW_HEIGHT,
    now_playing::{ANIM_FPS, Panel, drop_target},
    pill::{PILL_BOTTOM, PILL_CLEARANCE, pill_width},
    player::fmt_time,
    text::line_with_tip,
    theme::*,
};
use crate::engine::QueueEntry;
use crate::{t, tf};

/// Panelin genişliği.
pub(super) const PANEL_WIDTH: f32 = 360.;
/// Satırdaki küçük kapak.
const THUMB: f32 = 40.;
/// Sürüklerken listenin kenarına bu kadar yaklaşınca kendiliğinden kayar;
/// en hızlı hâli (px / kare, 60 kare/sn).
const EDGE: f32 = S7;
const AUTOSCROLL_MAX: f32 = 14.;

/// Sıra listesinin bir öğesi (hepsi 56 px: `uniform_list`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum QItem {
    NowHead,
    Current,
    UserHead,
    ContextHead,
    /// Görünür sıradaki yeri.
    Row(usize),
}

/// Sürüklenen sıra satırı.
#[derive(Clone)]
pub(super) struct QueueDrag {
    pub from: usize,
    pub name: String,
    pub artists: String,
}

/// Sürüklenen satırın imleçteki görüntüsü: yüzen bir satır.
pub(super) struct DragGhost {
    name: String,
    artists: String,
}

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // İmlecin sağ altında: bırakılacak yerin çizgisi altında kalmasın.
        div()
            .pl(px(S6))
            .pt(px(S5))
            .child(drag_ghost(&self.name, &self.artists))
    }
}

/// Sürüklenen satırın görüntüsü (vitrinde de).
pub(super) fn drag_ghost(name: &str, artists: &str) -> gpui::Div {
    div()
        .font_family(UI_FONT)
        .w(px(PANEL_WIDTH - S6))
        .h(px(ROW_HEIGHT))
        .px(px(S3))
        .flex()
        .items_center()
        .gap(px(S3))
        .rounded(px(R_MD))
        .bg(rgb(BAR))
        .border_1()
        .border_color(rgb(BORDER))
        .shadow(float_shadow())
        .child(icon("grip-vertical", 16., label()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .type_role(T_BODY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text())
                        .child(super::text::line(name.to_string())),
                )
                .child(
                    div()
                        .type_role(T_SECONDARY)
                        .text_color(muted())
                        .child(super::text::line(artists.to_string())),
                ),
        )
}

impl Shell {
    /// Panelin penceredeki yeri: başlık çubuğunun altında, sağda. Hap ile
    /// yatayda çakışıyorsa hapın üstünde biter (gölgeler üst üste binmesin
    /// diye arada 24 px).
    pub(super) fn side_panel_frame(&self, viewport_w: f32, viewport_h: f32) -> (f32, f32) {
        let top = TITLEBAR_HEIGHT + S2;
        let pill_right = (viewport_w + pill_width(viewport_w)) / 2.;
        let (width, right) = self.now.dock.panel_box;
        let panel_left = viewport_w - right - width;
        // Yerleşik panelin yanında hap hiç ona yetişmez (gerekirse daralır,
        // bkz. `DockLayout::pill_frame`): panel hep pencerenin altına (24 px)
        // kadar iner.
        // Arkadaş paneli (yerleşikken de) hapın üstünde biter ve hapı daraltmaz.
        let friends = self.now.shown_panel() == Some(Panel::Friends);
        let beside = !friends && (self.now.dock.visual_docked || pill_right + S5 <= panel_left);
        let bottom = if beside {
            PILL_BOTTOM
        } else {
            PILL_CLEARANCE + S5
        };
        let room = (viewport_h - top - bottom).max(ROW_HEIGHT * 2.);
        // Arkadaş paneli içerik kadar uzun (uzunsa kendi kaydırması): altında
        // büyük boş kart yok.
        (
            top,
            if friends {
                self.friends_panel_height(room)
            } else {
                room
            },
        )
    }

    /// Kök için: panel açıksa yerini tutan önbellekli görünüm.
    pub(super) fn side_panel_layer(&self, window: &Window) -> Option<AnyElement> {
        self.now.shown_panel()?;
        let viewport = window.viewport_size();
        let (top, height) =
            self.side_panel_frame(f32::from(viewport.width), f32::from(viewport.height));
        // Yerleşik panel 400 px, yüzen panel 360; ikisi de sağda 16 px
        // içeride (bkz. `now_panel::dock_layout`).
        let (width, right) = self.now.dock.panel_box;
        Some(
            AnyView::from(self.views.side.clone())
                .cached(
                    StyleRefinement::default()
                        .absolute()
                        .top(px(top))
                        .right(px(right))
                        .w(px(width))
                        .h(px(height)),
                )
                .into_any_element(),
        )
    }

    /// Panelin kendisi (`SideView`). Hareket sürüyorsa `true`.
    pub(super) fn render_side_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        let Some(panel) = self.now.shown_panel() else {
            return (div().into_any_element(), false);
        };
        let accent = self.accent().text;
        // Açılma / kapanma: yerleşik panelde konum `dock_layout_at`ten gelir (içeriği
        // iter), yüzen panelde sağdan 24 px ve saydamdan; ilerleme kökte
        // ilerler (bkz. `now_panel::Slide`), kareyi de kök ister.
        let e = self.now.dock.slide.value;
        let mut animating = false;

        let tab = |id: &'static str, label: &'static str, which: Panel, cx: &mut Context<Self>| {
            let (stop, look) = self.stop_look(id, super::tab::PANEL, true, window, cx);
            chip(id, label, panel == which, look, accent)
                .when_some(stop, |el, h| el.track_focus(&h))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.now.panel != Some(which) {
                        this.toggle_panel(which, cx);
                    }
                }))
        };
        let (full_stop, full_look) =
            self.stop_look("panel-full", super::tab::PANEL, true, window, cx);
        let (close_stop, close_look) =
            self.stop_look("panel-close", super::tab::PANEL, true, window, cx);
        let header = div()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .pl(px(S4))
            .pr(px(S2))
            .flex()
            .items_center()
            .gap(px(S2))
            .child(tab("tab-now", t!(NowTab), Panel::Now, cx))
            .child(tab("tab-queue", t!(QueueTab), Panel::Queue, cx))
            .child(tab("tab-lyrics", t!(Lyrics), Panel::Lyrics, cx))
            .child(div().flex_1())
            .child(
                icon_button("panel-full", "maximize-2", full_look, accent)
                    .when_some(full_stop, |el, h| el.track_focus(&h))
                    .tooltip(Tooltip::text(t!(FullScreen), Some("F11")))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx)),
                    ),
            )
            .child(
                // Yerleşik panel Esc ile kapanmaz (bkz. `now_key`).
                icon_button("panel-close", "x", close_look, accent)
                    .when_some(close_stop, |el, h| el.track_focus(&h))
                    .tooltip(Tooltip::text(
                        t!(Close),
                        (!self.now.docked()).then_some("Esc"),
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_panel(cx);
                        window.focus(&this.focus);
                    })),
            );

        let body = match panel {
            Panel::Now => {
                // Büyük kapak belirirken (kapak geçişi) kare istenir.
                animating |= self.hires_fading();
                self.render_now_body(window, cx)
            }
            Panel::Queue => self.render_queue_body(window, cx),
            Panel::Friends => self.render_friends_body(window, cx),
            Panel::Lyrics => {
                let (body, moving) =
                    self.render_lyrics(super::lyrics_view::Place::Panel, window, cx);
                animating |= moving;
                body
            }
        };
        // Sekme değişince yeni içerik belirir (140 ms, soluk bir tabandan: arada
        // boş kare yok). Panel zaten kayıp açılırken sekme geçişi sayılmaz; çıkış
        // hareketinde yeniden başlamaz (kimlik sekmeye bağlı, o sırada aynı).
        let body = if animations_enabled() {
            use gpui::AnimationExt;
            div()
                .size_full()
                .child(body)
                .with_animation(
                    ("panel-body", panel as usize),
                    gpui::Animation::new(motion_fast()).with_easing(gpui::ease_out_quint()),
                    |el, d| el.opacity(0.3 + 0.7 * d),
                )
                .into_any_element()
        } else {
            body
        };

        let element = div()
            .id("side-panel")
            .size_full()
            .flex()
            .flex_col()
            .rounded(px(R_XL))
            .bg(rgb(BAR))
            .border_1()
            .border_color(rgb(BORDER))
            // Yüzen panel içeriğin üstünde: yüzen gölge. Yerleşik panel
            // listenin yanında, aynı düzlemde: daha sakin gölge (bkz.
            // `dock_shadow`). (Eski "iki kenarda şeffaf delik" ekran görüntüsü
            // betiğinin GDI+ yolundandı, ekranda yoktu; 7 Kasım.)
            .shadow(if self.now.dock.visual_docked {
                dock_shadow()
            } else {
                float_shadow()
            })
            .overflow_hidden()
            .opacity(e)
            .ml(px(if self.now.dock.visual_docked {
                0.
            } else {
                S5 * (1. - e)
            }))
            .track_focus(&self.now.focus)
            // Panel altındaki içeriği örter; tıklayınca klavye paneldedir
            // (kök odağı görünmez öğeye geri vermesin).
            .occlude()
            .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.now.focus);
                this.keyboard_focus = false;
                cx.stop_propagation();
            }))
            .child(if panel == Panel::Friends {
                self.friends_header(window, cx)
            } else {
                header.into_any_element()
            })
            .child(div().flex_1().min_h_0().relative().child(body))
            .into_any_element();
        (element, animating)
    }

    /// Sıra listesinin öğeleri (hepsi aynı boyda).
    pub(super) fn queue_items(&self) -> Vec<QItem> {
        let q = &self.now.queue;
        let mut items = Vec::new();
        if q.current.is_some() {
            items.push(QItem::NowHead);
            items.push(QItem::Current);
        }
        let visible = self.now.visible_len();
        let user = q.user_len.min(visible);
        if user > 0 {
            items.push(QItem::UserHead);
            items.extend((0..user).map(QItem::Row));
        }
        if visible > user {
            items.push(QItem::ContextHead);
            items.extend((user..visible).map(QItem::Row));
        }
        items
    }

    fn render_queue_body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let items = self.queue_items();
        if items.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px(px(S5))
                .child(state_view(
                    "list-music",
                    t!(QueueEmptyTitle),
                    t!(QueueEmptyBody),
                ))
                .into_any_element();
        }
        if !cx.has_active_drag() {
            self.now.drop_at = None;
        }
        let count = items.len();
        let list = uniform_list(
            "queue",
            count,
            cx.processor(
                move |this: &mut Shell, range: std::ops::Range<usize>, _, cx| {
                    let items = this.queue_items();
                    let rows = range
                        .filter_map(|ix| items.get(ix).map(|item| this.queue_item(ix, *item, cx)))
                        .collect::<Vec<_>>();
                    this.flush_image_requests(cx);
                    rows
                },
            ),
        )
        .track_scroll(self.now.list.clone())
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .size_full()
        .px(px(S2))
        .pb(px(S2))
        .into_any_element();

        // Bırakılacak yer: kapak ışığı renginde 2 px'lik çizgi, satırın üst
        // kenarında.
        let insertion = self.now.drop_at.and_then(|before| {
            let offset = f32::from(self.now.list.0.borrow().base_handle.offset().y);
            let ix = items
                .iter()
                .position(|i| *i == QItem::Row(before))
                .or_else(|| {
                    items
                        .iter()
                        .rposition(|i| matches!(i, QItem::Row(_)))
                        .map(|i| i + 1)
                })?;
            let y = ix as f32 * ROW_HEIGHT + offset;
            Some(
                div()
                    .absolute()
                    .left(px(S3))
                    .right(px(S3))
                    .top(px(y - S0 / 2.))
                    .h(px(S0))
                    .rounded(px(R_XS))
                    .bg(rgb(self.accent().text)),
            )
        });

        // İz: bırakmadan sıranın yeni hâlinin boyanmasına geçen süre.
        let dropped = self
            .now
            .dropped_at
            .take()
            .filter(|_| super::now_playing::trace::enabled());
        let drop_probe = dropped.map(|t| {
            gpui::canvas(
                |_, _, _| {},
                move |_, _, _, _| {
                    super::now_playing::trace::event(
                        "queue-drop",
                        t.elapsed().as_secs_f32() * 1000.,
                    )
                },
            )
            .absolute()
            .size_0()
        });
        div()
            .id("queue-body")
            .size_full()
            .relative()
            .children(drop_probe)
            .on_drag_move(cx.listener(|this, ev: &DragMoveEvent<QueueDrag>, _, cx| {
                let top = f32::from(ev.bounds.top());
                let height = f32::from(ev.bounds.size.height);
                let y = f32::from(ev.event.position.y) - top;
                this.now.drag_y = Some(y);
                // Kenarda kendiliğinden kaydırma.
                this.now.autoscroll = if y < EDGE {
                    -AUTOSCROLL_MAX * ((EDGE - y) / EDGE).clamp(0., 1.)
                } else if y > height - EDGE {
                    AUTOSCROLL_MAX * ((y - (height - EDGE)) / EDGE).clamp(0., 1.)
                } else {
                    0.
                };
                if this.now.autoscroll != 0. && !this.now.autoscrolling {
                    this.start_autoscroll(cx);
                }
                this.update_drop_at(cx);
            }))
            .on_drop(cx.listener(|this, drag: &QueueDrag, _, cx| {
                let before = this.now.drop_at.take();
                this.now.autoscroll = 0.;
                this.now.drag_y = None;
                if let Some(before) = before {
                    let to = drop_target(drag.from, before);
                    this.move_queue_item(drag.from, to, cx);
                    this.now.cursor = Some(to);
                    this.now.dropped_at = Some(std::time::Instant::now());
                }
                this.views.side.update(cx, |_, cx| cx.notify());
            }))
            .child(list)
            .children(insertion)
            .into_any_element()
    }

    /// Fare yüksekliğinden bırakılacak yer (görünür sırada bu satırın önü).
    fn update_drop_at(&mut self, cx: &mut Context<Self>) {
        let Some(y) = self.now.drag_y else { return };
        let items = self.queue_items();
        let offset = f32::from(self.now.list.0.borrow().base_handle.offset().y);
        let content_y = (y - offset).max(0.);
        let k = (content_y / ROW_HEIGHT).floor() as usize;
        let lower = (content_y % ROW_HEIGHT) >= ROW_HEIGHT / 2.;
        let visible = self.now.visible_len();
        let user = self.now.queue.user_len.min(visible);
        let before = match items.get(k) {
            Some(QItem::Row(v)) => v + lower as usize,
            Some(QItem::NowHead | QItem::Current | QItem::UserHead) => 0,
            Some(QItem::ContextHead) => user,
            None => visible,
        };
        if self.now.drop_at != Some(before) {
            self.now.drop_at = Some(before);
            self.views.side.update(cx, |_, cx| cx.notify());
        }
    }

    /// Sürüklerken listenin kenarındaysa her karede (60 kare/sn) kaydır.
    fn start_autoscroll(&mut self, cx: &mut Context<Self>) {
        self.now.autoscrolling = true;
        let frame = Duration::from_secs_f32(1. / ANIM_FPS);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(frame).await;
                let go = this
                    .update(cx, |shell, cx| shell.autoscroll_step(cx))
                    .unwrap_or(false);
                if !go {
                    break;
                }
            }
        })
        .detach();
    }

    fn autoscroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        if !cx.has_active_drag()
            || self.now.autoscroll == 0.
            || self.now.panel != Some(Panel::Queue)
        {
            self.now.autoscrolling = false;
            self.now.autoscroll = 0.;
            return false;
        }
        let count = self.queue_items().len() as f32;
        {
            let state = self.now.list.0.borrow();
            let viewport = f32::from(state.base_handle.bounds().size.height);
            let content = count * ROW_HEIGHT + S2;
            let max = (content - viewport).max(0.);
            let offset = state.base_handle.offset();
            let y = (f32::from(offset.y) - self.now.autoscroll).clamp(-max, 0.);
            state.base_handle.set_offset(gpui::point(offset.x, px(y)));
        }
        self.update_drop_at(cx);
        self.views.side.update(cx, |_, cx| cx.notify());
        true
    }

    /// Klavyeyle seçilen satır görünsün: zaten görünüyorsa liste kıpırdamaz,
    /// değilse en az kayarak kenara gelir.
    pub(super) fn reveal_queue_row(&self, v: usize) {
        let Some(ix) = self.queue_items().iter().position(|i| *i == QItem::Row(v)) else {
            return;
        };
        let state = self.now.list.0.borrow();
        let viewport = f32::from(state.base_handle.bounds().size.height);
        let offset = state.base_handle.offset();
        let shown = -f32::from(offset.y);
        let top = ix as f32 * ROW_HEIGHT;
        let y = if top < shown {
            top
        } else if top + ROW_HEIGHT > shown + viewport {
            top + ROW_HEIGHT - viewport
        } else {
            return;
        };
        state
            .base_handle
            .set_offset(gpui::point(offset.x, px(-y.max(0.))));
    }

    fn queue_item(&self, ix: usize, item: QItem, cx: &mut Context<Self>) -> AnyElement {
        match item {
            QItem::NowHead => self.queue_head(ix, t!(NowPlaying).into(), false, cx),
            QItem::UserHead => self.queue_head(ix, t!(QueueUserHead).into(), true, cx),
            QItem::ContextHead => {
                let title = match (&self.now.queue.radio, &self.now.queue.context) {
                    // Radyo çalıyorsa: "Sıradaki: <ad> radyosu" (bkz. `listening`).
                    (Some(radio), _) => Self::radio_head(radio),
                    (None, Some(id)) => tf!(QueueNextFrom, self.library.title(id)),
                    (None, None) => t!(QueueNextFromThis).into(),
                };
                self.queue_head(ix, title, false, cx)
            }
            QItem::Current => match self.now.queue.current.clone() {
                Some(entry) => self.queue_row(ix, None, &entry, Look::Live, cx),
                None => div().into_any_element(),
            },
            QItem::Row(v) => match self.now.queue.upcoming.get(v).cloned() {
                Some(entry) => self.queue_row(ix, Some(v), &entry, Look::Live, cx),
                None => div().into_any_element(),
            },
        }
    }

    /// Bölüm başlığı: satır boyunda, yazı alt kenarda (ayrı gruplar büyük
    /// boşlukla ayrılır). Kullanıcı sırasında "Temizle".
    fn queue_head(
        &self,
        ix: usize,
        title: String,
        clear: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shuffle = !clear && title != t!(NowPlaying) && self.now.queue.shuffle;
        div()
            .id(("q-head", ix))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(S2))
            .flex()
            .flex_col()
            .justify_end()
            .child(
                div()
                    .h(px(S6))
                    .flex()
                    .items_center()
                    .gap(px(S2))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .type_role(T_LABEL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(label())
                            .child(line_with_tip(("q-head-t", ix), title).flex().min_w_0()),
                    )
                    .when(shuffle, |el| {
                        el.child(
                            div()
                                .type_role(T_LABEL)
                                .text_color(label())
                                .child(t!(QueueShuffled)),
                        )
                    })
                    .when(clear, |el| {
                        el.child(
                            div()
                                .id("q-clear")
                                .h(px(S6))
                                .px(px(S3))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .type_role(T_LABEL)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted())
                                .cursor_pointer()
                                .hover(|s| s.bg(veil(VEIL_HOVER)).text_color(text()))
                                .active(|s| s.bg(veil(VEIL_PRESSED)))
                                .on_click(cx.listener(|this, _, _, cx| this.clear_user_queue(cx)))
                                .child(t!(Clear)),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Sıra satırı. `v`: görünür sıradaki yeri (çalan şarkıda `None`).
    pub(super) fn queue_row(
        &self,
        ix: usize,
        v: Option<usize>,
        entry: &QueueEntry,
        look: Look,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let current = v.is_none();
        let selected = v.is_some() && v == self.now.cursor;
        let (name, artists, image, album, duration) = match &entry.track {
            Some(t) => (
                t.name.clone(),
                t.artists.clone(),
                t.image.clone(),
                t.album.clone(),
                Some(t.duration_ms),
            ),
            None => (
                t!(QueueInfoLoading).to_string(),
                String::new(),
                String::new(),
                String::new(),
                None,
            ),
        };
        let group = "q-row";
        let (rest, hover, pressed) = if current {
            (
                Some(accent_veil(accent, ACCENT_PLAYING)),
                accent_veil(accent, ACCENT_PLAYING_HOVER),
                accent_veil(accent, ACCENT_PLAYING_PRESSED),
            )
        } else if selected {
            (
                Some(veil(VEIL_SELECTED)),
                veil(VEIL_SELECTED),
                veil(VEIL_PRESSED),
            )
        } else {
            (None, veil(VEIL_HOVER), veil(VEIL_PRESSED))
        };
        let row = div()
            .id(("q-row", ix))
            .group(group)
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(S2))
            .rounded(px(R_MD))
            .flex()
            .items_center()
            .gap(px(S3))
            .when_some(rest, |el, bg| el.bg(bg));
        let row = match look {
            Look::Hover => row.bg(hover),
            Look::Pressed => row.bg(pressed),
            Look::Disabled => row,
            Look::Live | Look::Focus => row
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(pressed)),
        };
        let live = look == Look::Live;
        let row = match v {
            Some(v) if live => row
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        this.play_queue_item(v, cx);
                    } else {
                        this.now.cursor = Some(v);
                        window.focus(&this.now.focus);
                        this.views.side.update(cx, |_, cx| cx.notify());
                    }
                }))
                .on_drag(
                    QueueDrag {
                        from: v,
                        name: name.clone(),
                        artists: artists.clone(),
                    },
                    |drag, _, _, cx| {
                        cx.new(|_| DragGhost {
                            name: drag.name.clone(),
                            artists: drag.artists.clone(),
                        })
                    },
                ),
            _ => row,
        };

        // Kapak; üzerine gelince ortasında çal (çalan şarkıda yok).
        let cover = div()
            .relative()
            .flex_none()
            .child(cover_image(
                &self.images,
                &image,
                &album,
                THUMB,
                R_SM,
                "music",
            ))
            .when_some(v.filter(|_| live), |el, v| {
                el.child(
                    div()
                        .id(("q-play", ix))
                        .absolute()
                        .inset_0()
                        .rounded(px(R_SM))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(shade(SCRIM))
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.play_queue_item(v, cx);
                            cx.stop_propagation();
                        }))
                        .child(icon("play-fill", 14., text())),
                )
            })
            .when(look == Look::Hover && !current, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(R_SM))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(shade(SCRIM))
                        .child(icon("play-fill", 14., text())),
                )
            });

        let title_color = if current { rgb(accent) } else { text() };
        let titles = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .type_role(T_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(title_color)
                    .child(line_with_tip(("q-name", ix), name).flex().min_w_0()),
            )
            .child(
                div()
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(line_with_tip(("q-artists", ix), artists).flex().min_w_0()),
            );

        // Sağ: süre; üzerine gelince yerinde çıkarma düğmesi.
        let hover_look = look == Look::Hover;
        let right = div()
            .relative()
            .w(px(S7))
            .h(px(S6))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .child(
                div()
                    .type_role(T_LABEL)
                    .tabular()
                    .text_color(label())
                    .when(v.is_some(), |el| el.group_hover(group, |s| s.opacity(0.)))
                    .when(hover_look && v.is_some(), |el| el.opacity(0.))
                    .children(duration.map(fmt_time)),
            )
            .when_some(v, |el, v| {
                el.child(
                    div()
                        .id(("q-remove", ix))
                        .absolute()
                        .top_0()
                        .right_0()
                        .size(px(S6))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(if hover_look { 1. } else { 0. })
                        .group_hover(group, |s| s.opacity(1.))
                        .child(super::tween::hover_fill(
                            veil(VEIL_HOVER),
                            veil(VEIL_PRESSED),
                            0.,
                            super::tween::FULL,
                        ))
                        .tooltip(Tooltip::text(t!(QueueRemoveTip), Some("Del")))
                        .when(live, |el| {
                            el.on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_queue_item(v, cx);
                                cx.stop_propagation();
                            }))
                        })
                        .child(icon("x", 16., soft())),
                )
            });

        row.child(cover)
            .child(titles)
            .child(right)
            .into_any_element()
    }
}

// ── Vitrin ──────────────────────────────────────────────────────────────

impl Shell {
    // i18n-skip-start: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez
    /// Bileşen vitrini (`OPENIFY_GALLERY`): açılıp kapanan düğmeler, sıra
    /// satırları ve bölüm başlığı, bırakma çizgisi, sürüklenen satır, söz
    /// satırları.
    pub(super) fn now_playing_gallery(&self, cx: &mut Context<Self>) -> AnyElement {
        use super::pill::toggle_button;
        let accent = self.accent().text;
        let caption = |name: &'static str| {
            div()
                .w(px(160.))
                .type_role(T_SECONDARY)
                .text_color(muted())
                .child(name)
        };
        let looks = |name: &'static str, make: &dyn Fn(Look) -> AnyElement| {
            let mut row = div().flex().items_center().gap(px(S5)).child(caption(name));
            for look in Look::ALL {
                row = row.child(div().w(px(120.)).flex().child(make(look)));
            }
            row
        };
        let toggles = div()
            .flex()
            .flex_col()
            .gap(px(S4))
            .child(looks("Açılır düğme, kapalı", &|look| {
                toggle_button(
                    ("g-toggle-off", look as usize),
                    "shuffle",
                    false,
                    look,
                    accent,
                )
                .into_any_element()
            }))
            .child(looks("Açılır düğme, açık", &|look| {
                toggle_button(
                    ("g-toggle-on", look as usize),
                    "repeat-1",
                    true,
                    look,
                    accent,
                )
                .into_any_element()
            }));

        let sample = |n: usize, source: crate::engine::QueueSource| {
            let (name, artists, album) = [
                ("Sür Eve", "Güneş", "Sür Eve"),
                (
                    "Kışın arabada sohbet gibisi yok, hele radyoda eski bir şarkı varsa",
                    "Uzun Ad Denemesi",
                    "Uzun",
                ),
                ("Dal", "Adamlar", "Harekete kimse mâni olamaz."),
                ("Melekler Ölmez", "mor ve ötesi", "Melekler Ölmez"),
            ][n % 4];
            QueueEntry {
                uri: format!("spotify:track:g-queue{n}"),
                track: Some(crate::engine::TrackRow {
                    uri: format!("spotify:track:g-queue{n}"),
                    name: name.into(),
                    artists: artists.into(),
                    album: album.into(),
                    duration_ms: 181_000 + n as u32 * 23_000,
                    image: String::new(),
                }),
                source,
            }
        };
        use crate::engine::QueueSource::{Context as Ctx, User};
        let row = |name: &'static str, el: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(px(S5))
                .child(caption(name))
                .child(div().w(px(PANEL_WIDTH - S4)).child(el))
        };
        let rows = div()
            .flex()
            .flex_col()
            .gap(px(S1))
            .child(row(
                "Bölüm başlığı",
                self.queue_head(9000, "Sıradaki: Beğenilenler".into(), false, cx),
            ))
            .child(row(
                "Kullanıcı sırası",
                self.queue_head(9001, "Sıradaki".into(), true, cx),
            ))
            .child(row(
                "Çalan",
                self.queue_row(9002, None, &sample(0, Ctx), Look::Live, cx),
            ))
            .child(row(
                "Normal",
                self.queue_row(9003, Some(900), &sample(1, User), Look::Disabled, cx),
            ))
            .child(row(
                "Üzerine gelme",
                self.queue_row(9004, Some(901), &sample(2, Ctx), Look::Hover, cx),
            ))
            .child(row(
                "Basma",
                self.queue_row(9005, Some(902), &sample(3, Ctx), Look::Pressed, cx),
            ))
            .child(row(
                "Bırakılacak yer",
                div()
                    .relative()
                    .child(self.queue_row(9006, Some(903), &sample(0, Ctx), Look::Disabled, cx))
                    .child(
                        div()
                            .absolute()
                            .left(px(S3))
                            .right(px(S3))
                            .top(px(-S0 / 2.))
                            .h(px(S0))
                            .rounded(px(R_XS))
                            .bg(rgb(accent)),
                    )
                    .into_any_element(),
            ))
            .child(row(
                "Sürüklenen satır",
                drag_ghost("Dal", "Adamlar").into_any_element(),
            ));

        let lyric = |text_: &'static str, active: bool| {
            div()
                .relative()
                .pl(px(S3))
                .font_family(font())
                .type_role(T_TITLE)
                .font_weight(FontWeight::BOLD)
                .text_color(if active { text() } else { muted() })
                .child(text_)
                .when(active, |el| {
                    el.child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(S1))
                            .bottom(px(S1))
                            .w(px(S0))
                            .rounded(px(R_XS))
                            .bg(rgb(accent)),
                    )
                })
        };
        let lyrics = div()
            .w(px(PANEL_WIDTH))
            .p(px(S4))
            .rounded(px(R_XL))
            .bg(rgb(BAR))
            .flex()
            .flex_col()
            .gap(px(S3))
            .child(lyric("Gece uzun, şehir sessiz", false))
            .child(lyric("Bir şarkı çalıyor eski bir radyoda", true))
            .child(lyric("Sözler akıyor satır satır", false));

        let section = |title: &'static str, body: AnyElement| {
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
        };
        div()
            .flex()
            .flex_col()
            .gap(px(S7))
            .child(section(
                "Şimdi çalıyor: açılır düğmeler (karıştır, tekrar, sıra, sözler)",
                toggles.into_any_element(),
            ))
            .child(section(
                "Sıra paneli: başlık, satırlar, sürükle bırak",
                rows.into_any_element(),
            ))
            .child(section(
                "Sözler: okunan satır ve sessiz satırlar",
                lyrics.into_any_element(),
            ))
            .into_any_element()
    }
    // i18n-skip-end: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez
}
