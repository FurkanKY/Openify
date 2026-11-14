//! Kütüphane araçları sayfasının çizimi (`tools.rs` durum ve eylemleri tutar).
//!
//! Sayfa tek sütun (en çok `TOOLS_WIDTH`), üstte beş aracın seçenek düğmeleri
//! (← / → ile gezilir), altında seçili aracın gövdesi. Her etkileşimli öğe bir
//! Tab durağı ve odak halkası çizer (`stop_look`); seçim kutuları ve sonuç
//! tablosu tek durak, içinde ↑ / ↓ / Home / End / Sayfa tuşları ve Boşluk.

use gpui::{
    AnyElement, ClickEvent, Context, Div, ElementId, FontWeight, ScrollDelta, ScrollWheelEvent,
    Window, div, prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{
        Look, chip, primary_button_sized, secondary_button, segment, segmented_track, state_view,
    },
    cover_image,
    library::Loadable,
    text::line_with_tip,
    theme::*,
    tools::{
        BackupStatus, BoxId, Bulk, DupRow, ListBox, MAX_SORT_MOVES, MergeMode, Source, Sources,
        Tool, listened_text,
    },
};
use crate::engine::{
    TrackRow,
    tools::{
        dupes::{DedupeMode, DupKind},
        plan::{Dir, SortBy, SplitBy},
        stats::{StatsRange, TOP_N},
    },
};
use crate::{t, tf, tp};

