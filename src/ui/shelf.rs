//! Kapak rafı, şerit ve "Tümü" (taslak D, `DESIGN.md` → "Raf + yüzen hap").
//!
//! Kenar çubuğu yok: kütüphane başlık çubuğunun altında, son çalınana göre
//! sıralı bir kapak rafı. Sığdığı kadar liste görünür, sonda "Tümü · N".
//! Liste aşağı kayınca raf aynı hareketle 56 px'lik şeride iner (kapaklar
//! 136 → 40 px, arada ad solar); liste en üste dönünce yeniden açılır. Hareket
//! `LibraryState::header_collapse`'tan: raf ve liste başlığı aynı oranla
//! küçülür, ikisinin kaybettiği yükseklik tekerleğin aldığı yola eşit.
//!
//! "Tümü" bütün kütüphaneyi rafın ve listenin yerinde bir kapak ızgarasında
//! açar: yazdıkça süzülür (bulanık arama), türe göre haplar, Enter ilkini
//! açar, Esc kapatır.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, Bounds, Context, ElementId, FontWeight, KeyDownEvent, Pixels, ScrollDelta,
    SharedString, UniformListScrollHandle, Window, div, prelude::*, px, rgb, uniform_list,
};

use super::{
    Shell,
    components::{
        Look, PRIMARY_HEIGHT_SMALL, Tooltip, chip, ghost_button, icon_button, state_view,
    },
    fuzzy,
    library::{Loadable, centered},
    library_tree::Key,
    pill::PILL_CLEARANCE,
    smooth_scroll::{SmoothScroll, Wheel},
    text::line_with_tip,
    theme::*,
};
use crate::engine::CollectionId;
use crate::{k, t, tf, tp};

// ── Ölçüler ─────────────────────────────────────────────────────────────

/// Açık raf: kapak ve kartlar arası. 7 Kasım gecesi 112'den 136'ya (kullanıcı
/// geri bildirimi: kütüphane yorucu): 1180 px'te (kenar 32, iç en 1116) tam
/// yedi kart sığar (7 × 136 + 6 × 16 = 1048; artan 68 px aralara dağılır,
/// aralar ~27 px: raf nefes alır). 144'te de yedi kart sığardı (1104, aralar
/// 18) ama raf 8 px daha uzar ve aralar sıkışır; 128'de 1180'de aralar ~35 px
/// olurdu (kapaklar küçük kalır).
pub(super) const SHELF_COVER: f32 = 136.;
pub(super) const SHELF_GAP: f32 = S4;
/// Şerit: kapak ve kartlar arası.
pub(super) const STRIP_COVER: f32 = 40.;
const STRIP_GAP: f32 = S2;
/// Kapağın altındaki ad: boşluk ve bir satır.
pub(super) const SHELF_NAME: f32 = S2 + T_SECONDARY.line;
const SHELF_PAD_TOP: f32 = S2;
const SHELF_PAD_BOTTOM: f32 = S3;
const STRIP_PAD_BOTTOM: f32 = S2;
/// Rafın açık ve şerit hâlindeki yüksekliği (158 / 56 px).
pub(super) const SHELF_OPEN: f32 = SHELF_PAD_TOP + SHELF_COVER + SHELF_NAME + SHELF_PAD_BOTTOM;
pub(super) const STRIP_HEIGHT: f32 = SHELF_PAD_TOP + STRIP_COVER + STRIP_PAD_BOTTOM;
/// Liste başlığı: açık (iki satır: ad 28 px, altında meta 14 px; düğmeler
/// bloğun ortasında) ve küçülmüş (tek satır, ad 18 px, düğme 32 px) yükseklik.
pub(super) const HEADER_OPEN: f32 = S2 + T_HEADLINE.line + T_BODY.line + S2;
pub(super) const HEADER_COMPACT: f32 = S2 + PRIMARY_HEIGHT_SMALL + S2;
/// Raf ve başlık birlikte en çok bu kadar kapanır (px): tekerleğin listeye
/// geçmeden önce aldığı yol.
pub(super) const COLLAPSE_MAX: f32 = (SHELF_OPEN - STRIP_HEIGHT) + (HEADER_OPEN - HEADER_COMPACT);
/// Kapanma bu orana gelince başlık küçük hâline, raf kartları şerit
/// görünüşüne geçer.
pub(super) const COMPACT_AT: f32 = 0.5;
/// Kütüphane ızgarası (7 Kasım gecesi: kartlar 112'den büyüdü, aralar 24 px):
/// kart eni hedefi ~168 (sütun sayısı bu hedefe en yakın tam sayıya yuvarlanır,
/// kartlar sütuna göre esner: 1180'de 6 sütun × 166; 1920'de panel yerleşikken
/// 7 × 181; 760'ta 4 × 156). Sütun ve satır araları 24 px.
const GRID_TARGET: f32 = 168.;
const GRID_GAP: f32 = S5;
/// Liste görünümünün satırı ve kapağı.
const LIST_ROW: f32 = S7 + S2;
const LIST_COVER: f32 = S6 + S2;
/// Bu genişliğin altında "Tümü"deki "n / N" sayacı gizlenir, Araçlar ve Yeni
/// liste yalnız simge olur (sığmaz; 900'de "+" sağ kenardan taşıyordu).
const ALL_COUNT_MIN_WIDTH: f32 = 1180.;

/// "Tümü"deki süzme kutusunun genişliği (dar pencerede daralır).
const ALL_FIELD_WIDTH: f32 = 280.;
const ALL_FIELD_MIN: f32 = 160.;
/// Kartın üzerine gelme grubu (kapaktaki ton katmanı).
const CARD_GROUP: &str = "shelf-card";
/// Beğenilen şarkıların her yerdeki adı (7 Kasım kararı; önce yalnız kartta,
/// 96 px'e "Beğenilen şarkılar" sığmıyordu).
fn liked_card_name() -> &'static str {
    t!(LikedSongs)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0., 1.)
}

/// Rafın yüksekliği, kapanma oranına göre (0 açık, 1 şerit).
pub(super) fn shelf_height(p: f32) -> f32 {
    lerp(SHELF_OPEN, STRIP_HEIGHT, p)
}

/// Liste başlığının yüksekliği, aynı orana göre.
pub(super) fn header_height(p: f32) -> f32 {
    lerp(HEADER_OPEN, HEADER_COMPACT, p)
}

// ── Kartlar ─────────────────────────────────────────────────────────────

/// Klasör kartının bilgisi (yalnız klasör kartlarında).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FolderInfo {
    /// Spotify'ın klasör işareti (`Folder::key`).
    pub key: String,
    /// Liste görünümünde içi açık mı?
    pub open: bool,
    /// İçindeki liste sayısı (iç içe klasörler dahil).
    pub count: usize,
}

/// Kartın kütüphane düzenindeki yeri: sabitleme, klasör, girinti.
#[derive(Clone, Debug, Default)]
pub(super) struct CardMeta {
    /// Sabitlenme sırası (0 = en eski sabitlenen); sabit değilse `None`.
    pub pin: Option<usize>,
    /// Klasör kartıysa bilgisi.
    pub folder: Option<FolderInfo>,
    /// Liste görünümünde girinti (klasörün içinde 1 …).
    pub depth: usize,
    /// Kaynak listedeki sırası (eşit anahtarlarda sıra bozulmasın).
    pub seq: usize,
}

/// Rafta, şeritte ya da ızgarada bir liste (ya da klasör).
#[derive(Clone)]
pub(super) struct Card {
    pub id: CollectionId,
    pub name: String,
    pub subtitle: String,
    pub image: String,
    pub meta: CardMeta,
}

impl Card {
    pub(super) fn kind(&self) -> Kind {
        match &self.id {
            CollectionId::Liked => Kind::Lists,
            CollectionId::Album(_) => Kind::Albums,
            CollectionId::Artist(_) => Kind::Artists,
            CollectionId::Playlist(_) if self.subtitle.starts_with("Spotify") => Kind::ForYou,
            CollectionId::Playlist(_) => Kind::Lists,
        }
    }

    /// Klasör kartı mı?
    pub(super) fn is_folder(&self) -> bool {
        self.meta.folder.is_some()
    }

    pub(super) fn pinned(&self) -> bool {
        self.meta.pin.is_some()
    }
}

