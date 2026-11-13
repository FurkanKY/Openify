//! Üzerine gelme / basma rengi geçişi (7 Kasım akşam).
//!
//! gpui'nin `.hover()` / `.active()` stilleri anında değişir; "sakin ve
//! kesin" hissi için renk 140 ms'de (`motion_fast`) yumuşakça gelir, basınca
//! anında netleşir. `HoverFill`, kapsayıcısının ilk (en alttaki) çocuğu olan
//! mutlak konumlu görünmez bir katman: kendi isabet kutusuyla fare üstünde mi
//! diye bakar, ilerlemeyi pencere öğesi durumunda tutar ve kaplayıcısının
//! zemininin üstüne, içeriğinin altına bir dörtgen boyar.
//!
//! Kare istemek: yalnızca ilerleme hedefine varmamışken (en çok ~9 kare);
//! hedefe varınca saat durur, boşta kare yok. "Animasyonları azalt"ta
//! ilerleme hedefe anında atlar. Girdiyi bekletmez (renk, tıklamadan bağımsız).

use std::{cell::RefCell, panic::Location, rc::Rc, time::Duration, time::Instant};

use gpui::{
    App, Bounds, Corners, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Refineable, Style, StyleRefinement, Styled, Window, fill,
    px,
};

use super::theme::{animations_enabled, ease_out, motion_fast};

/// Bir öğenin üzerine gelme / basma katmanı. `hover` ve `pressed`, öğenin
/// durduğu zemin yerine geçecek renkler (bileşenlerdeki `states` ile aynı);
/// `rest_alpha`: öğenin kendi zemininin saydamlığı (0 = zemin yok). Saydam
/// bir renk, zeminin üstüne eklenince bileşik sonuç hedef renk olsun diye
/// aradaki farka çevrilir.
/// Hap biçimli (tam yuvarlak) öğenin katman köşesi: dörtgenin yarı kısa
/// kenarına kırpılır.
pub const FULL: f32 = 1000.;

pub struct HoverFill {
    hover: Hsla,
    pressed: Hsla,
    rest_alpha: f32,
    radius: f32,
}

pub fn hover_fill(
    hover: impl Into<Hsla>,
    pressed: impl Into<Hsla>,
    rest_alpha: f32,
    radius: f32,
) -> HoverFill {
    HoverFill {
        hover: hover.into(),
        pressed: pressed.into(),
        rest_alpha,
        radius,
    }
}

#[derive(Default)]
struct Level {
    /// 0 (dururken) – 1 (üzerinde).
    level: f32,
    /// Son çizim anı; hedefe varınca `None` (saat durur).
    at: Option<Instant>,
    pressed: bool,
    started: bool,
}

/// Hedef rengin zeminin üstündeki katman rengi (`amount`: 0–1).
fn overlay(target: Hsla, rest_alpha: f32, amount: f32) -> Hsla {
    let a = if target.a >= 0.99 {
        1.
    } else if rest_alpha > 0. {
        ((target.a - rest_alpha) / (1. - rest_alpha)).max(0.)
    } else {
        target.a
    };
    Hsla {
        a: a * amount,
        ..target
    }
}

/// `level`'ı `target`'a doğru `dt` kadar ilerletir (doğrusal, süre `motion_fast`).
fn step(level: f32, target: f32, dt: Duration) -> f32 {
    let delta = dt.as_secs_f32() / motion_fast().as_secs_f32();
    if level < target {
        (level + delta).min(target)
    } else {
        (level - delta).max(target)
    }
}

impl Element for HoverFill {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name("hf".into()))
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.refine(
            &StyleRefinement::default()
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) -> Hitbox {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        _cx: &mut App,
    ) {
        // Ölçüm kancası: OPENIFY_PROF_HIDE="tween" katmanı hiç çizmez (A/B).
        let Some(id) = id.filter(|_| !super::frames::hidden("tween")) else {
            return;
        };
        let state: Rc<RefCell<Level>> =
            window.with_element_state(id, |s: Option<Rc<RefCell<Level>>>, _| {
                let s = s.unwrap_or_default();
                (s.clone(), s)
            });
        let hovered = hitbox.is_hovered(window);
        let target = if hovered { 1. } else { 0. };
        let view = window.current_view();

        let (level, pressed, moving) = {
            let mut s = state.borrow_mut();
            let now = Instant::now();
            if !s.started || !animations_enabled() {
                // İlk çizim (öğe zaten fare altında belirdi) ya da hareket
                // kapalı: atla.
                s.started = true;
                s.level = target;
                s.at = None;
            } else if s.level != target {
                // Saat durmuşsa (ilk kare) bir ekran karesi sayılır.
                let dt =
                    s.at.map(|t| now.duration_since(t))
                        .unwrap_or(Duration::from_millis(8));
                s.level = step(s.level, target, dt);
                s.at = Some(now);
            } else {
                s.at = None;
            }
            (s.level, s.pressed && hovered, s.level != target)
        };
        if moving {
            window.request_animation_frame();
        }

        let color = if pressed {
            overlay(self.pressed, self.rest_alpha, 1.)
        } else {
            overlay(self.hover, self.rest_alpha, ease_out(level))
        };
        if color.a > 0.002 {
            let max = bounds.size.width.min(bounds.size.height) / 2.;
            let radius = px(self.radius).min(max);
            window.paint_quad(fill(bounds, color).corner_radii(Corners::all(radius)));
        }

        // Fare girip çıkınca ve basılıp bırakılınca bu görünüm yeniden çizilir.
        let was = hovered;
        let hb = hitbox.clone();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && hb.is_hovered(window) != was {
                cx.notify(view);
            }
        });
        // Basma dinleyicileri yalnız gerekince kurulur (her karede her öğe için
        // üç kapanış kurmak liste çiziminde ölçülür pahalıydı): basma yalnız
        // fare üstündeyken başlar; bırakma da fare üstündeyken (ya da basılıyken)
        // dinlenir, hızlı bir tıklamada bırakma yeniden çizimi beklemesin.
        if hovered {
            let (hb, st) = (hitbox.clone(), state.clone());
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && e.button == MouseButton::Left
                    && hb.is_hovered(window)
                {
                    st.borrow_mut().pressed = true;
                    cx.notify(view);
                }
            });
        }
        if hovered || state.borrow().pressed {
            let st = state;
            window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture
                    && e.button == MouseButton::Left
                    && st.borrow().pressed
                {
                    st.borrow_mut().pressed = false;
                    cx.notify(view);
                }
            });
        }
    }
}

