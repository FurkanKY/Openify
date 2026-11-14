//! Mini oynatıcı: ayrı, küçük, hep üstte duran ikinci pencere.
//!
//! Ürün kararı (13 Kasım): çalma denetimleri ana pencereden bağımsız, her
//! zaman görünür kalsın. Pencere kendi başlık çubuğunu çizmez (sistem
//! çerçevesi ve görev çubuğu girdisi yok, `WindowKind::PopUp`); sürükleme
//! alanı üstteki ince şerittir, kapatma düğmesi sağındadır.
//!
//! Durum KOPYALANMAZ: görünüm ana pencerenin `Shell` varlığını gözler ve
//! şarkıyı, çalmayı, ilerlemeyi oradan okur (`player::PlayerState`). Kapalıyken
//! ne pencere ne görünüm vardır; maliyet yoktur.
//!
//! Zemin: hapın cam zemini ile çalan kapağın ışığının karışımı
//! (`theme::mini_fill`, `pill_glass` / arama yüzeyiyle aynı parlaklık tavanı);
//! açık temada fildişi, koyuda gece kalır.

use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    MouseDownEvent, Pixels, Render, Subscription, Task, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowKind, WindowOptions, div,
    img, linear_color_stop, linear_gradient, point, prelude::*, px, rgb, size,
};

use super::{
    Shell,
    components::{Look, Tooltip, icon_button, icon_button_tinted, play_button},
    integration::Op,
    pill::{fraction, slider_hit},
    text::line,
    theme::*,
};
use crate::engine::TrackRow;
use crate::t;

/// Görünen kart: 340 × 96 mantıksal px (yatay düzen).
pub(super) const MINI_W: f32 = 340.;
pub(super) const MINI_H: f32 = 96.;
/// Yüzen pencerenin gölgesine kalan saydam pay (kart ortada durur).
const SHADOW_ROOM: f32 = S3;
/// Pencerenin tam boyu: kart + iki yanda gölge payı.
pub(super) const MINI_WINDOW_W: f32 = MINI_W + 2. * SHADOW_ROOM;
pub(super) const MINI_WINDOW_H: f32 = MINI_H + 2. * SHADOW_ROOM;
/// Kapak ve düğmeler.
const COVER: f32 = 48.;
const HEADER: f32 = S5;
const BUTTON: f32 = 28.;
const PROGRESS_ROW: f32 = S3;

/// Mini pencerenin ekrandaki yeri, ekranlara göre görünür alana çekilir.
/// Pencere hiçbir ekranla kesişmiyorsa merkezi en yakın ekrana göre seçilir;
/// ekran pencereden küçükse sol üst köşeye hizalanır. `screens` mantıksal px.
pub(super) fn visible_pos(
    pos: (f32, f32),
    size: (f32, f32),
    screens: &[Bounds<Pixels>],
) -> (f32, f32) {
    let Some(i) = screen_of(pos, size, screens) else {
        return pos;
    };
    let b = screens[i];
    let (left, top) = (f32::from(b.origin.x), f32::from(b.origin.y));
    let (right, bottom) = (
        left + f32::from(b.size.width),
        top + f32::from(b.size.height),
    );
    let x = if right - left <= size.0 {
        left
    } else {
        pos.0.clamp(left, right - size.0)
    };
    let y = if bottom - top <= size.1 {
        top
    } else {
        pos.1.clamp(top, bottom - size.1)
    };
    (x, y)
}

