use super::components::{Look, Tooltip, icon_button, secondary_button};
use super::history::Route;
use super::*;
use crate::{
    engine::{AddPosition, EditOp, PlaylistHistoryChangeKind as ChangeKind, TrackRow},
    tf,
};

impl Shell {
    pub(super) fn playlist_history_body(&mut self, uri: &str, cx: &mut Context<Self>) -> gpui::Div {
        let id = CollectionId::Playlist(uri.to_string());
        let title = self
            .library
            .item(&id)
            .map(|item| item.name.clone())
            .unwrap_or_else(|| uri.to_string());
        let subtitle = div().child(t!(PlaylistHistoryContext)).into_any_element();
        let mut body = div().flex().flex_col().gap(px(S4)).child(self.page_header(
            None,
            &title,
            subtitle,
            Vec::new(),
        ));
        match self.library.collections.get(&id) {
            Some(super::library::Loadable::Loading) => {
                return body
                    .child(
                        div()
                            .px(px(self.margin()))
                            .type_role(T_BODY)
                            .text_color(muted())
                            .child(t!(PlaylistHistoryLoading)),
                    )
                    .child(history_footer_note(self.margin()));
            }
            Some(super::library::Loadable::Failed(_)) => {
                return body
                    .child(
                        div()
                            .px(px(self.margin()))
                            .type_role(T_BODY)
                            .text_color(muted())
                            .child(t!(PlaylistHistoryError)),
                    )
                    .child(history_footer_note(self.margin()));
            }
            _ => {}
        }
        if !engine::playlist_history_enabled() {
            return body
                .child(
                    div()
                        .px(px(self.margin()))
                        .type_role(T_BODY)
                        .text_color(muted())
                        .child(t!(PlaylistHistoryDisabled)),
                )
                .child(history_footer_note(self.margin()));
        }
        let snapshot = match engine::playlist_history(&id) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Liste geçmişi okunamadı: {e:#}");
                return body
                    .child(
                        div()
                            .px(px(self.margin()))
                            .type_role(T_BODY)
                            .text_color(muted())
                            .child(t!(PlaylistHistoryError)),
                    )
                    .child(history_footer_note(self.margin()));
            }
        };
        let Some(snapshot) = snapshot else {
            return body
                .child(
                    div()
                        .px(px(self.margin()))
                        .type_role(T_BODY)
                        .text_color(muted())
                        .child(t!(PlaylistHistoryEmpty)),
                )
                .child(history_footer_note(self.margin()));
        };
        if snapshot.events.is_empty() {
            return body
                .child(
                    div()
                        .px(px(self.margin()))
                        .type_role(T_BODY)
                        .text_color(muted())
                        .child(t!(PlaylistHistoryEmpty)),
                )
                .child(history_footer_note(self.margin()));
        }
        let uris: Vec<String> = snapshot.events.iter().map(|e| e.uri.clone()).collect();
        let names = engine::playlist_history_tracks(&uris).unwrap_or_default();
        let own = self.own_playlists().iter().any(|p| p.id == id);
        if !own {
            body = body.child(
                div()
                    .p(px(S3))
                    .mx(px((self.margin() - S3).max(0.)))
                    .rounded(px(R_MD))
                    .bg(veil(VEIL_HOVER))
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(t!(PlaylistHistoryReadOnly)),
            );
        }
        let mut grouped: std::collections::BTreeMap<String, Vec<_>> =
            std::collections::BTreeMap::new();
        for event in &snapshot.events {
            let day = crate::engine::tools::civil::date_stamp(
                event.at,
                crate::engine::tools::civil::local_offset_minutes(),
            );
            grouped.entry(day).or_default().push(event);
        }
        let current = self
            .library
            .tracks(&id)
            .map(|tracks| tracks.iter().map(|t| t.uri.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        let offset = crate::engine::tools::civil::local_offset_minutes();
        let now = system_now_ms();
        for (_day, events) in grouped.into_iter().rev() {
            let added = events
                .iter()
                .filter(|e| e.kind == ChangeKind::Added)
                .count();
            let removed = events
                .iter()
                .filter(|e| e.kind == ChangeKind::Removed)
                .count();
            let moved = events
                .iter()
                .filter(|e| e.kind == ChangeKind::Moved)
                .count();
            let mut summary = format!(
                "{} · {}",
                tf!(PlaylistHistoryRemoved, removed),
                tf!(PlaylistHistoryAdded, added)
            );
            if moved > 0 {
                summary.push_str(&format!(" · {}", tf!(PlaylistHistoryMovedCount, moved)));
            }
            let at = events.last().map(|e| e.at).unwrap_or(snapshot.baseline_at);
            let mut group = div()
                .flex()
                .flex_col()
                .gap(px(S2))
                .p(px(S3))
                .mx(px((self.margin() - S3).max(0.)))
                .rounded(px(R_LG))
                .bg(shade(SHADE_INPUT))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(S3))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(S1))
                                .child(
                                    div()
                                        .type_role(T_BODY)
                                        .text_color(text())
                                        .child(history_date_label(at, now, offset)),
                                )
                                .child(
                                    div()
                                        .type_role(T_SECONDARY)
                                        .text_color(muted())
                                        .child(summary),
                                ),
                        )
                        .when(
                            own && !self
                                .history_preview
                                .as_ref()
                                .is_some_and(|(key, selected)| key == uri && *selected == at),
                            |el| {
                                let uri = uri.to_string();
                                el.child(
                                    secondary_button(
                                        ("history-restore-preview", at as u64),
                                        "refresh-cw",
                                        t!(PlaylistHistoryRestored),
                                        Look::Live,
                                        self.accent().text,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.history_preview = Some((uri.clone(), at));
                                            cx.notify();
                                        },
                                    )),
                                )
                            },
                        ),
                );
            for (row_ix, event) in events.iter().rev().enumerate() {
                let row = names.get(&event.uri);
                let track = row.cloned().unwrap_or_else(|| history_row(&event.uri));
                let description = match event.kind {
                    ChangeKind::Added => t!(PlaylistHistoryAddedOne).to_string(),
                    ChangeKind::Removed => t!(PlaylistHistoryRemovedOne).to_string(),
                    ChangeKind::Moved => t!(PlaylistHistoryMoved).to_string(),
                };
                let description = format!(
                    "{}{}{}",
                    if event.source == "sen" {
                        format!("{} · ", t!(PlaylistHistoryYou))
                    } else {
                        String::new()
                    },
                    description,
                    event
                        .added_by
                        .as_ref()
                        .map(|by| format!(" · {}", tf!(PlaylistHistoryAddedBy, by)))
                        .unwrap_or_default(),
                );
                let local_secs = event.at.div_euclid(1000) + offset * 60;
                let time = format!(
                    "{:02}:{:02}",
                    local_secs.rem_euclid(86_400) / 3600,
                    local_secs.rem_euclid(3600) / 60,
                );
                let action_slot: AnyElement = if own && event.kind == ChangeKind::Removed {
                    let uri = event.uri.clone();
                    let pos = event.from.unwrap_or(usize::MAX);
                    let add_back_track = track.clone();
                    let forced_hover = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
                        && std::env::var("OPENIFY_TEST_HISTORY_HOVER")
                            .ok()
                            .is_some_and(|uri| uri == event.uri);
                    icon_button(
                        ("history-add-back", row_ix),
                        "plus",
                        Look::Live,
                        self.accent().text,
                    )
                    .tooltip(Tooltip::text(t!(PlaylistHistoryReadd), None))
                    .opacity(if forced_hover { 1. } else { 0. })
                    .group_hover("playlist-history-row", |el| el.opacity(1.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_history_track(uri.clone(), pos, add_back_track.clone(), cx)
                    }))
                    .into_any_element()
                } else {
                    div()
                        .size(px(super::components::ICON_BUTTON_SIZE))
                        .flex_none()
                        .into_any_element()
                };
                let track_line = history_event_row(
                    &self.images,
                    &track,
                    &description,
                    &time,
                    row_ix,
                    action_slot,
                );
                group = group.child(track_line);
            }
            let preview = if self
                .history_preview
                .as_ref()
                .is_some_and(|(key, _)| key == uri)
            {
                let (restore_uri, selected) = self.history_preview.clone().unwrap();
                if selected == at {
                    let target =
                        crate::engine::playlist_history_state_at(&current, &snapshot.events, at);
                    let changes = crate::engine::playlist_history_diff(&current, &target);
                    let additions = changes
                        .iter()
                        .filter(|c| matches!(c.kind, ChangeKind::Added | ChangeKind::Moved))
                        .count();
                    let removals = changes
                        .iter()
                        .filter(|c| matches!(c.kind, ChangeKind::Removed | ChangeKind::Moved))
                        .count();
                    Some((restore_uri, target, additions, removals))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((restore_uri, target, additions, removals)) = preview {
                group = group.child(
                    div()
                        .mt(px(S2))
                        .p(px(S3))
                        .rounded(px(R_MD))
                        .bg(veil(VEIL_HOVER))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(S3))
                        .child(div().type_role(T_SECONDARY).text_color(muted()).child(tf!(
                            PlaylistHistoryPreview,
                            additions,
                            removals
                        )))
                        .child(
                            secondary_button(
                                "history-restore-confirm",
                                "refresh-cw",
                                t!(PlaylistHistoryConfirm),
                                Look::Live,
                                self.accent().text,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.restore_history_version(&restore_uri, target.clone(), cx)
                                },
                            )),
                        ),
                );
            }
            body = body.child(group);
        }
        if !own {
            let rows: Vec<TrackRow> = current
                .iter()
                .map(|u| {
                    self.library
                        .tracks(&id)
                        .and_then(|r| r.iter().find(|t| &t.uri == u))
                        .cloned()
                        .unwrap_or_else(|| history_row(u))
                })
                .collect();
            body =
                body.child(
                    secondary_button(
                        "history-copy",
                        "copy",
                        t!(PlaylistHistoryCopy),
                        Look::Live,
                        self.accent().text,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_new_playlist(rows.clone(), false, cx)
                    })),
                );
        }
        body.child(history_footer_note(self.margin()))
    }

    fn add_history_track(
        &mut self,
        uri: String,
        position: usize,
        row: TrackRow,
        cx: &mut Context<Self>,
    ) {
        let Route::PlaylistHistory(playlist) = self.pages.history.current().clone() else {
            return;
        };
        let current_len = self
            .library
            .tracks(&CollectionId::Playlist(playlist.clone()))
            .map_or(0, |r| r.len());
        let at = if position == usize::MAX {
            current_len
        } else {
            position.min(current_len)
        };
        let op = EditOp::AddToPlaylist {
            playlist: playlist.clone(),
            uris: vec![uri.clone()],
            position: AddPosition::At(at as u32),
        };
        let inverse = EditOp::RemoveFromPlaylist {
            playlist,
            uris: vec![uri],
            indices: vec![at as u32],
        };
        self.commit(
            vec![(op, vec![row], None)],
            Some((
                t!(PlaylistHistoryReadd).to_string(),
                vec![(inverse, Vec::new(), None)],
            )),
            cx,
        );
    }

    fn restore_history_version(&mut self, uri: &str, target: Vec<String>, cx: &mut Context<Self>) {
        let id = CollectionId::Playlist(uri.to_string());
        let Some(current_rows) = self.library.tracks(&id).map(|r| r.as_ref().clone()) else {
            return;
        };
        let current: Vec<String> = current_rows.iter().map(|r| r.uri.clone()).collect();
        let changes = crate::engine::playlist_history_diff(&current, &target);
        let mut removals: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c.kind, ChangeKind::Removed | ChangeKind::Moved))
            .cloned()
            .collect();
        removals.sort_by_key(|c| std::cmp::Reverse(c.from.unwrap_or(0)));
        let mut additions: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c.kind, ChangeKind::Added | ChangeKind::Moved))
            .cloned()
            .collect();
        additions.sort_by_key(|c| c.to.unwrap_or(0));
        let mut ops = Vec::new();
        let mut undo_removed = Vec::new();
        for change in removals {
            let at = change.from.unwrap_or(0);
            let row = current_rows
                .iter()
                .find(|r| r.uri == change.uri)
                .cloned()
                .unwrap_or_else(|| history_row(&change.uri));
            ops.push((
                EditOp::RemoveFromPlaylist {
                    playlist: uri.into(),
                    uris: vec![change.uri.clone()],
                    indices: vec![at as u32],
                },
                Vec::new(),
                None,
            ));
            undo_removed.push((
                EditOp::AddToPlaylist {
                    playlist: uri.into(),
                    uris: vec![change.uri],
                    position: AddPosition::At(at as u32),
                },
                vec![row],
                None,
            ));
        }
        let lookups = engine::playlist_history_tracks(
            &additions.iter().map(|c| c.uri.clone()).collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let mut undo_added = Vec::new();
        for change in additions {
            let position = change.to.unwrap_or(0);
            let row = current_rows
                .iter()
                .find(|r| r.uri == change.uri)
                .cloned()
                .or_else(|| lookups.get(&change.uri).cloned())
                .unwrap_or_else(|| history_row(&change.uri));
            ops.push((
                EditOp::AddToPlaylist {
                    playlist: uri.into(),
                    uris: vec![change.uri.clone()],
                    position: AddPosition::At(position as u32),
                },
                vec![row],
                None,
            ));
            undo_added.push((
                EditOp::RemoveFromPlaylist {
                    playlist: uri.into(),
                    uris: vec![change.uri],
                    indices: vec![position as u32],
                },
                Vec::new(),
                None,
            ));
        }
        let undo = undo_added
            .into_iter()
            .rev()
            .chain(undo_removed.into_iter().rev())
            .collect();
        if ops.is_empty() {
            self.history_preview = None;
            cx.notify();
            return;
        }
        self.history_preview = None;
        self.commit(
            ops,
            Some((t!(PlaylistHistoryRestored).to_string(), undo)),
            cx,
        );
    }
}

