//! Açılır pencereler (Adım 10): tepsi menüsü ve her yerden palet.
//!
//! İkisi de ayrı, küçük gpui pencereleri: diğer pencerelerin üstünde
//! (`HWND_TOPMOST`), Windows'un çerçevesi kapalı, zemini saydam. Panel ve
//! yüzen gölgesi bizim çizimimiz (DESIGN.md "Menu", komut paleti); pencere
//! gölgeye yer kalsın diye panelden her yanda `SHADOW_ROOM` büyük.
//!
//! - Tepsi menüsü: Windows'un menüsü değil, uygulamanın menü bileşeni
//!   (`components::menu_panel`). Odak başka yere geçince kapanır.
//! - Her yerden palet (Ctrl+Shift+Boşluk): ana penceredeki paletin kendisi
//!   (`Shell::render_palette_panel`, aynı durum, aynı tuşlar). Ana pencere
//!   öne gelmez; Esc ya da Enter'dan sonra odak önceki uygulamaya döner,
//!   başka bir yere tıklanınca palet kendiliğinden kapanır.
//!
//! Pencereler bir kez açılır, kapanınca gizlenir, sonraki açılışta yeniden
//! gösterilir: gpui'de her yeni pencere kapansa da ~1,2–1,4 MB özel bellek
//! bırakıyordu (40 açılışta +44 MB, ölçüldü). Yeniden göstermek pencere
//! kurmaktan da hızlı.

use std::time::Instant;

use gpui::{
    AsyncApp, Bounds, Context, Entity, FocusHandle, KeyDownEvent, MouseButton, Subscription,
    WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, canvas, div, point, prelude::*, px, size,
};
use windows::Win32::Foundation::HWND;

use super::{
    Shell,
    components::{self, Look, MenuItem},
    integration::Op,
    palette::{PANEL_MAX_HEIGHT, PANEL_WIDTH},
    player::Status,
    text::line_with_tip,
    theme::*,
    windows_titlebar,
};
use crate::system;
use crate::t;

/// Panelin yüzen gölgesi için pencerede her yanda bırakılan saydam pay.
const SHADOW_ROOM: f32 = S6;
/// Tepsi menüsünün eni (şarkı adı tek satırda "…" ile kısalır).
const MENU_WIDTH: f32 = 248.;
/// Menünün üstündeki çalan şarkı satırı: ad ve sanatçı.
const MENU_HEADER_HEIGHT: f32 = T_BODY.line + T_SECONDARY.line + 2. * S2;
/// Her yerden paletin giriş satırı ekranın (çalışma alanının) bu
/// yüksekliğinde: ortanın biraz üstü, sonuçlar aşağı doğru uzar.
const PALETTE_TOP: f32 = 0.25;

/// Gizli açılır: yerine `system::show_popup` koyar ve etkinleştirmeden
/// gösterir (ilk kare öne almayı beklemesin).
fn popup_options(
    bounds: Bounds<gpui::Pixels>,
    display_id: Option<gpui::DisplayId>,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("Openify".into()),
        ..Default::default()
    }
}

fn display_id(index: usize, cx: &mut AsyncApp) -> Option<gpui::DisplayId> {
    cx.update(|cx| cx.displays().get(index).map(|d| d.id()))
        .ok()
        .flatten()
}

/// Var olan açılır pencereyi yeniden kullan (`show` onu açılışa hazırlar),
/// yoksa (ilk açılış ya da pencere Windows'ça kapatıldıysa) yenisini kur.
fn reuse_or_open<V: 'static + Render>(
    existing: Option<WindowHandle<V>>,
    show: impl Fn(&mut V, &mut Window, &mut Context<V>),
    open: impl FnOnce(&mut AsyncApp) -> anyhow::Result<WindowHandle<V>>,
    cx: &mut AsyncApp,
) -> anyhow::Result<WindowHandle<V>> {
    if let Some(handle) = existing
        && handle
            .update(cx, |view, window, cx| show(view, window, cx))
            .is_ok()
    {
        return Ok(handle);
    }
    let handle = open(cx)?;
    handle.update(cx, |view, window, cx| show(view, window, cx))?;
    Ok(handle)
}