/// Pencerenin en çok kesiştiği ekran; hiçbiriyle kesişmiyorsa merkezi en
/// yakın olan.
fn screen_of(pos: (f32, f32), size: (f32, f32), screens: &[Bounds<Pixels>]) -> Option<usize> {
    let mut best = None;
    let mut best_area = 0.;
    for (i, b) in screens.iter().enumerate() {
        let (left, top) = (f32::from(b.origin.x), f32::from(b.origin.y));
        let (right, bottom) = (
            left + f32::from(b.size.width),
            top + f32::from(b.size.height),
        );
        let w = (pos.0 + size.0).min(right) - pos.0.max(left);
        let h = (pos.1 + size.1).min(bottom) - pos.1.max(top);
        let area = w.max(0.) * h.max(0.);
        if area > best_area {
            best_area = area;
            best = Some(i);
        }
    }
    if best.is_some() {
        return best;
    }
    let (cx, cy) = (pos.0 + size.0 / 2., pos.1 + size.1 / 2.);
    screens
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let d = |s: &Bounds<Pixels>| {
                let (x, y) = (
                    f32::from(s.origin.x) + f32::from(s.size.width) / 2.,
                    f32::from(s.origin.y) + f32::from(s.size.height) / 2.,
                );
                (x - cx).powi(2) + (y - cy).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
        .map(|(i, _)| i)
}

/// Mini pencerenin varsayılan yeri: hedef ekranın sağ üst köşesi.
fn default_pos(screen: Option<Bounds<Pixels>>) -> (f32, f32) {
    let Some(b) = screen else {
        return (S4, S4);
    };
    let right = f32::from(b.origin.x) + f32::from(b.size.width);
    let top = f32::from(b.origin.y);
    (right - MINI_WINDOW_W - S4, top + S4)
}

/// Mini pencerede Tab ile gezilen düğmeler.
struct Stops {
    close: FocusHandle,
    prev: FocusHandle,
    play: FocusHandle,
    next: FocusHandle,
    like: FocusHandle,
    show_main: FocusHandle,
    progress: FocusHandle,
}

/// Görünümün her karede ana pencereden okuduğu değerler (kopya durum yok).
struct MiniModel {
    title: String,
    has_track: bool,
    playing: bool,
    has_next: bool,
    progress: f32,
    liked: bool,
    cover: Option<Arc<gpui::Image>>,
    accent: u32,
    row: Option<TrackRow>,
}

pub struct MiniView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
    focus: FocusHandle,
    stops: Stops,
    /// Son hareket klavyeden mi (odak halkası yalnızca o zaman görünür).
    keyboard_focus: bool,
    /// Çalarken ilerlemeyi saniyede bir ilerleten tek atımlık görev.
    tick: Option<Task<()>>,
    /// İlerleme çubuğunun ekrandaki yeri (tıklamayı orana çevirmek için).
    progress_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Konum değişti mi (ayarlara yazmak için son görülen sol üst).
    last_pos: Option<(f32, f32)>,
    /// Sürüklerken konumu izleyen görev.
    _track_pos: Task<()>,
}

