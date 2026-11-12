//! Tek satırlık, içindeki adları tıklanabilen yazı ("Sanatçı A, Sanatçı B").
//!
//! Satırdaki sanatçı adları ayrı ayrı tıklanabilmeli ama bütün satır yine tek
//! yazı gibi kısalmalı ("Ufo361, Ezh…"): her ad ayrı bir öğe olsaydı her biri
//! ayrı ayrı "…" ile kısalırdı. Bu öğe `text::Line` gibi tek satırı genişlik
//! kesinleşince kısaltır, sonra her adın yazıdaki yerini (x aralığını) bulur
//! ve her birine ayrı bir isabet alanı koyar: üzerine gelince el imleci ve
//! alt çizgi, tıklayınca o adın sırası bildirilir. Kısalmada görünmez kalan
//! adlar tıklanamaz.
//!
//! Tıklama satırın kendi tıklamasına (seç / çal) geçmez; sağ tık geçer
//! (satırın menüsü açılsın).

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::{
    App, AppContext, AvailableSpace, Bounds, Context, CursorStyle, DispatchPhase, Div, Element,
    ElementId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, InteractiveElement,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels,
    Render, ShapedLine, SharedString, Stateful, StatefulInteractiveElement, Style, Styled, Window,
    div, fill, point, px, size,
};

const ELLIPSIS: &str = "…";

/// Ad tıklandı / üzerine gelindi: adın sırası.
pub type SpanHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct LinkLine {
    text: SharedString,
    spans: Vec<Range<usize>>,
    on_click: SpanHandler,
    on_hover: Option<SpanHandler>,
    truncated: Rc<Cell<bool>>,
}

/// Adların ", " ile birleşmiş yazısı ve her adın bayt aralığı.
pub fn join_names<'a>(names: impl IntoIterator<Item = &'a str>) -> (String, Vec<Range<usize>>) {
    let mut text = String::new();
    let mut spans = Vec::new();
    for name in names {
        if !text.is_empty() {
            text.push_str(", ");
        }
        let start = text.len();
        text.push_str(name);
        spans.push(start..text.len());
    }
    (text, spans)
}

/// "A, B" yazısını adlara böler (`TrackRow::artists` biçimi).
pub fn split_names(text: &str) -> Vec<&str> {
    text.split(", ").map(str::trim).filter(|n| !n.is_empty()).collect()
}

/// Tıklanabilen adlı tek satır; kısaltıldıysa üzerine gelince tamamı ipucunda.
pub fn link_line(
    id: impl Into<ElementId>,
    names: &[&str],
    on_click: SpanHandler,
    on_hover: Option<SpanHandler>,
) -> Stateful<Div> {
    let (text, spans) = join_names(names.iter().copied());
    let text: SharedString = text.into();
    let truncated = Rc::new(Cell::new(false));
    let flag = truncated.clone();
    let full = text.clone();
    InteractiveElement::id(div(), id)
        .min_w_0()
        .child(LinkLine { text, spans, on_click, on_hover, truncated })
        .tooltip(move |_, cx| {
            let tip = FullText { text: full.clone(), show: flag.get() };
            cx.new(|_| tip).into()
        })
}

struct FullText {
    text: SharedString,
    show: bool,
}

impl Render for FullText {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if !self.show {
            return div();
        }
        div()
            .pt(px(super::theme::S2))
            .child(super::components::Tooltip::element(self.text.clone(), None))
    }
}

