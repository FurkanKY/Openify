//! Kütüphane düzeni: klasörler, sabitleme, sıralama menüsü, klavyeyle gezinme.
//!
//! Veri `Library::meta`dan gelir (`engine::library`): Spotify'ın klasörleri
//! kök listeden OKUNUR, sabitlemeler yerel (`library_pins`). Klasör
//! oluşturma / yeniden adlandırma / taşıma YOK: edit kuyruğunda kök liste
//! işlemi bulunmuyor, yeni canlı yazma eklenmedi.
//!
//! Görünüm: liste görünümünde klasörler satır arasında açılır / kapanır
//! (açık olanlar `Settings::library_open_folders`, kalıcı kimlikle); ızgarada
//! klasör kartına tıklayınca içine girilir (üstte yol çubuğu, Esc / Backspace /
//! geri düğmesi bir üst klasöre). Sıralama ve süzme yalnız `shelf.rs`te; ağacın
//! saf mantığı `library_tree.rs`te.

use std::collections::HashSet;

use gpui::{
    AnyElement, Context, ElementId, FontWeight, ScrollStrategy, Window, canvas, div, point,
    prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{Tooltip, icon_button},
    library::Loadable,
    library_tree::{self, Key, Node, Options},
    manage::{Act, Entry},
    shelf::{Card, CardMeta, FolderInfo, Kind, Sort, SortCtx, sort_key},
    theme::*,
};
use crate::engine::{CollectionId, Command, LibraryCommand, folder_id};
use crate::{t, tp};

impl Shell {
    /// Kütüphanede klasör var mı?
    pub(super) fn has_folders(&self) -> bool {
        matches!(&self.library.library, Loadable::Loaded(l) if l.meta.has_folders())
    }

    /// Klasör ağacı gösteriliyor mu? Süzgeç ve yazı yokken, listeler (ve
    /// albümler) görünümünde; sanatçılar ve aramada düz.
    pub(super) fn tree_mode(&self) -> bool {
        self.all.query.is_empty()
            && matches!(self.all.kind, Kind::All | Kind::Lists)
            && self.has_folders()
    }

    /// Klasör liste görünümünde açık mı (kalıcı kimlikle)?
    pub(super) fn is_folder_open(&self, key: &str) -> bool {
        let id = folder_id(key);
        self.prefs.library_open_folders.iter().any(|k| k == id)
    }

