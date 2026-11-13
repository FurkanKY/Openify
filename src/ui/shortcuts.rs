//! Klavye kısayolları: tek tablo + yardım katmanı.
//!
//! `Shortcut`, uygulamanın bağlı kısayollarının TEK kaynağıdır: tuş tutamları
//! (`Chord`) burada tanımlanır, bütün tuş işleyicileri (`on_key_down`,
//! `now_key`, `manage_key`, `settings_key`) tuşu `Shortcut::of` ile çözüp onunla
//! eşleşir; yardım katmanı (`?` / Ctrl+/ ya da palette "Kısayollar") aynı tablodan
//! çizilir. Böylece ekranda yazan tuş, çalışan tuştur. Tabloya yeni bir eylem
//! eklemek derleme zorunluluğudur (`label` ve `group` kapsamlı `match`), bir eylemi
//! işleyicide kullanmamak ise bir testle yakalanır (`every_shortcut_is_used_by_a_handler`).
//!
//! Katman: Esc ya da `?` kapatır, odak açıldığı yere döner; ↑ ↓ / Page Up / Page
//! Down / Home / End kaydırır.

use gpui::{
    AnyElement, Context, FocusHandle, MouseButton, ScrollHandle, Window, deferred, div,
    linear_color_stop, linear_gradient, prelude::*, px, rgb,
};

use super::{
    Shell, TITLEBAR_HEIGHT,
    components::{Look, Tooltip, icon_button, kbd},
    theme::*,
};
use crate::t;

/// Bir tuş tutamı: gpui'nin tuş adı + tam değiştirici durumu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub key: &'static str,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

const fn key(key: &'static str) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        alt: false,
    }
}
const fn ctrl(key: &'static str) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: false,
        alt: false,
    }
}
const fn shift(key: &'static str) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: true,
        alt: false,
    }
}
const fn alt(key: &'static str) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: false,
        alt: true,
    }
}
const fn ctrl_shift(key: &'static str) -> Chord {
    Chord {
        key,
        ctrl: true,
        shift: true,
        alt: false,
    }
}
const fn alt_shift(key: &'static str) -> Chord {
    Chord {
        key,
        ctrl: false,
        shift: true,
        alt: true,
    }
}

impl Chord {
    /// Ekranda görünen biçim: "Ctrl Shift ↓". Boşluk tuşu yerelleşir.
    pub fn display(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        parts.push(match self.key {
            "left" => "←".into(),
            "right" => "→".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "space" => t!(KeySpace).into(),
            "delete" => "Del".into(),
            "escape" => "Esc".into(),
            "enter" => "Enter".into(),
            "home" => "Home".into(),
            k if k.len() == 1 => k.to_uppercase(),
            k if k.starts_with('f') && k[1..].parse::<u8>().is_ok() => k.to_uppercase(),
            k => k.to_string(),
        });
        parts.join(" ")
    }
}

/// Eylemin tuşu nasıl yakalanıyor?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum How {
    /// `Shortcut::of` ile, tuş + değiştiriciyle.
    Keys,
    /// Yazılan karakterle (`?`: klavye düzenine göre başka tuşta): bkz. `is_help_key`.
    Char,
    /// İşletim sistemi düzeyinde (uygulama odakta olmasa da), bkz. `system::hook`.
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Playback,
    Panels,
    Navigate,
    Lists,
}

/// Bağlı kısayollar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shortcut {
    PlayPause,
    Next,
    Previous,
    SeekForward,
    SeekBack,
    VolumeUp,
    VolumeDown,
    Mute,
    Shuffle,
    Repeat,
    QueuePanel,
    LyricsPanel,
    FullScreen,
    Search,
    GlobalSearch,
    PasteLink,
    Back,
    Forward,
    GoHome,
    TabHome,
    TabExplore,
    TabLibrary,
    Settings,
    Help,
    Find,
    SelectAll,
    Undo,
    Delete,
    Rename,
    RowMenu,
}

struct Entry {
    id: Shortcut,
    group: Group,
    how: How,
    chords: &'static [Chord],
}

const fn entry(id: Shortcut, group: Group, how: How, chords: &'static [Chord]) -> Entry {
    Entry {
        id,
        group,
        how,
        chords,
    }
}