impl MiniView {
    pub(super) fn new(shell: &Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let stop = |index| cx.focus_handle().tab_stop(true).tab_index(index);
        let stops = Stops {
            close: stop(0),
            prev: stop(1),
            play: stop(2),
            next: stop(3),
            like: stop(4),
            show_main: stop(5),
            progress: stop(6),
        };
        let focus = cx.focus_handle();
        window.focus(&focus);
        // Kullanıcı pencereyi kapatırsa (Alt+F4): ayarlara kapalı yazılsın.
        let closing = shell.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let origin = window.window_bounds().get_bounds().origin;
            let pos = (f32::from(origin.x), f32::from(origin.y));
            let _ = closing.update(cx, |shell, cx| shell.mini_closed(pos, cx));
            true
        });
        // Sürükleme gpui'ye Windows'un kendi taşıma döngüsüyle gelir: yeni
        // konumu kare çizmeden okuyamayız, kısa aralıkla yokla ve ayarlardaki
        // değeri güncelle (dosyaya kapanışta / çıkışta yazılır).
        let _track_pos = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if this
                    .update_in(cx, |view, window, cx| view.note_pos(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        #[cfg(windows)]
        if let Some(hwnd) = super::windows_titlebar::hwnd(window) {
            crate::system::prepare_popup(hwnd);
        }
        Self {
            shell: shell.downgrade(),
            _follow: cx.observe(shell, |_, _, cx| cx.notify()),
            focus,
            stops,
            keyboard_focus: false,
            tick: None,
            progress_bounds: Rc::default(),
            last_pos: None,
            _track_pos,
        }
    }

    fn look(&self, handle: &FocusHandle, window: &Window, enabled: bool) -> Look {
        if !enabled {
            Look::Disabled
        } else if self.keyboard_focus && handle.is_focused(window) {
            Look::Focus
        } else {
            Look::Live
        }
    }

    /// Ana pencerenin o anki durumu (kopya değil, aynı `Shell`ten okunur).
    fn model(&self, cx: &App) -> Option<MiniModel> {
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        let (name, artists) = match (&shell.state.track, &shell.state.pending) {
            (Some(t), _) => (t.name.clone(), t.artists.clone()),
            (None, Some(p)) => (p.name.clone(), String::new()),
            _ => (t!(NothingPlaying).to_string(), String::new()),
        };
        let title = if artists.is_empty() {
            name
        } else {
            format!("{name} · {artists}")
        };
        let now = Instant::now();
        let track = shell.state.track.as_ref();
        let uri = track.map(|t| t.uri.clone());
        let row = track.map(|t| shell.pill_row(&t.uri, &t.name, &t.artists));
        Some(MiniModel {
            title,
            has_track: track.is_some(),
            playing: shell.state.is_playing(),
            has_next: shell.state.queue.has_next,
            progress: shell.state.progress(now),
            liked: uri.as_deref().is_some_and(|u| shell.is_liked(u)),
            cover: shell.state.cover.clone(),
            accent: pill_accent(shell.accent().text),
            row,
        })
    }

    /// Konumu ana pencereye bildir (dosyaya kapanışta / çıkışta yazılır).
    fn note_pos(&mut self, window: &Window, cx: &mut Context<Self>) {
        let origin = window.window_bounds().get_bounds().origin;
        let pos = (f32::from(origin.x), f32::from(origin.y));
        if self.last_pos == Some(pos) {
            return;
        }
        self.last_pos = Some(pos);
        let _ = self.shell.update(cx, |shell, _| shell.note_mini_pos(pos));
    }

    /// Çalarken ilerleme saniyede bir ilerlesin; durunca hiç çizim yok.
    fn tick_progress(&mut self, cx: &mut Context<Self>) {
        let playing = self
            .shell
            .upgrade()
            .is_some_and(|shell| shell.read(cx).state.is_playing());
        if !playing {
            self.tick = None;
            return;
        }
        if self.tick.is_some() {
            return;
        }
        self.tick = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update(cx, |view, cx| {
                view.tick = None;
                cx.notify();
            });
        }));
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(shell) = self.shell.upgrade() {
            let origin = window.window_bounds().get_bounds().origin;
            let pos = (f32::from(origin.x), f32::from(origin.y));
            shell.update(cx, |shell, cx| shell.mini_closed(pos, cx));
        }
        window.remove_window();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let on_stop = [
            &self.stops.close,
            &self.stops.prev,
            &self.stops.play,
            &self.stops.next,
            &self.stops.like,
            &self.stops.show_main,
            &self.stops.progress,
        ]
        .iter()
        .any(|h| h.is_focused(window));
        match key {
            "escape" => self.close(window, cx),
            "tab" => {
                self.keyboard_focus = true;
                if event.keystroke.modifiers.shift {
                    window.focus_prev();
                } else {
                    window.focus_next();
                }
                cx.notify();
            }
            // Odaktaki düğme Enter / Boşluk'u kendisi alır: ikinci kez çalmasın.
            "space" if !on_stop && !event.is_held => {
                let _ = self.shell.update(cx, |shell, cx| shell.toggle_play(cx));
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for MiniView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(m) = self.model(cx) else {
            return div().into_any_element();
        };
        let accent = m.accent;
        let (bg_top, bg_bottom) = mini_fill(m.accent);
        let look =
            |this: &Self, handle: &FocusHandle, enabled: bool| this.look(handle, window, enabled);

        let close = div()
            .id("mini-close")
            .size(px(HEADER))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_2()
            .border_color(if look(self, &self.stops.close, true) == Look::Focus {
                rgb(accent).into()
            } else {
                gpui::transparent_black()
            })
            .cursor_pointer()
            .group("mini-close")
            .hover(|s| s.bg(veil(VEIL_HOVER)))
            .track_focus(&self.stops.close)
            .tooltip(Tooltip::text(t!(MiniClose), None))
            .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                if !ev.is_keyboard() {
                    window.focus(&this.focus);
                    this.keyboard_focus = false;
                }
                this.close(window, cx);
            }))
            .child(icon("x", 14., muted()).group_hover("mini-close", |s| s.text_color(text())));

        let art: AnyElement = match &m.cover {
            Some(image) => img(image.clone())
                .size(px(COVER))
                .rounded(px(R_MD))
                .into_any_element(),
            None => div()
                .size(px(COVER))
                .flex_none()
                .rounded(px(R_MD))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(rgb(mix(palette().placeholder, 0xFFFFFF, 0.05)), 0.),
                    linear_color_stop(rgb(palette().placeholder), 1.),
                ))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("music", 18., label()))
                .into_any_element(),
        };

        // Şarkı adı satırı hem sürükleme tutamağı hem kapat düğmesinin yeri:
        // kapat ayrı bir satır kaplamasın (14 Kasım), kart da sürüklenebilsin.
        let title = div()
            .h(px(HEADER))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(S1))
            .child(
                div()
                    .id("mini-drag")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .type_role(T_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(line(m.title.clone())),
            )
            .child(close);

        let prev = icon_button(
            "mini-prev",
            "skip-back-fill",
            look(self, &self.stops.prev, m.has_track),
            accent,
        )
        .size(px(BUTTON))
        .when(m.has_track, |el| {
            el.track_focus(&self.stops.prev)
                .on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.shell.update(cx, |shell, cx| shell.previous(cx));
                }))
        })
        .tooltip(Tooltip::text(t!(PrevTrack), None));

        let play = play_button(
            "mini-play",
            m.playing,
            look(self, &self.stops.play, m.has_track),
            accent,
        )
        .size(px(BUTTON))
        .when(m.has_track, |el| {
            el.track_focus(&self.stops.play)
                .on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.shell.update(cx, |shell, cx| shell.toggle_play(cx));
                }))
        })
        .tooltip(Tooltip::text(
            if m.playing { t!(Pause) } else { t!(Play) },
            Some(t!(KeySpace)),
        ));

        let next = icon_button(
            "mini-next",
            "skip-forward-fill",
            look(self, &self.stops.next, m.has_next),
            accent,
        )
        .size(px(BUTTON))
        .when(m.has_next, |el| {
            el.track_focus(&self.stops.next)
                .on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.shell.update(cx, |shell, cx| shell.next(cx));
                }))
        })
        .tooltip(Tooltip::text(t!(NextTrack), None));

        let heart_icon = if m.liked { "heart-fill" } else { "heart" };
        let heart = icon_button_tinted(
            "mini-like",
            heart_icon,
            if m.liked { rgb(accent) } else { soft() },
            look(self, &self.stops.like, m.has_track),
            accent,
        )
        .size(px(BUTTON))
        .when_some(m.row.clone(), |el, row| {
            el.track_focus(&self.stops.like).on_click(cx.listener(
                move |this, ev: &ClickEvent, window, cx| {
                    if !ev.is_keyboard() {
                        window.focus(&this.focus);
                        this.keyboard_focus = false;
                    }
                    let on = !this
                        .shell
                        .upgrade()
                        .is_some_and(|shell| shell.read(cx).is_liked(&row.uri));
                    let _ = this
                        .shell
                        .update(cx, |shell, cx| shell.set_liked(vec![row.clone()], on, cx));
                },
            ))
        })
        .tooltip(Tooltip::text(
            if m.liked { t!(UnlikeTip) } else { t!(LikeTip) },
            None,
        ));

        let show_main = icon_button(
            "mini-main",
            "maximize-2",
            look(self, &self.stops.show_main, true),
            accent,
        )
        .size(px(BUTTON))
        .track_focus(&self.stops.show_main)
        .tooltip(Tooltip::text(t!(MiniShowMain), None))
        .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
            if !ev.is_keyboard() {
                window.focus(&this.focus);
                this.keyboard_focus = false;
            }
            let _ = this.shell.update(cx, |shell, _| {
                shell.desk.send(Op::Show {
                    front: true,
                    at: Instant::now(),
                    why: "mini",
                });
            });
        }));

        let controls = div()
            .h(px(BUTTON))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(S1))
            .child(prev)
            .child(play)
            .child(next)
            .child(div().flex_1())
            .child(heart)
            .child(show_main);

        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .gap(px(S3))
            .child(art)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(title)
                    .child(controls),
            );

        let progress = div()
            .h(px(PROGRESS_ROW))
            .flex_none()
            .flex()
            .items_center()
            .child(slider_hit(
                "mini-progress",
                m.progress,
                accent,
                m.has_track,
                Some((
                    self.stops.progress.clone(),
                    look(self, &self.stops.progress, m.has_track) == Look::Focus,
                )),
                self.progress_bounds.clone(),
                Some(Box::new(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    if let Some(f) = fraction(&this.progress_bounds, ev.position) {
                        let _ = this.shell.update(cx, |shell, cx| {
                            let target = (shell.state.duration_ms() as f32 * f) as u32;
                            shell.seek_to(target, cx);
                        });
                    }
                }))),
                PROGRESS_ROW,
            ));

        let column = div()
            .size_full()
            .pt(px(S2))
            .pl(px(S3))
            .pr(px(S2))
            .flex()
            .flex_col()
            .child(body)
            .child(progress);

        // Kapak ışığı soldan: kapağın arkasında kapağın rengi (yazının
        // üstünde değil; okunurluk `mini_fill` ile korunur).
        let cover_glow = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(COVER + S5))
            .h_full()
            .bg(linear_gradient(
                90.,
                linear_color_stop(accent_veil(accent, 0.14), 0.),
                linear_color_stop(accent_veil(accent, 0.), 1.),
            ));

        let card = div()
            .size_full()
            .rounded(px(R_XL))
            .shadow(float_shadow())
            .child(
                div()
                    .size_full()
                    .rounded(px(R_XL))
                    .overflow_hidden()
                    .border_1()
                    .border_color(veil(PILL_RING))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgb(bg_top), 0.),
                        linear_color_stop(rgb(bg_bottom), 1.),
                    ))
                    .child(cover_glow)
                    .child(column),
            );

        self.tick_progress(cx);

        div()
            .size_full()
            .p(px(SHADOW_ROOM))
            .font_family(UI_FONT)
            .text_color(text())
            .on_key_down(cx.listener(Self::on_key_down))
            .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus);
                if std::mem::take(&mut this.keyboard_focus) {
                    cx.notify();
                }
            }))
            .child(div().track_focus(&self.focus).absolute().size_0())
            .child(card)
            .into_any_element()
    }
}