/// Kartın o anki görünüşü (vitrinde zorlanabilsin diye ayrı).
#[derive(Clone, Copy)]
pub(super) struct CardState {
    pub look: Look,
    /// Açık olan liste: kapak ışığı renginde halka ve ad.
    pub selected: bool,
    /// Çalan liste: kapağın köşesinde ses simgesi.
    pub playing: bool,
}

/// Karta tıklayınca ne olur.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum CardAction {
    /// Listeyi aç (raf, şerit).
    Open,
    /// Listeyi aç ve "Tümü"yü kapat.
    OpenAndClose,
    /// Vitrin: hiçbir şey.
    Nothing,
}

/// "Tümü"deki tür süzgeci.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    All,
    Albums,
    Lists,
    ForYou,
    /// Takip edilen sanatçılar (yuvarlak portre). Yalnız bu süzgeçte ve
    /// yazı yazılınca görünür: süzgeçsiz "Tümü" listeler + albümler kalır,
    /// yüzlerce sanatçı onları boğmaz.
    Artists,
}

/// Kütüphane ızgarasındaki tür süzgeçleri.
const KINDS: [(Kind, crate::i18n::K); 3] = [
    (Kind::Lists, k!(KindLists)),
    (Kind::Albums, k!(KindAlbums)),
    (Kind::Artists, k!(KindArtists)),
];

/// Kütüphanenin sıralaması (kalıcı: `Settings::library_sort`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    /// Son çalınan önde (raftaki sıra).
    Recent,
    /// Kütüphaneye eklenme tarihi, en yeni önde. Albümlerin tarihi Spotify'ın
    /// koleksiyonundan, listelerinki kök listeden gelir; bilinmeyenler
    /// (Spotify vermediyse) sonda, kütüphane sırasıyla.
    Added,
    /// Ada göre (alfabetik, Türkçe harfler katlanarak).
    Name,
    /// Yaratıcıya göre: albümde sanatçı, listede sahibi; kendi listelerin
    /// ve Spotify'ınkiler sahip adlarıyla (aynı yaratıcıda ada göre).
    Creator,
}

impl Sort {
    /// Menüdeki sıra.
    pub const ALL: [Sort; 4] = [Sort::Recent, Sort::Added, Sort::Name, Sort::Creator];

    pub fn key(self) -> &'static str {
        match self {
            Sort::Recent => "recent",
            Sort::Added => "added",
            Sort::Name => "name",
            Sort::Creator => "creator",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "added" => Sort::Added,
            "name" => Sort::Name,
            "creator" => Sort::Creator,
            _ => Sort::Recent,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Recent => t!(RecentlyPlayed),
            Sort::Added => t!(SortAdded),
            Sort::Name => t!(SortName),
            Sort::Creator => t!(SortCreator),
        }
    }

    /// Sanatçılar süzgecinde "son çalınan" diye bir sıra yok (sanatçı çalma
    /// bağlamı kaydedilmiyor): aynı sıra, en son takip edilen önde.
    pub(super) fn label_for(self, kind: Kind) -> &'static str {
        if self == Sort::Recent && kind == Kind::Artists {
            t!(SortRecentlyFollowed)
        } else {
            self.label()
        }
    }
}

/// Kütüphane görünümünün durumu (adı eski "Tümü"den kaldı).
pub struct AllState {
    /// Kütüphane sekmesinin ızgarası / listesi açık mı (`Route::Library`)?
    pub open: bool,
    pub query: String,
    pub kind: Kind,
    /// Süzgeçten geçen kartlar, gösterilecek sırayla.
    pub cards: Vec<Card>,
    /// Kütüphanedeki bütün listeler.
    pub total: usize,
    /// Satır görünümü mü (değilse ızgara)? Ayarlarda saklanır.
    pub list: bool,
    pub sort: Sort,
    pub scroll: UniformListScrollHandle,
    pub glide: Rc<SmoothScroll>,
    /// Izgarada içine girilmiş klasör (Spotify'ın klasör işareti); kökteyse
    /// `None`. Liste görünümünde klasörler satır arasında açılır, bu kullanılmaz.
    pub folder: Option<String>,
    /// Klavyeyle gezilen kart (görünen kartların sırası; ok tuşları).
    pub cursor: Option<usize>,
    /// Sıralama düğmesinin son bilinen yeri (menü onun altında açılır).
    pub sort_anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl AllState {
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        Self {
            open: false,
            query: String::new(),
            kind: Kind::All,
            cards: Vec::new(),
            total: 0,
            list: settings.library_view == "list",
            sort: Sort::from_key(&settings.library_sort),
            scroll: UniformListScrollHandle::new(),
            glide: Rc::default(),
            folder: None,
            cursor: None,
            sort_anchor: Rc::default(),
        }
    }
}

/// Sıralama anahtarının ihtiyaç duyduğu veri.
pub(super) struct SortCtx<'a> {
    /// Son çalınanlar, en yenisi başta.
    pub recent: &'a [String],
    pub meta: &'a crate::engine::LibraryMeta,
}

/// Kartın kardeşleri arasındaki sıralama anahtarı (bkz. `library_tree`).
/// Grup 0 en öne, büyük gruplar sona gider.
pub(super) fn sort_key(sort: Sort, card: &Card, ctx: &SortCtx) -> Key {
    let fold = |s: &str| fuzzy::fold(s);
    if card.is_folder() {
        // Klasörün "son çalınan" / "eklenme" anahtarı içindekilerden gelir
        // (`library_tree::Options::folders_by_children`); ad ve yaratıcıda kendi adı.
        return match sort {
            Sort::Creator => Key::new(0, 0, format!("\u{1}{}", fold(&card.name))),
            _ => Key::new(0, 0, fold(&card.name)),
        };
    }
    let key = card.id.key();
    let liked = card.id == CollectionId::Liked;
    match sort {
        Sort::Recent => match ctx.recent.iter().position(|k| k == key) {
            Some(r) => Key::new(1, r as i64, ""),
            None if liked => Key::new(0, 0, ""),
            None => Key::new(2, 0, ""),
        },
        Sort::Added => {
            if liked {
                Key::new(0, 0, "")
            } else if matches!(card.id, CollectionId::Artist(_)) {
                // Takip edilen sanatçılar zaten en son takip edilen önde sıralı gelir.
                Key::new(3, card.meta.seq as i64, "")
            } else {
                match ctx.meta.added.get(key) {
                    Some(at) => Key::new(1, -at, ""),
                    None => Key::new(2, card.meta.seq as i64, ""),
                }
            }
        }
        Sort::Name => Key::new(
            0,
            0,
            fold(if liked { liked_card_name() } else { &card.name }),
        ),
        Sort::Creator => {
            if liked {
                return Key::new(0, 0, "");
            }
            let creator = match &card.id {
                CollectionId::Album(_) => card.subtitle.clone(),
                CollectionId::Artist(_) => card.name.clone(),
                _ => ctx.meta.creators.get(key).cloned().unwrap_or_default(),
            };
            Key::new(1, 0, format!("{}\u{1}{}", fold(&creator), fold(&card.name)))
        }
    }
}

/// Düz sıralama: önce sabitlenenler (sabitlenme sırasıyla), sonra seçilen sıra.
/// Kararlı: eşit anahtarlarda kaynak sırası kalır.
pub(super) fn sort_flat(cards: &mut [Card], sort: Sort, ctx: &SortCtx) {
    cards.sort_by_cached_key(|c| {
        (
            c.meta.pin.is_none(),
            c.meta.pin.unwrap_or(0),
            sort_key(sort, c, ctx),
        )
    });
}

/// Raftaki sıra: sabitlenenler önde; hiç çalınmamışsa beğenilenler, sonra son
/// çalınanlar (en yenisi önce), sonra geri kalanlar kütüphane sırasıyla.
fn shelf_order(cards: &mut [Card], recent: &[String]) {
    let meta = crate::engine::LibraryMeta::default();
    sort_flat(
        cards,
        Sort::Recent,
        &SortCtx {
            recent,
            meta: &meta,
        },
    );
}