impl IntoElement for LinkLine {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

pub struct Prepainted {
    shaped: ShapedLine,
    line_height: Pixels,
    /// Görünen her adın x aralığı (öğeye göre) ve isabet alanı.
    spans: Vec<(usize, Pixels, Pixels, Hitbox)>,
}

impl Element for LinkLine {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = text_style.line_height_in_pixels(window.rem_size());
        let run = text_style.to_run(self.text.len());
        let text = self.text.clone();
        let mut style = Style::default();
        style.size.height = line_height.into();
        style.min_size.width = px(0.).into();
        style.flex_shrink = 1.;
        let layout_id = window.request_measured_layout(style, move |known, available, window, _| {
            let natural = window
                .text_system()
                .shape_line(text.clone(), font_size, std::slice::from_ref(&run), None)
                .width;
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(space) => natural.min(space),
                AvailableSpace::MinContent => px(0.),
                AvailableSpace::MaxContent => natural,
            });
            size(width, line_height)
        });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height_in_pixels(window.rem_size());
        let mut runs = vec![style.to_run(self.text.len())];
        // Sığan yazı kısaltılmaz: `truncate_line` kesirli ölçekte (%125) kutu tam
        // yazı enindeyken sığanı da kısaltabiliyordu (bkz. `text.rs`).
        let full_width = window.text_system().shape_line(self.text.clone(), font_size, &runs, None).width;
        let shown = if full_width <= bounds.size.width + px(1.) {
            self.text.clone()
        } else {
            cx.text_system()
                .line_wrapper(style.font(), font_size)
                .truncate_line(self.text.clone(), bounds.size.width, ELLIPSIS, &mut runs)
        };
        let truncated = shown != self.text;
        self.truncated.set(truncated);
        // Kısaltıldıysa yazının ilk `visible` baytı görünüyor ("…" hariç).
        let visible = if truncated { shown.len().saturating_sub(ELLIPSIS.len()) } else { shown.len() };
        let shaped = window.text_system().shape_line(shown, font_size, &runs, None);
        let mut spans = Vec::new();
        for (i, range) in self.spans.iter().enumerate() {
            if range.start >= visible {
                break;
            }
            let end = range.end.min(visible);
            let x0 = shaped.x_for_index(range.start);
            let x1 = shaped.x_for_index(end);
            if x1 <= x0 {
                continue;
            }
            let hit = Bounds::new(point(bounds.origin.x + x0, bounds.origin.y), size(x1 - x0, bounds.size.height));
            spans.push((i, x0, x1, window.insert_hitbox(hit, HitboxBehavior::Normal)));
        }
        Prepainted { shaped, line_height, spans }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let color = window.text_style().color;
        let _ = state.shaped.paint(bounds.origin, state.line_height, window, cx);
        let hovered = state.spans.iter().find(|(_, _, _, h)| h.is_hovered(window)).map(|(i, ..)| *i);
        for (i, x0, x1, hitbox) in &state.spans {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
            if hovered == Some(*i) {
                // Alt çizgi: yazının tabanının hemen altında, 1 px.
                let y = bounds.origin.y + state.line_height - px(2.);
                window.paint_quad(fill(
                    Bounds::new(point(bounds.origin.x + *x0, y), size(*x1 - *x0, px(1.))),
                    color,
                ));
            }
        }
        let hitboxes: Rc<Vec<(usize, Hitbox)>> =
            Rc::new(state.spans.iter().map(|(i, _, _, h)| (*i, h.clone())).collect());

        // Tıklama: satırın kendi tıklamasından önce (üstteki öğe önce alır).
        let on_click = self.on_click.clone();
        let boxes = hitboxes.clone();
        window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left {
                return;
            }
            if let Some((i, _)) = boxes.iter().find(|(_, h)| h.is_hovered(window)) {
                cx.stop_propagation();
                on_click(*i, window, cx);
            }
        });
        // Üzerine gelinen ad değişince yeniden çiz (alt çizgi) ve haber ver.
        let on_hover = self.on_hover.clone();
        let painted = Cell::new(hovered);
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let now = hitboxes.iter().find(|(_, h)| h.is_hovered(window)).map(|(i, _)| *i);
            if now != painted.get() {
                painted.set(now);
                window.refresh();
                if let (Some(i), Some(f)) = (now, &on_hover) {
                    f(i, window, cx);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_join_and_split_back() {
        let (text, spans) = join_names(["Ufo361", "Ezhel", "Şanışer"]);
        assert_eq!(text, "Ufo361, Ezhel, Şanışer");
        assert_eq!(&text[spans[2].clone()], "Şanışer");
        assert_eq!(split_names(&text), ["Ufo361", "Ezhel", "Şanışer"]);
        assert!(split_names("").is_empty());
    }
}