fn history_row(uri: &str) -> TrackRow {
    TrackRow {
        uri: uri.to_string(),
        name: uri.to_string(),
        artists: String::new(),
        album: String::new(),
        duration_ms: 0,
        image: String::new(),
        plays: None,
        features: None,
    }
}

fn history_event_row(
    images: &super::images::ImageStore,
    track: &TrackRow,
    status: &str,
    time: &str,
    row_ix: usize,
    action_slot: AnyElement,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("playlist-history-row", row_ix))
        .group("playlist-history-row")
        .w_full()
        .h(px(super::library::ROW_HEIGHT))
        .px(px(S3))
        .rounded(px(R_MD))
        .flex()
        .items_center()
        .gap(px(S4))
        .hover(|el| el.bg(veil(VEIL_HOVER)))
        .child(super::cover_image(
            images,
            &track.image,
            &track.album,
            super::library::ROW_THUMB,
            R_SM,
            "music",
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .justify_center()
                .child(
                    super::text::line_with_tip(("history-track-title", row_ix), track.name.clone())
                        .type_role(T_BODY)
                        .text_color(text()),
                )
                .child(
                    super::text::line_with_tip(
                        ("history-track-artist", row_ix),
                        track.artists.clone(),
                    )
                    .type_role(T_SECONDARY)
                    .text_color(muted()),
                ),
        )
        .child(
            div()
                .w(px(super::library::PLAYS_COLUMN))
                .flex_none()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(S1))
                .type_role(T_LABEL)
                .text_color(muted())
                .tabular()
                .child(time.to_string())
                .child(status.to_string()),
        )
        .child(action_slot)
}