impl Shell {
    /// Kütüphanedeki bütün listeler, kütüphane sırasıyla (beğenilenler,
    /// albümler, listeler). Kütüphane gelmediyse boş.
    pub(super) fn library_cards(&self) -> Vec<Card> {
        let Loadable::Loaded(library) = &self.library.library else {
            return Vec::new();
        };
        let pin = |key: &str| library.meta.pinned.iter().position(|k| k == key);
        let mut cards = vec![Card {
            id: CollectionId::Liked,
            name: liked_card_name().into(),
            subtitle: tp!(Songs, library.liked_count),
            image: String::new(),
            meta: CardMeta {
                pin: pin("liked"),
                seq: 0,
                ..Default::default()
            },
        }];
        cards.extend(
            library
                .albums
                .iter()
                .chain(&library.playlists)
                .enumerate()
                .map(|(n, i)| Card {
                    id: i.id.clone(),
                    name: i.name.clone(),
                    subtitle: crate::i18n::canon::localize(&i.subtitle).into_owned(),
                    image: i.image.clone(),
                    meta: CardMeta {
                        pin: pin(i.id.key()),
                        seq: n + 1,
                        ..Default::default()
                    },
                }),
        );
        cards
    }

    /// Takip edilen sanatçılar, en son takip edilen önde (rafta yer almazlar:
    /// yalnız "Tümü"nün Sanatçılar süzgecinde ve aramada). Altyazı "Sanatçı".
    pub(super) fn artist_cards(&self) -> Vec<Card> {
        let Loadable::Loaded(library) = &self.library.library else {
            return Vec::new();
        };
        library
            .artists
            .iter()
            .enumerate()
            .map(|(n, i)| Card {
                id: i.id.clone(),
                name: i.name.clone(),
                subtitle: crate::i18n::canon::localize(&i.subtitle).into_owned(),
                image: i.image.clone(),
                meta: CardMeta {
                    pin: library.meta.pinned.iter().position(|k| k == i.id.key()),
                    seq: n,
                    ..Default::default()
                },
            })
            .collect()
    }

    /// Rafa kaç kart sığar: pencere genişliğinden.
    fn shelf_slots(&self) -> usize {
        shelf_slot_count(self.window_width)
    }

    /// Raftaki listeler, ekrandaki sırayla. Açık liste rafta yoksa (Kütüphane
    /// ızgarasından ya da aramadan açıldı) son yerde durur: nerede olunduğu
    /// rafta hep görünsün.
    pub(super) fn shelf_cards(&self) -> Vec<Card> {
        let slots = self.shelf_slots();
        let mut cards = self.library_cards();
        shelf_order(&mut cards, &self.recent);
        let pinned = self.library.selected.as_ref().and_then(|selected| {
            if cards.iter().take(slots).any(|c| &c.id == selected) {
                return None;
            }
            cards
                .iter()
                .find(|c| &c.id == selected)
                .cloned()
                .or_else(|| {
                    self.library.titles.get(selected).map(|title| Card {
                        id: selected.clone(),
                        name: title.clone(),
                        subtitle: String::new(),
                        image: String::new(),
                        meta: CardMeta::default(),
                    })
                })
        });
        cards.truncate(slots);
        if let Some(card) = pinned {
            if cards.len() == slots {
                cards.pop();
            }
            cards.push(card);
        }
        cards
    }

    /// Açık listenin raftaki sırası (yoksa 0).
    pub(super) fn selected_shelf_index(&self) -> usize {
        let selected = self.library.selected.as_ref();
        self.shelf_cards()
            .iter()
            .position(|c| Some(&c.id) == selected)
            .unwrap_or(0)
    }