/// Tablo; yardım katmanındaki sıra da budur.
const TABLE: &[Entry] = &[
    entry(
        Shortcut::PlayPause,
        Group::Playback,
        How::Keys,
        &[key("space")],
    ),
    entry(Shortcut::Next, Group::Playback, How::Keys, &[ctrl("right")]),
    entry(
        Shortcut::Previous,
        Group::Playback,
        How::Keys,
        &[ctrl("left")],
    ),
    entry(
        Shortcut::SeekForward,
        Group::Playback,
        How::Keys,
        &[shift("right")],
    ),
    entry(
        Shortcut::SeekBack,
        Group::Playback,
        How::Keys,
        &[shift("left")],
    ),
    entry(
        Shortcut::VolumeUp,
        Group::Playback,
        How::Keys,
        &[ctrl("up")],
    ),
    entry(
        Shortcut::VolumeDown,
        Group::Playback,
        How::Keys,
        &[ctrl("down")],
    ),
    entry(
        Shortcut::Mute,
        Group::Playback,
        How::Keys,
        &[ctrl_shift("down")],
    ),
    entry(Shortcut::Shuffle, Group::Playback, How::Keys, &[alt("s")]),
    entry(Shortcut::Repeat, Group::Playback, How::Keys, &[alt("r")]),
    entry(
        Shortcut::QueuePanel,
        Group::Panels,
        How::Keys,
        &[alt_shift("q")],
    ),
    entry(
        Shortcut::LyricsPanel,
        Group::Panels,
        How::Keys,
        &[alt_shift("l")],
    ),
    entry(
        Shortcut::FullScreen,
        Group::Panels,
        How::Keys,
        &[key("f11")],
    ),
    entry(
        Shortcut::Help,
        Group::Panels,
        How::Char,
        &[key("?"), ctrl("/")],
    ),
    entry(Shortcut::Search, Group::Navigate, How::Keys, &[ctrl("k")]),
    entry(
        Shortcut::GlobalSearch,
        Group::Navigate,
        How::System,
        &[ctrl_shift("space")],
    ),
    entry(
        Shortcut::PasteLink,
        Group::Navigate,
        How::Keys,
        &[ctrl("v")],
    ),
    entry(Shortcut::Back, Group::Navigate, How::Keys, &[alt("left")]),
    entry(
        Shortcut::Forward,
        Group::Navigate,
        How::Keys,
        &[alt("right")],
    ),
    entry(Shortcut::GoHome, Group::Navigate, How::Keys, &[alt("home")]),
    entry(Shortcut::TabHome, Group::Navigate, How::Keys, &[ctrl("1")]),
    entry(
        Shortcut::TabExplore,
        Group::Navigate,
        How::Keys,
        &[ctrl("2")],
    ),
    entry(
        Shortcut::TabLibrary,
        Group::Navigate,
        How::Keys,
        &[ctrl("3")],
    ),
    entry(Shortcut::Settings, Group::Navigate, How::Keys, &[ctrl(",")]),
    entry(
        Shortcut::Find,
        Group::Lists,
        How::Keys,
        &[ctrl("f"), key("f3")],
    ),
    entry(Shortcut::SelectAll, Group::Lists, How::Keys, &[ctrl("a")]),
    entry(Shortcut::Undo, Group::Lists, How::Keys, &[ctrl("z")]),
    entry(Shortcut::Delete, Group::Lists, How::Keys, &[key("delete")]),
    entry(Shortcut::Rename, Group::Lists, How::Keys, &[key("f2")]),
    entry(Shortcut::RowMenu, Group::Lists, How::Keys, &[shift("f10")]),
];

impl Shortcut {
    /// Bütün kısayollar, yardım katmanındaki sırayla.
    pub fn all() -> impl Iterator<Item = Shortcut> {
        TABLE.iter().map(|e| e.id)
    }