impl Shell {
    /// Mini pencerenin son bilinen yeri (dosyaya kapanışta / çıkışta yazılır).
    pub(super) fn note_mini_pos(&mut self, pos: (f32, f32)) {
        self.prefs.mini_pos = Some([pos.0, pos.1]);
    }

    /// Mini pencere kapandı (kapatma düğmesi ya da Alt+F4): tercih kapalı.
    pub(super) fn mini_closed(&mut self, pos: (f32, f32), cx: &mut Context<Self>) {
        self.mini = None;
        self.prefs.mini_open = false;
        self.prefs.mini_pos = Some([pos.0, pos.1]);
        self.save_settings();
        cx.notify();
    }

    fn mini_is_open(&mut self, cx: &mut App) -> bool {
        if let Some(handle) = self.mini
            && handle.update(cx, |_, _, _| {}).is_ok()
        {
            return true;
        }
        self.mini = None;
        false
    }

    /// Hap / palet / kısayol: mini pencereyi aç ya da kapat.
    pub(super) fn toggle_mini(&mut self, screen: Option<Bounds<Pixels>>, cx: &mut Context<Self>) {
        if self.mini_is_open(cx) {
            self.close_mini(cx);
        } else {
            // Pencere Shell güncellenirken yaratılamaz: Windows ilk kareyi
            // hemen çizer, MiniView de Shell'i okur (gpui "already being
            // updated" paniği). Güncelleme bitince aç.
            let shell = cx.entity();
            cx.defer(move |cx| open_mini_window(&shell, screen, true, cx));
        }
    }