    /// Raf odaktayken: ← → gez, Home / End uçlar, Enter / Boşluk aç.
    pub(super) fn shelf_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let cards = self.shelf_cards();
        let last = cards.len().saturating_sub(1);
        let cursor = self.shelf_cursor.min(last);
        self.shelf_cursor = match key {
            "left" => cursor.saturating_sub(1),
            "right" => (cursor + 1).min(last),
            "home" => 0,
            "end" => last,
            _ => {
                if let Some(card) = cards.get(cursor) {
                    self.select_collection(card.id.clone(), cx);
                }
                cursor
            }
        };
        cx.notify();
    }

    // ── Raf ─────────────────────────────────────────────────────────────

    pub(super) fn render_shelf(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.collapse_progress();
        let cover = lerp(SHELF_COVER, STRIP_COVER, p);
        let name_h = lerp(SHELF_NAME, 0., p);
        let keyboard_here = self.keyboard_focus && self.nav.shelf.is_focused(window);
        let mut row = div()
            .id("shelf")
            .track_focus(&self.nav.shelf)
            .relative()
            .size_full()
            .overflow_hidden()
            .flex()
            .items_start()
            .px(px(page_margin(self.window_width)))
            .pt(px(SHELF_PAD_TOP))
            .pb(px(lerp(SHELF_PAD_BOTTOM, STRIP_PAD_BOTTOM, p)))
            .gap(px(lerp(SHELF_GAP, STRIP_GAP, p)));

        // Raf doluyken artan yer kartların arasına eşit dağılır ("Tümü"
        // ızgarası gibi): "Tümü" kartı sağ sayfa kenarına, liste başlığındaki
        // Çal düğmesinin ve süre sütununun hizasına oturur (7 Kasım; önce
        // aralar hep 16 px'ti, sağda 0–111 px boş kalıyordu). Şeride inerken
        // aralar 8 px'e daralır, şerit soldan dizilir. Kütüphane rafı
        // doldurmuyorsa kartlar soldan 16 px arayla.
        let slots = self.shelf_slots();
        let spread = |row: gpui::Stateful<gpui::Div>, shown: usize| {
            if shown >= slots {
                row.gap(px(lerp(shelf_gap(self.window_width, slots), STRIP_GAP, p)))
            } else {
                row
            }
        };
        match &self.library.library {
            Loadable::Loading => {
                row = spread(row, slots);
                // İskelet kartlar: kütüphane gelince yerleşim zıplamaz.
                for i in 0..slots {
                    row = row.child(skeleton_card(i, cover, name_h));
                }
            }
            // Kütüphane hatası `welcome`ın tam sayfa durumunda (`hide_shelf`: raf
            // hiç çizilmez); buraya gelinmez.
            Loadable::Failed(_) => {}
            Loadable::Loaded(_) => {
                let cards = self.shelf_cards();
                row = spread(row, cards.len());
                let cursor = self.shelf_cursor.min(cards.len().saturating_sub(1));
                for (i, card) in cards.iter().enumerate() {
                    let state = CardState {
                        look: if keyboard_here && cursor == i {
                            Look::Focus
                        } else {
                            Look::Live
                        },
                        selected: self.library.selected.as_ref() == Some(&card.id),
                        playing: self.playing_collection() == Some(&card.id),
                    };
                    row = row.child(self.shelf_card(
                        ElementId::Name(format!("shelf-{}", card.id.key()).into()),
                        card,
                        cover,
                        name_h,
                        state,
                        CardAction::Open,
                        cx,
                    ));
                }
            }
        }

        // Rafın üstündeki tekerlek listeyi kaydırır (raf kendisi kaymaz):
        // aşağı çevirmek rafı şeride indirir. Yalnız liste görünümü uyarılır.
        let this = cx.entity().downgrade();
        let list = self.views.list.downgrade();
        let wheel = super::smooth_scroll::wheel_zone(move |ev, _, _, window, cx| {
            let line_height = window.line_height();
            let fallback = f32::from(window.viewport_size().height) / 2.;
            let handled = this
                .update(cx, |shell, cx| {
                    if shell.all.open {
                        let viewport = shell.all_extent().map_or(fallback, |(v, _)| v);
                        return shell.wheel_all(ev.delta, line_height, viewport, cx);
                    }
                    let viewport = shell.list_extent().map_or(fallback, |(v, _)| v);
                    shell.wheel_list(ev.delta, line_height, viewport, cx)
                })
                .unwrap_or(false);
            if handled {
                let _ = list.update(cx, |_, cx| cx.notify());
            }
            handled
        });

        row.child(wheel).into_any_element()
    }

    /// Şu an çalan liste (duraklatılmışken `None`: "çalıyor" demez).
    fn playing_collection(&self) -> Option<&CollectionId> {
        self.state
            .queue
            .context
            .as_ref()
            .filter(|_| self.state.is_playing())
    }

    /// Raf, şerit ya da ızgara kartı: kapak ve altında tek satır ad. Kapak
    /// boyu `cover` (136 → 40), adın yüksekliği `name_h` (26 → 0, yarı yolda
    /// solmuş olur).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn shelf_card(
        &self,
        id: ElementId,
        card: &Card,
        cover: f32,
        name_h: f32,
        state: CardState,
        action: CardAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let big = cover > (SHELF_COVER + STRIP_COVER) / 2.;
        // Takip edilen sanatçı yuvarlak portre: ton katmanı ve halka da yuvarlak.
        let round = matches!(card.id, CollectionId::Artist(_));
        let radius = if round {
            cover / 2.
        } else if big {
            R_LG
        } else {
            R_SM
        };
        // Grup adı her kartta aynı: gpui en yakın atadaki grubu bulur.
        let group: SharedString = CARD_GROUP.into();
        let art = if card.is_folder() {
            self.folder_art(cover, radius)
        } else {
            self.collection_art(&card.id, &card.name, &card.image, cover, radius)
        };
        let look = state.look;

        // Üzerine gelince kapağın üstünde ton katmanı, basınca bir basamak daha.
        let veil_layer = div()
            .id("card-veil")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .rounded(px(radius))
            .when(look == Look::Hover, |el| el.bg(veil(VEIL_HOVER)))
            .when(look == Look::Pressed, |el| el.bg(veil(VEIL_PRESSED)))
            .when(matches!(look, Look::Live | Look::Focus), |el| {
                el.group_hover(group.clone(), |s| s.bg(veil(VEIL_HOVER)))
                    .group_active(group.clone(), |s| s.bg(veil(VEIL_PRESSED)))
            });
        // Seçili ya da klavye odağı: kapak ışığı renginde, araya 2 px boşluk
        // bırakan 2 px halka.
        let ring = (state.selected || look == Look::Focus).then(|| {
            div()
                .absolute()
                .top(px(-S1))
                .left(px(-S1))
                .size(px(cover + 2. * S1))
                .rounded(px(radius + S1))
                .border_2()
                .border_color(rgb(accent))
        });
        // Çalan listenin kapağının köşesinde ses simgesi.
        let badge = state.playing.then(|| {
            let side = if big { S5 } else { S4 };
            div()
                .absolute()
                .right(px(S1))
                .bottom(px(S1))
                .size(px(side))
                .rounded_full()
                .bg(cover_scrim(SCRIM))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("volume-2", side * 0.6, cover_accent(accent)))
        });
        // Sabitli listenin kapağının sol üstünde küçük iğne.
        let pin = card
            .pinned()
            .then(|| self.pin_mark(if big { S5 } else { S4 }));

        let name_opacity = (name_h / SHELF_NAME * 2. - 1.).clamp(0., 1.);
        let name = div().h(px(name_h)).overflow_hidden().child(
            div()
                .pt(px(S2))
                .opacity(name_opacity)
                .type_role(T_SECONDARY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(if state.selected { rgb(accent) } else { text() })
                .child(line_with_tip("card-name", card.name.clone())),
        );

        let card = card.clone();
        // Sağ tık: kart menüsü (aç, çal, bağlantı, yeniden adlandır, sil).
        let menu_target = card_menu_target(&card);
        div()
            .id(id)
            .group(group)
            .flex_none()
            .w(px(cover))
            .flex()
            .flex_col()
            // Şeritte ad görünmez: ipucu yalnız o zaman (ad görünürken gereksiz).
            .when(name_opacity < 0.5, |el| {
                el.tooltip(Tooltip::text(card.name.clone(), None))
            })
            .when(
                action != CardAction::Nothing && look != Look::Disabled,
                |el| {
                    el.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if action == CardAction::OpenAndClose {
                                this.close_all(cx);
                            }
                            this.activate_card(&card, cx);
                        }))
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_card_menu(menu_target.clone(), ev.position, cx);
                            }),
                        )
                },
            )
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(cover))
                    .child(art)
                    .child(veil_layer)
                    .children(badge)
                    .children(pin)
                    .children(ring),
            )
            .child(name)
            .into_any_element()
    }

    // ── Kütüphane (kapak ızgarası / liste) ──────────────────────────────

    /// Kütüphane sekmesini aç: raf üstte, altında kütüphanedeki her şey.
    /// `scroll`: geri / ileri ile dönülünce bırakılan konum.
    pub(super) fn open_library(&mut self, scroll: Option<f32>, cx: &mut Context<Self>) {
        self.all.open = true;
        self.all.query.clear();
        self.all.kind = Kind::All;
        self.all.cursor = None;
        // Sekmeye tıklayınca kütüphane kökten açılır; geri / ileri bıraktığın klasöre döner.
        if scroll.is_none() {
            self.all.folder = None;
        }
        self.recompute_all();
        self.scroll_all_to_top();
        if let Some(y) = scroll.filter(|y| *y > 0.) {
            let state = self.all.scroll.0.borrow();
            let x = state.base_handle.offset().x;
            state.base_handle.set_offset(gpui::point(x, px(-y)));
        }
        // Yazılanlar süzgece gitsin: odak düğmelerden alınır.
        self.refocus_anchor = true;
        cx.notify();
    }

    pub(super) fn close_all(&mut self, cx: &mut Context<Self>) {
        if !self.all.open {
            return;
        }
        self.all.open = false;
        self.all.glide.cancel();
        cx.notify();
    }

    pub(super) fn scroll_all_to_top(&mut self) {
        self.all.glide.cancel();
        let state = self.all.scroll.0.borrow();
        let x = state.base_handle.offset().x;
        state.base_handle.set_offset(gpui::point(x, px(0.)));
    }

    /// Süzgece göre kartları baştan hesapla (yazınca, tür değişince,
    /// kütüphane gelince).
    pub(super) fn recompute_all(&mut self) {
        let mut cards = self.library_cards();
        let artists = self.artist_cards();
        self.all.total = cards.len() + artists.len();
        let kind = self.all.kind;
        // İçine girilmiş klasör kütüphaneden kalktıysa (tazeleme) köke dön.
        if let Some(folder) = &self.all.folder
            && !matches!(&self.library.library, Loadable::Loaded(l) if l.meta.folders.iter().any(|f| &f.key == folder))
        {
            self.all.folder = None;
        }
        // Klasörlü kütüphane, süzgeç ve yazı yokken ağaç olarak (klasörler iç içe).
        if self.tree_mode() {
            self.all.cards = self.tree_cards(kind);
            self.clamp_all_cursor();
            return;
        }
        // Sanatçılar yalnız kendi süzgeçlerinde ve yazı yazılınca gelir.
        if kind == Kind::Artists || (kind == Kind::All && !self.all.query.is_empty()) {
            cards.extend(artists);
        }
        // "Listeler" Spotify'ın sana özel listelerini de kapsar.
        cards.retain(|c| {
            kind == Kind::All
                || c.kind() == kind
                || (kind == Kind::Lists && c.kind() == Kind::ForYou)
        });
        // Sıralama: sabitlenenler önde, sonra seçilen sıra (sanatçılar kendi
        // aralarında en son takip edilen önde).
        {
            let default_meta = crate::engine::LibraryMeta::default();
            let meta = match &self.library.library {
                Loadable::Loaded(l) => &l.meta,
                _ => &default_meta,
            };
            sort_flat(
                &mut cards,
                self.all.sort,
                &SortCtx {
                    recent: &self.recent,
                    meta,
                },
            );
        }
        if let Some(mut f) = fuzzy::Fuzzy::new(&self.all.query) {
            let keys: Vec<String> = cards
                .iter()
                .map(|c| {
                    let name = if c.id == CollectionId::Liked {
                        t!(LikedSearchWords)
                    } else {
                        &c.name
                    };
                    // Sanatçının altyazısı yalnız "Sanatçı": her "ar" araması hepsini getirmesin.
                    let subtitle = if c.kind() == Kind::Artists {
                        ""
                    } else {
                        c.subtitle.as_str()
                    };
                    fuzzy::fold(&format!("{name} {subtitle}"))
                })
                .collect();
            let order = f.top(keys.iter().map(String::as_str), usize::MAX);
            cards = order.into_iter().map(|i| cards[i].clone()).collect();
        }
        self.all.cards = cards;
        self.clamp_all_cursor();
    }

    /// Klavye imleci artık görünmeyen bir karta kaldıysa bırak.
    fn clamp_all_cursor(&mut self) {
        let shown = self.all_shown();
        self.all.cursor = self.all.cursor.filter(|c| *c < shown);
    }

    fn all_query_changed(&mut self, cx: &mut Context<Self>) {
        self.all.cursor = None;
        self.recompute_all();
        self.scroll_all_to_top();
        cx.notify();
    }

    fn set_all_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.all.kind = kind;
        self.all_query_changed(cx);
    }

    /// "Tümü" açıkken tuşlar: yazılanlar süzgece, Enter ilk kartı açar, Esc
    /// kapatır. İşlendiyse `true` (değilse genel kısayollara gider).
    pub(super) fn all_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control;
        match ks.key.as_str() {
            // Esc yazılan süzgeci siler; boşken başka işe kalır (odağı bırakır).
            "escape" if !self.all.query.is_empty() => {
                self.all.query.clear();
                self.all_query_changed(cx);
            }
            // Izgarada bir klasörün içindeyken Esc / Backspace bir üst klasöre çıkar.
            "escape" | "backspace"
                if self.all.query.is_empty() && self.all.folder.is_some() && !self.all.list =>
            {
                self.leave_folder(cx);
            }
            "escape" => return false,
            "enter" => {
                let card = match self.all.cursor.filter(|c| *c < self.all_shown()) {
                    Some(i) => self.shown_card(i).cloned(),
                    None => self.all.cards.get(self.all_skip()).cloned(),
                };
                if let Some(card) = card {
                    self.activate_card(&card, cx);
                }
            }
            "backspace" => {
                if ctrl {
                    let trimmed = self.all.query.trim_end().len();
                    let cut = self.all.query[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                    self.all.query.truncate(cut);
                } else {
                    self.all.query.pop();
                }
                self.all_query_changed(cx);
            }
            "v" if ctrl => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    self.all
                        .query
                        .push_str(text.lines().next().unwrap_or_default());
                    self.all_query_changed(cx);
                }
            }
            // Boş kutuda Boşluk yine çal / duraklat (başa boşluk yazmanın
            // anlamı yok).
            "space" if !ctrl && !self.all.query.is_empty() => {
                self.all.query.push(' ');
                self.all_query_changed(cx);
            }
            "space" => return false,
            // Ok tuşları kartlar arasında gezer (arkadaki gizli listede satır seçilmesin).
            "up" | "down" | "left" | "right"
                if !ctrl && !ks.modifiers.alt && !ks.modifiers.shift =>
            {
                self.move_all_cursor(ks.key.as_str(), cx);
            }
            "up" | "down" if !ctrl => {}
            _ if !ctrl && !ks.modifiers.alt => {
                let Some(ch) = ks
                    .key_char
                    .as_deref()
                    .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                else {
                    return false;
                };
                self.all.query.push_str(ch);
                self.all_query_changed(cx);
            }
            _ => return false,
        }
        true
    }

    /// Izgarada bir satırda kaç kart var.
    pub(super) fn all_columns(&self) -> usize {
        if self.all.list {
            1
        } else {
            all_columns(self.window_width)
        }
    }

    /// Izgarada baştan kaç kart atlanır (7 Kasım gecesi): sıralama "Son
    /// çalınan", süzgeç ve yazı yokken raftaki kartlar ızgaranın başında bir
    /// daha çıkmaz (raf zaten onları gösteriyor); ızgara rafın bıraktığı
    /// yerden başlar. Süzülünce ya da başka sıralamada hiçbir şey atlanmaz;
    /// raftan fazlası yoksa da (ızgara boş kalmasın) atlanmaz.
    pub(super) fn all_skip(&self) -> usize {
        let slots = self.shelf_slots();
        let plain = self.all.sort == Sort::Recent
            && self.all.query.is_empty()
            && self.all.kind == Kind::All
            // Klasörlü kütüphane ağaç olarak çizilir: rafın kartlarını atlamak ağacı bozar.
            && !self.has_folders();
        if plain && self.all.cards.len() > slots {
            slots
        } else {
            0
        }
    }

    /// Izgaranın (atlananlar dışında) kart sayısı.
    pub(super) fn all_shown(&self) -> usize {
        self.all.cards.len() - self.all_skip()
    }

    /// Izgaranın görünen yüksekliği ve içeriğin boyu (px; bkz. `list_extent`).
    pub(super) fn all_extent(&self) -> Option<(f32, f32)> {
        let viewport = f32::from(self.all.scroll.0.borrow().last_item_size?.item.height);
        let rows = self.all_shown().div_ceil(self.all_columns());
        let row = if self.all.list {
            LIST_ROW
        } else {
            all_row_height(self.window_width)
        };
        Some((viewport, rows as f32 * row + PILL_CLEARANCE))
    }

    fn scroll_all_by(&self, d: f32) -> f32 {
        let max = self
            .all_extent()
            .map_or(0., |(viewport, content)| (content - viewport).max(0.));
        let handle = self.all.scroll.0.borrow().base_handle.clone();
        let offset = handle.offset();
        let from = (-f32::from(offset.y)).max(0.);
        let to = (from + d).clamp(0., max);
        if to != from {
            handle.set_offset(gpui::point(offset.x, px(-to)));
        }
        to - from
    }

    fn wheel_all(
        &mut self,
        delta: ScrollDelta,
        line_height: Pixels,
        viewport: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(handled) = self.wheel_gate(cx) {
            return handled;
        }
        let now = std::time::Instant::now();
        let glide = self.all.glide.clone();
        match glide.classify(delta, line_height, viewport, now) {
            Some(Wheel::Glide(d)) => glide.push(d, viewport, now),
            Some(Wheel::Now(d)) => {
                glide.cancel();
                self.scroll_all_by(d);
            }
            None => return false,
        }
        true
    }

    /// Izgaradaki tekerlek hareketinin bu karelik adımı; sürüyorsa `true`.
    pub(super) fn tick_all_glide(&mut self) -> bool {
        let glide = self.all.glide.clone();
        if !glide.is_active() {
            return false;
        }
        let position = (-f32::from(self.all.scroll.0.borrow().base_handle.offset().y)).max(0.);
        glide.tick(std::time::Instant::now(), position, |d| {
            self.scroll_all_by(d)
        })
    }

    pub(super) fn render_all(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // Kütüphane yok ve bir nedeni var (giriş, çevrimdışı, Premium …): iskelet
        // yerine tam sayfa durum (bkz. `welcome`).
        if let Some(gate) = self.library_gate() {
            return self.gate_fill(&gate, window, cx);
        }
        let accent = self.accent().text;
        let empty = self.all.query.is_empty();
        let caret = || {
            div()
                .w(px(2.))
                .h(px(T_SECONDARY.line))
                .flex_shrink_0()
                .bg(rgb(accent))
        };
        // Süzme kutusu: açılır açılmaz yazılır (odak hep burada, kenarı kapak
        // ışığı renginde).
        let field = div()
            .flex_basis(px(ALL_FIELD_WIDTH))
            .flex_shrink()
            .min_w(px(ALL_FIELD_MIN))
            .h(px(super::SEARCH_HEIGHT))
            .px(px(S3))
            .rounded(px(R_LG))
            .flex()
            .items_center()
            .gap(px(S2))
            // Sakin: boşken kenarsız ton zemini; yazılırken kenar kapak ışığı.
            .bg(veil(VEIL_HOVER))
            .border_1()
            .border_color(if empty {
                gpui::transparent_black()
            } else {
                rgb(accent).into()
            })
            .child(icon("search", 14., muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .type_role(T_SECONDARY)
                    .text_color(if empty { muted() } else { text() })
                    .when(empty, |el| el.child(caret()).child(t!(FindInLibrary)))
                    .when(!empty, |el| el.child(self.all.query.clone()).child(caret())),
            );

        // Tür süzgeçleri: yeniden tıklamak süzgeci kaldırır (hepsi).
        let mut chips = div().flex_none().flex().items_center().gap(px(S2));
        for (i, (kind, name_key)) in KINDS.into_iter().enumerate() {
            let name = crate::i18n::t(name_key);
            let (stop, look) =
                self.stop_look(format!("all-kind-{i}"), super::tab::PAGE, true, window, cx);
            chips = chips.child(
                chip(("all-kind", i), name, self.all.kind == kind, look, accent)
                    .when_some(stop, |el, h| el.track_focus(&h))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let next = if this.all.kind == kind {
                            Kind::All
                        } else {
                            kind
                        };
                        this.set_all_kind(next, cx);
                    })),
            );
        }

        let sort = self.all.sort;
        let list = self.all.list;
        let view_button = |id: &'static str,
                           icon_name: &'static str,
                           tip: &'static str,
                           on: bool,
                           to_list: bool,
                           cx: &mut Context<Self>| {
            let (stop, look) = self.stop_look(id, super::tab::PAGE, true, window, cx);
            icon_button(id, icon_name, look, accent)
                .when_some(stop, |el, h| el.track_focus(&h))
                .tooltip(Tooltip::text(tip, None))
                .when(on, |el| el.bg(veil(VEIL_SELECTED)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_all_view(to_list, cx)))
        };
        let (sort_stop, sort_look) = self.stop_look("all-sort", super::tab::PAGE, true, window, cx);
        let (new_stop, new_look) = self.stop_look("all-new", super::tab::PAGE, true, window, cx);
        let (tools_stop, tools_look) =
            self.stop_look("all-tools", super::tab::PAGE, true, window, cx);
        let wide = self.window_width >= ALL_COUNT_MIN_WIDTH;
        // Sayaç yalnız süzülürken anlamlı ("n / N"); süzgeçsiz hâlde sessiz.
        let filtered = !empty || self.all.kind != Kind::All;
        let top = div()
            .flex_shrink_0()
            .px(px(page_margin(self.window_width)))
            .pt(px(S4))
            .pb(px(S5))
            .flex()
            .items_center()
            .gap(px(S5))
            .child(field)
            .child(chips)
            // Sayaç doğal genişliğinde (flex_none: "…" olmasın); sığmayan dar
            // pencerede hiç çizilmez, boşluğu yalnız ara dolgu alır.
            .child(div().flex_1().min_w_0())
            .when(wide && filtered, |el| {
                el.child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .type_role(T_LABEL)
                        .tabular()
                        .text_color(label())
                        .child(format!("{} / {}", self.all.cards.len(), self.all.total)),
                )
            })
            // Sıralama: tıklayınca dört seçenekli menü (Son çalınan / Eklenme tarihi /
            // Alfabetik / Yaratıcı) düğmenin altında açılır; seçim kalıcı.
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        ghost_button(
                            "all-sort",
                            "arrow-up-down",
                            sort.label_for(self.all.kind),
                            false,
                            sort_look,
                            accent,
                        )
                        .when_some(sort_stop, |el, h| el.track_focus(&h))
                        // Menü açıkken ipucu menünün üstüne binmesin.
                        .when(self.manage.menu.is_none(), |el| {
                            el.tooltip(Tooltip::text(
                                tf!(SortTip, sort.label_for(self.all.kind)),
                                None,
                            ))
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.open_sort_menu(cx))),
                    )
                    .child(self.sort_anchor_probe()),
            )
            // Izgara / liste (seçim hatırlanır).
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(view_button(
                        "all-view-grid",
                        "layout-grid",
                        t!(ViewGrid),
                        !list,
                        false,
                        cx,
                    ))
                    .child(view_button(
                        "all-view-list",
                        "list",
                        t!(ViewList),
                        list,
                        true,
                        cx,
                    )),
            )
            // Kütüphane araçları sayfası (yinelenenler, birleştir / böl, sıralama, yedek, istatistik).
            .child(if wide {
                ghost_button(
                    "all-tools",
                    "wrench",
                    t!(ToolsTitle),
                    false,
                    tools_look,
                    accent,
                )
                .when_some(tools_stop, |el, h| el.track_focus(&h))
                .on_click(cx.listener(|this, _, _, cx| this.open_tools(cx)))
                .into_any_element()
            } else {
                icon_button("all-tools", "wrench", tools_look, accent)
                    .when_some(tools_stop, |el, h| el.track_focus(&h))
                    .tooltip(Tooltip::text(t!(ToolsTitle), None))
                    .on_click(cx.listener(|this, _, _, cx| this.open_tools(cx)))
                    .into_any_element()
            })
            // Yeni (boş) liste: adı sorulur, oluşunca açılır.
            .child(if wide {
                ghost_button("all-new", "plus", t!(NewPlaylist), true, new_look, accent)
                    .when_some(new_stop, |el, h| el.track_focus(&h))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.ask_new_playlist(Vec::new(), true, cx)),
                    )
                    .into_any_element()
            } else {
                icon_button("all-new", "plus", new_look, accent)
                    .when_some(new_stop, |el, h| el.track_focus(&h))
                    .tooltip(Tooltip::text(t!(NewPlaylist), None))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.ask_new_playlist(Vec::new(), true, cx)),
                    )
                    .into_any_element()
            });

        let body: AnyElement = match &self.library.library {
            Loadable::Loading if !self.page_skeleton_due("library", window) => {
                div().into_any_element()
            }
            Loadable::Loading => self.library_skeleton(),
            // Hata `library_gate`te (yukarıda tam sayfa durum döndü); buraya gelinmez.
            Loadable::Failed(_) => div().into_any_element(),
            // İçine girilen klasör boş.
            Loadable::Loaded(_)
                if self.all.cards.is_empty() && self.tree_mode() && self.all.folder.is_some() =>
            {
                centered(state_view(
                    "folder",
                    t!(FolderEmptyTitle),
                    t!(FolderEmptyBody),
                ))
            }
            // Boş kütüphane; ya da yalnız sanatçı takip ediliyor (süzgeçsiz
            // "Tümü" listeleri ve albümleri gösterir, sanatçılar kendi süzgecinde).
            Loadable::Loaded(_)
                if self.all.total == 0
                    || (self.all.cards.is_empty()
                        && self.all.kind == Kind::All
                        && self.all.query.is_empty()) =>
            {
                centered(state_view(
                    "library",
                    t!(LibraryEmptyTitle),
                    t!(LibraryEmptyBody),
                ))
            }
            Loadable::Loaded(_)
                if self.all.cards.is_empty() && self.all.kind == Kind::Artists && empty =>
            {
                centered(state_view(
                    "user",
                    t!(LibraryNoArtistsTitle),
                    t!(LibraryNoArtistsBody),
                ))
            }
            Loadable::Loaded(_) if self.all.cards.is_empty() => centered(state_view(
                "search",
                t!(NoMatchingPlaylists),
                t!(NoMatchingPlaylistsBody),
            )),
            Loadable::Loaded(_) => self.render_all_grid(cx),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(top)
            .children(self.render_folder_crumbs(window, cx))
            .child(body)
            .into_any_element()
    }

    /// Kütüphane yüklenirken: ızgara ya da liste biçiminde iskelet (yerleşim
    /// zıplamaz).
    fn library_skeleton(&self) -> AnyElement {
        let margin = page_margin(self.window_width);
        if self.all.list {
            let mut rows = div().flex().flex_col().px(px(margin));
            for i in 0..8 {
                let w = [0.34, 0.22, 0.4, 0.28][i % 4];
                rows = rows.child(
                    div()
                        .h(px(LIST_ROW))
                        .px(px(S2))
                        .flex()
                        .items_center()
                        .gap(px(S3))
                        .child(
                            div()
                                .size(px(LIST_COVER))
                                .rounded(px(R_SM))
                                .bg(veil(VEIL_HOVER)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(S1))
                                .child(
                                    div()
                                        .w(px(240. * w * 2.))
                                        .h(px(T_SECONDARY.size))
                                        .rounded(px(R_XS))
                                        .bg(veil(VEIL_HOVER)),
                                )
                                .child(
                                    div()
                                        .w(px(120.))
                                        .h(px(T_LABEL.size))
                                        .rounded(px(R_XS))
                                        .bg(veil(VEIL_HOVER)),
                                ),
                        ),
                );
            }
            return rows.into_any_element();
        }
        let cols = self.all_columns();
        let cover = all_cover(self.window_width);
        let mut grid = div().px(px(margin)).pt(px(S1)).flex().flex_col();
        for r in 0..3 {
            let mut line = div()
                .h(px(all_row_height(self.window_width)))
                .flex()
                .items_start()
                .gap(px(GRID_GAP));
            for c in 0..cols {
                line = line.child(skeleton_card(r * cols + c, cover, SHELF_NAME));
            }
            grid = grid.child(line);
        }
        grid.into_any_element()
    }

    fn set_all_view(&mut self, list: bool, cx: &mut Context<Self>) {
        if self.all.list == list {
            return;
        }
        self.all.list = list;
        // Klasörler listede satır arasında açılır, ızgarada içine girilir: kartlar yeniden kurulur.
        self.all.cursor = None;
        self.recompute_all();
        self.scroll_all_to_top();
        self.save_settings();
        cx.notify();
    }

    pub(super) fn set_all_sort(&mut self, sort: Sort, cx: &mut Context<Self>) {
        self.all.sort = sort;
        self.save_settings();
        self.all_query_changed(cx);
    }

    fn render_all_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let cols = self.all_columns();
        let list = self.all.list;
        let rows = if list {
            self.all_shown()
        } else {
            self.all_shown().div_ceil(cols)
        };
        // Satırlar şarkı listesi gibi hapın altından geçer (bkz.
        // `render_track_list`).
        let grid = uniform_list(
            "all-grid",
            rows,
            cx.processor(
                move |this: &mut Shell, range: std::ops::Range<usize>, _, cx| {
                    let rows = range
                        .map(|r| {
                            if list {
                                this.all_list_row(r, cx)
                            } else {
                                this.all_row(r, cols, cx)
                            }
                        })
                        .collect();
                    this.flush_image_requests(cx);
                    rows
                },
            ),
        )
        .track_scroll(self.all.scroll.clone())
        // Boyu içerikten değil, iki kenarından (üst ve alt) gelsin.
        .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .bottom(px(-PILL_CLEARANCE))
        .pb(px(PILL_CLEARANCE))
        .into_any_element();

        let this = cx.entity().downgrade();
        let wheel = super::smooth_scroll::wheel_zone(move |ev, bounds, view, window, cx| {
            let line_height = window.line_height();
            let viewport = f32::from(bounds.size.height);
            let handled = this
                .update(cx, |shell, cx| {
                    shell.wheel_all(ev.delta, line_height, viewport, cx)
                })
                .unwrap_or(false);
            if handled {
                cx.notify(view);
            }
            handled
        });

        div()
            .relative()
            .group("all-scroll")
            .flex_1()
            .min_h_0()
            .child(grid)
            .child(wheel)
            .children(self.scrollbar(super::ScrollBar::All, cx))
            .into_any_element()
    }

    /// Görünen `i`. karta klavye halkası çizilsin mi: ok tuşlarıyla gelinmişse imleçteki,
    /// değilse yazarken Enter'ın açacağı ilk kart.
    fn card_focus(&self, i: usize) -> bool {
        match self.all.cursor {
            Some(c) => c == i,
            None => i == 0 && !self.all.query.is_empty(),
        }
    }

    fn all_row(&self, row: usize, cols: usize, cx: &mut Context<Self>) -> AnyElement {
        let skip = self.all_skip();
        let start = skip + row * cols;
        let end = (start + cols).min(self.all.cards.len());
        let cover = all_cover(self.window_width);
        let mut el = div()
            .w_full()
            .h(px(all_row_height(self.window_width)))
            .px(px(page_margin(self.window_width)))
            .pt(px(S1))
            .flex()
            .items_start()
            .gap(px(GRID_GAP));
        for i in start..end {
            let card = &self.all.cards[i];
            let state = CardState {
                // Klavye imleci (ok tuşları) ya da yazarken Enter'ın açacağı kart (ilki) halkayla belli.
                look: if self.card_focus(i - skip) {
                    Look::Focus
                } else {
                    Look::Live
                },
                selected: self.library.selected.as_ref() == Some(&card.id),
                playing: self.playing_collection() == Some(&card.id),
            };
            el = el.child(self.shelf_card(
                ElementId::Name(format!("all-{}", card.id.key()).into()),
                card,
                cover,
                SHELF_NAME,
                state,
                CardAction::Open,
                cx,
            ));
        }
        el.into_any_element()
    }

    /// Liste görünümünde bir satır: küçük kapak, ad ve açıklama, sağda tür.
    /// Klasörlü kütüphanede (ağaç) satırın başında 16 px'lik ok yeri durur
    /// (klasörde ▸ / ▾, listelerde boş): kapaklar aynı hizada kalır; klasörün
    /// içindekiler `depth` kadar girintili.
    fn all_list_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let card = &self.all.cards[self.all_skip() + ix];
        let playing = self.playing_collection() == Some(&card.id);
        let selected = self.library.selected.as_ref() == Some(&card.id);
        let focus = self.card_focus(ix);
        let name_color = if playing || selected {
            rgb(accent)
        } else {
            text()
        };
        let folder = card.meta.folder.clone();
        let art = if folder.is_some() {
            self.folder_art(LIST_COVER, R_SM)
        } else {
            self.collection_art(&card.id, &card.name, &card.image, LIST_COVER, R_SM)
        };
        let kind = match card.kind() {
            _ if folder.is_some() => t!(KindFolder),
            Kind::Albums => t!(Album),
            Kind::ForYou => t!(KindForYou),
            // Sanatçının altyazısı zaten "Sanatçı": sağda yinelenmez.
            Kind::Artists => "",
            _ => t!(Playlist),
        };
        let tree = self.has_folders() && self.tree_mode();
        let chevron = tree.then(|| {
            div()
                .size(px(S4))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .children(folder.as_ref().map(|f| {
                    icon(
                        if f.open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        14.,
                        label(),
                    )
                }))
        });
        // Ağaçta satır, klasör satırlarıyla aynı kimlikte olmasın diye sırası da kimliğe girer
        // (sabitli bir liste hem en üstte hem klasörünün içinde görünebilir).
        let key = card.id.key();
        let card = card.clone();
        let menu_target = card_menu_target(&card);
        div()
            .id(ElementId::Name(format!("all-list-{ix}-{key}").into()))
            .w_full()
            .h(px(LIST_ROW))
            .px(px(page_margin(self.window_width)))
            .flex()
            .items_center()
            .child(
                div()
                    .id(ElementId::Name(format!("all-list-in-{ix}-{key}").into()))
                    .relative()
                    .size_full()
                    .pl(px(S2 + card.meta.depth as f32 * S6))
                    .pr(px(S2))
                    .rounded(px(R_MD))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .cursor_pointer()
                    .when(focus, |el| el.border_1().border_color(rgb(accent)))
                    .child(super::tween::hover_fill(
                        veil(VEIL_HOVER),
                        veil(VEIL_PRESSED),
                        0.,
                        R_MD,
                    ))
                    .on_click(cx.listener({
                        let card = card.clone();
                        move |this, _, _, cx| this.activate_card(&card, cx)
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_card_menu(menu_target.clone(), ev.position, cx);
                        }),
                    )
                    .children(chevron)
                    .child(div().flex_none().child(art))
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
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(name_color)
                                    .child(
                                        line_with_tip(
                                            ElementId::Name(format!("alln-{ix}-{key}").into()),
                                            card.name.clone(),
                                        )
                                        .flex()
                                        .min_w_0(),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .type_role(T_SECONDARY)
                                    .text_color(muted())
                                    .child(
                                        line_with_tip(
                                            ElementId::Name(format!("alls-{ix}-{key}").into()),
                                            card.subtitle.clone(),
                                        )
                                        .flex()
                                        .min_w_0(),
                                    ),
                            ),
                    )
                    .when(playing, |el| el.child(icon("volume-2", 16., rgb(accent))))
                    .when(card.pinned(), |el| el.child(icon("pin", 14., label())))
                    .child(
                        div()
                            .flex_none()
                            .type_role(T_LABEL)
                            .text_color(label())
                            .child(kind),
                    ),
            )
            .into_any_element()
    }
}