    fn entry(self) -> &'static Entry {
        TABLE
            .iter()
            .find(|e| e.id == self)
            .expect("her kısayol tabloda")
    }

    pub fn chords(self) -> &'static [Chord] {
        self.entry().chords
    }

    pub fn group(self) -> Group {
        self.entry().group
    }

    #[cfg(test)]
    pub fn how(self) -> How {
        self.entry().how
    }

    /// Tuş + değiştirici → bağlı kısayol (değiştiriciler TAM eşleşir: Ctrl+K
    /// ile Ctrl+Shift+K farklı şeydir). Yalnız `How::Keys` olanlar çözülür.
    pub fn of(key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Shortcut> {
        TABLE
            .iter()
            .filter(|e| e.how == How::Keys)
            .find(|e| {
                e.chords
                    .iter()
                    .any(|c| c.key == key && c.ctrl == ctrl && c.shift == shift && c.alt == alt)
            })
            .map(|e| e.id)
    }

    /// Bir gpui tuş vuruşunun kısayolu (`?` / Ctrl+/ dahil).
    pub fn from_keystroke(ks: &gpui::Keystroke) -> Option<Shortcut> {
        let m = &ks.modifiers;
        if is_help_key(&ks.key, ks.key_char.as_deref(), m.control, m.alt) {
            return Some(Shortcut::Help);
        }
        Self::of(&ks.key, m.control, m.shift, m.alt)
    }

    /// Yardım katmanında görünen ad (mevcut kısayol metinleri).
    pub fn label(self) -> &'static str {
        match self {
            Shortcut::PlayPause => t!(ShPlayPause),
            Shortcut::Next => t!(CmdNext),
            Shortcut::Previous => t!(CmdPrevious),
            Shortcut::SeekForward => t!(ShSeekFwd),
            Shortcut::SeekBack => t!(ShSeekBack),
            Shortcut::VolumeUp => t!(ShVolUp),
            Shortcut::VolumeDown => t!(ShVolDown),
            Shortcut::Mute => t!(ShMute),
            Shortcut::Shuffle => t!(ShShuffle),
            Shortcut::Repeat => t!(ShRepeat),
            Shortcut::QueuePanel => t!(ShQueuePanel),
            Shortcut::LyricsPanel => t!(ShLyricsPanel),
            Shortcut::FullScreen => t!(ShFullScreen),
            Shortcut::Search => t!(ShSearch),
            Shortcut::GlobalSearch => t!(ShGlobalSearch),
            Shortcut::PasteLink => t!(ShPasteLink),
            Shortcut::Back => t!(ShBack),
            Shortcut::Forward => t!(ShForward),
            Shortcut::GoHome => t!(NavHome),
            Shortcut::TabHome => t!(ShTabHome),
            Shortcut::TabExplore => t!(ShTabExplore),
            Shortcut::TabLibrary => t!(ShTabLibrary),
            Shortcut::Settings => t!(Settings),
            Shortcut::Help => t!(ShHelp),
            Shortcut::Find => t!(ShFind),
            Shortcut::SelectAll => t!(ShSelectAll),
            Shortcut::Undo => t!(ShUndo),
            Shortcut::Delete => t!(ShDelete),
            Shortcut::Rename => t!(ShRename),
            Shortcut::RowMenu => t!(ShContextMenu),
        }
    }

    /// Ekranda gösterilen tuş yazıları, tutam başına.
    pub fn keys_text(self) -> Vec<String> {
        self.chords().iter().map(Chord::display).collect()
    }
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Playback => t!(ShGroupPlayback),
            Group::Panels => t!(ShGroupPanels),
            Group::Navigate => t!(ShGroupNavigate),
            Group::Lists => t!(ShGroupLists),
        }
    }
}

/// `?` ya da Ctrl+/ mi? `?` bazı klavye düzenlerinde başka bir tuşta
/// (Türkçe Q'da Shift+*), bu yüzden yazılan karaktere (`key_char`) bakılır.
pub fn is_help_key(key: &str, key_char: Option<&str>, ctrl: bool, alt: bool) -> bool {
    if alt {
        return false;
    }
    if ctrl {
        return key == "/" || key_char == Some("/");
    }
    key_char == Some("?")
}

/// Bağlama göre değişen (tek bir eyleme bağlanmayan) tuşlar: ekran yalnız
/// anlatır, işleyiciler yerel (`nav_key`, `shelf_key`, `queue_key`, `menu_key`…).
pub fn context_keys() -> Vec<(String, &'static str)> {
    vec![
        ("↑ ↓".into(), t!(ShSelectSong)),
        ("Enter".into(), t!(ShEnter)),
        ("Tab".into(), t!(ShTab)),
        ("← →".into(), t!(ShArrows)),
        ("Home End".into(), t!(ShHomeEnd)),
        ("Alt ↑ ↓".into(), t!(ShMoveInQueue)),
        (t!(KeyCtrlClick).into(), t!(ShMultiSelect)),
        (t!(NavLibrary).into(), t!(ShTypeFilter)),
        ("Esc".into(), t!(ShEsc)),
    ]
}

