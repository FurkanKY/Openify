//! Şarkı künyesi penceresi: şarkı satırının sağ tık menüsündeki ve "Şimdi
//! çalıyor" panelindeki "Künyeyi göster" ile açılır. Veri `engine::credits`'ten
//! gelir (Spotify'ın künye ucu, salt okuma, gayriresmî; canlı yanıtla
//! doğrulanmadı).
//!
//! Görünüm: pencereyi karartan ortada küçük bir kart (kısayol yardımı ve onay
//! kutusuyla aynı katman düzeni). Başlıkta "Künye", altında şarkının adı ve
//! sanatçısı. Gövde iki sütunlu bir liste: solda rolün adı (Seslendiren, Yazan,
//! Yapımcı, sonra Spotify'ın verdiği başka roller; sonda Kaynak), sağda adlar.
//! Başlık ya da küçük başlıklı bölüm yok: satırın kendisi etiketi taşır.
//! Sanatçı adresi olan adlar tıklanır (sanatçı sayfası açılır, pencere
//! kapanır); adresi olmayan ad düz yazıdır.
//!
//! Durumlar tek satırdır: yükleniyor, künye yok, alınamadı (hata türüne göre
//! kısa bir cümle).
//!
//! Klavye: Esc kapatır; ↑ ↓ / Tab / Shift+Tab tıklanabilir adlar arasında
//! gezer, Enter sanatçıyı açar; öbür tuşlar yutulur (arkadaki sayfa işlemez).
//! Kapanınca odak açıldığı yere döner (yardım katmanı gibi).
//!
//! Sınama kancaları (yalnız sınama koşusunda): `OPENIFY_TEST_CREDITS_JSON=<dosya>`
//! künyeyi dosyadan okur (`engine/fixtures/track_credits.json`), ağa hiç
//! çıkmaz; dosya yerine `!error` / `!busy` / `!denied` hata hâllerini,
//! `!loading` yükleniyor hâlini, `!empty` künyesiz hâli gösterir.
//! `OPENIFY_TEST_VIEW=credits` çalan şarkının künyesini açık getirir.

use std::sync::{Arc, OnceLock};

use gpui::{
    AnyElement, Context, FocusHandle, FontWeight, KeyDownEvent, MouseButton, ScrollHandle, Window,
    deferred, div, prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{Look, Tooltip, icon_button},
    history::Route,
    now_playing::Panel,
    theme::*,
};
use crate::engine::{self, CreditRoleKind, Credits, CreditsFailure, LibraryCommand};
use crate::t;

/// Kartın eni (dar pencerede pencereden 2 × 16 px dar).
const CARD_WIDTH: f32 = 460.;
/// Rol adı sütununun eni.
const LABEL_WIDTH: f32 = 112.;

/// Künyenin ekrandaki durumu.
pub(super) enum Status {
    Loading,
    Ready(Arc<Credits>),
    Failed(CreditsFailure),
}

/// Penceresinin durumu (`SettingsUi::credits`).
pub struct CreditsUi {
    pub open: bool,
    /// Künyesi istenen şarkı (gelen cevap başka şarkınınsa atılır).
    uri: String,
    /// Başlıkta gösterilen, bilinen şarkı adı ve sanatçıları.
    name: String,
    artists: String,
    status: Status,
    /// Klavyeyle gezilen tıklanabilir ad (sırası: roller sırasıyla).
    cursor: Option<usize>,
    /// Açılırken odakta olan öğe: kapanınca odak ona döner.
    restore: Option<FocusHandle>,
    scroll: ScrollHandle,
}