/// Kartın sağ tık menüsü: sanatçı kartı sanatçı menüsünü (takip, bağlantı),
/// gerisi liste / albüm menüsünü açar.
fn card_menu_target(card: &Card) -> super::manage::CardTarget {
    if let Some(folder) = &card.meta.folder {
        return super::manage::CardTarget::Folder {
            key: folder.key.clone(),
        };
    }
    match &card.id {
        CollectionId::Artist(uri) => super::manage::CardTarget::Artist(uri.clone()),
        id => super::manage::CardTarget::Collection {
            id: id.clone(),
            title: if *id == CollectionId::Liked {
                liked_card_name().into()
            } else {
                card.name.clone()
            },
        },
    }
}

/// Rafa kaç kart sığar: kartlar en az 16 px arayla dizilir.
fn shelf_slot_count(window_width: f32) -> usize {
    let room = window_width - 2. * page_margin(window_width);
    (((room + SHELF_GAP) / (SHELF_COVER + SHELF_GAP)).floor() as usize).max(2)
}

/// Raf doluyken kartlar arası (7 Kasım kararı): kartlar sabit boyda kalır,
/// artan yer aralara eşit dağılır; son kart sağ sayfa kenarına oturur (liste
/// başlığındaki Çal düğmesinin hizası). Önce aralar hep 16 px'ti, pencere
/// enine göre sağda boş yer kalıyordu.
fn shelf_gap(window_width: f32, slots: usize) -> f32 {
    if slots < 2 {
        return SHELF_GAP;
    }
    let room = window_width - 2. * page_margin(window_width);
    ((room - slots as f32 * SHELF_COVER) / (slots - 1) as f32).max(SHELF_GAP)
}