/// Pencereyi yerine koy, diğerlerinin üstünde göster, sonra odağı ver
/// (sınama koşusunda vermez). `rect` ekran pikseli: sol, üst, en, boy.
fn raise<V: 'static>(
    handle: WindowHandle<V>,
    rect: (i32, i32, i32, i32),
    cx: &mut AsyncApp,
) -> bool {
    let hwnd = handle
        .update(cx, |_, window, _| windows_titlebar::hwnd(window))
        .ok()
        .flatten();
    hwnd.is_some_and(|hwnd| system::show_popup(hwnd, rect, !system::test_mode()))
}

/// Mantıksal (gpui) dikdörtgenin ekran pikseli karşılığı.
fn physical(bounds: Bounds<gpui::Pixels>, scale: f32) -> (i32, i32, i32, i32) {
    let p = |v: gpui::Pixels| (f32::from(v) * scale).round() as i32;
    (
        p(bounds.origin.x),
        p(bounds.origin.y),
        p(bounds.size.width),
        p(bounds.size.height),
    )
}

/// Kapanan açılır pencere gizlensin (gpui güncellemesinin dışında).
fn hide(shell: &WeakEntity<Shell>, hwnd: Option<HWND>, cx: &mut gpui::App) {
    if let Some(hwnd) = hwnd {
        let _ = shell.update(cx, |s, _| s.desk.send(Op::HidePopup(hwnd)));
    }
}

// ── Tepsi menüsü ────────────────────────────────────────────────────────

fn menu_height() -> f32 {
    // Kenar (1 + 1), iç boşluk (S1 × 2), çalan şarkı, ayırıcı, dört öğe,
    // ayırıcı, "Çık".
    2. + 2. * S1
        + MENU_HEADER_HEIGHT
        + 2. * components::MENU_SEPARATOR_HEIGHT
        + 5. * components::MENU_ITEM_HEIGHT
}