impl CreditsUi {
    pub fn new() -> Self {
        Self {
            open: false,
            uri: String::new(),
            name: String::new(),
            artists: String::new(),
            status: Status::Loading,
            cursor: None,
            restore: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// Tıklanabilir adların adresleri, gösterim sırasıyla.
    fn links(&self) -> Vec<String> {
        match &self.status {
            Status::Ready(c) => c
                .roles
                .iter()
                .flat_map(|r| &r.people)
                .filter_map(|p| p.uri.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Sınama koşusu mu? (Kanca gerçek hesaba dokunamasın.)
fn test_run() -> bool {
    std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
        || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some()
}

/// `OPENIFY_TEST_CREDITS_JSON` (bkz. modül başlığı): kancanın durumu; yoksa `None`.
fn fixture() -> Option<Status> {
    static SPEC: OnceLock<Option<String>> = OnceLock::new();
    let spec = SPEC
        .get_or_init(|| {
            std::env::var("OPENIFY_TEST_CREDITS_JSON")
                .ok()
                .filter(|_| test_run())
        })
        .as_deref()?;
    Some(match spec {
        "!error" => Status::Failed(CreditsFailure::Offline),
        "!busy" => Status::Failed(CreditsFailure::RateLimited),
        "!denied" => Status::Failed(CreditsFailure::Denied),
        "!loading" => Status::Loading,
        "!empty" => Status::Ready(Arc::new(Credits::default())),
        path => match std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(value) => Status::Ready(Arc::new(engine::parse_credits(&value))),
            None => {
                log::warn!("OPENIFY_TEST_CREDITS_JSON okunamadı: {path}");
                Status::Failed(CreditsFailure::Unavailable)
            }
        },
    })
}

/// Hatanın tek satırlık metni.
fn failure_text(why: CreditsFailure) -> &'static str {
    match why {
        CreditsFailure::RateLimited => t!(CreditsFailedBusy),
        CreditsFailure::Denied => t!(CreditsFailedDenied),
        CreditsFailure::Offline | CreditsFailure::Unavailable => t!(CreditsFailedOffline),
    }
}

/// Rolün ekranda görünen adı: tanınan üçü arayüz dilinde, gerisi Spotify'ın başlığı.
fn role_label(kind: CreditRoleKind, title: &str) -> String {
    match kind {
        CreditRoleKind::Performer => t!(CreditsPerformers).to_string(),
        CreditRoleKind::Writer => t!(CreditsWriters).to_string(),
        CreditRoleKind::Producer => t!(CreditsProducers).to_string(),
        CreditRoleKind::Other => match title.trim().to_lowercase().as_str() {
            "mixing engineer" | "mixer" => t!(CreditsMixing).to_string(),
            "mastering engineer" => t!(CreditsMastering).to_string(),
            "engineer" | "recording engineer" => t!(CreditsRecording).to_string(),
            "composer" => t!(CreditsComposer).to_string(),
            "lyricist" => t!(CreditsLyricist).to_string(),
            "arranger" => t!(CreditsArranger).to_string(),
            _ => title.to_string(),
        },
    }
}

impl Shell {
    /// Pencereyi aç ve künyeyi iste. Tam ekranda katmanlar çizilmez: açılmaz.
    /// `window`: odak açıldığı yere dönsün diye (yoksa kök çapaya).
    pub(super) fn open_credits(
        &mut self,
        uri: String,
        name: String,
        artists: String,
        window: Option<&Window>,
        cx: &mut Context<Self>,
    ) {
        if self.now.full.open || !uri.starts_with("spotify:track:") {
            return;
        }
        self.sui.close_menus();
        self.manage.menu = None;
        let restore = window.and_then(|w| w.focused(cx));
        let status = match fixture() {
            Some(status) => status,
            None => {
                self.engine
                    .send(engine::Command::Library(LibraryCommand::OpenCredits(
                        uri.clone(),
                    )));
                Status::Loading
            }
        };
        let c = &mut self.sui.credits;
        c.restore = restore;
        c.open = true;
        c.uri = uri;
        c.name = name;
        c.artists = artists;
        c.status = status;
        c.cursor = None;
        c.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    /// Çalan şarkının künyesi ("Şimdi çalıyor" paneli, `OPENIFY_TEST_VIEW=credits`).
    pub(super) fn open_current_credits(&mut self, window: Option<&Window>, cx: &mut Context<Self>) {
        if let Some(t) = self.state.track.clone() {
            self.open_credits(t.uri, t.name, t.artists, window, cx);
        }
    }

    /// Pencereyi kapat; odak açıldığı yere (yoksa kök çapaya) döner.
    pub(super) fn close_credits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sui.credits.open {
            return;
        }
        self.sui.credits.open = false;
        match self.sui.credits.restore.take() {
            Some(handle) if self.keyboard_focus => window.focus(&handle),
            _ => window.focus(&self.focus),
        }
        cx.notify();
    }

    /// Motorun cevabı (`LibraryEvent::Credits`): başka şarkınınsa atılır.
    pub(super) fn on_credits(
        &mut self,
        uri: String,
        result: Result<Arc<Credits>, CreditsFailure>,
        cx: &mut Context<Self>,
    ) {
        let c = &mut self.sui.credits;
        if !c.open || c.uri != uri || fixture().is_some() {
            return;
        }
        c.status = match result {
            Ok(credits) => Status::Ready(credits),
            Err(why) => Status::Failed(why),
        };
        c.cursor = None;
        cx.notify();
    }

    /// Sanatçıya git: pencere kapanır, sayfa açılır.
    fn open_credits_artist(&mut self, uri: String, window: &mut Window, cx: &mut Context<Self>) {
        self.sui.credits.open = false;
        self.sui.credits.restore = None;
        window.focus(&self.focus);
        self.navigate(Route::Artist(uri), cx);
        cx.notify();
    }

    /// Pencere açıkken tuşlar (bkz. modül başlığı). Hep işlenir (`true`).
    pub(super) fn credits_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &event.keystroke;
        let links = self.sui.credits.links();
        let step = |cursor: Option<usize>, back: bool| -> Option<usize> {
            let n = links.len();
            if n == 0 {
                return None;
            }
            Some(match (cursor, back) {
                (None, false) => 0,
                (None, true) => n - 1,
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
            })
        };
        match ks.key.as_str() {
            "escape" => self.close_credits(window, cx),
            "down" => self.sui.credits.cursor = step(self.sui.credits.cursor, false),
            "up" => self.sui.credits.cursor = step(self.sui.credits.cursor, true),
            "tab" => self.sui.credits.cursor = step(self.sui.credits.cursor, ks.modifiers.shift),
            "enter" => {
                if let Some(uri) = self.sui.credits.cursor.and_then(|i| links.get(i).cloned()) {
                    self.open_credits_artist(uri, window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
        true
    }

    /// Pencere: pencereyi karartır, ortada kart. Tekerlek ve tıklama arkadaki
    /// sayfaya geçmez; dışarı tıklamak kapatır.
    pub(super) fn render_credits(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let c = &self.sui.credits;
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let width = CARD_WIDTH.min(vw - 2. * S4);
        let max_h = (vh - 2. * S6).max(S8 * 3.);

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(t!(CreditsTitle)),
            )
            .child(
                icon_button("credits-close", "x", Look::Live, accent)
                    .tooltip(Tooltip::text(t!(Close), Some("Esc")))
                    .on_click(cx.listener(|this, _, window, cx| this.close_credits(window, cx))),
            );

        // Şarkı: ad (okunur) ve sanatçılar (daha sessiz); uzunsa kırpılır.
        let song = div()
            .flex()
            .flex_col()
            .pb(px(S4))
            .child(
                div()
                    .type_role(T_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(
                        super::text::line_with_tip("credits-name", c.name.clone())
                            .flex()
                            .min_w_0(),
                    ),
            )
            .when(!c.artists.is_empty(), |el| {
                el.child(
                    div().type_role(T_SECONDARY).text_color(muted()).child(
                        super::text::line_with_tip("credits-artists", c.artists.clone())
                            .flex()
                            .min_w_0(),
                    ),
                )
            });

        let body: AnyElement = match &c.status {
            Status::Loading => note(t!(CreditsLoading)),
            Status::Failed(why) => note(failure_text(*why)),
            Status::Ready(credits) if credits.is_empty() => note(t!(CreditsEmpty)),
            Status::Ready(credits) => {
                let mut list = div().flex().flex_col().gap(px(S3));
                let mut link_ix = 0usize;
                for (r, role) in credits.roles.iter().enumerate() {
                    let mut names = div().flex_1().min_w_0().flex().flex_wrap().items_center();
                    let last = role.people.len().saturating_sub(1);
                    for (p, person) in role.people.iter().enumerate() {
                        let more = p < last;
                        let name = match &person.uri {
                            Some(uri) => {
                                let focused = c.cursor == Some(link_ix);
                                link_ix += 1;
                                let uri = uri.clone();
                                div()
                                    .id(("credit-person", r * 100 + p))
                                    .pl(px(S1))
                                    .pr(px(if more { 0. } else { S1 }))
                                    .rounded(px(R_SM))
                                    .cursor_pointer()
                                    .type_role(T_BODY)
                                    .text_color(text())
                                    .when(focused, |el| el.bg(veil(VEIL_SELECTED)))
                                    .hover(|s| s.bg(veil(VEIL_HOVER)))
                                    .tooltip(Tooltip::text(t!(MenuGoArtist), None))
                                    .child(person.name.clone())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_credits_artist(uri.clone(), window, cx);
                                    }))
                                    .into_any_element()
                            }
                            None => div()
                                .pl(px(S1))
                                .pr(px(if more { 0. } else { S1 }))
                                .type_role(T_BODY)
                                .text_color(soft())
                                .child(person.name.clone())
                                .into_any_element(),
                        };
                        // Virgül adın hemen yanında; boşluk virgülden sonra.
                        names = names.child(name);
                        if more {
                            names =
                                names.child(div().type_role(T_BODY).text_color(muted()).child(","));
                        }
                    }
                    list = list.child(row(
                        role_label(role.kind, &role.title),
                        names.into_any_element(),
                    ));
                }
                if !credits.sources.is_empty() {
                    let sources = div()
                        .flex_1()
                        .min_w_0()
                        .px(px(S1))
                        .type_role(T_BODY)
                        .text_color(soft())
                        .child(credits.sources.join(" · "));
                    list = list.child(row(
                        t!(CreditsSource).to_string(),
                        sources.into_any_element(),
                    ));
                }
                list.into_any_element()
            }
        };

        let scroll = div()
            .id("credits-scroll")
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&c.scroll)
            .child(song)
            .child(body);

        let panel = div()
            .id("credits")
            .occlude()
            .w(px(width))
            .max_h(px(max_h))
            .p(px(S5))
            .flex()
            .flex_col()
            .gap(px(S3))
            .rounded(px(R_XL))
            .bg(rgb(BAR))
            .border_1()
            .border_color(rgb(BORDER))
            .shadow(float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(scroll);
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
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_credits(window, cx)),
                )
                .child(panel),
        )
        .with_priority(4)
        .into_any_element()
    }

    /// "Şimdi çalıyor" panelindeki künye düğmesi (başlığın yanında, kalbin
    /// solunda). Şarkı yokken ya da şarkı Spotify'ın şarkısı değilken yok.
    pub(super) fn credits_button(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let track = self
            .state
            .track
            .as_ref()
            .filter(|t| t.uri.starts_with("spotify:track:"))?;
        let (stop, look) = self.stop_look("panel-credits", super::tab::PANEL, true, window, cx);
        let (uri, name, artists) = (track.uri.clone(), track.name.clone(), track.artists.clone());
        Some(
            icon_button("panel-credits", "info", look, self.accent().text)
                .when_some(stop, |el, h| el.track_focus(&h))
                .tooltip(Tooltip::text(t!(MenuShowCredits), None))
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                    this.leave_button(ev, window);
                    this.open_credits(uri.clone(), name.clone(), artists.clone(), Some(window), cx);
                }))
                .into_any_element(),
        )
    }

    /// `OPENIFY_TEST_VIEW=credits`: panel ve künye açık gelir.
    pub(super) fn apply_credits_test_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_panel(Panel::Now, cx);
        self.open_current_credits(Some(window), cx);
    }
}

