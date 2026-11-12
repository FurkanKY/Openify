//! Karaoke düğmesi ve miktar sürgüsü: sözler panelinin üstünde ve tam ekran
//! sözlerde. Ses tarafı `engine::output::karaoke` (orta kanalın konuşma bandı
//! bastırılır, yan kanal ve bas kalır; mono kaynakta kendiliğinden etkisiz,
//! arayüz bunu göstermez).
//!
//! Hap (`chip`) açılıp kapanır; açıkken yanında küçük bir sürgü (0–100) belirir.
//! Sürgü sürüklenirken motora her harekette gider (ses tarafı 46 ms'de süzer,
//! tıkırtı yok), bırakınca diske yazılır. Panelde hap ve sürgü Tab durağıdır,
//! sürgüde ← / → 5 puan oynatır; tam ekranda denetimler gibi yalnız fare.
//! Tercih kalıcı (`Settings::karaoke_enabled` / `karaoke_amount`).

use std::{cell::Cell, rc::Rc};

use gpui::{Bounds, Context, MouseDownEvent, Pixels, Window, div, prelude::*, px};

use super::{
    Drag, Shell,
    components::{Look, Tooltip, chip},
    lyrics_view::Place,
    pill::{fraction, slider},
    tab,
    theme::*,
};
use crate::engine::Command;
use crate::t;

/// Sürgünün eni.
const SLIDER_W: f32 = 88.;
/// Klavye adımı (0..1).
const KEY_STEP: f32 = 0.05;
/// Panelde sürgünün Tab durağı anahtarı.
pub(super) const SLIDER_KEY: &str = "lyrics-karaoke-amount";

pub(super) type Bar = Rc<Cell<Option<Bounds<Pixels>>>>;

impl Shell {
    /// Motora güncel karaoke ayarını yollar (beklemez; motor tutar ve çıkışa verir).
    pub(super) fn karaoke_send(&self) {
        self.engine.send(Command::SetKaraoke {
            enabled: self.prefs.karaoke_enabled,
            amount: self.prefs.karaoke_amount,
        });
    }

    fn karaoke_bar(&self, place: Place) -> &Bar {
        match place {
            Place::Panel => &self.karaoke_bounds,
            Place::Full => &self.karaoke_full_bounds,
        }
    }

    /// Miktarı ayarlar (0..1, 5 puana oturur); motora gider, görünümler
    /// yenilenir. Diske yazmak çağıranın işi (sürüklemede bırakınca).
    pub(super) fn set_karaoke_amount(&mut self, amount: f32, cx: &mut Context<Self>) {
        let amount = ((amount.clamp(0., 1.) / KEY_STEP).round() * KEY_STEP).clamp(0., 1.);
        if (amount - self.prefs.karaoke_amount).abs() < 0.001 {
            return;
        }
        self.prefs.karaoke_amount = amount;
        self.karaoke_send();
        self.notify_lyric_views(cx);
    }

    /// Panelde sürgü klavye odağında mı.
    pub(super) fn karaoke_slider_focused(&self, window: &Window) -> bool {
        self.nav
            .extra_focused(window)
            .is_some_and(|(key, _)| key == SLIDER_KEY)
    }

    /// Sürgüde ← / →: bir adım. Klavye değişimi hemen kaydedilir.
    pub(super) fn karaoke_step(&mut self, up: bool, cx: &mut Context<Self>) {
        let now = self.prefs.karaoke_amount;
        self.set_karaoke_amount(now + if up { KEY_STEP } else { -KEY_STEP }, cx);
        self.save_settings();
    }

    /// "Karaoke" hapı ve (açıkken) miktar sürgüsü. `Place::Panel`de Tab durağı
    /// ve odak halkası var; `Place::Full`de denetimler gibi yalnız fare.
    pub(super) fn karaoke_group(
        &self,
        place: Place,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let on = self.prefs.karaoke_enabled;
        let amount = self.prefs.karaoke_amount;
        let accent = self.accent().text;
        let (stop, look, slider_focus) = if place == Place::Panel {
            let (stop, look) = self.stop_look("lyrics-karaoke", tab::PANEL, true, window, cx);
            let slider_stop = on.then(|| self.stop_look(SLIDER_KEY, tab::PANEL, true, window, cx));
            let focus = slider_stop.and_then(|(h, l)| h.map(|h| (h, l == Look::Focus)));
            (stop, look, focus)
        } else {
            (None, Look::Live, None)
        };
        let id = if place == Place::Panel {
            "lyrics-karaoke"
        } else {
            "full-karaoke"
        };
        let pill = chip(id, t!(Karaoke), on, look, accent)
            .when_some(stop, |el, h| el.track_focus(&h))
            .tooltip(Tooltip::text(
                if on {
                    t!(KaraokeOffTip)
                } else {
                    t!(KaraokeOnTip)
                },
                None,
            ))
            .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                this.leave_button(ev, window);
                this.toggle_karaoke(cx);
            }));
        let bar = self.karaoke_bar(place).clone();
        let control = on.then(|| {
            div()
                .id(if place == Place::Panel {
                    "lyrics-karaoke-amount"
                } else {
                    "full-karaoke-amount"
                })
                .w(px(SLIDER_W))
                .flex_none()
                .flex()
                .tooltip(Tooltip::text(t!(KaraokeAmount), None))
                .child(slider(
                    if place == Place::Panel {
                        "karaoke-amount"
                    } else {
                        "karaoke-amount-full"
                    },
                    amount,
                    accent,
                    true,
                    slider_focus,
                    bar,
                    Some(Box::new(cx.listener(
                        move |this, ev: &MouseDownEvent, _, cx| {
                            if let Some(f) = fraction(this.karaoke_bar(place), ev.position) {
                                this.dragging = Some(Drag::Karaoke(place));
                                this.set_karaoke_amount(f, cx);
                            }
                        },
                    ))),
                ))
        });
        div()
            .flex()
            .items_center()
            .gap(px(S2))
            .child(pill)
            .children(control)
            .into_any_element()
    }

    /// Sürükleme: fare konumuna göre miktar.
    pub(super) fn karaoke_drag_to(
        &mut self,
        place: Place,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some(f) = fraction(self.karaoke_bar(place), position) {
            self.set_karaoke_amount(f, cx);
        }
    }
}