// ── Yardım katmanı ──────────────────────────────────────────────────────

/// Katmanın ekran durumu (`SettingsUi::help`).
#[derive(Default)]
pub struct HelpState {
    pub open: bool,
    /// Açılırken odakta olan öğe: kapanınca odak ona döner.
    restore: Option<FocusHandle>,
    scroll: ScrollHandle,
}

/// Katmanın en geniş eni (iki sütun) ve tek sütundaki eni.
const HELP_WIDTH_WIDE: f32 = 880.;
const HELP_WIDTH_NARROW: f32 = 520.;
/// Bu pencere eninden itibaren iki sütun.
const HELP_TWO_COLUMNS_FROM: f32 = 1000.;

impl Shell {
    /// Katmanı aç. Tam ekranda katmanlar çizilmez: açılmaz.
    pub(super) fn open_help(&mut self, window: Option<&Window>, cx: &mut Context<Self>) {
        if self.now.full.open || self.sui.help.open {
            return;
        }
        self.sui.close_menus();
        self.manage.menu = None;
        self.sui.help.restore = window.and_then(|w| w.focused(cx));
        self.sui.help.open = true;
        self.sui.help.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    /// Katmanı kapat; odak açıldığı yere (yoksa kök çapaya) döner.
    pub(super) fn close_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sui.help.open {
            return;
        }
        self.sui.help.open = false;
        match self.sui.help.restore.take() {
            Some(handle) if self.keyboard_focus => window.focus(&handle),
            _ => window.focus(&self.focus),
        }
        cx.notify();
    }