/// Tepsi simgesine sağ tıklandı: menüyü imlecin yanında aç.
pub fn open_tray_menu(shell: &WeakEntity<Shell>, x: i32, y: i32, at: Instant, cx: &mut AsyncApp) {
    let Some(monitor) = system::monitor_at(Some((x, y))) else {
        return;
    };
    let s = monitor.scale;
    let (w, h) = (MENU_WIDTH, menu_height());
    let (pw, ph) = ((w * s).round() as i32, (h * s).round() as i32);
    let (l, t, r, b) = monitor.work;
    // Panel imlecin üstünde ve solunda açılır (tepsi sağ altta); sığmazsa
    // öbür yanda; her durumda çalışma alanının (görev çubuğu hariç) içinde.
    let left = (if x - pw >= l { x - pw } else { x }).clamp(l, (r - pw).max(l));
    let top = (if y - ph >= t { y - ph } else { y }).clamp(t, (b - ph).max(t));
    let bounds = Bounds {
        origin: point(
            px(left as f32 / s - SHADOW_ROOM),
            px(top as f32 / s - SHADOW_ROOM),
        ),
        size: size(px(w + 2. * SHADOW_ROOM), px(h + 2. * SHADOW_ROOM)),
    };
    let display = display_id(monitor.index, cx);
    let Some(entity) = shell.upgrade() else {
        return;
    };
    let existing = shell.update(cx, |s, _| s.desk.menu_window).ok().flatten();
    let handle = reuse_or_open(
        existing,
        |menu: &mut TrayMenu, window, cx| menu.show(window, cx),
        |cx| {
            cx.update(|cx| {
                cx.open_window(popup_options(bounds, display), |window, cx| {
                    cx.new(|cx| TrayMenu::new(entity, window, cx))
                })
            })?
        },
        cx,
    );
    let handle = match handle {
        Ok(handle) => handle,
        Err(e) => {
            log::warn!("Tepsi menüsü açılamadı: {e:#}");
            return;
        }
    };
    let _ = shell.update(cx, |s, _| s.desk.menu_window = Some(handle));
    let fronted = raise(handle, physical(bounds, s), cx);
    log::info!(
        "sistem: tepsi menüsü açıldı, {:.1} ms (öne alındı: {fronted})",
        at.elapsed().as_secs_f64() * 1000.
    );
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MenuAction {
    Open,
    TogglePlay,
    Next,
    Previous,
    Quit,
}

/// Menünün çizildiği an okunan durum.
struct MenuState {
    accent: u32,
    title: String,
    subtitle: String,
    playing: bool,
    can_play: bool,
    can_next: bool,
    can_previous: bool,
}

pub struct TrayMenu {
    shell: WeakEntity<Shell>,
    focus: FocusHandle,
    hwnd: Option<HWND>,
    /// Menü ekranda mı (kapanınca pencere gizlenir, silinmez).
    open: bool,
    /// Klavyeyle seçilen öğe (fareyle açılınca hiçbiri).
    selected: Option<usize>,
    /// Bu açılışta bir kez öne geldi mi? (Odak gidince kapanmak için.)
    was_active: bool,
    _follow: Subscription,
    _activation: Subscription,
}

impl TrayMenu {
    fn new(shell: Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Çalma durumu değişirse ("Çal" ↔ "Duraklat") açık menü de yenilensin.
        let follow = cx.observe(&shell, |this: &mut Self, _, cx| {
            if this.open {
                cx.notify();
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = this.open;
            } else if this.was_active {
                this.close(cx);
            }
        });
        // Alt+F4: pencere silinmesin, menü kapansın.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = this.update(cx, |menu, cx| menu.close(cx));
            false
        });
        TrayMenu {
            shell: shell.downgrade(),
            focus: cx.focus_handle(),
            hwnd: windows_titlebar::hwnd(window),
            open: false,
            selected: None,
            was_active: false,
            _follow: follow,
            _activation: activation,
        }
    }

    /// Açılışa hazırla (pencere henüz gizli; `raise` gösterir).
    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.selected = None;
        self.was_active = false;
        window.focus(&self.focus);
        cx.notify();
    }

    fn state(&self, cx: &Context<Self>) -> Option<MenuState> {
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        let track = shell.state.track.as_ref();
        let (title, subtitle) = match track {
            Some(t) => (t.name.clone(), t.artists.clone()),
            None => (crate::brand::APP_NAME.into(), t!(NothingPlaying).into()),
        };
        Some(MenuState {
            accent: shell.accent().text,
            title,
            subtitle,
            playing: matches!(shell.state.status, Status::Playing | Status::Loading),
            can_play: track.is_some() || shell.startable_collection().is_some(),
            can_next: shell.state.queue.has_next,
            can_previous: track.is_some(),
        })
    }

    fn entries(state: &MenuState) -> Vec<(MenuAction, MenuItem)> {
        let item = |label: &'static str, icon: Option<&'static str>, enabled: bool| {
            let mut item = MenuItem::new(label);
            if let Some(icon) = icon {
                item = item.icon(icon);
            }
            item.look(if enabled { Look::Live } else { Look::Disabled })
        };
        vec![
            (MenuAction::Open, item(t!(TrayOpen), None, true)),
            if state.playing {
                (
                    MenuAction::TogglePlay,
                    item(t!(Pause), Some("pause-fill"), true),
                )
            } else {
                (
                    MenuAction::TogglePlay,
                    item(t!(Play), Some("play-fill"), state.can_play),
                )
            },
            (
                MenuAction::Next,
                item(t!(NextTrack), Some("skip-forward-fill"), state.can_next),
            ),
            (
                MenuAction::Previous,
                item(t!(PrevTrack), Some("skip-back-fill"), state.can_previous),
            ),
            (MenuAction::Quit, item(t!(TrayQuit), Some("x"), true)),
        ]
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.open, false) {
            return;
        }
        self.was_active = false;
        hide(&self.shell, self.hwnd, cx);
    }

    fn run(&mut self, action: MenuAction, cx: &mut Context<Self>) {
        self.close(cx);
        let _ = self.shell.update(cx, |shell, cx| match action {
            MenuAction::Open => shell.desk.send(Op::Show {
                front: true,
                at: Instant::now(),
                why: "tepsi menüsü",
            }),
            MenuAction::TogglePlay => shell.toggle_play(cx),
            MenuAction::Next => shell.next(cx),
            MenuAction::Previous => shell.previous(cx),
            MenuAction::Quit => shell.quit_app(cx),
        });
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state(cx) else { return };
        let entries = Self::entries(&state);
        let enabled: Vec<bool> = entries.iter().map(|(_, i)| i.look.enabled()).collect();
        let step = |from: Option<usize>, delta: isize| -> Option<usize> {
            let n = enabled.len() as isize;
            let mut i = from.map_or(if delta > 0 { -1 } else { n }, |i| i as isize);
            for _ in 0..n {
                i = (i + delta).rem_euclid(n);
                if enabled[i as usize] {
                    return Some(i as usize);
                }
            }
            None
        };
        match event.keystroke.key.as_str() {
            "escape" => self.close(cx),
            "down" | "tab" => self.selected = step(self.selected, 1),
            "up" => self.selected = step(self.selected, -1),
            "enter" | "space" => {
                if let Some(i) = self.selected.filter(|i| enabled[*i]) {
                    self.run(entries[i].0, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for TrayMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(state) = self.state(cx).filter(|_| self.open) else {
            return div().into_any_element();
        };
        let header = div()
            .h(px(MENU_HEADER_HEIGHT))
            .px(px(S2))
            .py(px(S2))
            .flex()
            .flex_col()
            // Uzun şarkı / sanatçı adı kısalırsa tamamı ipucunda.
            .child(
                div().flex().type_role(T_BODY).text_color(text()).child(
                    line_with_tip("tray-title", state.title.clone())
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
                        line_with_tip("tray-sub", state.subtitle.clone())
                            .flex()
                            .min_w_0(),
                    ),
            );
        let mut panel = components::menu_panel("tray-menu")
            .w(px(MENU_WIDTH))
            // Panelin içine tıklamak menüyü kapatmasın (dışı kapatır).
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(components::menu_separator());
        for (i, (action, item)) in Self::entries(&state).into_iter().enumerate() {
            if action == MenuAction::Quit {
                panel = panel.child(components::menu_separator());
            }
            let enabled = item.look.enabled();
            let look = if enabled && self.selected == Some(i) {
                Look::Focus
            } else {
                item.look
            };
            let el = components::menu_item(i, item.look(look), state.accent);
            panel = panel.child(if enabled {
                el.on_click(cx.listener(move |this, _, _, cx| this.run(action, cx)))
            } else {
                el
            });
        }
        div()
            .size_full()
            .p(px(SHADOW_ROOM))
            .font_family(UI_FONT)
            .text_color(text())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close(cx)),
            )
            .child(panel)
            .into_any_element()
    }
}

// ── Her yerden palet ────────────────────────────────────────────────────

/// Kısayola basıldı (Openify öndeyken değil): paleti kendi penceresinde,
/// öndeki pencerenin ekranında aç.
pub fn open_global_palette(shell: &WeakEntity<Shell>, at: Instant, cx: &mut AsyncApp) {
    let started = Instant::now();
    let previous = system::foreground_window();
    // Sınama koşusunda ana pencerenin ekranında (2. monitör): kullanıcının
    // ekranında açılmasın.
    let main = shell.update(cx, |s, _| s.desk.hwnd).ok().flatten();
    let anchor = if system::test_mode() { main } else { previous };
    let Some(monitor) = anchor
        .and_then(system::monitor_of)
        .or_else(|| system::monitor_at(None))
    else {
        return;
    };
    let s = monitor.scale;
    let (l, t, r, b) = monitor.work;
    let (work_x, work_y) = (l as f32 / s, t as f32 / s);
    let (work_w, work_h) = ((r - l) as f32 / s, (b - t) as f32 / s);
    let (w, h) = (
        PANEL_WIDTH + 2. * SHADOW_ROOM,
        PANEL_MAX_HEIGHT + 2. * SHADOW_ROOM,
    );
    let top = (work_y + work_h * PALETTE_TOP - SHADOW_ROOM)
        .min(work_y + work_h - h)
        .max(work_y);
    let bounds = Bounds {
        origin: point(px(work_x + (work_w - w) / 2.), px(top)),
        size: size(px(w), px(h)),
    };
    let display = display_id(monitor.index, cx);
    let Some(entity) = shell.upgrade() else {
        return;
    };
    let ready = shell.update(cx, |shell, cx| {
        shell.open_palette(cx);
        shell.palette.detached = true;
        shell.desk.return_focus = previous.filter(|p| Some(*p) != shell.desk.hwnd);
        cx.notify();
    });
    if ready.is_err() {
        return;
    }
    let existing = shell
        .update(cx, |s, _| s.desk.palette_window)
        .ok()
        .flatten();
    let handle = reuse_or_open(
        existing,
        |palette: &mut QuickPalette, window, cx| palette.show(at, window, cx),
        |cx| {
            cx.update(|cx| {
                cx.open_window(popup_options(bounds, display), |window, cx| {
                    cx.new(|cx| QuickPalette::new(entity, window, cx))
                })
            })?
        },
        cx,
    );
    let handle = match handle {
        Ok(handle) => handle,
        Err(e) => {
            log::warn!("Her yerden palet açılamadı: {e:#}");
            let _ = shell.update(cx, |s, cx| s.close_palette(cx));
            return;
        }
    };
    let _ = shell.update(cx, |s, _| s.desk.palette_window = Some(handle));
    let opened_ms = started.elapsed().as_secs_f64() * 1000.;
    let fronted = raise(handle, physical(bounds, s), cx);
    log::info!(
        "sistem: her yerden palet açıldı: kısayoldan {:.1} ms (pencere hazır {opened_ms:.1} ms), öne alındı: {fronted}",
        at.elapsed().as_secs_f64() * 1000.
    );
}

pub struct QuickPalette {
    shell: WeakEntity<Shell>,
    focus: FocusHandle,
    hwnd: Option<HWND>,
    /// Palet bu pencerede açık mı (kapanınca pencere gizlenir, silinmez).
    open: bool,
    /// Kısayolun geldiği an (ilk karenin gecikmesi kayda düşer).
    at: Instant,
    painted: bool,
    was_active: bool,
    _follow: Subscription,
    _activation: Subscription,
}

impl QuickPalette {
    fn new(shell: Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Palet her değiştiğinde yenilen; kapandıysa (Esc, Enter, Ctrl+K)
        // pencere gizlenir.
        let follow = cx.observe_in(&shell, window, |this, shell, window, cx| {
            let (open, wants_window) = {
                let s = shell.read(cx);
                (s.palette.open && s.palette.detached, s.palette.wants_window)
            };
            if open {
                if this.open {
                    cx.notify();
                }
            } else {
                this.closed(wants_window, window, cx);
            }
        });
        // Başka bir uygulamaya tıklandı: palet kapanır, odak orada kalır.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = this.open;
            } else if this.was_active {
                this.was_active = false;
                let _ = this.shell.update(cx, |s, cx| {
                    s.desk.return_focus = None;
                    s.close_palette(cx);
                });
            }
        });
        // Alt+F4: pencere silinmesin, palet kapansın (Esc gibi).
        let weak_shell = shell.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = weak_shell.update(cx, |s, cx| s.close_palette(cx));
            false
        });
        QuickPalette {
            shell: shell.downgrade(),
            focus: cx.focus_handle(),
            hwnd: windows_titlebar::hwnd(window),
            open: false,
            at: Instant::now(),
            painted: false,
            was_active: false,
            _follow: follow,
            _activation: activation,
        }
    }

    /// Açılışa hazırla (pencere henüz gizli; `raise` gösterir).
    fn show(&mut self, at: Instant, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.at = at;
        self.painted = false;
        self.was_active = false;
        window.focus(&self.focus);
        cx.notify();
    }

    fn closed(&mut self, wants_window: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.open, false) {
            return;
        }
        self.was_active = false;
        // Kullanıcı paletin içinden kapattıysa (Esc, Enter) odak önceki
        // uygulamaya döner; odak zaten başka yere gittiyse dokunulmaz.
        let active = window.is_window_active();
        hide(&self.shell, self.hwnd, cx);
        let _ = self.shell.update(cx, |shell, _| {
            shell.palette.detached = false;
            let back = shell.desk.return_focus.take();
            if wants_window {
                shell.desk.send(Op::Show {
                    front: true,
                    at: Instant::now(),
                    why: "palet",
                });
            } else if let (true, Some(hwnd)) = (active, back) {
                shell.desk.send(Op::Focus(hwnd));
            }
        });
    }
}

impl Render for QuickPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = if self.open {
            self.shell
                .update(cx, |shell, cx| shell.render_palette_panel(cx))
                .ok()
        } else {
            None
        };
        // Açılıştan sonraki ilk kare: kısayoldan buraya kadar geçen süre kayda düşer.
        let first = panel.is_some() && !std::mem::replace(&mut self.painted, true);
        let at = self.at;
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(SHADOW_ROOM))
            .font_family(UI_FONT)
            .text_color(text())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let _ = this.shell.update(cx, |s, cx| s.palette_key(event, cx));
                cx.stop_propagation();
            }))
            // Panelin dışına (saydam paya) tıklamak paleti kapatır.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let _ = this.shell.update(cx, |s, cx| s.close_palette(cx));
                }),
            )
            .children(panel)
            .when(first, |el| {
                el.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, _, _| {
                            log::info!(
                                "sistem: her yerden palet ilk karesi: kısayoldan {:.1} ms",
                                at.elapsed().as_secs_f64() * 1000.
                            )
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
    }
}