/// Kütüphane ızgarasında bir satırdaki kart sayısı: kart eni hedefine (168)
/// en yakın tam sayı.
fn all_columns(window_width: f32) -> usize {
    let room = window_width - 2. * page_margin(window_width);
    (((room + GRID_GAP) / (GRID_TARGET + GRID_GAP)).round() as usize).max(1)
}

/// Izgara kartının eni: sütunlar içerik enini 24 px aralarla tam doldurur, son
/// sütun sağ sayfa kenarına oturur (yarım kart, sağda boşluk yok).
fn all_cover(window_width: f32) -> f32 {
    let room = window_width - 2. * page_margin(window_width);
    let cols = all_columns(window_width) as f32;
    (room - GRID_GAP * (cols - 1.)) / cols
}

/// Izgarada bir satırın yüksekliği: kart, ad ve satır arası 24 px (seçim
/// halkası için üstte 4 px pay).
fn all_row_height(window_width: f32) -> f32 {
    S1 + all_cover(window_width) + SHELF_NAME + GRID_GAP
}

/// Kütüphane yüklenirken raftaki iskelet kart.
fn skeleton_card(i: usize, cover: f32, name_h: f32) -> AnyElement {
    let name = [0.8, 0.55, 0.7, 0.9, 0.6, 0.75][i % 6] * cover;
    div()
        .flex_none()
        .w(px(cover))
        .flex()
        .flex_col()
        .child(div().size(px(cover)).rounded(px(R_LG)).bg(veil(VEIL_HOVER)))
        .child(
            div().h(px(name_h)).overflow_hidden().child(
                div().pt(px(S2)).child(
                    div()
                        .mt(px(S1))
                        .w(px(name))
                        .h(px(10.))
                        .rounded(px(R_XS))
                        .bg(veil(VEIL_HOVER)),
                ),
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        TITLEBAR_HEIGHT,
        library::{COLUMN_HEADER_HEIGHT, ROW_HEIGHT},
    };

    fn card(id: CollectionId) -> Card {
        Card {
            id,
            name: String::new(),
            subtitle: String::new(),
            image: String::new(),
            meta: CardMeta::default(),
        }
    }

    #[test]
    fn shelf_puts_recent_first_and_liked_first_if_never_played() {
        let album = |s: &str| CollectionId::Album(format!("spotify:album:{s}"));
        let mut cards = vec![
            card(CollectionId::Liked),
            card(album("a")),
            card(album("b")),
            card(album("c")),
        ];
        shelf_order(
            &mut cards,
            &["spotify:album:c".into(), "spotify:album:a".into()],
        );
        let keys: Vec<&str> = cards.iter().map(|c| c.id.key()).collect();
        assert_eq!(
            keys,
            [
                "liked",
                "spotify:album:c",
                "spotify:album:a",
                "spotify:album:b"
            ]
        );

        let mut cards = vec![
            card(CollectionId::Liked),
            card(album("a")),
            card(album("b")),
        ];
        shelf_order(&mut cards, &["spotify:album:b".into(), "liked".into()]);
        let keys: Vec<&str> = cards.iter().map(|c| c.id.key()).collect();
        assert_eq!(keys, ["spotify:album:b", "liked", "spotify:album:a"]);
    }

    #[test]
    fn followed_artists_have_their_own_kind_chip_and_menu() {
        use crate::ui::manage::CardTarget;
        let artist = Card {
            id: CollectionId::Artist("spotify:artist:a".into()),
            name: "A".into(),
            subtitle: "Sanatçı".into(),
            image: String::new(),
            meta: CardMeta::default(),
        };
        assert_eq!(artist.kind(), Kind::Artists);
        assert!(
            matches!(card_menu_target(&artist), CardTarget::Artist(u) if u == "spotify:artist:a")
        );
        // Liste ve albüm kartları eskisi gibi liste menüsünü açar.
        let album = card(CollectionId::Album("spotify:album:x".into()));
        assert!(matches!(
            card_menu_target(&album),
            CardTarget::Collection { .. }
        ));
        // Süzgeç hapları: Listeler, Albümler, Sanatçılar; sanatçı yalnız kendi hapında.
        let kinds: Vec<Kind> = KINDS.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, [Kind::Lists, Kind::Albums, Kind::Artists]);
        // "Son takip edilen" yalnız sanatçı süzgecinde; öbür yerde "Son çalınan".
        assert_ne!(
            Sort::Recent.label_for(Kind::Artists),
            Sort::Recent.label_for(Kind::Lists)
        );
        assert_eq!(
            Sort::Name.label_for(Kind::Artists),
            Sort::Name.label_for(Kind::All)
        );
    }

    #[test]
    fn shelf_and_header_lose_exactly_the_scrolled_distance() {
        for i in 0..=10 {
            let p = i as f32 / 10.;
            let lost = (SHELF_OPEN - shelf_height(p)) + (HEADER_OPEN - header_height(p));
            assert!((lost - p * COLLAPSE_MAX).abs() < 1e-3, "p = {p}");
        }
        assert_eq!(SHELF_OPEN, 182.);
        assert_eq!(STRIP_HEIGHT, 56.);
    }

    /// Raf: son kart her pencere eninde sağ sayfa kenarında, aralar en az 16 px.
    #[test]
    fn shelf_fills_the_row() {
        let mut w = 640.;
        while w <= 2560. {
            let slots = shelf_slot_count(w);
            let gap = shelf_gap(w, slots);
            assert!(gap >= SHELF_GAP, "{w}: {gap}");
            let right = page_margin(w) + slots as f32 * SHELF_COVER + (slots - 1) as f32 * gap;
            assert!((right - (w - page_margin(w))).abs() < 0.01, "{w}: {right}");
            w += 1.;
        }
        // 1180: yedi kart, aralar ~27 px (gerekçe `SHELF_COVER`'da).
        assert_eq!(shelf_slot_count(1180.), 7);
        assert!((shelf_gap(1180., 7) - 28.).abs() < 1.5);
    }

    /// Kütüphane ızgarası: kartlar ~168 px (144–200: sütun sayısı değişirken
    /// uçlara gider), aralar 24 px, son sütun sağ sayfa kenarında (kapatma
    /// düğmesinin sağ kenarı).
    #[test]
    fn library_grid_fills_the_row_with_airy_gaps() {
        let mut w = 640.;
        while w <= 2560. {
            let cols = all_columns(w);
            let cover = all_cover(w);
            assert!((140.0..=200.0).contains(&cover), "{w}: {cover}");
            let right = page_margin(w) + cols as f32 * cover + (cols - 1) as f32 * GRID_GAP;
            assert!((right - (w - page_margin(w))).abs() < 0.01, "{w}: {right}");
            w += 1.;
        }
        assert_eq!(all_columns(1180.), 6);
        assert_eq!(GRID_GAP, 24.);
    }

    #[test]
    fn density_bar_at_1180x760() {
        let pill_top = 760. - PILL_CLEARANCE;
        let rows = |shelf: f32, header: f32| {
            (pill_top - (TITLEBAR_HEIGHT + shelf + header + COLUMN_HEADER_HEIGHT)) / ROW_HEIGHT
        };
        let open = rows(SHELF_OPEN, HEADER_OPEN);
        let scrolled = rows(STRIP_HEIGHT, HEADER_COMPACT);
        assert!(open >= 5.7, "raf açık: {open:.2} satır");
        assert!(scrolled >= 8.6, "kayınca: {scrolled:.2} satır");
    }
}
