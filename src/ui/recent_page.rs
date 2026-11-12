//! "Son çalınanlar" sayfası (`Route::Recent`): çalınan listeler ve albümler,
//! en yenisi başta, kart ızgarasında.
//!
//! Kaynak motorun kendi kaydı: bir liste çalmaya başlanınca `Settings::recent`
//! başına yazılır (en çok `RECENT_MAX`, bkz. `settings::note_played`); raf
//! sırası, ana sayfa ve palet de aynı kayda bakar. Kayıt yalnız bağlam
//! anahtarı tutar: kütüphanede olmayan bir bağlam (silinmiş liste, kaydedilmemiş
//! albüm) gösterilemez ve atlanır (`engine::home`). Tek şarkı çalmalar
//! kaydedilmez.

use gpui::{Context, Div, Window, div, prelude::*, px};

use super::{
    Shell, components::secondary_button, history::Tab, library::Loadable, pages::collection_card,
};
use crate::{t, tp};

impl Shell {
    pub(super) fn recent_body(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let Loadable::Loaded(library) = &self.library.library else {
            return self.page_skeleton_plain(false);
        };
        let items = crate::engine::home(library, &self.recent, crate::settings::RECENT_MAX).recent;
        if items.is_empty() {
            return self.recent_empty(window, cx);
        }
        let meta = div()
            .child(tp!(RecentPageCount, items.len()))
            .into_any_element();
        let cards: Vec<_> = items.iter().map(collection_card).collect();
        div()
            .flex()
            .flex_col()
            .child(self.page_header(None, t!(RecentPageTitle), meta, Vec::new()))
            .child(self.card_grid(&cards, cx))
    }

    /// Henüz bir şey çalınmadı: hapın üstünde kalan alanın ortasında sakin
    /// durum görünümü ve tek eylem (`page_error` ile aynı yerleşim).
    fn recent_empty(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let accent = self.accent().text;
        let height = f32::from(window.viewport_size().height) - super::TITLEBAR_HEIGHT;
        let (stop, look) = self.stop_look("recent-explore", super::tab::PAGE, true, window, cx);
        div()
            .h(px(height))
            .flex()
            .items_center()
            .justify_center()
            .child(
                super::components::state_view("music", t!(RecentEmptyTitle), t!(RecentEmptyBody))
                    .child(
                        secondary_button(
                            "recent-explore",
                            "compass",
                            t!(RecentEmptyAction),
                            look,
                            accent,
                        )
                        .when_some(stop, |el, h| el.track_focus(&h))
                        .on_click(cx.listener(|this, _, _, cx| this.go_tab(Tab::Explore, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{CollectionId, Library, LibraryItem, home};

    fn item(id: CollectionId, name: &str) -> LibraryItem {
        LibraryItem {
            id,
            name: name.into(),
            subtitle: "12 şarkı".into(),
            image: String::new(),
        }
    }

    /// Sayfa `engine::home`un çözdüğü listeyi gösterir: sıra kayıttaki sıra,
    /// kütüphanede olmayan bağlam atlanır, Beğenilenler gelir.
    #[test]
    fn page_lists_recent_contexts_in_order_and_skips_unknown_ones() {
        let library = Library {
            liked_count: 7,
            albums: vec![item(CollectionId::Album("spotify:album:a".into()), "Albüm")],
            playlists: vec![item(
                CollectionId::Playlist("spotify:playlist:p".into()),
                "Liste",
            )],
            artists: Vec::new(),
            ..Default::default()
        };
        let recent: Vec<String> = [
            "spotify:playlist:p",
            "gone",
            "liked",
            "spotify:album:a",
            "spotify:playlist:silinmis",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let shown: Vec<String> = crate::i18n::with_lang(crate::i18n::Lang::Tr, || {
            home(&library, &recent, crate::settings::RECENT_MAX)
                .recent
                .into_iter()
                .map(|i| i.name)
                .collect()
        });
        assert_eq!(shown, ["Liste", "Beğenilenler", "Albüm"]);
        // Kayıt boşsa sayfa boş durumu gösterir.
        assert!(
            home(&library, &[], crate::settings::RECENT_MAX)
                .recent
                .is_empty()
        );
    }
}