fn history_footer_note(margin: f32) -> gpui::Div {
    div()
        .px(px(margin))
        .mt(px(S5))
        .type_role(T_LABEL)
        .text_color(label())
        .child(t!(PlaylistHistoryPrivateNote))
}

fn history_date_label(at_ms: i64, now_ms: i64, offset_minutes: i64) -> String {
    use crate::i18n::K;

    let at_day = crate::engine::tools::civil::date_stamp(at_ms, offset_minutes);
    let today = crate::engine::tools::civil::date_stamp(now_ms, offset_minutes);
    if at_day == today {
        return t!(PlaylistHistoryToday).into();
    }
    let yesterday = crate::engine::tools::civil::date_stamp(now_ms - 86_400_000, offset_minutes);
    if at_day == yesterday {
        return t!(PlaylistHistoryYesterday).into();
    }
    let local_secs = at_ms.div_euclid(1000) + offset_minutes * 60;
    let (year, month, day) =
        crate::engine::tools::civil::civil_from_days(local_secs.div_euclid(86_400));
    const MONTHS: [K; 12] = [
        crate::k!(PlaylistHistoryMonthJanuary),
        crate::k!(PlaylistHistoryMonthFebruary),
        crate::k!(PlaylistHistoryMonthMarch),
        crate::k!(PlaylistHistoryMonthApril),
        crate::k!(PlaylistHistoryMonthMay),
        crate::k!(PlaylistHistoryMonthJune),
        crate::k!(PlaylistHistoryMonthJuly),
        crate::k!(PlaylistHistoryMonthAugust),
        crate::k!(PlaylistHistoryMonthSeptember),
        crate::k!(PlaylistHistoryMonthOctober),
        crate::k!(PlaylistHistoryMonthNovember),
        crate::k!(PlaylistHistoryMonthDecember),
    ];
    let month = crate::i18n::t(MONTHS[(month as usize - 1).min(11)]);
    if year == crate::engine::tools::civil::year_of(now_ms, offset_minutes) {
        tf!(PlaylistHistoryDateMonthDay, month, day)
    } else {
        tf!(PlaylistHistoryDateMonthDayYear, month, day, year)
    }
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use crate::i18n::{Lang, with_lang};

    #[test]
    fn history_dates_are_relative_and_localized_with_year_only_when_needed() {
        let now = crate::engine::tools::civil::unix_ms(2026, 11, 15, 12, 0, 0, 0);
        let today = crate::engine::tools::civil::unix_ms(2026, 11, 15, 8, 30, 0, 0);
        let yesterday = crate::engine::tools::civil::unix_ms(2026, 11, 14, 8, 30, 0, 0);
        let same_year = crate::engine::tools::civil::unix_ms(2026, 11, 12, 8, 30, 0, 0);
        let old_year = crate::engine::tools::civil::unix_ms(2025, 11, 12, 8, 30, 0, 0);
        with_lang(Lang::En, || {
            assert_eq!(super::history_date_label(today, now, 0), "Today");
            assert_eq!(super::history_date_label(yesterday, now, 0), "Yesterday");
            assert_eq!(super::history_date_label(same_year, now, 0), "November 12");
            assert_eq!(
                super::history_date_label(old_year, now, 0),
                "November 12, 2025"
            );
        });
        with_lang(Lang::Tr, || {
            assert_eq!(super::history_date_label(today, now, 0), "Bugün");
            assert_eq!(super::history_date_label(yesterday, now, 0), "Dün");
            assert_eq!(super::history_date_label(same_year, now, 0), "12 Kasım");
            assert_eq!(super::history_date_label(old_year, now, 0), "12 Kasım 2025");
        });
    }
}