    /// Kullanıcı kapattı (palet, kısayol, hap): pencereyi yok et, tercihi yaz.
    fn close_mini(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.mini.take()
            && let Ok(pos) = handle.update(cx, |_, window, _| {
                let origin = window.window_bounds().get_bounds().origin;
                let pos = (f32::from(origin.x), f32::from(origin.y));
                window.remove_window();
                pos
            })
        {
            self.prefs.mini_pos = Some([pos.0, pos.1]);
        }
        if self.prefs.mini_open {
            self.prefs.mini_open = false;
            self.save_settings();
        }
        cx.notify();
    }
}

/// Mini pencereyi yarat (kapalıyken hiçbir maliyet yok). Shell'in bir
/// güncellemesi içinden ÇAĞRILMAZ (`cx.defer` ile): yeni pencerenin ilk
/// karesi Shell'i okur. `screen`: açılışta yerleşecek ekran (ana pencerenin
/// ekranı); yoksa birincil ekran. `remember`: açıldıysa tercihe "açık" yaz.
pub(super) fn open_mini_window(
    shell: &Entity<Shell>,
    screen: Option<Bounds<Pixels>>,
    remember: bool,
    cx: &mut App,
) {
    if shell.update(cx, |shell, cx| shell.mini_is_open(cx)) {
        return;
    }
    let saved = shell.read(cx).prefs.mini_pos;
    let displays: Vec<(gpui::DisplayId, Bounds<Pixels>)> =
        cx.displays().iter().map(|d| (d.id(), d.bounds())).collect();
    let rects: Vec<Bounds<Pixels>> = displays.iter().map(|(_, b)| *b).collect();
    let screen = screen.or_else(|| cx.primary_display().map(|d| d.bounds()));
    {
        let start = saved
            .map(|p| (p[0], p[1]))
            .unwrap_or_else(|| default_pos(screen));
        let (x, y) = visible_pos(start, (MINI_WINDOW_W, MINI_WINDOW_H), &rects);
        let display_id = screen_of((x, y), (MINI_WINDOW_W, MINI_WINDOW_H), &rects)
            .and_then(|i| displays.get(i))
            .map(|(id, _)| *id);
        let bounds = Bounds::new(
            point(px(x), px(y)),
            size(px(MINI_WINDOW_W), px(MINI_WINDOW_H)),
        );
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            display_id,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("Openify".into()),
            ..Default::default()
        };
        let owner = shell.clone();
        match cx.open_window(options, move |window, cx| {
            cx.new(|cx| MiniView::new(&owner, window, cx))
        }) {
            Ok(handle) => {
                shell.update(cx, |shell, cx| {
                    shell.mini = Some(handle);
                    if remember && !shell.prefs.mini_open {
                        shell.prefs.mini_open = true;
                        shell.save_settings();
                    }
                    cx.notify();
                });
                log::info!("mini oynatıcı açıldı");
            }
            Err(e) => log::warn!("mini oynatıcı açılamadı: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn pos(p: (f32, f32)) -> (f32, f32) {
        (p.0.round(), p.1.round())
    }

    #[test]
    fn visible_position_keeps_an_inside_window_untouched() {
        let screens = [screen(0., 0., 1920., 1080.)];
        assert_eq!(
            pos(visible_pos(
                (100., 80.),
                (MINI_WINDOW_W, MINI_WINDOW_H),
                &screens
            )),
            (100., 80.)
        );
        // Tam kenarda da yerinde kalır.
        assert_eq!(
            pos(visible_pos(
                (1920. - MINI_WINDOW_W, 1080. - MINI_WINDOW_H),
                (MINI_WINDOW_W, MINI_WINDOW_H),
                &screens
            )),
            (1920. - MINI_WINDOW_W, 1080. - MINI_WINDOW_H)
        );
    }

    #[test]
    fn off_screen_positions_are_pulled_into_view() {
        let screens = [screen(0., 0., 1920., 1080.)];
        let size = (MINI_WINDOW_W, MINI_WINDOW_H);
        // Sağ altta ekran dışı.
        assert_eq!(
            pos(visible_pos((5000., 5000.), size, &screens)),
            (1920. - MINI_WINDOW_W, 1080. - MINI_WINDOW_H)
        );
        // Sol üstte eksi.
        assert_eq!(pos(visible_pos((-400., -300.), size, &screens)), (0., 0.));
        // Ekrandan büyük pencere sol üste hizalanır.
        assert_eq!(
            pos(visible_pos((500., 500.), (3000., 2000.), &screens)),
            (0., 0.)
        );
    }

    #[test]
    fn the_nearest_screen_wins_across_monitors() {
        // Yan yana iki ekran: 0–1920 ve 1920–3840.
        let screens = [
            screen(0., 0., 1920., 1080.),
            screen(1920., 0., 1920., 1080.),
        ];
        let size = (MINI_WINDOW_W, MINI_WINDOW_H);
        // İkinci ekranda, içeride: dokunulmaz.
        assert_eq!(
            pos(visible_pos((2500., 100.), size, &screens)),
            (2500., 100.)
        );
        // İkinci ekranın sağında: ikinci ekrana çekilir.
        assert_eq!(
            pos(visible_pos((9000., 100.), size, &screens)),
            (3840. - MINI_WINDOW_W, 100.)
        );
        // Birinci ekranın solunda: birinci ekrana.
        assert_eq!(pos(visible_pos((-900., 200.), size, &screens)), (0., 200.));
        // İki ekranın arasındaki boşluk yok: sınırda pencere ikisinden birine sığar.
        let (x, _) = visible_pos((-20., 100.), size, &screens);
        assert!((0. ..=1920. - MINI_WINDOW_W).contains(&x));
    }

    #[test]
    fn default_position_sits_in_the_top_right_corner() {
        let (x, y) = default_pos(Some(screen(0., 0., 1920., 1080.)));
        assert_eq!(x, 1920. - MINI_WINDOW_W - S4);
        assert_eq!(y, S4);
    }

    #[test]
    fn mini_window_is_the_card_plus_its_shadow_room() {
        assert_eq!(MINI_WINDOW_W, MINI_W + 2. * SHADOW_ROOM);
        assert_eq!(MINI_WINDOW_H, MINI_H + 2. * SHADOW_ROOM);
        const { assert!(MINI_W >= 320. && MINI_W <= 360. && MINI_H >= 88. && MINI_H <= 104.) };
    }
}