    /// Katman açıkken tuşlar: Esc / `?` kapatır, oklar kaydırır, gerisi yutulur
    /// (arkadaki sayfa tuşları işlemesin). İşlendiyse `true` (hep).
    pub(super) fn help_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &event.keystroke;
        let handle = self.sui.help.scroll.clone();
        let page = f32::from(handle.bounds().size.height) * 0.8;
        let scroll_by = |dy: f32| {
            let offset = handle.offset();
            let max = f32::from(handle.max_offset().height);
            let y = (f32::from(offset.y) - dy).clamp(-max, 0.);
            handle.set_offset(gpui::point(offset.x, px(y)));
        };
        match ks.key.as_str() {
            "escape" => self.close_help(window, cx),
            "up" => scroll_by(-40.),
            "down" => scroll_by(40.),
            "pageup" => scroll_by(-page),
            "pagedown" => scroll_by(page),
            "home" => scroll_by(-1.0e6),
            "end" => scroll_by(1.0e6),
            _ if Shortcut::from_keystroke(ks) == Some(Shortcut::Help) => {
                self.close_help(window, cx)
            }
            _ => {}
        }
        cx.notify();
        true
    }

    /// Katman: pencereyi karartır, ortada kısayol listesi. Tekerlek ve tıklama
    /// arkadaki sayfaya geçmez; dışarı tıklamak kapatır.
    pub(super) fn render_help(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let wide = vw >= HELP_TWO_COLUMNS_FROM;
        let width = if wide {
            HELP_WIDTH_WIDE
        } else {
            HELP_WIDTH_NARROW
        }
        .min(vw - 2. * S4);
        let max_h = (vh - 2. * S6).max(S8 * 3.);

        let block = |title: &'static str, rows: Vec<(Vec<String>, &'static str)>| -> AnyElement {
            div()
                .flex()
                .flex_col()
                .pb(px(S5))
                .child(
                    div()
                        .pb(px(S1))
                        .type_role(T_BODY)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(text())
                        .child(title),
                )
                .children(rows.into_iter().map(|(keys, what)| help_row(keys, what)))
                .into_any_element()
        };
        let group_rows = |group: Group| -> Vec<(Vec<String>, &'static str)> {
            Shortcut::all()
                .filter(|s| s.group() == group)
                .map(|s| (s.keys_text(), s.label()))
                .collect()
        };
        let playback = block(Group::Playback.title(), group_rows(Group::Playback));
        let panels = block(Group::Panels.title(), group_rows(Group::Panels));
        let navigate = block(Group::Navigate.title(), group_rows(Group::Navigate));
        let lists = block(Group::Lists.title(), group_rows(Group::Lists));
        let around = block(
            t!(ShGroupContext),
            context_keys()
                .into_iter()
                .map(|(k, w)| (vec![k], w))
                .collect(),
        );

        let column = || div().flex_1().min_w_0().flex().flex_col();
        let body = (if wide {
            div()
                .flex()
                .gap(px(S6))
                .child(column().child(playback).child(around))
                .child(column().child(navigate).child(panels).child(lists))
        } else {
            div()
                .flex()
                .flex_col()
                .child(playback)
                .child(panels)
                .child(navigate)
                .child(lists)
                .child(around)
        })
        // Son satırın altında nefes payı: kaydırmanın sonunda harf yarım kalmaz.
        .pb(px(S4));

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .pb(px(S3))
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(gpui::FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(t!(Shortcuts)),
            )
            .child(
                icon_button("help-close", "x", Look::Live, accent)
                    .tooltip(Tooltip::text(t!(Close), Some("Esc")))
                    .on_click(cx.listener(|this, _, window, cx| this.close_help(window, cx))),
            );

        let scroll = div()
            .id("help-scroll")
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.sui.help.scroll)
            .child(body);

        // Kaydırma ipucu: üst ve alt kenarda panel yüzeyinden saydama geçiş;
        // yalnız o yönde içerik kaldıysa (güç, gizli içeriğe göre 32 px'e kadar
        // doğrusal). Tıklama ve tekerlek altındaki satıra geçer.
        let handle = &self.sui.help.scroll;
        let (top_strength, bottom_strength) = help_fade_strengths(
            f32::from(handle.offset().y),
            f32::from(handle.max_offset().height),
        );
        // Yerleşim henüz ölçülmediyse (katman yeni açıldı) bir sonraki karede
        // yeniden çiz: alt geçiş ilk karede eksik kalmasın.
        if f32::from(handle.bounds().size.height) <= 0. {
            let this = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = this.update(cx, |_, cx| cx.notify());
            });
        }
        let scroll_area = div()
            .relative()
            .min_h_0()
            .flex()
            .flex_col()
            .child(scroll)
            .children(help_veil(true, top_strength))
            .children(help_veil(false, bottom_strength));

        let panel = div()
            .id("help")
            .occlude()
            .w(px(width))
            .max_h(px(max_h))
            .p(px(S5))
            .pb(px(S2))
            .flex()
            .flex_col()
            .rounded(px(R_XL))
            .bg(rgb(palette().bar))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow(float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(scroll_area);
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(shade(SCRIM))
                .flex()
                .items_center()
                .justify_center()
                // Başlık çubuğunun altını kaplayan, olayları yutan örtü: arkadaki
                // sayfa (üzerine gelme, tıklama) katmanın içinden tepki vermez.
                // Dışarı tıklamak kapatır.
                .child(
                    div()
                        .absolute()
                        .top(px(TITLEBAR_HEIGHT))
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.close_help(window, cx)),
                        ),
                )
                .child(panel),
        )
        .with_priority(4)
        .into_any_element()
    }
}

/// Kaydırma geçişlerinin gücü (üst, alt; 0..1): en üstteyken üst, en alttayken
/// alt yok; içerik sığıyorsa ikisi de yok (lyrics paneliyle aynı kural).
/// `offset`: kaydırma konumu (eksi ya da 0), `max`: en büyük kaydırma.
fn help_fade_strengths(offset: f32, max: f32) -> (f32, f32) {
    (
        (-offset / S6).clamp(0., 1.),
        ((max + offset) / S6).clamp(0., 1.),
    )
}

/// Kaydırma alanının üst ya da alt kenarındaki 32 px'lik geçiş: panel
/// yüzeyinden (`palette().bar`) saydama. `strength` 0..1; ~0 ise hiç öğe yok. Köşeleri
/// yuvarlak değil (yuvarlak köşe alt köşede satırın bir kırıntısını açıkta
/// bırakıyordu). Hiçbir olaya bağlanmaz: tıklama ve tekerlek satıra geçer.
fn help_veil(top: bool, strength: f32) -> Option<AnyElement> {
    let strength = strength.clamp(0., 1.);
    if strength < 0.01 {
        return None;
    }
    let solid = linear_color_stop(
        with_alpha(palette().bar, strength),
        if top { 0. } else { 1. },
    );
    let clear = linear_color_stop(with_alpha(palette().bar, 0.), if top { 1. } else { 0. });
    let (from, to) = if top { (solid, clear) } else { (clear, solid) };
    let veil = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(S6))
        .bg(linear_gradient(180., from, to));
    Some(if top { veil.top_0() } else { veil.bottom_0() }.into_any_element())
}

