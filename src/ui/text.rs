//! Tek satırlık, sığmazsa "…" ile kısalan yazı.
//!
//! gpui'nin kendi `truncate()`'i iki sebeple işimize yaramıyor:
//! - yazının ilk ölçümünü önbelleğe alıyor; ilk ölçüm genişlik bilinmeden
//!   yapılınca hiç kısaltmıyor, yazı kenarda kesik kalıyor;
//! - yazı ikinci kez ölçülürse ilk kısaltmanın bozduğu uzunluklarla
//!   çalışıyor ve "ş" gibi çok baytlı harflerde uygulamayı çökertiyor.
//!
//! Bu öğe kısaltmayı genişlik kesinleştikten sonra (prepaint) her karede
//! baştan yapıyor. Genişliği yazının doğal genişliği kadardır, kutu daha
//! darsa kutuya uyar; yüksekliği bir satırdır. Yazı rengi, boyutu ve
//! kalınlığı üstteki kutudan gelir.

use std::{cell::Cell, rc::Rc};

use gpui::{
    App, AppContext, AvailableSpace, Bounds, Context, Div, Element, ElementId,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels, Render,
    ShapedLine, SharedString, Stateful, StatefulInteractiveElement, Style, Styled, Window, div,
    px, size,
};

const ELLIPSIS: &str = "…";

pub struct Line {
    text: SharedString,
    /// Kısaltıldıysa `true` yazılır (ipucu yalnızca o zaman görünsün).
    truncated: Option<Rc<Cell<bool>>>,
}

/// Tek satır yazı; sığmazsa sonu "…" ile kısalır.
pub fn line(text: impl Into<SharedString>) -> Line {
    Line { text: text.into(), truncated: None }
}

/// Tek satır yazı; kısaltıldıysa üzerine gelince tamamı ipucunda görünür.
/// Kısaltılmadıysa ipucu çıkmaz.
pub fn line_with_tip(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    let text = text.into();
    let truncated = Rc::new(Cell::new(false));
    let flag = truncated.clone();
    let full = text.clone();
    InteractiveElement::id(div(), id)
        .min_w_0()
        .child(Line { text, truncated: Some(truncated) })
        .tooltip(move |_, cx| {
            let tip = FullText { text: full.clone(), show: flag.get() };
            cx.new(|_| tip).into()
        })
}

/// Kısaltılan yazının tamamı (yazı sığıyorsa boş, görünmez).
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


impl IntoElement for Line {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Line {
    type RequestLayoutState = ();
    type PrepaintState = Option<(ShapedLine, Pixels)>;

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

        // Genişlik: yazının doğal genişliği, ama kutu ne kadar izin verirse
        // (sığmazsa prepaint'te kısaltılır). Yükseklik: tek satır.
        let mut style = Style::default();
        style.size.height = line_height.into();
        style.min_size.width = px(0.).into();
        style.flex_shrink = 1.;
        let layout_id = window.request_measured_layout(style, move |known, available, window, _| {
            let started = std::time::Instant::now();
            let natural = window
                .text_system()
                .shape_line(text.clone(), font_size, std::slice::from_ref(&run), None)
                .width;
            super::frames::text_spent(started.elapsed(), false);
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
    ) -> Self::PrepaintState {
        let started = std::time::Instant::now();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height_in_pixels(window.rem_size());
        // Önce yazının tamamı: kutu yazının doğal genişliğinden (`request_layout`)
        // geliyor, `truncate_line` ise harf genişliklerini ayrı ölçüyor. %125 gibi
        // kesirli ölçekte iki ölçüm bir pikselin altında ayrışıyor ve sığan
        // "Night Drive" kutusu tam kendi eninde "Night Dri…" oluyordu
        // (ekran sınaması, %125). Yazı sığıyorsa (1 px payla) olduğu gibi çizilir.
        let full_run = style.to_run(self.text.len());
        let full = window.text_system().shape_line(self.text.clone(), font_size, std::slice::from_ref(&full_run), None);
        if full.width <= bounds.size.width + px(1.) {
            if let Some(flag) = &self.truncated {
                flag.set(false);
            }
            super::frames::text_spent(started.elapsed(), true);
            return Some((full, line_height));
        }
        // Her seferinde taze bir "run" listesi: gpui'nin kısaltması listeyi
        // yerinde değiştiriyor.
        let mut runs = vec![style.to_run(self.text.len())];
        let text = cx
            .text_system()
            .line_wrapper(style.font(), font_size)
            .truncate_line(self.text.clone(), bounds.size.width, ELLIPSIS, &mut runs);
        if let Some(flag) = &self.truncated {
            flag.set(text != self.text);
        }
        let shaped = window.text_system().shape_line(text, font_size, &runs, None);
        super::frames::text_spent(started.elapsed(), true);
        Some((shaped, line_height))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some((shaped, line_height)) = prepaint {
            let started = std::time::Instant::now();
            let _ = shaped.paint(bounds.origin, *line_height, window, cx);
            super::frames::text_spent(started.elapsed(), false);
        }
    }
}