impl IntoElement for HoverFill {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Anahtarın oluğu ya da topuzu (13 Kasım): açılıp kapanınca oluk rengi
/// yumuşakça döner, topuz yerine sıçramaz, `motion_fast` içinde kayar.
/// Anahtarın içine mutlak konumlu katman olarak konur (oluk ilk çocuk, topuz
/// üzerine gelme katmanından sonra). İkisi aynı karede başlayıp aynı sürede
/// ilerler; durum, anahtarın kimliği altında parça başına tutulur.
pub enum SwitchPart {
    Track {
        off: Hsla,
        on: Hsla,
    },
    Knob {
        off: Hsla,
        on: Hsla,
        size: f32,
        inset: f32,
    },
}

pub struct SwitchPaint {
    on: bool,
    part: SwitchPart,
}

pub fn switch_paint(on: bool, part: SwitchPart) -> SwitchPaint {
    SwitchPaint { on, part }
}

#[derive(Default)]
struct Glide {
    level: f32,
    at: Option<Instant>,
    started: bool,
}

fn lerp_hsla(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    gpui::Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}

impl Element for SwitchPaint {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name(
            match self.part {
                SwitchPart::Track { .. } => "swt",
                SwitchPart::Knob { .. } => "swk",
            }
            .into(),
        ))
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.refine(
            &StyleRefinement::default()
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let Some(id) = id else { return };
        let target = if self.on { 1. } else { 0. };
        let state: Rc<RefCell<Glide>> =
            window.with_element_state(id, |s: Option<Rc<RefCell<Glide>>>, _| {
                let s = s.unwrap_or_default();
                (s.clone(), s)
            });
        let level = {
            let mut s = state.borrow_mut();
            let now = Instant::now();
            if !s.started || !animations_enabled() {
                // İlk çizimde anahtar zaten durumunda belirir.
                s.started = true;
                s.level = target;
                s.at = None;
            } else if s.level != target {
                let dt =
                    s.at.map(|t| now.duration_since(t))
                        .unwrap_or(Duration::from_millis(8));
                s.level = step(s.level, target, dt);
                s.at = Some(now);
            } else {
                s.at = None;
            }
            s.level
        };
        if level != target {
            window.request_animation_frame();
        }
        let eased = ease_out(level);
        match self.part {
            SwitchPart::Track { off, on } => {
                let r = bounds.size.height / 2.;
                window.paint_quad(
                    fill(bounds, lerp_hsla(off, on, eased)).corner_radii(Corners::all(r)),
                );
            }
            SwitchPart::Knob {
                off,
                on,
                size,
                inset,
            } => {
                let size = px(size);
                let travel = bounds.size.width - size - px(inset) * 2.;
                let origin = gpui::point(
                    bounds.origin.x + px(inset) + travel * eased,
                    bounds.origin.y + (bounds.size.height - size) / 2.,
                );
                let knob = Bounds::new(origin, gpui::size(size, size));
                window.paint_quad(
                    fill(knob, lerp_hsla(off, on, eased)).corner_radii(Corners::all(size / 2.)),
                );
            }
        }
    }
}

impl IntoElement for SwitchPaint {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgba;

    #[test]
    fn overlay_over_a_veil_composites_to_the_target() {
        // Zemin %6 beyaz, hedef %10: katman c ise 0,06 + 0,94·c = 0,10.
        let target: Hsla = rgba(0xFFFFFF1A).into();
        let o = overlay(target, 0.06, 1.);
        assert!((0.06 + (1. - 0.06) * o.a - target.a).abs() < 0.002);
        // Zemin yoksa hedefin kendisi; hiç ilerleme yoksa hiç katman.
        assert!((overlay(target, 0., 1.).a - target.a).abs() < 1e-6);
        assert_eq!(overlay(target, 0., 0.).a, 0.);
        // Hedef zeminden açık değilse katman yok (negatife düşmez).
        assert_eq!(overlay(rgba(0xFFFFFF0F).into(), 0.10, 1.).a, 0.);
        // Opak hedef: ilerleme doğrudan saydamlık.
        assert!((overlay(rgba(0x112233FF).into(), 0.5, 0.4).a - 0.4).abs() < 1e-6);
    }

    #[test]
    fn level_steps_to_the_target_and_stops() {
        let dur = motion_fast();
        assert!((step(0., 1., dur / 2) - 0.5).abs() < 1e-3);
        assert_eq!(step(0.9, 1., dur), 1.);
        assert_eq!(step(0.3, 0., dur), 0.);
        assert_eq!(step(1., 0., Duration::ZERO), 1.);
    }
}