/// Bir kısayol satırı: solda ne yaptığı, sağda tuşlar (her tuş bir etiket;
/// birden çok tutam "/" ile ayrılır).
fn help_row(chords: Vec<String>, what: &'static str) -> AnyElement {
    let mut keys = div().flex_none().flex().items_center().gap(px(S1));
    for (i, chord) in chords.iter().enumerate() {
        if i > 0 {
            keys = keys.child(
                div()
                    .px(px(S1))
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child("/"),
            );
        }
        keys = keys.children(
            chord
                .split(' ')
                .filter(|k| !k.is_empty())
                .map(|k| kbd(k.to_string())),
        );
    }
    div()
        .min_h(px(32.))
        .py(px(S1))
        .flex()
        .items_center()
        .gap(px(S4))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .type_role(T_SECONDARY)
                .text_color(muted())
                .child(what),
        )
        .child(keys)
        .into_any_element()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_fades_show_only_where_content_remains() {
        // İçerik sığıyor: geçiş yok.
        assert_eq!(help_fade_strengths(0., 0.), (0., 0.));
        // En üstte: yalnız alt (çok içerik kaldı → tam güç).
        assert_eq!(help_fade_strengths(0., 400.), (0., 1.));
        // Ortada: ikisi de tam.
        assert_eq!(help_fade_strengths(-200., 400.), (1., 1.));
        // En altta: yalnız üst (End sonrası).
        assert_eq!(help_fade_strengths(-400., 400.), (1., 0.));
        // Kenara yaklaşırken doğrusal söner (32 px).
        let (_, bottom) = help_fade_strengths(-390., 400.);
        assert!((bottom - 10. / S6).abs() < 1e-6);
        let (top, _) = help_fade_strengths(-16., 400.);
        assert!((top - 0.5).abs() < 1e-6);
        // Taşma payı olmadan aşırı kaydırma da güvenli.
        assert_eq!(help_fade_strengths(-500., 400.), (1., 0.));
    }

    #[test]
    fn every_shortcut_has_a_unique_set_of_chords() {
        let mut seen: Vec<(Chord, Shortcut)> = Vec::new();
        for s in Shortcut::all() {
            assert!(!s.chords().is_empty(), "{s:?}");
            for c in s.chords() {
                assert!(
                    seen.iter().all(|(other, _)| other != c),
                    "{s:?}: {c:?} başka bir eylemde de bağlı"
                );
                seen.push((*c, s));
            }
        }
        assert_eq!(Shortcut::all().count(), TABLE.len());
    }

    #[test]
    fn of_round_trips_every_key_chord_and_demands_exact_modifiers() {
        for s in Shortcut::all().filter(|s| s.how() == How::Keys) {
            for c in s.chords() {
                assert_eq!(
                    Shortcut::of(c.key, c.ctrl, c.shift, c.alt),
                    Some(s),
                    "{s:?} {c:?}"
                );
                // Ek bir değiştirici başka bir şeye dönüşmemeli (bu tuşta bağlı değilse).
                let extra = Shortcut::of(c.key, c.ctrl, !c.shift, c.alt);
                assert!(
                    extra.is_none() || extra != Some(s),
                    "{s:?}: Shift farkı aynı eylemi vermemeli"
                );
            }
        }
        assert_eq!(
            Shortcut::of("k", true, false, false),
            Some(Shortcut::Search)
        );
        assert_eq!(
            Shortcut::of("k", true, true, false),
            None,
            "Ctrl+Shift+K bağlı değil"
        );
        assert_eq!(Shortcut::of("k", false, false, false), None);
        assert_eq!(
            Shortcut::of("space", false, false, false),
            Some(Shortcut::PlayPause)
        );
        assert_eq!(
            Shortcut::of("left", true, false, false),
            Some(Shortcut::Previous)
        );
        assert_eq!(
            Shortcut::of("left", false, true, false),
            Some(Shortcut::SeekBack)
        );
        assert_eq!(
            Shortcut::of("left", false, false, true),
            Some(Shortcut::Back)
        );
        assert_eq!(
            Shortcut::of("down", true, true, false),
            Some(Shortcut::Mute)
        );
        assert_eq!(
            Shortcut::of("f3", false, false, false),
            Some(Shortcut::Find)
        );
    }

    #[test]
    fn global_hotkey_and_help_are_not_resolved_by_key_name() {
        // Her yerden arama işletim sistemi düzeyinde; yardım yazılan karakterle.
        assert_eq!(Shortcut::of("space", true, true, false), None);
        assert_eq!(Shortcut::of("?", false, false, false), None);
        assert_eq!(Shortcut::GlobalSearch.how(), How::System);
        assert_eq!(Shortcut::Help.how(), How::Char);
    }

    #[test]
    fn help_key_works_on_any_layout() {
        // ABD: Shift+/ → key "/", key_char "?".
        assert!(is_help_key("/", Some("?"), false, false));
        // Türkçe Q: ? başka tuşta ("-" tuşu, Shift), yine key_char "?".
        assert!(is_help_key("-", Some("?"), false, false));
        assert!(is_help_key("/", Some("/"), true, false), "Ctrl+/");
        assert!(is_help_key("/", None, true, false), "Ctrl+/ (karakter yok)");
        assert!(
            !is_help_key("/", Some("/"), false, false),
            "çıplak / yardım değil"
        );
        assert!(
            !is_help_key("/", Some("?"), false, true),
            "Alt basılıyken yok"
        );
        assert!(!is_help_key("k", Some("k"), false, false));
    }

    #[test]
    fn chord_display_reads_like_the_old_palette_list() {
        let shown = |s: Shortcut| s.keys_text();
        assert_eq!(shown(Shortcut::Mute), ["Ctrl Shift ↓"]);
        assert_eq!(shown(Shortcut::Next), ["Ctrl →"]);
        assert_eq!(shown(Shortcut::Shuffle), ["Alt S"]);
        assert_eq!(shown(Shortcut::QueuePanel), ["Alt Shift Q"]);
        assert_eq!(shown(Shortcut::FullScreen), ["F11"]);
        assert_eq!(shown(Shortcut::Find), ["Ctrl F", "F3"]);
        assert_eq!(shown(Shortcut::Delete), ["Del"]);
        assert_eq!(shown(Shortcut::Settings), ["Ctrl ,"]);
        assert_eq!(shown(Shortcut::GoHome), ["Alt Home"]);
        assert_eq!(shown(Shortcut::TabLibrary), ["Ctrl 3"]);
        assert_eq!(shown(Shortcut::RowMenu), ["Shift F10"]);
        assert_eq!(shown(Shortcut::Help), ["?", "Ctrl /"]);
    }

    /// Tabloya eklenip hiçbir işleyicide kullanılmayan ("hayalet") kısayol,
    /// yardımda görünür ama çalışmaz: kaynakta aranır.
    #[test]
    fn every_shortcut_is_used_by_a_handler() {
        let handlers = [
            include_str!("mod.rs"),
            include_str!("manage.rs"),
            include_str!("now_playing.rs"),
            include_str!("settings.rs"),
            include_str!("palette.rs"),
        ]
        .join("\n");
        for s in Shortcut::all() {
            if s.how() == How::System {
                continue;
            }
            let needle = format!("Shortcut::{s:?}");
            assert!(
                handlers.contains(&needle),
                "{needle} hiçbir tuş işleyicisinde kullanılmıyor"
            );
        }
    }

    /// Sistem düzeyindeki kısayol gerçekten kaydediliyor.
    #[test]
    fn the_global_hotkey_is_registered_with_the_chord_the_help_shows() {
        let hook = include_str!("../system/hook.rs");
        assert!(
            hook.contains("VK_SPACE") && hook.contains("MOD_CONTROL") && hook.contains("MOD_SHIFT"),
            "Ctrl+Shift+Boşluk kaydı bulunamadı (yardım bunu gösteriyor)"
        );
    }

    #[test]
    fn labels_groups_and_context_rows_are_filled() {
        for s in Shortcut::all() {
            assert!(!s.label().is_empty(), "{s:?}");
            assert!(!s.group().title().is_empty());
        }
        assert!(
            context_keys()
                .iter()
                .all(|(k, w)| !k.is_empty() && !w.is_empty())
        );
    }
}