/// Tek satırlık durum yazısı (yükleniyor, künye yok, hata).
fn note(message: &'static str) -> AnyElement {
    div()
        .py(px(S2))
        .type_role(T_SECONDARY)
        .text_color(muted())
        .child(message)
        .into_any_element()
}

/// Bir künye satırı: solda rol adı (sabit enli), sağda adlar.
fn row(label_text: String, content: AnyElement) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(px(S3))
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_none()
                .type_role(T_BODY)
                .text_color(muted())
                .child(label_text),
        )
        .child(content)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_follow_the_display_order_and_skip_unlinked_names() {
        let mut ui = CreditsUi::new();
        assert!(ui.links().is_empty(), "yüklenirken tıklanacak ad yok");
        let value: serde_json::Value =
            serde_json::from_str(include_str!("../engine/fixtures/track_credits.json")).unwrap();
        ui.status = Status::Ready(Arc::new(engine::parse_credits(&value)));
        assert_eq!(
            ui.links(),
            [
                "spotify:artist:r01",
                "spotify:artist:r02",
                "spotify:artist:w01",
                "spotify:artist:p01"
            ],
            "Seslendiren, Yazan, Yapımcı sırasıyla; adresi olmayanlar atlanır"
        );
        ui.status = Status::Failed(CreditsFailure::Offline);
        assert!(ui.links().is_empty());
    }

    #[test]
    fn role_labels_use_the_ui_language_for_the_three_known_roles() {
        use crate::i18n::{Lang, with_lang};
        with_lang(Lang::Tr, || {
            assert_eq!(
                role_label(CreditRoleKind::Performer, "Performed By"),
                "Seslendiren"
            );
            assert_eq!(role_label(CreditRoleKind::Writer, "Written By"), "Yazan");
            assert_eq!(
                role_label(CreditRoleKind::Producer, "Produced By"),
                "Yapımcı"
            );
            assert_eq!(
                role_label(CreditRoleKind::Other, "Mixing Engineer"),
                "Miks mühendisi"
            );
            assert_eq!(role_label(CreditRoleKind::Other, "MIXER"), "Miks mühendisi");
            assert_eq!(
                role_label(CreditRoleKind::Other, "mastering engineer"),
                "Mastering mühendisi"
            );
            assert_eq!(
                role_label(CreditRoleKind::Other, "Engineer"),
                "Kayıt mühendisi"
            );
            assert_eq!(
                role_label(CreditRoleKind::Other, "Recording Engineer"),
                "Kayıt mühendisi"
            );
            assert_eq!(role_label(CreditRoleKind::Other, "Composer"), "Besteci");
            assert_eq!(role_label(CreditRoleKind::Other, "LYRICIST"), "Söz yazarı");
            assert_eq!(role_label(CreditRoleKind::Other, "Arranger"), "Aranjör");
            assert_eq!(
                role_label(CreditRoleKind::Other, "String Quartet"),
                "String Quartet"
            );
        });
        with_lang(Lang::En, || {
            assert_eq!(
                role_label(CreditRoleKind::Other, "Mixing Engineer"),
                "Mixing engineer"
            );
            assert_eq!(
                role_label(CreditRoleKind::Other, "engineer"),
                "Recording engineer"
            );
            assert_eq!(role_label(CreditRoleKind::Other, "Composer"), "Composer");
            assert_eq!(
                role_label(CreditRoleKind::Other, "Foley Artist"),
                "Foley Artist"
            );
            assert_eq!(
                role_label(CreditRoleKind::Performer, "Performed By"),
                "Performed by"
            );
            assert_eq!(role_label(CreditRoleKind::Writer, "x"), "Written by");
        });
    }

    #[test]
    fn every_failure_has_a_single_line_text() {
        for why in [
            CreditsFailure::Offline,
            CreditsFailure::RateLimited,
            CreditsFailure::Denied,
            CreditsFailure::Unavailable,
        ] {
            let s = failure_text(why);
            assert!(!s.is_empty() && !s.contains('\n'));
        }
    }
}