    /// Ağaç olarak görünen kartlar: klasörler, iç içe; sabitliler en üstte.
    /// Liste görünümünde açık klasörlerin içi hemen altında, ızgarada yalnız
    /// girilen klasörün (`all.folder`) çocukları.
    pub(super) fn tree_cards(&self, kind: Kind) -> Vec<Card> {
        let Loadable::Loaded(library) = &self.library.library else {
            return Vec::new();
        };
        let meta = &library.meta;
        let mut items = self.library_cards();
        items.retain(|c| {
            kind == Kind::All
                || c.kind() == kind
                || (kind == Kind::Lists && c.kind() == Kind::ForYou)
        });

        // Kök listedeki yer: Beğenilenler 0, albümler 1.., listeler ve klasörler
        // `taban + 2 × yer`: klasör, kendinden önceki listeden sonra, sonrakinden önce.
        let base = 1 + library.albums.len();
        let list_ix: std::collections::HashMap<&str, usize> = library
            .playlists
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id.key(), i))
            .collect();
        let album_ix: std::collections::HashMap<&str, usize> = library
            .albums
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id.key(), i))
            .collect();
        for card in &mut items {
            let key = card.id.key();
            card.meta.seq = if let Some(i) = list_ix.get(key) {
                base + 2 * i + 1
            } else if let Some(i) = album_ix.get(key) {
                1 + i
            } else {
                0
            };
        }

        let ctx = SortCtx {
            recent: &self.recent,
            meta,
        };
        let sort = self.all.sort;
        let mut cards: Vec<Card> = items;
        let mut nodes: Vec<Node> = cards
            .iter()
            .map(|c| Node {
                id: c.id.key().to_string(),
                parent: meta.parents.get(c.id.key()).cloned(),
                folder: false,
                key: sort_key(sort, c, &ctx),
                pin: c.meta.pin,
                seq: c.meta.seq,
            })
            .collect();
        // Klasör kartları: adı boşsa "Klasör".
        let folder_nodes: Vec<Node> = meta
            .folders
            .iter()
            .map(|f| Node {
                id: f.key.clone(),
                parent: f.parent.clone(),
                folder: true,
                key: Key::last(),
                pin: None,
                seq: base + 2 * f.at,
            })
            .collect();
        let counts = library_tree::item_counts(&[nodes.clone(), folder_nodes.clone()].concat());
        for (f, mut node) in meta.folders.iter().zip(folder_nodes) {
            let name = if f.name.trim().is_empty() {
                t!(KindFolder).to_string()
            } else {
                f.name.clone()
            };
            let count = counts.get(&f.key).copied().unwrap_or(0);
            let card = Card {
                id: CollectionId::Playlist(f.key.clone()),
                name,
                subtitle: tp!(FolderLists, count),
                image: String::new(),
                meta: CardMeta {
                    pin: None,
                    folder: Some(FolderInfo {
                        key: f.key.clone(),
                        open: self.is_folder_open(&f.key),
                        count,
                    }),
                    depth: 0,
                    seq: node.seq,
                },
            };
            // Ad ve yaratıcıda klasörün kendi anahtarı; son çalınan / eklenmede
            // içindekilerden (`folders_by_children`).
            node.key = sort_key(sort, &card, &ctx);
            nodes.push(node);
            cards.push(card);
        }
        debug_assert_eq!(nodes.len(), cards.len());

        let open_ids: HashSet<&str> = self
            .prefs
            .library_open_folders
            .iter()
            .map(String::as_str)
            .collect();
        let is_open = |key: &str| open_ids.contains(folder_id(key));
        let inline = self.all.list;
        let opts = Options {
            root: if inline {
                None
            } else {
                self.all.folder.as_deref()
            },
            is_open: inline.then_some(&is_open as &dyn Fn(&str) -> bool),
            lift_pins: true,
            folders_by_children: matches!(sort, Sort::Recent | Sort::Added),
        };
        library_tree::layout(&nodes, &opts)
            .into_iter()
            .map(|p| {
                let mut card = cards[p.ix].clone();
                card.meta.depth = p.depth;
                card
            })
            .collect()
    }

    // ── Klasörler ───────────────────────────────────────────────────────

    /// Klasör kartı / satırı tıklandı: liste görünümünde aç / kapa, ızgarada içine gir.
    pub(super) fn open_folder(&mut self, key: String, cx: &mut Context<Self>) {
        if self.all.list {
            self.toggle_folder(key, cx);
            return;
        }
        self.all.folder = Some(key);
        self.all.cursor = None;
        self.recompute_all();
        self.scroll_all_to_top();
        cx.notify();
    }

    /// Liste görünümünde klasörü aç ya da kapa (durum kalıcı).
    pub(super) fn toggle_folder(&mut self, key: String, cx: &mut Context<Self>) {
        self.set_folder_open(&key, !self.is_folder_open(&key), cx);
    }

    pub(super) fn set_folder_open(&mut self, key: &str, open: bool, cx: &mut Context<Self>) {
        let id = folder_id(key).to_string();
        let ids = &mut self.prefs.library_open_folders;
        if ids.iter().any(|k| *k == id) == open {
            return;
        }
        if open {
            ids.push(id);
            ids.truncate(crate::settings::LIBRARY_OPEN_FOLDERS_MAX);
        } else {
            ids.retain(|k| *k != id);
        }
        self.save_settings();
        self.recompute_all();
        cx.notify();
    }

    /// Izgarada bir üst klasöre (ya da köke) çık.
    pub(super) fn leave_folder(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.all.folder.clone() else {
            return;
        };
        let parent = match &self.library.library {
            Loadable::Loaded(l) => l
                .meta
                .folders
                .iter()
                .find(|f| f.key == current)
                .and_then(|f| f.parent.clone()),
            _ => None,
        };
        self.go_to_folder(parent, cx);
    }

    /// Izgarada verilen klasöre (`None`: kök) git.
    pub(super) fn go_to_folder(&mut self, folder: Option<String>, cx: &mut Context<Self>) {
        self.all.folder = folder;
        self.all.cursor = None;
        self.recompute_all();
        self.scroll_all_to_top();
        cx.notify();
    }

    /// Kartı etkinleştir (tıklama, Enter): klasörse aç / gir, değilse listeyi aç.
    pub(super) fn activate_card(&mut self, card: &Card, cx: &mut Context<Self>) {
        match &card.meta.folder {
            Some(folder) => self.open_folder(folder.key.clone(), cx),
            None => self.select_collection(card.id.clone(), cx),
        }
    }

    /// Klasör yolu (kökten girilen klasöre), yol çubuğu için: (anahtar, ad).
    fn folder_path(&self) -> Vec<(String, String)> {
        let (Loadable::Loaded(library), Some(current)) =
            (&self.library.library, self.all.folder.as_deref())
        else {
            return Vec::new();
        };
        let nodes: Vec<Node> = library
            .meta
            .folders
            .iter()
            .enumerate()
            .map(|(i, f)| Node {
                id: f.key.clone(),
                parent: f.parent.clone(),
                folder: true,
                key: Key::last(),
                pin: None,
                seq: i,
            })
            .collect();
        library_tree::path_to(&nodes, current)
            .into_iter()
            .map(|n| {
                let name = library
                    .meta
                    .folders
                    .iter()
                    .find(|f| f.key == n.id)
                    .map(|f| f.name.clone())
                    .unwrap_or_default();
                (
                    n.id.clone(),
                    if name.trim().is_empty() {
                        t!(KindFolder).to_string()
                    } else {
                        name
                    },
                )
            })
            .collect()
    }

    /// Izgarada içine girilmiş klasörün yol çubuğu: geri düğmesi ve
    /// "Kütüphane › Yol › Spor" (son parça kalın, öncekiler tıklanır).
    /// Liste görünümünde ve kökte yok.
    pub(super) fn render_folder_crumbs(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.all.list || !self.tree_mode() || self.all.folder.is_none() {
            return None;
        }
        let path = self.folder_path();
        if path.is_empty() {
            return None;
        }
        let accent = self.accent().text;
        let (stop, look) = self.stop_look("all-up", super::tab::PAGE, true, window, cx);
        let up = icon_button("all-up", "arrow-left", look, accent)
            .when_some(stop, |el, h| el.track_focus(&h))
            .tooltip(Tooltip::text(t!(FolderUpTip), Some("Esc")))
            .on_click(cx.listener(|this, _, _, cx| this.leave_folder(cx)));
        let segment = |id: ElementId,
                       label: String,
                       target: Option<String>,
                       last: bool,
                       cx: &mut Context<Self>| {
            let el = div()
                .id(id)
                .flex_shrink_0()
                .h(px(S6))
                .px(px(S2))
                .rounded(px(R_MD))
                .flex()
                .items_center()
                .type_role(T_BODY)
                .font_weight(if last {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(if last { text() } else { muted() });
            if last {
                el.child(label)
            } else {
                el.cursor_pointer()
                    .hover(|s| s.bg(veil(VEIL_HOVER)).text_color(text()))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.go_to_folder(target.clone(), cx)),
                    )
                    .child(label)
            }
        };
        let mut row = div()
            .flex_shrink_0()
            .px(px(page_margin(self.window_width)))
            .pb(px(S3))
            .flex()
            .items_center()
            .gap(px(S1))
            .child(up)
            .child(segment(
                "crumb-root".into(),
                t!(NavLibrary).to_string(),
                None,
                false,
                cx,
            ));
        let last = path.len() - 1;
        for (i, (key, name)) in path.into_iter().enumerate() {
            row = row
                .child(icon("chevron-right", 14., label()))
                .child(segment(
                    ElementId::Name(format!("crumb-{i}").into()),
                    name,
                    Some(key),
                    i == last,
                    cx,
                ));
        }
        Some(row.into_any_element())
    }

    /// Klasör kartının kapağı: ton karosu üstünde klasör simgesi (Beğenilenler'in kalbi gibi).
    pub(super) fn folder_art(&self, side: f32, radius: f32) -> AnyElement {
        div()
            .size(px(side))
            .flex_shrink_0()
            .rounded(px(radius))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(PLACEHOLDER))
            .child(icon("folder", (side * 0.42).max(14.), muted()))
            .into_any_element()
    }

    /// Sabitli kartın kapağının köşesindeki küçük iğne (sol üst; sağ alttaki
    /// "çalıyor" simgesiyle karışmaz).
    pub(super) fn pin_mark(&self, side: f32) -> AnyElement {
        div()
            .absolute()
            .left(px(S1))
            .top(px(S1))
            .size(px(side))
            .rounded_full()
            .bg(shade(SCRIM))
            .flex()
            .items_center()
            .justify_center()
            .child(icon("pin", side * 0.6, soft()))
            .into_any_element()
    }

    // ── Sabitleme ───────────────────────────────────────────────────────

    /// Anahtar sabitli mi?
    pub(super) fn is_pinned(&self, key: &str) -> bool {
        matches!(&self.library.library, Loadable::Loaded(l) if l.meta.pinned.iter().any(|k| k == key))
    }

    /// Sabitlenebilir mi: Beğenilenler ya da kütüphanede olan liste / albüm /
    /// takip edilen sanatçı.
    pub(super) fn can_pin(&self, id: &CollectionId) -> bool {
        *id == CollectionId::Liked || self.library.item(id).is_some()
    }

    /// Sağ tık menüsündeki "En üste sabitle" / "Sabitlemeyi kaldır" (sabitlenemiyorsa yok).
    pub(super) fn pin_entry(&self, id: &CollectionId) -> Option<Entry> {
        if !self.can_pin(id) {
            return None;
        }
        let pinned = self.is_pinned(id.key());
        Some(Entry::Item {
            icon: if pinned { "pin-off" } else { "pin" },
            label: if pinned { t!(MenuUnpin) } else { t!(MenuPin) }.into(),
            shortcut: None,
            danger: false,
            act: Some(Act::Pin {
                key: id.key().to_string(),
                on: !pinned,
            }),
            sub: Vec::new(),
        })
    }

    /// Sabitle / kaldır: ekran aynı karede güncellenir (iyimser), yerel tabloya
    /// motor yazar; Spotify'a hiçbir şey gitmez.
    pub(super) fn set_pinned(&mut self, key: String, on: bool, cx: &mut Context<Self>) {
        if let Loadable::Loaded(library) = &mut self.library.library {
            library.meta.pinned.retain(|k| *k != key);
            if on {
                library.meta.pinned.push(key.clone());
            }
        }
        self.engine
            .send(Command::Library(LibraryCommand::Pin { key, pinned: on }));
        self.recompute_all();
        self.show_toast(
            if on {
                t!(ToastPinned)
            } else {
                t!(ToastUnpinned)
            }
            .into(),
            false,
            false,
            cx,
        );
        cx.notify();
    }

    // ── Sıralama menüsü ─────────────────────────────────────────────────

    /// Sıralama düğmesinin altında menü: dört sıralama, geçerli olan işaretli.
    pub(super) fn open_sort_menu(&mut self, cx: &mut Context<Self>) {
        let at = match self.all.sort_anchor.get() {
            Some(b) => point(b.origin.x, b.origin.y + b.size.height + px(S1)),
            None => point(px(S6), px(S7 * 2.)),
        };
        let current = self.all.sort;
        let kind = self.all.kind;
        let entries: Vec<Entry> = Sort::ALL
            .iter()
            .map(|s| Entry::Item {
                icon: if *s == current { "check" } else { "" },
                label: s.label_for(kind).into(),
                shortcut: None,
                danger: false,
                act: Some(Act::SetSort(*s)),
                sub: Vec::new(),
            })
            .collect();
        self.show_menu(at, entries, cx);
        if let Some(menu) = self.manage.menu.as_mut() {
            menu.cursor = Sort::ALL.iter().position(|s| *s == current);
        }
    }

    /// Sıralama düğmesinin yerini ölçen görünmez katman (menü bunun altında açılır).
    pub(super) fn sort_anchor_probe(&self) -> AnyElement {
        let anchor = self.all.sort_anchor.clone();
        canvas(
            move |bounds, _, _| anchor.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    // ── Klavye ──────────────────────────────────────────────────────────

    /// Görünen kartın sırası → kart (ızgarada rafın gösterdiği baştaki kartlar atlanır).
    pub(super) fn shown_card(&self, i: usize) -> Option<&Card> {
        self.all.cards.get(self.all_skip() + i)
    }

    /// Ok tuşları: ızgarada dört yön, listede yukarı / aşağı; listede → klasörü
    /// açar (açıksa ilk çocuğa geçer), ← kapatır (kapalıysa / klasör değilse
    /// üst klasöre geçer). Enter etkinleştirir (`all_key`).
    pub(super) fn move_all_cursor(&mut self, key: &str, cx: &mut Context<Self>) {
        let shown = self.all_shown();
        if shown == 0 {
            return;
        }
        let cols = self.all_columns();
        let list = self.all.list;
        let Some(cur) = self.all.cursor.filter(|c| *c < shown) else {
            // Hiçbiri seçili değilken: aşağı / sağ ilkine, yukarı / sol sonuncuya.
            self.all.cursor = Some(if matches!(key, "down" | "right") {
                0
            } else {
                shown - 1
            });
            self.reveal_all_cursor();
            cx.notify();
            return;
        };
        let next = match (key, list) {
            ("down", true) => (cur + 1).min(shown - 1),
            ("up", true) => cur.saturating_sub(1),
            ("down", false) => {
                if cur + cols < shown {
                    cur + cols
                } else {
                    cur
                }
            }
            ("up", false) => {
                if cur >= cols {
                    cur - cols
                } else {
                    cur
                }
            }
            ("right", false) => (cur + 1).min(shown - 1),
            ("left", false) => cur.saturating_sub(1),
            ("right", true) => {
                let card = self.shown_card(cur).cloned();
                match card.as_ref().and_then(|c| c.meta.folder.clone()) {
                    Some(f) if !f.open => {
                        self.set_folder_open(&f.key, true, cx);
                        cur
                    }
                    Some(_) => (cur + 1).min(shown - 1),
                    None => cur,
                }
            }
            ("left", true) => {
                let card = self.shown_card(cur).cloned();
                match card {
                    Some(c) if c.meta.folder.as_ref().is_some_and(|f| f.open) => {
                        let key = c.meta.folder.map(|f| f.key).unwrap_or_default();
                        self.set_folder_open(&key, false, cx);
                        cur
                    }
                    // Üst klasörün satırına çık.
                    Some(c) if c.meta.depth > 0 => (0..cur)
                        .rev()
                        .find(|i| {
                            self.shown_card(*i).is_some_and(|p| {
                                p.meta.folder.is_some() && p.meta.depth + 1 == c.meta.depth
                            })
                        })
                        .unwrap_or(cur),
                    _ => cur,
                }
            }
            _ => cur,
        };
        self.all.cursor = Some(next.min(self.all_shown().saturating_sub(1)));
        self.reveal_all_cursor();
        cx.notify();
    }

    /// İmleçteki kart görünmüyorsa kaydır.
    fn reveal_all_cursor(&self) {
        let Some(cur) = self.all.cursor else { return };
        let row = if self.all.list {
            cur
        } else {
            cur / self.all_columns()
        };
        self.all.scroll.scroll_to_item(row, ScrollStrategy::Center);
    }
}