/// İçerik sütununun en büyük eni.
pub(super) const TOOLS_WIDTH: f32 = 760.;
/// Seçim kutusu ve sonuç tablosunun satır yüksekliği.
const ROW_H: f32 = 40.;
/// Ad alanının eni.
const NAME_FIELD_WIDTH: f32 = 360.;
/// Saat dağılımı çubuklarının en büyük yüksekliği.
const HOURS_HEIGHT: f32 = 96.;
/// Bu pencere eninin altında albüm sütunu gizlenir.
const ALBUM_MIN_WIDTH: f32 = 1000.;
/// Önizlemede gösterilen en çok satır / liste.
const PREVIEW_ROWS: usize = 6;
/// İstatistik listelerinde varsayılan satır sayısı ("Tümünü göster" ile `TOP_N`).
///
/// Onay kutusu: bir işaret (seçili), çoklu seçimde kare, tekilde yuvarlak.
fn mark(on: bool, radio: bool, accent: u32) -> Div {
    let el = div()
        .size(px(18.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    let el = if radio {
        el.rounded_full()
    } else {
        el.rounded(px(R_SM))
    };
    if on {
        el.bg(rgb(accent))
            .child(super::theme::icon("check", 12., rgb(palette().bg)))
    } else {
        el.border_1().border_color(veil(LINE_HOVER))
    }
}

impl Shell {
    // ── Sayfa ───────────────────────────────────────────────────────────

    pub(super) fn tools_body(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let meta = div().child(t!(ToolsSubtitle)).into_any_element();
        let body = match self.tools.tool {
            Tool::Duplicates => self.dupes_tool(window, cx),
            Tool::MergeSplit => self.merge_tool(window, cx),
            Tool::Sort => self.sort_tool(window, cx),
            Tool::Backup => self.backup_tool(window, cx),
        };
        div()
            .flex()
            .flex_col()
            .child(self.page_header(None, t!(ToolsTitle), meta, Vec::new()))
            .child(self.tools_tabs(window, cx))
            .child(self.tools_column(body))
    }

    pub(super) fn stats_body(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let content = self.stats_content(window, cx);
        div()
            .flex()
            .flex_col()
            .child(self.page_header(
                None,
                t!(NavStats),
                div().child(t!(StatsSubtitle)).into_any_element(),
                Vec::new(),
            ))
            .child(self.tools_column(content))
    }

    /// Sütunun eni: içerik eni (`Shell::grid` ile aynı kaynak: pencere eni) eksi
    /// iki kenar boşluğu, en çok `TOOLS_WIDTH`. Açık en gerekli: `w_full` +
    /// `max_w` altında sarılan paragraf 1 satır ölçülüp sonraki öğeye yapışıyordu.
    fn tools_col_width(&self) -> f32 {
        (self.window_width - 2. * self.margin()).clamp(0., TOOLS_WIDTH)
    }

    /// İçerik sütunu: sayfa kenar boşluğu, en çok `TOOLS_WIDTH`.
    fn tools_column(&self, children: Vec<AnyElement>) -> Div {
        div().px(px(self.margin())).pb(px(S6)).child(
            div()
                .w(px(self.tools_col_width()))
                .flex()
                .flex_col()
                .gap(px(S4))
                .children(children),
        )
    }

    fn tools_tabs(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let accent = self.accent().text;
        let mut track = segmented_track();
        for tool in Tool::ALL {
            let (stop, look) = self.stop_look(
                format!("seg:tools-tab:{}", tool.key()),
                super::tab::PAGE,
                true,
                window,
                cx,
            );
            track = track.child(
                segment(
                    ElementId::Name(format!("tools-tab-{}", tool.key()).into()),
                    tool.label(),
                    self.tools.tool == tool,
                    look,
                    accent,
                )
                .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                .on_click(cx.listener(move |this, _, _, cx| this.tools_select_tool(tool, cx))),
            );
        }
        div().px(px(self.margin())).pb(px(S5)).flex().child(track)
    }

    // ── Ortak parçalar ──────────────────────────────────────────────────

    fn tools_text(&self, text: impl Into<gpui::SharedString>) -> AnyElement {
        div()
            .pb(px(S2))
            .type_role(T_BODY)
            .text_color(muted())
            .child(text.into())
            .into_any_element()
    }

    fn tools_step(&self, title: impl Into<gpui::SharedString>, hint: Option<String>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(S0))
            .pt(px(S2))
            .child(
                div()
                    .type_role(T_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(title.into()),
            )
            .children(hint.map(|h| div().type_role(T_SECONDARY).text_color(muted()).child(h)))
            .into_any_element()
    }

    fn tools_note(&self, text: impl Into<gpui::SharedString>, problem: bool) -> AnyElement {
        div()
            .type_role(T_SECONDARY)
            .text_color(if problem {
                rgb(palette().danger_text)
            } else {
                label()
            })
            .child(text.into())
            .into_any_element()
    }

    /// Seçenek satırı: solda ad, sağda seçenek düğmeleri (← / → gruptaki komşuya).
    fn tools_options<F>(
        &self,
        group: &'static str,
        title: &str,
        options: &[(&'static str, &'static str)],
        current: &str,
        pick: F,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        F: Fn(&mut Shell, &'static str, &mut Context<Shell>) + Clone + 'static,
    {
        let accent = self.accent().text;
        let mut track = segmented_track();
        for (key, text_label) in options {
            let key: &'static str = key;
            let (stop, look) = self.stop_look(
                format!("seg:{group}:{key}"),
                super::tab::PAGE,
                true,
                window,
                cx,
            );
            let pick = pick.clone();
            track = track.child(
                segment(
                    ElementId::Name(format!("{group}-{key}").into()),
                    *text_label,
                    current == key,
                    look,
                    accent,
                )
                .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                .on_click(cx.listener(move |this, _, _, cx| pick(this, key, cx))),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(S4))
            .flex_wrap()
            .child(
                div()
                    .min_w(px(120.))
                    .type_role(T_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text())
                    .child(title.to_string()),
            )
            .child(track)
            .into_any_element()
    }

    /// Düğme: Tab durağı, odak halkası; `enabled` değilse pasif ve tıklanmaz.
    fn tools_action(
        &self,
        id: &'static str,
        primary: bool,
        icon_name: &'static str,
        label_text: String,
        enabled: bool,
        window: &Window,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
    ) -> AnyElement {
        let accent = self.accent().text;
        let (stop, look) = self.stop_look(id, super::tab::PAGE, enabled, window, cx);
        let el = if primary {
            primary_button_sized(id, icon_name, label_text, look, accent, false)
        } else {
            secondary_button(id, icon_name, label_text, look, accent)
        };
        let button = el
            .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
            .when(enabled, |el| {
                el.on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    this.leave_button(ev, window);
                    on_click(this, cx);
                }))
            });
        // Sütunda tam enine uzamasın: kendi genişliğinde.
        div().flex().flex_none().child(button).into_any_element()
    }

    /// Kaydırmalı seçim kutusu / tablo: tek Tab durağı; `row(i, cursor)` o satırın çizimi.
    fn tools_list_box(
        &self,
        id: BoxId,
        len: usize,
        window: &Window,
        cx: &mut Context<Self>,
        row: impl Fn(usize, bool, &mut Context<Self>) -> AnyElement,
    ) -> AnyElement {
        let accent = self.accent().text;
        let lb: ListBox = self.tools_box_value(id);
        let visible = id.visible();
        let (stop, look) = self.stop_look(id.key(), super::tab::PAGE, true, window, cx);
        let focused = look == Look::Focus;
        let active = focused || self.tools.active == Some(id);
        let shown = len.saturating_sub(lb.offset).min(visible);
        let rows: Vec<AnyElement> = (lb.offset..lb.offset + shown)
            .map(|i| row(i, active && lb.cursor == i, cx))
            .collect();
        let scroll = len > visible;
        let thumb = scroll.then(|| {
            // İnce kaydırma izi: kutunun yüksekliğinde orantılı bir çubuk.
            let track = visible as f32 * ROW_H;
            let h = (track * visible as f32 / len as f32).max(S5);
            let y = (track - h) * lb.offset as f32 / (len - visible) as f32;
            div()
                .absolute()
                .top(px(S1 + y))
                .right(px(S0))
                .w(px(S0 + S0))
                .h(px(h))
                .rounded_full()
                .bg(veil(LINE_HOVER))
        });
        div()
            .id(id.key())
            .relative()
            .flex()
            .flex_col()
            .h(px(visible as f32 * ROW_H + 2. * S1 + 2.))
            .p(px(S1))
            .rounded(px(R_LG))
            .overflow_hidden()
            .bg(shade(SHADE_INPUT))
            .border_1()
            .border_color(if focused {
                rgb(accent)
            } else {
                veil(LINE_INPUT)
            })
            .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
            .when(scroll, |el| el.occlude())
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                let notches = match ev.delta {
                    ScrollDelta::Lines(l) => l.y,
                    ScrollDelta::Pixels(p) => f32::from(p.y) / ROW_H,
                };
                if notches != 0. {
                    this.tools_box_wheel(
                        id,
                        -(notches.signum() as isize) * notches.abs().ceil().max(1.) as isize,
                        cx,
                    );
                    cx.stop_propagation();
                }
            }))
            // Klavyeyle odaktayken Enter / Boşluk gpui'de tıklamadır: imleçteki satır.
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.is_keyboard() {
                    let at = this.tools_box_value(id).cursor;
                    this.tools_box_activate(id, at, cx);
                }
            }))
            .children(rows)
            .children(thumb)
            .into_any_element()
    }

    /// Seçim kutusunun bir satırı: işaret, ad, şarkı sayısı.
    fn tools_pick_row(
        &self,
        id: BoxId,
        i: usize,
        cursor: bool,
        selected: bool,
        radio: bool,
        name: &str,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        div()
            .id((id.key(), i))
            .h(px(ROW_H))
            .px(px(S3))
            .rounded(px(R_MD))
            .flex()
            .items_center()
            .gap(px(S3))
            .cursor_pointer()
            .when(cursor, |el| el.bg(veil(VEIL_SELECTED)))
            .when(!cursor, |el| el.hover(|s| s.bg(veil(VEIL_HOVER))))
            .on_click(cx.listener(move |this, _, _, cx| this.tools_box_activate(id, i, cx)))
            .child(mark(selected, radio, accent))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .type_role(T_BODY)
                    .text_color(text())
                    .child(
                        line_with_tip(("tools-pick-name", i), name.to_string())
                            .flex()
                            .min_w_0(),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .type_role(T_SECONDARY)
                    .tabular()
                    .text_color(label())
                    .child(count.map_or_else(String::new, |n| tp!(Songs, n))),
            )
            .into_any_element()
    }

    /// Bu liste hâlâ yükleniyor mu / alınamadı mı: kutunun altındaki sessiz not.
    fn tools_load_note(&self, ids: &[crate::engine::CollectionId]) -> Option<AnyElement> {
        match self.tools_sources(ids) {
            Sources::Ready(_) => None,
            Sources::Loading => Some(self.tools_note(t!(ToolsLoading), false)),
            Sources::Failed => Some(self.tools_note(t!(ToolsLoadFailed), true)),
        }
    }

    /// Toplu işlem onay kartı: uyarı, vazgeç / onayla.
    fn tools_confirm_card(
        &self,
        bulk: Bulk,
        count: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let cancel = self.tools_action(
            "tools-confirm-cancel",
            false,
            "x",
            t!(Cancel).to_string(),
            true,
            window,
            cx,
            |this, cx| this.tools_cancel_confirm(cx),
        );
        let ok_bulk = bulk.clone();
        let ok = self.tools_action(
            "tools-confirm-ok",
            true,
            "check",
            t!(ToolsConfirmApply).to_string(),
            true,
            window,
            cx,
            move |this, cx| this.tools_request(ok_bulk.clone(), cx),
        );
        div()
            .p(px(S4))
            .rounded(px(R_LG))
            .border_1()
            .border_color(rgb(accent))
            .bg(accent_veil(accent, ACCENT_PLAYING))
            .flex()
            .flex_col()
            .gap(px(S3))
            .child(
                div()
                    .type_role(T_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(tf!(ToolsConfirmTitle, count)),
            )
            .child(
                div()
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(match bulk {
                        Bulk::Dupes => t!(ToolsConfirmBodyDupes),
                        Bulk::Merge => t!(ToolsConfirmBodyMerge),
                        Bulk::Split => t!(ToolsConfirmBodySplit),
                        Bulk::Sort => t!(ToolsConfirmBodySort),
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .child(ok)
                    .child(cancel),
            )
            .into_any_element()
    }

    // ── Yinelenenler ────────────────────────────────────────────────────

    fn dupes_tool(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![self.tools_text(t!(ToolsDupesIntro))];
        let items = self.tools_items(Source::Own);
        if matches!(self.library.library, Loadable::Loaded(_)) && items.is_empty() {
            out.push(self.tools_note(t!(ToolsNoOwnLists), false));
            return out;
        }
        out.push(self.tools_step(t!(ToolsPickList), None));
        let selected = self.tools.dup.list.clone();
        let items_ref = items.clone();
        out.push(
            self.tools_list_box(BoxId::DupPick, items.len(), window, cx, |i, cursor, cx| {
                let it = &items_ref[i];
                self.tools_pick_row(
                    BoxId::DupPick,
                    i,
                    cursor,
                    selected.as_ref() == Some(&it.id),
                    true,
                    &it.name,
                    it.count,
                    cx,
                )
            }),
        );
        let Some(id) = selected else {
            out.push(self.tools_note(t!(ToolsDupesPick), false));
            return out;
        };
        if let Some(note) = self.tools_load_note(std::slice::from_ref(&id)) {
            out.push(note);
            return out;
        }
        let Some(view) = self.tools_dup_view() else {
            return out;
        };
        if view.groups.is_empty() {
            out.push(
                div()
                    .py(px(S4))
                    .flex()
                    .justify_center()
                    .child(state_view(
                        "check",
                        t!(ToolsDupesNoneTitle),
                        t!(ToolsDupesNoneBody),
                    ))
                    .into_any_element(),
            );
            return out;
        }
        let removals = self.tools_dup_removals();
        let same = view
            .groups
            .iter()
            .filter(|g| g.kind == DupKind::SameTrack)
            .count();
        let versions = view.groups.len() - same;
        out.push(self.tools_step(
            format!(
                "{} · {}",
                tp!(ToolsDupeSets, view.groups.len()),
                tp!(
                    ToolsDupeExtra,
                    view.groups.iter().map(|g| g.removable()).sum::<usize>()
                )
            ),
            Some(tf!(ToolsDupesKinds, same, versions)),
        ));
        let album_col = self.window_width - self.dock_inset() >= ALBUM_MIN_WIDTH;
        let view_ref = view.clone();
        let spared = self.tools.dup.spared.clone();
        out.push(self.tools_list_box(
            BoxId::DupRows,
            view.rows.len(),
            window,
            cx,
            |i, cursor, cx| self.dup_row(&view_ref, i, cursor, &spared, album_col, cx),
        ));
        let n = removals.len();
        out.push(self.tools_note(t!(ToolsDupesHint), false));
        if self.tools.confirm == Some(Bulk::Dupes) {
            out.push(self.tools_confirm_card(Bulk::Dupes, n, window, cx));
        } else {
            out.push(self.tools_action(
                "tools-dupes-remove",
                true,
                "trash-2",
                tf!(ToolsDupesRemove, n),
                n > 0,
                window,
                cx,
                |this, cx| this.tools_request(Bulk::Dupes, cx),
            ));
        }
        out
    }

    fn dup_row(
        &self,
        view: &super::tools::DupView,
        i: usize,
        cursor: bool,
        spared: &std::collections::HashSet<usize>,
        album_col: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        match &view.rows[i] {
            DupRow::Header { kind, removable } => {
                let kind_text = match kind {
                    DupKind::SameTrack => t!(ToolsDupSame),
                    DupKind::Versions => t!(ToolsDupVersions),
                };
                div()
                    .id((BoxId::DupRows.key(), i))
                    .h(px(ROW_H))
                    .px(px(S3))
                    .rounded(px(R_MD))
                    .flex()
                    .items_center()
                    .gap(px(S2))
                    .cursor_pointer()
                    .when(cursor, |el| el.bg(veil(VEIL_SELECTED)))
                    .when(!cursor, |el| el.hover(|s| s.bg(veil(VEIL_HOVER))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tools_box_activate(BoxId::DupRows, i, cx)
                    }))
                    .type_role(T_SECONDARY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(kind_text)
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(label())
                            .child(format!("· {}", tf!(ToolsDupRemovable, removable))),
                    )
                    .into_any_element()
            }
            DupRow::Item { group, item } => {
                let it = &view.groups[*group].items[*item];
                let remove = !it.keep && !spared.contains(&it.index);
                let row: &TrackRow = &it.row;
                div()
                    .id((BoxId::DupRows.key(), i))
                    .h(px(ROW_H))
                    .pl(px(S5))
                    .pr(px(S3))
                    .rounded(px(R_MD))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .when(!it.keep, |el| el.cursor_pointer())
                    .when(cursor, |el| el.bg(veil(VEIL_SELECTED)))
                    .when(!cursor && !it.keep, |el| {
                        el.hover(|s| s.bg(veil(VEIL_HOVER)))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tools_box_activate(BoxId::DupRows, i, cx)
                    }))
                    .child(if it.keep {
                        div()
                            .size(px(18.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(super::theme::icon("pin", 14., label()))
                    } else {
                        mark(remove, false, accent)
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .type_role(T_BODY)
                                    .text_color(if remove || it.keep { text() } else { muted() })
                                    .child(
                                        line_with_tip(("tools-dup-title", i), row.name.clone())
                                            .flex()
                                            .min_w_0(),
                                    ),
                            )
                            .child(
                                div().flex().type_role(T_LABEL).text_color(label()).child(
                                    line_with_tip(("tools-dup-artist", i), row.artists.clone())
                                        .flex()
                                        .min_w_0(),
                                ),
                            ),
                    )
                    .when(album_col, |el| {
                        el.child(
                            div()
                                .w(px(S8 * 2.))
                                .flex_none()
                                .flex()
                                .type_role(T_SECONDARY)
                                .text_color(label())
                                .child(
                                    line_with_tip(("tools-dup-album", i), row.album.clone())
                                        .flex()
                                        .min_w_0(),
                                ),
                        )
                    })
                    .child(
                        div()
                            .w(px(S7))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .type_role(T_SECONDARY)
                            .tabular()
                            .text_color(label())
                            .child(super::player::fmt_time(row.duration_ms)),
                    )
                    .child(
                        div()
                            .w(px(S8 + S5))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .type_role(T_LABEL)
                            .text_color(if it.keep { text() } else { label() })
                            .child(if it.keep { t!(ToolsDupKeep) } else { "" }),
                    )
                    .into_any_element()
            }
        }
    }

    // ── Birleştir / böl ─────────────────────────────────────────────────

    fn merge_tool(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mode = self.tools.merge.mode;
        let mut out = vec![self.tools_text(t!(ToolsMergeIntro))];
        out.push(self.tools_options(
            "tools-mode",
            t!(ToolsMode),
            &[("merge", t!(ToolsModeMerge)), ("split", t!(ToolsModeSplit))],
            if mode == MergeMode::Merge {
                "merge"
            } else {
                "split"
            },
            |this, key, cx| {
                this.tools_set_mode(
                    if key == "split" {
                        MergeMode::Split
                    } else {
                        MergeMode::Merge
                    },
                    cx,
                )
            },
            window,
            cx,
        ));
        if self.tools.merge.review {
            out.extend(self.merge_review(window, cx));
        } else {
            out.extend(self.merge_setup(window, cx));
        }
        out
    }

    fn merge_setup(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mode = self.tools.merge.mode;
        let mut out = Vec::new();
        let items = self.tools_items(Source::Readable);
        if matches!(self.library.library, Loadable::Loaded(_))
            && items.len() < 2
            && mode == MergeMode::Merge
        {
            out.push(self.tools_note(t!(ToolsMergeTooFew), false));
        }
        out.push(match mode {
            MergeMode::Merge => self.tools_step(
                t!(ToolsPickLists),
                Some(tf!(ToolsPickedCount, self.tools.merge.picked.len())),
            ),
            MergeMode::Split => self.tools_step(t!(ToolsPickList), None),
        });
        let picked = self.tools_merge_selection();
        let items_ref = items.clone();
        out.push(self.tools_list_box(
            BoxId::MergePick,
            items.len(),
            window,
            cx,
            |i, cursor, cx| {
                let it = &items_ref[i];
                self.tools_pick_row(
                    BoxId::MergePick,
                    i,
                    cursor,
                    picked.contains(&it.id),
                    mode == MergeMode::Split,
                    &it.name,
                    it.count,
                    cx,
                )
            },
        ));
        match mode {
            MergeMode::Merge => out.push(self.tools_options(
                "tools-dedupe",
                t!(ToolsDropDupes),
                &[
                    ("off", t!(ToolsDedupeOff)),
                    ("same", t!(ToolsDedupeSame)),
                    ("versions", t!(ToolsDedupeVersions)),
                ],
                match self.tools.merge.dedupe {
                    DedupeMode::Off => "off",
                    DedupeMode::SameTrack => "same",
                    DedupeMode::Versions => "versions",
                },
                |this, key, cx| {
                    this.tools.merge.dedupe = match key {
                        "same" => DedupeMode::SameTrack,
                        "versions" => DedupeMode::Versions,
                        _ => DedupeMode::Off,
                    };
                    cx.notify();
                },
                window,
                cx,
            )),
            MergeMode::Split => out.push(self.tools_options(
                "tools-splitby",
                t!(ToolsSplitBy),
                &[
                    ("artist", t!(ToolsByArtist)),
                    ("album", t!(ToolsByAlbum)),
                    ("year", t!(ToolsByYear)),
                ],
                match self.tools.merge.split_by {
                    SplitBy::Artist => "artist",
                    SplitBy::Album => "album",
                    SplitBy::AddedYear => "year",
                },
                |this, key, cx| {
                    this.tools.merge.split_by = match key {
                        "album" => SplitBy::Album,
                        "year" => SplitBy::AddedYear,
                        _ => SplitBy::Artist,
                    };
                    this.tools.merge.review = false;
                    cx.notify();
                },
                window,
                cx,
            )),
        }
        if mode == MergeMode::Split && self.tools.merge.split_by == SplitBy::AddedYear {
            out.push(self.tools_note(t!(ToolsByYearNote), false));
        }
        let ids = self.tools_merge_selection();
        let ready =
            self.tools_merge_ready() && matches!(self.tools_sources(&ids), Sources::Ready(_));
        if let Some(note) = self.tools_load_note(&ids).filter(|_| !ids.is_empty()) {
            out.push(note);
        }
        out.push(self.tools_action(
            "tools-review",
            true,
            "list-music",
            t!(ToolsPreview).to_string(),
            ready,
            window,
            cx,
            |this, cx| this.tools_review(cx),
        ));
        out
    }

    fn merge_review(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mode = self.tools.merge.mode;
        let mut out =
            vec![self.tools_step(t!(ToolsReviewTitle), Some(t!(ToolsReviewHint).to_string()))];
        out.push(self.tools_name_field(mode, window, cx));
        let count;
        match mode {
            MergeMode::Merge => {
                let Some(plan) = self.tools_merge_plan() else {
                    return out;
                };
                count = plan.rows.len() + 1;
                out.push(self.tools_summary(&[
                    tp!(Songs, plan.rows.len()),
                    tp!(FolderLists, self.tools.merge.picked.len()),
                    if plan.dropped > 0 {
                        tf!(ToolsMergeDropped, plan.dropped)
                    } else {
                        String::new()
                    },
                ]));
                out.push(
                    self.tools_preview_rows(
                        plan.rows
                            .iter()
                            .take(PREVIEW_ROWS)
                            .map(|r| (r.name.clone(), r.artists.clone()))
                            .collect(),
                        plan.rows.len(),
                    ),
                );
            }
            MergeMode::Split => {
                let Some(groups) = self.tools_split_groups() else {
                    return out;
                };
                count = groups.iter().map(|g| g.rows.len() + 1).sum();
                out.push(self.tools_summary(&[tp!(ToolsNewLists, groups.len())]));
                let lines: Vec<(String, String)> = groups
                    .iter()
                    .take(PREVIEW_ROWS)
                    .map(|g| (self.tools_split_name(g), tp!(Songs, g.rows.len())))
                    .collect();
                out.push(self.tools_preview_rows(lines, groups.len()));
            }
        }
        out.push(self.tools_note(t!(ToolsSourcesStay), false));
        let can = !self.tools.merge.name.trim().is_empty() && count > 1;
        let back = self.tools_action(
            "tools-back",
            false,
            "arrow-left",
            t!(Back).to_string(),
            true,
            window,
            cx,
            |this, cx| this.tools_back_from_review(cx),
        );
        if let Some(bulk) = self.tools.confirm.clone() {
            out.push(self.tools_confirm_card(bulk, count, window, cx));
        } else {
            let bulk = if mode == MergeMode::Merge {
                Bulk::Merge
            } else {
                Bulk::Split
            };
            let apply = self.tools_action(
                "tools-apply",
                true,
                "plus",
                if mode == MergeMode::Merge {
                    t!(ToolsCreateList).to_string()
                } else {
                    t!(ToolsCreateLists).to_string()
                },
                can,
                window,
                cx,
                move |this, cx| this.tools_request(bulk.clone(), cx),
            );
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .child(apply)
                    .child(back)
                    .into_any_element(),
            );
        }
        out
    }

    /// Adı yazılan alan (Tab durağı; yazı `tools_key`'te).
    fn tools_name_field(
        &self,
        mode: MergeMode,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let (stop, look) = self.stop_look("tools-name", super::tab::PAGE, true, window, cx);
        let focused = look == Look::Focus;
        let name = self.tools.merge.name.clone();
        div()
            .flex()
            .items_center()
            .gap(px(S4))
            .flex_wrap()
            .child(
                div()
                    .min_w(px(120.))
                    .type_role(T_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text())
                    .child(if mode == MergeMode::Merge {
                        t!(ToolsListName)
                    } else {
                        t!(ToolsListPrefix)
                    }),
            )
            .child(
                div()
                    .id("tools-name")
                    .relative()
                    .w(px(NAME_FIELD_WIDTH))
                    .max_w_full()
                    .h(px(32.))
                    .px(px(S3))
                    .rounded(px(R_LG))
                    .flex()
                    .items_center()
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
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .child(line_with_tip("tools-name-text", name))
                            .child(
                                // Yazma imleci: odaktayken ince çizgi.
                                div().when(focused, |el| {
                                    el.ml(px(S0)).w(px(S0)).h(px(S4)).bg(rgb(accent))
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn tools_summary(&self, parts: &[String]) -> AnyElement {
        div()
            .type_role(T_BODY)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(text())
            .child(
                parts
                    .iter()
                    .filter(|p| !p.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
            .into_any_element()
    }

    /// Önizleme satırları: (üst yazı, alt yazı); `total` gösterilenden çoksa "ve N daha".
    fn tools_preview_rows(&self, lines: Vec<(String, String)>, total: usize) -> AnyElement {
        let shown = lines.len();
        div()
            .p(px(S1))
            .rounded(px(R_LG))
            .bg(shade(SHADE_INPUT))
            .border_1()
            .border_color(veil(LINE_INPUT))
            .flex()
            .flex_col()
            .children(lines.into_iter().enumerate().map(|(i, (title, sub))| {
                div()
                    .h(px(S7))
                    .px(px(S3))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .type_role(T_BODY)
                            .text_color(text())
                            .child(
                                line_with_tip(("tools-prev-title", i), title)
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .type_role(T_SECONDARY)
                            .text_color(label())
                            .child(sub),
                    )
            }))
            .when(total > shown, |el| {
                el.child(
                    div()
                        .h(px(S6))
                        .px(px(S3))
                        .flex()
                        .items_center()
                        .type_role(T_SECONDARY)
                        .text_color(label())
                        .child(tf!(ToolsAndMore, total - shown)),
                )
            })
            .into_any_element()
    }

    // ── Sıralama ────────────────────────────────────────────────────────

    fn sort_tool(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![self.tools_text(t!(ToolsSortIntro))];
        let items = self.tools_items(Source::Own);
        if matches!(self.library.library, Loadable::Loaded(_)) && items.is_empty() {
            out.push(self.tools_note(t!(ToolsNoOwnLists), false));
            return out;
        }
        out.push(self.tools_step(t!(ToolsPickList), None));
        let selected = self.tools.sort.list.clone();
        let items_ref = items.clone();
        out.push(
            self.tools_list_box(BoxId::SortPick, items.len(), window, cx, |i, cursor, cx| {
                let it = &items_ref[i];
                self.tools_pick_row(
                    BoxId::SortPick,
                    i,
                    cursor,
                    selected.as_ref() == Some(&it.id),
                    true,
                    &it.name,
                    it.count,
                    cx,
                )
            }),
        );
        out.push(self.tools_options(
            "tools-sortby",
            t!(ToolsSortBy),
            &[
                ("name", t!(ToolsByName)),
                ("artist", t!(ToolsByArtist)),
                ("album", t!(ToolsByAlbum)),
                ("added", t!(SortAdded)),
                ("duration", t!(ToolsByDuration)),
            ],
            match self.tools.sort.by {
                SortBy::Name => "name",
                SortBy::Artist => "artist",
                SortBy::Album => "album",
                SortBy::Added => "added",
                SortBy::Duration => "duration",
            },
            |this, key, cx| {
                let by = match key {
                    "artist" => SortBy::Artist,
                    "album" => SortBy::Album,
                    "added" => SortBy::Added,
                    "duration" => SortBy::Duration,
                    _ => SortBy::Name,
                };
                this.tools_set_sort(by, cx)
            },
            window,
            cx,
        ));
        out.push(self.tools_options(
            "tools-sortdir",
            t!(ToolsSortDir),
            &[("asc", t!(ToolsAsc)), ("desc", t!(ToolsDesc))],
            if self.tools.sort.dir == Dir::Asc {
                "asc"
            } else {
                "desc"
            },
            |this, key, cx| {
                this.tools_set_dir(if key == "desc" { Dir::Desc } else { Dir::Asc }, cx)
            },
            window,
            cx,
        ));
        let Some(id) = selected else {
            out.push(self.tools_note(t!(ToolsSortPick), false));
            return out;
        };
        if let Some(note) = self.tools_load_note(std::slice::from_ref(&id)) {
            out.push(note);
            return out;
        }
        let Some(view) = self.tools_sort_view() else {
            return out;
        };
        let (tracks, order, n) = (&view.tracks, &view.order, view.count);
        if self.tools.sort.by == SortBy::Added && !self.library.added.contains_key(&id) {
            out.push(self.tools_note(t!(ToolsSortDatesLoading), false));
        }
        if n == 0 {
            out.push(self.tools_summary(&[t!(ToolsSortAlready).to_string()]));
            return out;
        }
        out.push(self.tools_summary(&[tf!(ToolsSortMoves, n), tf!(ToolsSortOf, tracks.len())]));
        let first: Vec<(String, String)> = order
            .iter()
            .take(PREVIEW_ROWS)
            .enumerate()
            .map(|(k, &i)| {
                (
                    format!("{}. {}", k + 1, tracks[i].name),
                    tracks[i].artists.clone(),
                )
            })
            .collect();
        out.push(self.tools_preview_rows(first, tracks.len()));
        if n > MAX_SORT_MOVES {
            out.push(self.tools_note(tf!(ToolsSortTooMany, n, MAX_SORT_MOVES), true));
            return out;
        }
        out.push(self.tools_note(t!(ToolsSortHint), false));
        if self.tools.confirm == Some(Bulk::Sort) {
            out.push(self.tools_confirm_card(Bulk::Sort, n, window, cx));
        } else {
            out.push(self.tools_action(
                "tools-sort-apply",
                true,
                "arrow-up-down",
                tp!(ToolsSortApply, n),
                true,
                window,
                cx,
                |this, cx| this.tools_request(Bulk::Sort, cx),
            ));
        }
        out
    }

    // ── Yedek ───────────────────────────────────────────────────────────

    fn backup_tool(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![self.tools_text(t!(ToolsBackupIntro))];
        if let Loadable::Loaded(lib) = &self.library.library {
            out.push(self.tools_summary(&[
                tf!(ToolsBackupLists, lib.playlists.len()),
                tf!(ToolsBackupFolders, lib.meta.folders.len()),
                tf!(ToolsBackupPinned, lib.meta.pinned.len()),
                tf!(ToolsBackupArtists, lib.artists.len()),
            ]));
        }
        out.push(self.tools_note(t!(ToolsBackupNoRestore), false));
        let busy = matches!(
            self.tools.backup,
            BackupStatus::Choosing | BackupStatus::Writing
        );
        let ready = matches!(self.library.library, Loadable::Loaded(_));
        out.push(self.tools_action(
            "tools-backup-save",
            true,
            "cloud-upload",
            t!(ToolsBackupSave).to_string(),
            ready && !busy,
            window,
            cx,
            |this, cx| this.tools_backup(cx),
        ));
        match &self.tools.backup {
            BackupStatus::Idle => {}
            BackupStatus::Choosing => out.push(self.tools_note(t!(ToolsBackupChoosing), false)),
            BackupStatus::Writing => out.push(self.tools_note(t!(ToolsBackupWriting), false)),
            BackupStatus::Failed => out.push(self.tools_note(t!(ToolsBackupFailed), true)),
            BackupStatus::Done(s) => {
                let path = s.path.clone();
                out.push(self.tools_summary(&[
                    t!(ToolsBackupDone).to_string(),
                    tf!(ToolsBackupCounts, s.lists, s.tracks),
                ]));
                out.push(
                    div()
                        .flex()
                        .min_w_0()
                        .type_role(T_SECONDARY)
                        .text_color(muted())
                        .child(
                            line_with_tip("tools-backup-path", s.path.display().to_string())
                                .flex()
                                .min_w_0(),
                        )
                        .into_any_element(),
                );
                if s.missing > 0 {
                    out.push(self.tools_note(tf!(ToolsBackupMissing, s.missing), false));
                }
                out.push(self.tools_action(
                    "tools-backup-open",
                    false,
                    "folder-open",
                    t!(ToolsBackupShow).to_string(),
                    true,
                    window,
                    cx,
                    move |this, _| this.tools_open_backup_folder(&path),
                ));
            }
        }
        out
    }

    // ── İstatistik ──────────────────────────────────────────────────────

    fn stats_content(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let accent = self.accent().text;
        let mut out = Vec::new();
        // Aralık düğmeleri.
        let mut chips = div().flex().items_center().gap(px(S2)).flex_wrap();
        for range in StatsRange::ALL {
            let (stop, look) = self.stop_look(
                format!("tools-range-{range:?}"),
                super::tab::PAGE,
                true,
                window,
                cx,
            );
            let name = match range {
                StatsRange::Weeks4 => t!(ToolsRange4w),
                StatsRange::Months6 => t!(ToolsRange6m),
                StatsRange::All => t!(ToolsRangeAll),
            };
            chips = chips.child(
                chip(
                    ElementId::Name(format!("tools-range-{range:?}").into()),
                    name,
                    self.tools.stats.range == range,
                    look,
                    accent,
                )
                .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                .on_click(cx.listener(move |this, _, _, cx| this.tools_set_range(range, cx))),
            );
        }
        out.push(chips.into_any_element());
        let Some(report) = self.tools.stats.report.clone() else {
            out.push(self.tools_note(t!(ToolsLoading), false));
            return out;
        };
        if report.plays == 0 {
            out.push(
                div()
                    .py(px(S5))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(S4))
                    .child(state_view("music", t!(StatsEmptyTitle), t!(StatsEmptyBody)))
                    .child(self.tools_action(
                        "stats-import-empty",
                        true,
                        "download",
                        t!(HistoryImportButton).to_string(),
                        true,
                        window,
                        cx,
                        |this, cx| this.import_spotify_history(cx),
                    ))
                    .when_some(match &self.tools.history_import {
                        super::tools::ImportStatus::Done(s) => Some(tf!(HistoryImportDone, s.added, s.existing, s.skipped)),
                        super::tools::ImportStatus::Failed => Some(t!(HistoryImportFailed).to_string()),
                        _ => None,
                    }, |view, note| view.child(self.tools_note(note, false)))
                    .when(matches!(&self.tools.history_import, super::tools::ImportStatus::Done(s) if s.private_session), |view| view.child(self.tools_note(t!(HistoryImportedPrivate), false)))
                    .into_any_element(),
            );
            return out;
        }
        // Özet değerleri.
        out.push(
            div()
                .flex()
                .gap(px(S5))
                .flex_wrap()
                .child(self.stat_figure(report.plays.to_string(), t!(StatsPlays).to_string()))
                .child(self.stat_figure(
                    format!("{}", report.listened_ms / 60_000),
                    t!(StatsMinutes).to_string(),
                ))
                .child(self.stat_figure(
                    report.unique_tracks.to_string(),
                    t!(StatsTracks).to_string(),
                ))
                .child(self.stat_figure(
                    report.unique_artists.to_string(),
                    t!(StatsArtists).to_string(),
                ))
                .into_any_element(),
        );
        out.push(self.tools_note(
            format!(
                "{} · {}",
                listened_text(report.listened_ms),
                t!(StatsTotalTime)
            ),
            false,
        ));
        // Saat dağılımı.
        out.push(self.tools_step(t!(StatsHours), None));
        out.push(self.hours_chart(&report.hours, accent));
        out.push(self.tools_step(t!(StatsWeekdays), None));
        out.push(self.weekdays_chart(&report.weekdays, accent));
        out.push(self.tools_step(t!(ToolsTopTracks), None));
        out.push(self.stats_track_list(&report.tracks, cx));
        out.push(self.tools_step(t!(ToolsTopArtists), None));
        out.push(
            self.top_list(
                "tools-top-artists",
                report
                    .artists
                    .iter()
                    .take(TOP_N)
                    .map(|x| (x.name.clone(), String::new(), x.plays))
                    .collect(),
            ),
        );
        out.push(self.tools_step(t!(ToolsTopAlbums), None));
        out.push(
            self.top_list(
                "tools-top-albums",
                report
                    .albums
                    .iter()
                    .take(TOP_N)
                    .map(|x| (x.album.clone(), x.artist.clone(), x.plays))
                    .collect(),
            ),
        );
        let status = match &self.tools.history_import {
            super::tools::ImportStatus::Choosing | super::tools::ImportStatus::Importing => {
                Some(t!(HistoryImporting).to_string())
            }
            super::tools::ImportStatus::Done(summary) => Some(tf!(
                HistoryImportDone,
                summary.added,
                summary.existing,
                summary.skipped
            )),
            super::tools::ImportStatus::Failed => Some(t!(HistoryImportFailed).to_string()),
            super::tools::ImportStatus::Idle => None,
        };
        if let Some(status) = status {
            out.push(self.tools_note(
                status,
                matches!(
                    self.tools.history_import,
                    super::tools::ImportStatus::Failed
                ),
            ));
        }
        out.push(self.tools_action(
            "stats-import",
            true,
            "download",
            t!(HistoryImportButton).to_string(),
            true,
            window,
            cx,
            |this, cx| this.import_spotify_history(cx),
        ));
        if matches!(&self.tools.history_import, super::tools::ImportStatus::Done(s) if s.private_session)
        {
            out.push(self.tools_note(t!(HistoryImportedPrivate), false));
        }
        out.push(self.tools_note(t!(StatsLocal), false));
        out
    }

    fn stats_track_list(
        &self,
        rows: &[crate::engine::tools::stats::TopTrack],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .children(rows.iter().enumerate().map(|(i, row)| {
                let row = row.clone();
                let cover = cover_image(&self.images, &row.image, &row.name, 40., R_SM, "music");
                div()
                    .id(("stats-track", i))
                    .min_h(px(ROW_H))
                    .px(px(S3))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .rounded(px(R_MD))
                    .hover(|s| s.bg(veil(VEIL_HOVER)))
                    .child(
                        div()
                            .w(px(S3))
                            .type_role(T_LABEL)
                            .text_color(label())
                            .child((i + 1).to_string()),
                    )
                    .child(cover)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .type_role(T_SECONDARY)
                                    .text_color(text())
                                    .truncate()
                                    .child(row.name.clone()),
                            )
                            .child(
                                div()
                                    .type_role(T_LABEL)
                                    .text_color(muted())
                                    .truncate()
                                    .child(row.artists.clone()),
                            ),
                    )
                    .child(
                        div()
                            .type_role(T_LABEL)
                            .text_color(muted())
                            .child(tp!(ToolsPlays, row.plays)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.play_stats_track(&row, cx)))
            }))
            .into_any_element()
    }

    fn weekdays_chart(&self, days: &[u64; 7], accent: u32) -> AnyElement {
        let max = days.iter().copied().max().unwrap_or(1).max(1);
        let names = [
            t!(WeekdayMon),
            t!(WeekdayTue),
            t!(WeekdayWed),
            t!(WeekdayThu),
            t!(WeekdayFri),
            t!(WeekdaySat),
            t!(WeekdaySun),
        ];
        let bars = (0..7).map(|d| {
            div().flex_1().h(px(HOURS_HEIGHT)).flex().items_end().child(
                div()
                    .w_full()
                    .h(px(if days[d] == 0 {
                        S0
                    } else {
                        (HOURS_HEIGHT * days[d] as f32 / max as f32).max(S1)
                    }))
                    .rounded(px(R_XS))
                    .bg(if days[d] == max && days[d] > 0 {
                        rgb(accent)
                    } else {
                        accent_veil(accent, 0.45)
                    }),
            )
        });
        let axis = names.into_iter().map(|name| {
            div()
                .flex_1()
                .flex()
                .justify_center()
                .type_role(T_LABEL)
                .text_color(label())
                .child(name)
        });
        div()
            .flex()
            .flex_col()
            .gap(px(S1))
            .child(div().flex().gap(px(S0)).items_end().children(bars))
            .child(div().flex().gap(px(S0)).children(axis))
            .into_any_element()
    }

    fn stat_figure(&self, value: String, caption: String) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .font_family(font())
                    .type_role(T_HEADLINE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .tabular()
                    .text_color(text())
                    .child(value),
            )
            .child(
                div()
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(caption),
            )
            .into_any_element()
    }

    /// 24 sütun: saat başına dinlenen süre; en yoğun saat vurgu renginde.
    fn hours_chart(&self, hours: &[u64; 24], accent: u32) -> AnyElement {
        let max = hours.iter().copied().max().unwrap_or(0).max(1);
        let peak = hours
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(h, _)| h)
            .unwrap_or(0);
        let bars =
            (0..24).map(|h| {
                let frac = hours[h] as f32 / max as f32;
                let height = if hours[h] == 0 {
                    S0
                } else {
                    (HOURS_HEIGHT * frac).max(S1)
                };
                div().flex_1().h(px(HOURS_HEIGHT)).flex().items_end().child(
                    div().w_full().h(px(height)).rounded(px(R_XS)).bg(
                        if h == peak && hours[h] > 0 {
                            rgb(accent)
                        } else {
                            accent_veil(accent, 0.45)
                        },
                    ),
                )
            });
        let axis = (0..24).map(|h| {
            div()
                .flex_1()
                .flex()
                .justify_center()
                .type_role(T_LABEL)
                .tabular()
                .text_color(label())
                .child(if h % 6 == 0 {
                    h.to_string()
                } else {
                    String::new()
                })
        });
        div()
            .flex()
            .flex_col()
            .gap(px(S1))
            .child(div().flex().gap(px(S0)).items_end().children(bars))
            .child(div().flex().gap(px(S0)).children(axis))
            .child(
                div()
                    .pt(px(S1))
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(tf!(ToolsPeakHour, format!("{peak:02}:00"))),
            )
            .into_any_element()
    }

    /// En çok dinlenenler: sıra, ad (+ alt yazı), çalma sayısı.
    fn top_list(&self, id: &'static str, rows: Vec<(String, String, u32)>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .children(rows.into_iter().enumerate().map(|(i, (name, sub, plays))| {
                div()
                    .id((id, i))
                    .min_h(px(ROW_H))
                    .py(px(S1))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .border_t_1()
                    .border_color(if i == 0 {
                        gpui::transparent_black()
                    } else {
                        veil(LINE).into()
                    })
                    .child(
                        div()
                            .w(px(S5))
                            .flex_none()
                            .type_role(T_SECONDARY)
                            .tabular()
                            .text_color(label())
                            .child((i + 1).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .type_role(T_BODY)
                                    .text_color(text())
                                    .child(line_with_tip((id, i + 100), name).flex().min_w_0()),
                            )
                            .children((!sub.is_empty()).then(|| {
                                div()
                                    .flex()
                                    .type_role(T_LABEL)
                                    .text_color(label())
                                    .child(line_with_tip((id, i + 200), sub).flex().min_w_0())
                            })),
                    )
                    .child(
                        div()
                            .flex_none()
                            .type_role(T_SECONDARY)
                            .tabular()
                            .text_color(muted())
                            .child(tp!(ToolsPlays, plays)),
                    )
            }))
            .into_any_element()
    }
}
