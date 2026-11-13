//! Kütüphane görünümü: açık listenin başlığı ve şarkıları. Listeler arasında
//! kapak rafıyla (`shelf.rs`) gezilir.

use std::{collections::HashMap, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    Animation, AnimationExt, AnyElement, AppContext, ClickEvent, Context, ElementId, FontWeight,
    Pixels, ScrollDelta, ScrollStrategy, SharedString, UniformListScrollHandle, div,
    ease_out_quint, img, prelude::*, px, rgb, uniform_list,
};

use super::{
    Shell,
    components::{
        Look, explain_error, icon_button, icon_button_tinted, is_forbidden, primary_button_sized,
        secondary_button, state_view,
    },
    cover_image,
    manage::ListKey,
    pill::{PILL_CLEARANCE, PILL_LIST_END},
    player::fmt_time,
    shelf::{COLLAPSE_MAX, COMPACT_AT, header_height},
    smooth_scroll::{SmoothScroll, Wheel},
    text::line_with_tip,
    theme::*,
};
use crate::engine::{CollectionId, Command, Library, LibraryItem, TrackRow};
use crate::{t, tf, tp};

pub(super) const ROW_HEIGHT: f32 = 56.;
const ROW_THUMB: f32 = 40.;
/// Numara / çal hücresi: en küçük tıklama alanı (32×32 px).
const NUMBER_CELL: f32 = 32.;
/// En dar sayfa kenar boşluğu (32 px). Gerçek boşluk pencere enine göre
/// `theme::page_margin`'den gelir; bu sabit yalnız eşik hesaplarında
/// (yerleşik panel eşiği, hap eni) ve henüz geçmemiş yerlerde kalır.
pub(super) const PAGE_PAD: f32 = S6;
/// Satırın iç yatay boşluğu (seçili satırın zemini kenardan bu kadar taşar).
/// Satırlar, sütun başlıkları ve liste başlığı içerik alanının bütün enini
/// kullanır (iki yanda 32 px sayfa kenarı; 7 Kasım, kullanıcı geri
/// bildirimi: 1280 px'lik üst sınır geniş pencerede ve panel kapanınca sağda
/// büyük bir boşluk bırakıyordu). Süre sağ sayfa kenarında, şarkı ve albüm
/// sütunları aradaki yeri 4'e 3 paylaşır.
const ROW_INSET: f32 = S3;

/// Bir şarkı satırının neyi gösterdiği ve nasıl davrandığı.
pub(super) struct RowSpec {
    pub list: ListKey,
    /// Listedeki sırası (süzgeç açıkken de asıl sıra).
    pub ix: usize,
    pub show_cover: bool,
    pub show_album: bool,
    /// Albümde sanatçı albümünkiyle aynıysa yazılmaz.
    pub hide_artist: bool,
    /// Sanatçılar başlığın altında değil, ayrı sütunda (geniş albüm sayfası).
    pub artist_column: bool,
    pub playing: bool,
    pub selected: bool,
    pub look: Look,
    /// Kendi listende sürükleyip sıralanabilir.
    pub reorder: bool,
    /// "Eklenme tarihi" sütunu: `Some(yazı)` sütun var (tarih bilinmiyorsa yazı boş),
    /// `None` sütun yok (albüm, sayfalar, dar pencere).
    pub added: Option<SharedString>,
    /// Dinlenme sayısı sütunu: `Some(sayı)` sütun var (sayfa satırları,
    /// yeterince geniş pencere), `None` yok (kütüphane listeleri, dar pencere).
    pub plays: Option<u64>,
}

/// Seçim bitişik bir blok mu: (ilk satır, uzunluk).
fn contiguous(rows: &std::collections::BTreeSet<usize>) -> Option<(usize, usize)> {
    let first = *rows.first()?;
    let last = *rows.last()?;
    (last - first + 1 == rows.len()).then_some((first, rows.len()))
}

pub enum Loadable<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

pub struct LibraryState {
    pub library: Loadable<Library>,
    pub selected: Option<CollectionId>,
    pub collections: HashMap<CollectionId, Loadable<Arc<Vec<TrackRow>>>>,
    /// Tek tıklamayla ya da ↑/↓ ile seçilmiş satır.
    pub selected_row: Option<usize>,
    /// Kütüphanede olmayan ama aramadan açılan listelerin adları.
    pub titles: HashMap<CollectionId, String>,
    pub scroll: UniformListScrollHandle,
    /// Rafın ve liste başlığının kaydırmayla birlikte ne kadar kapandığı
    /// (px, 0..`COLLAPSE_MAX`). Liste kaymadan önce raf şeride iner ve
    /// başlık küçülür; liste en üstteyken yukarı kaydırınca açılırlar.
    pub header_collapse: f32,
    /// Tekerlekle yumuşak kaydırma (raf, başlık ve liste tek hareket).
    pub list_glide: Rc<SmoothScroll>,
    /// Listelerin şarkılarının eklenme anları (şarkı adresi → unix ms); motor
    /// önbellekten gönderir (`LibraryEvent::AddedDates`).
    pub added: HashMap<CollectionId, Arc<HashMap<String, i64>>>,
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            library: Loadable::Loading,
            selected: None,
            collections: HashMap::new(),
            selected_row: None,
            titles: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
            header_collapse: 0.,
            list_glide: Rc::default(),
            added: HashMap::new(),
        }
    }

    pub(super) fn item(&self, id: &CollectionId) -> Option<&LibraryItem> {
        match &self.library {
            Loadable::Loaded(library) => library
                .albums
                .iter()
                .chain(&library.playlists)
                .chain(&library.artists)
                .find(|i| &i.id == id),
            _ => None,
        }
    }

    pub(super) fn title(&self, id: &CollectionId) -> String {
        if id == &CollectionId::Liked {
            return t!(LikedSongs).into();
        }
        self.item(id)
            .map(|i| i.name.clone())
            .or_else(|| self.titles.get(id).cloned())
            .unwrap_or_default()
    }

    pub fn tracks(&self, id: &CollectionId) -> Option<&Arc<Vec<TrackRow>>> {
        match self.collections.get(id) {
            Some(Loadable::Loaded(tracks)) => Some(tracks),
            _ => None,
        }
    }
}

impl Shell {
    /// Listeyi aç (geri / ileri geçmişine eklenir, bkz. `pages::navigate`).
    pub(super) fn select_collection(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        // Takip edilen sanatçı bir şarkı listesi değil: sanatçı sayfası açılır.
        if let CollectionId::Artist(uri) = id {
            self.navigate(super::history::Route::Artist(uri), cx);
            return;
        }
        self.navigate(super::history::Route::Collection(id), cx);
    }

    /// Listeyi göster. Raf açık ya da şerit hâlinde, nasılsa öyle kalır:
    /// şeritteki bir kapağa tıklayınca raf farenin altında yeniden açılmasın.
    /// `scroll`: geri / ileri ile dönülünce bırakıldığı konum (raf + liste).
    pub(super) fn show_collection(
        &mut self,
        id: CollectionId,
        scroll: Option<f32>,
        cx: &mut Context<Self>,
    ) {
        if self.library.selected.as_ref() == Some(&id) && scroll.is_none() {
            cx.notify();
            return;
        }
        self.library.selected = Some(id.clone());
        self.library.selected_row = None;
        self.library.list_glide.cancel();
        match scroll {
            Some(position) => {
                let collapse = position.min(COLLAPSE_MAX);
                self.library.header_collapse = collapse;
                let handle = self.library.scroll.0.borrow().base_handle.clone();
                handle.set_offset(gpui::point(px(0.), px(-(position - collapse))));
            }
            None => self
                .library
                .scroll
                .scroll_to_item_strict(0, ScrollStrategy::Top),
        }
        self.trace_scroll("select", || id.key().to_string());
        // Geliştirme denemesi: OPENIFY_TEST_FIND="sorgu" ilk açılan listede
        // bulu açar, OPENIFY_TEST_SELECT="a-b" satırları seçer (Ctrl / Shift
        // tuşları pencereye iletiyle gönderilemiyor).
        static TEST_HOOKS: std::sync::Once = std::sync::Once::new();
        TEST_HOOKS.call_once(|| {
            if let Ok(query) = std::env::var("OPENIFY_TEST_FIND") {
                self.manage.find = Some(super::manage::Find::with_query(query));
            }
            if let Some((a, b)) = std::env::var("OPENIFY_TEST_SELECT").ok().and_then(|v| {
                v.split_once('-')
                    .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
            }) {
                self.manage.sel = super::manage::Selection {
                    list: Some(ListKey::Collection(id.clone())),
                    rows: (a..=b).collect(),
                    anchor: Some(a),
                };
            }
        });
        // Kütüphanedeki albüm: sanatçı adlarının bağı ve yılı albüm sayfasından.
        if let CollectionId::Album(uri) = &id {
            // Önbellek okuması arka plan yürütücüsünde; sonuç gelince yerleşir
            // (yoksa albüm arkadan istenir).
            let uri = uri.clone();
            cx.spawn(async move |this, cx| {
                let hit = cx
                    .background_executor()
                    .spawn({
                        let uri = uri.clone();
                        async move { crate::engine::cached_album(&uri) }
                    })
                    .await;
                let _ = this.update(cx, |shell, cx| {
                    match hit {
                        Some((page, _)) => {
                            shell
                                .pages
                                .lists
                                .insert(format!("album:{uri}"), Arc::new(page.tracks.clone()));
                            shell
                                .pages
                                .albums
                                .insert(uri, super::pages::PageLoad::Ready(page));
                        }
                        None => shell.prefetch(uri),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        self.library
            .collections
            .entry(id.clone())
            .or_insert(Loadable::Loading);
        self.engine.send(Command::OpenCollection(id));
        cx.notify();
    }

    /// Eklenme tarihi sütunu bu listede görünür mü: listeler ve Beğenilenler
    /// (albümde yok), içerik yeterince genişse (bkz. `ADDED_MIN_WIDTH`).
    pub(super) fn shows_added_column(&self, id: &CollectionId) -> bool {
        matches!(id, CollectionId::Liked | CollectionId::Playlist(_))
            && self.window_width - self.dock_inset() >= ADDED_MIN_WIDTH
    }

    /// Listenin şarkılarının eklenme anlarını motordan iste (önbellekten gelir,
    /// ağa gidilmez; liste her yüklendiğinde yenilenir).
    pub(super) fn request_added_dates(&self, id: &CollectionId) {
        if matches!(id, CollectionId::Liked | CollectionId::Playlist(_)) {
            self.engine
                .send(Command::Library(crate::engine::LibraryCommand::AddedDates(
                    id.clone(),
                )));
        }
    }

    /// Alınamayan listeyi yeniden iste.
    fn retry_collection(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        self.library
            .collections
            .insert(id.clone(), Loadable::Loading);
        self.engine.send(Command::OpenCollection(id));
        cx.notify();
    }

    /// ↑/↓: seçili satırı taşı ve görünür tut.
    pub(super) fn move_row(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.page_open() {
            return;
        }
        let Some(id) = self.library.selected.clone() else {
            return;
        };
        let list = ListKey::Collection(id.clone());
        let Some(count) = self.library.tracks(&id).map(|t| t.len()).filter(|n| *n > 0) else {
            return;
        };
        // Süzgeç açıksa yalnız görünen satırlarda gezilir; konum görünen sıradır.
        let visible = self.visible_rows(&list);
        let shown = visible.as_ref().map_or(count, |v| v.len());
        if shown == 0 {
            return;
        }
        let pos = self.library.selected_row.and_then(|r| {
            visible
                .as_ref()
                .map_or(Some(r), |v| v.iter().position(|x| *x == r))
        });
        let next_pos = match pos {
            None if delta > 0 => 0,
            None => shown - 1,
            Some(i) => (i as isize + delta).clamp(0, shown as isize - 1) as usize,
        };
        let next = visible.as_ref().map_or(next_pos, |v| v[next_pos]);
        self.library.selected_row = Some(next);
        self.select_single(list, next);
        self.library.list_glide.cancel();
        self.reveal_row(next_pos);
        self.trace_scroll("row", || format!("row={next}"));
        cx.notify();
    }

    /// Satır görünmüyorsa en az yolla görünür yap (alttaysa alt kenara,
    /// üstteyse üst kenara). Raf, başlık ve liste tek eksen üzerinden kayar
    /// (bkz. `scroll_list_by`): aşağı inerken raf açıksa önce o kapanır;
    /// yukarı çıkarken raf kapalı kalır (onu tekerlek açar). gpui'nin
    /// `scroll_to_item`'ı listeyi raftan bağımsız kaydırıyordu: raf açıkken
    /// liste kayıyor, sonraki tekerlek bütün yolu listeye verip rafı hiç
    /// kapatmıyordu. Üstelik o
    /// kaydırma ancak liste çizilirken uygulanıyordu. Liste henüz hiç
    /// çizilmediyse ona bırakılır (çizimdeki onarım değişmezi kurar).
    fn reveal_row(&mut self, ix: usize) {
        let Some((viewport, content)) = self.list_extent() else {
            self.library
                .scroll
                .scroll_to_item(ix, ScrollStrategy::Center);
            return;
        };
        let y = self.list_offset();
        let top = ix as f32 * ROW_HEIGHT;
        let target = if top < y {
            top
        } else if top + ROW_HEIGHT > y + viewport {
            top + ROW_HEIGHT - viewport
        } else {
            return;
        };
        // Aşağı giderken rafın kapandığı kadar görünen alan büyür ve liste o
        // kadar yukarı çıkar: satırın alt kenarı yine görünen alanın alt
        // kenarına denk gelir.
        let target = target.clamp(0., (content - viewport).max(0.));
        self.scroll_list_by(target - y);
    }

    /// Enter: seçili satırı çal.
    pub(super) fn play_selected_row(&mut self, cx: &mut Context<Self>) {
        if self.page_open() {
            return;
        }
        if let Some(i) = self.library.selected_row {
            self.play_row(i, cx);
        }
    }

    /// Başlıktaki düğme: listeyi baştan çal (çalıyorsa duraklat/sürdür).
    pub(super) fn play_collection(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected.clone() else {
            return;
        };
        if self.state.queue.context.as_ref() == Some(&id) && self.state.track.is_some() {
            self.toggle_play(cx);
        } else if self.library.tracks(&id).is_some_and(|t| !t.is_empty()) {
            self.play_from_start(cx);
        }
    }

    /// Hiçbir şey çalmıyorken Boşluk / çal düğmesiyle başlatılabilecek
    /// açık liste (yüklenmiş ve boş değilse): adı.
    pub(super) fn startable_collection(&self) -> Option<String> {
        if self.state.track.is_some() || self.state.pending.is_some() {
            return None;
        }
        let id = self.library.selected.as_ref()?;
        self.library.tracks(id).filter(|t| !t.is_empty())?;
        Some(self.library.title(id))
    }

    /// Geliştirme denemesi: açık listenin ilk şarkısını çalar
    /// (bkz. `OPENIFY_TEST_PLAY_AFTER`).
    pub(super) fn test_play(&mut self, cx: &mut Context<Self>) {
        // OPENIFY_TEST_PLAY_ROW ile başka bir satır (denemelerde her seferinde
        // aynı şarkının anahtarı istenmesin).
        let row = std::env::var("OPENIFY_TEST_PLAY_ROW")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        self.play_row(row, cx);
    }

    /// Açık listenin `index`'inci şarkısından çalmaya başla. Çalmayı
    /// başlatmanın bütün yolları (satır, başlıktaki düğme, çalar hapı ve
    /// paletteki "Çal") buradan geçer: liste "son çalınan" olarak kaydedilir
    /// (raf bu sırayla dizilir).
    pub(super) fn play_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected.clone() else {
            return;
        };
        if !self.ready_to_play(cx) {
            return;
        }
        self.library.selected_row = Some(index);
        if !self.is_selected(&ListKey::Collection(id.clone()), index) {
            self.select_single(ListKey::Collection(id.clone()), index);
        }
        self.state.notice = None;
        self.state.pending = self
            .library
            .tracks(&id)
            .and_then(|t| t.get(index))
            .map(|t| super::player::PendingTrack {
                name: t.name.clone(),
                image: t.image.clone(),
            });
        self.note_played(&id);
        self.engine.send(Command::PlayCollection { id, index });
        cx.notify();
    }

    // ── Şarkı listesi ───────────────────────────────────────────────────

    /// Rafın ve başlığın ne kadar kapandığı, 0 (açık) – 1 (şerit).
    pub(super) fn collapse_progress(&self) -> f32 {
        // Kütüphane ızgarasında raf hep açık.
        if self.all.open {
            return 0.;
        }
        (self.library.header_collapse / COLLAPSE_MAX).clamp(0., 1.)
    }

    /// Şarkı listesinin görünen yüksekliği ve içeriğin boyu (px). Görünen
    /// yükseklik listenin son yerleşiminden (değişirse liste kendini bir
    /// kare sonra yeniden çizer, bkz. `render_track_list`); içerik şarkı
    /// sayısından (+ hapın altında kalmasın diye pay), böylece liste
    /// değişince hemen doğru. Liste hiç çizilmediyse `None`.
    pub(super) fn list_extent(&self) -> Option<(f32, f32)> {
        let viewport = f32::from(self.library.scroll.0.borrow().last_item_size?.item.height);
        Some((
            viewport,
            self.shown_rows() as f32 * ROW_HEIGHT + PILL_LIST_END,
        ))
    }

    /// Açık listede görünen satır sayısı (süzgeç açıksa eşleşenler).
    fn shown_rows(&self) -> usize {
        let Some(id) = self.library.selected.as_ref() else {
            return 0;
        };
        match self.visible_rows(&ListKey::Collection(id.clone())) {
            Some(v) => v.len(),
            None => self.library.tracks(id).map_or(0, |t| t.len()),
        }
    }

    /// Listenin en üstten ne kadar aşağıda olduğu (px, 0 = en üst).
    pub(super) fn list_offset(&self) -> f32 {
        -f32::from(self.library.scroll.0.borrow().base_handle.offset().y)
    }

    /// Kaydırma izine bir olay satırı (bkz. `scroll_trace`).
    pub(super) fn trace_scroll(&self, event: &str, detail: impl FnOnce() -> String) {
        if scroll_trace::enabled() {
            scroll_trace::event(
                event,
                self.library.header_collapse.min(COLLAPSE_MAX),
                self.list_offset(),
                detail,
            );
        }
    }

    /// Raf, başlık ve listenin birlikte ne kadar kaydığı (px): tekerlek
    /// hareketi bunu izler.
    pub(super) fn list_position(&self) -> f32 {
        let collapse = self.library.header_collapse.min(COLLAPSE_MAX);
        collapse - f32::from(self.library.scroll.0.borrow().base_handle.offset().y)
    }

    /// Raf, başlık ve listeyi tek bir hareket gibi `d` px kaydır (+ aşağı);
    /// gerçekten alınan yolu döndürür. Aşağı: liste en üstteyken önce raf
    /// şeride iner ve başlık küçülür, kalan yol listeye geçer. Yukarı: önce
    /// liste kayar, en üste gelince raf ve başlık açılır.
    pub(super) fn scroll_list_by(&mut self, d: f32) -> f32 {
        let from_collapse = self.library.header_collapse.min(COLLAPSE_MAX);
        let max_y = self
            .list_extent()
            .map_or(0., |(viewport, content)| (content - viewport).max(0.));
        let handle = self.library.scroll.0.borrow().base_handle.clone();
        let offset = handle.offset();
        // En üst konum tam 0 değil, eksi tarafta çok küçük bir değer
        // olabiliyor ("-0").
        let from_y = (-f32::from(offset.y)).max(0.);
        // Raf, başlık ve liste tek bir eksen: konum = kapanma + liste
        // konumu. Yeni konumun ilk `COLLAPSE_MAX`'ı kapanmaya, kalanı listeye
        // düşer. Böylece değişmez (raf tam kapanmadan liste en üstte)
        // yapının kendisinden gelir: başlangıç durumu bozuk olsa bile
        // (bkz. `heal_list_collapse`) ilk adım onu düzeltir, satırlar
        // ekranda yerinden oynamaz.
        let (collapse, y) = move_axis(from_collapse, from_y, d, max_y);
        if collapse != from_collapse {
            self.library.header_collapse = collapse;
        }
        if y != from_y {
            handle.set_offset(gpui::point(offset.x, px(-y)));
        }
        let took = (collapse + y) - (from_collapse + from_y);
        self.trace_scroll("step", || {
            format!("d={d:.2} took={took:.2} from_c={from_collapse:.2} from_y={from_y:.2} raw_y={:.3} max_y={max_y:.1}", f32::from(offset.y))
        });
        took
    }

    /// Değişmezi onarır: raf ve başlık tam kapanmamışken liste en üstte
    /// olmalı. Listeyi tekerlek dışında bir şey kaydırdıysa (klavyeyle satıra
    /// gitme, kaydırma çubuğu, gpui'nin sınıra kırpması) liste konumu
    /// kapanmaya aktarılır. Toplam konum aynı kalır: raf ne kadar kapanırsa
    /// liste o kadar yukarı çıkar, satırlar ekranda yerinden oynamaz (yalnız
    /// raf ve başlık küçülür). Kök çizilirken, raf ve liste çizilmeden önce
    /// çağrılır; bir şey değiştiyse `true`.
    pub(super) fn heal_list_collapse(&mut self) -> bool {
        let collapse = self.library.header_collapse.min(COLLAPSE_MAX);
        let y = self.list_offset();
        let Some((to_collapse, to_y)) = fold_axis(collapse, y) else {
            return false;
        };
        self.library.header_collapse = to_collapse;
        let handle = self.library.scroll.0.borrow().base_handle.clone();
        let x = handle.offset().x;
        handle.set_offset(gpui::point(x, px(-to_y)));
        self.trace_scroll("heal", || format!("from_c={collapse:.2} from_y={y:.2}"));
        true
    }

    /// Tekerleğin altındaki sayfaya gitmemesi gereken, konumdan bağımsız
    /// durumlar: palet (kendi listesi kayar) ve tam ekran (kendi sözleri
    /// kayar) tekerleği yakalamadan altına bırakır (`Some(false)`); sağ tık
    /// menüsü / ad kutusu açıkken tekerlek orada biter (`Some(true)`: arkadaki
    /// sayfa kaymaz, menü kapanır). Konuma bağlı örtüler (sıra / söz paneli,
    /// eşitleme paneli, bildirim) `smooth_scroll::wheel_zone`'un isabet
    /// kuralıyla elenir. `None`: tekerlek sayfanın.
    pub(super) fn wheel_gate(&mut self, cx: &mut Context<Self>) -> Option<bool> {
        if self.palette.open || self.now.full.open {
            return Some(false);
        }
        if self.settings_wheel(cx) {
            return Some(true);
        }
        if self.manage.modal() {
            self.close_menu(cx);
            return Some(true);
        }
        None
    }

    /// Liste alanının (ya da rafın) üstünde tekerlek. İşlendiyse `true`.
    pub(super) fn wheel_list(
        &mut self,
        delta: ScrollDelta,
        line_height: Pixels,
        viewport: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(handled) = self.wheel_gate(cx) {
            return handled;
        }
        let now = Instant::now();
        let glide = self.library.list_glide.clone();
        let wheel = glide.classify(delta, line_height, viewport, now);
        self.trace_scroll("wheel", || {
            format!(
                "{wheel:?} viewport={viewport:.1} line={:.2}",
                f32::from(line_height)
            )
        });
        match wheel {
            Some(Wheel::Glide(d)) => glide.push(d, viewport, now),
            Some(Wheel::Now(d)) => {
                glide.cancel();
                self.scroll_list_by(d);
            }
            None => return false,
        }
        true
    }

    /// Tekerlek hareketinin bu karelik adımı (kök çizilirken, raf ve liste
    /// çizilmeden önce: üçü aynı karede aynı konumu okusun). Hareket
    /// sürüyorsa `true`: bir sonraki kare istenmeli.
    pub(super) fn tick_list_glide(&mut self) -> bool {
        let glide = self.library.list_glide.clone();
        if !glide.is_active() {
            return false;
        }
        let position = self.list_position();
        glide.tick(Instant::now(), position, |d| self.scroll_list_by(d))
    }

    pub(super) fn render_collection(
        &self,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // `Route::Collection` hep `show_collection` ile açılır, o da `selected`i
        // koyar; yani burada seçili liste vardır. Kütüphane yok / bağlantı yok
        // durumları `welcome`ın tam sayfa durumunda (Ana sayfa, Kütüphane).
        let Some(id) = self.library.selected.clone() else {
            return div().flex_1().into_any_element();
        };

        let title = self.library.title(&id);
        let item = self.library.item(&id);
        let owner = item.map(|i| i.subtitle.clone()).unwrap_or_default();
        // Meta satırı: önce kimin (sanatçı / liste sahibi) kalın, sonra tür,
        // şarkı sayısı ve süre soluk. Başlığın üstüne ayrı etiket konmaz.
        let (lead, kind): (SharedString, Option<&str>) = match &id {
            CollectionId::Liked => (t!(LibraryYours).into(), None),
            CollectionId::Album(_) => (owner.clone().into(), Some(t!(Album))),
            CollectionId::Playlist(_) if crate::i18n::canon::is_spotify(&owner) => {
                ("Spotify".into(), None)
            }
            CollectionId::Playlist(_) if crate::i18n::canon::is_followed(&owner) => {
                (t!(LibraryFollowedList).into(), None)
            }
            CollectionId::Playlist(_) => (t!(LibraryYourPlaylist).into(), None),
            // Sanatçı kartı bu sayfada açılmaz (`select_collection` sanatçı sayfasına yollar).
            CollectionId::Artist(_) => (t!(Artist).into(), None),
        };

        let (rest, body): (SharedString, AnyElement) = match self.library.collections.get(&id) {
            // Yüklenirken satırların yerini iskelet satırlar tutar.
            None | Some(Loadable::Loading) => (
                t!(Loading).into(),
                skeleton_rows_at(
                    matches!(id, CollectionId::Album(_)),
                    page_margin(self.window_width),
                ),
            ),
            Some(Loadable::Failed(e)) => {
                let (icon_name, title, message) = explain_error(e);
                let retry_id = id.clone();
                // Erişim yoksa yeniden denemek bir şey değiştirmez: çıkış yolu
                // beğenilen şarkılar.
                let forbidden = is_forbidden(e);
                let action =
                    if forbidden {
                        secondary_button(
                            "collection-back",
                            "heart-fill",
                            t!(BackToLiked),
                            Look::Live,
                            self.accent().text,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_collection(CollectionId::Liked, cx)
                        }))
                    } else {
                        secondary_button(
                            "collection-retry",
                            "refresh-cw",
                            t!(Retry),
                            Look::Live,
                            self.accent().text,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.retry_collection(retry_id.clone(), cx)
                        }))
                    };
                (
                    t!(LibraryLoadFailed).into(),
                    centered(state_view(icon_name, title, message).child(action)),
                )
            }
            Some(Loadable::Loaded(tracks)) if tracks.is_empty() => (
                tp!(Songs, 0).into(),
                centered(state_view(
                    "list-music",
                    t!(EmptyListTitle),
                    t!(EmptyListBody),
                )),
            ),
            Some(Loadable::Loaded(tracks)) => {
                let total_ms: u64 = tracks.iter().map(|t| t.duration_ms as u64).sum();
                let mut parts: Vec<String> = kind.map(str::to_string).into_iter().collect();
                parts.push(tp!(Songs, tracks.len()));
                parts.push(fmt_total(total_ms));
                let visible = self.visible_rows(&ListKey::Collection(id.clone()));
                let body = match &visible {
                    Some(v) if v.is_empty() => {
                        centered(state_view("search", t!(NoMatchTitle), t!(NoMatchBody)))
                    }
                    _ => self.render_track_list(&id, visible, cx),
                };
                (parts.join(" · ").into(), body)
            }
        };

        // Liste başlığı tek satır: ad, meta, sağda Çal / Duraklat. Büyük kapak
        // yok (kapak rafta). Raf şeride inerken başlık da küçülür: yarı
        // yoldan sonra ad 28 px'ten 18 px'e, düğme 40 px'ten 32 px'e iner.
        let progress = self.collapse_progress();
        let compact = progress >= COMPACT_AT;
        let accent = self.accent().text;
        let playing_here =
            self.state.queue.context.as_ref() == Some(&id) && self.state.is_playing();
        let play = primary_button_sized(
            "collection-play",
            if playing_here {
                "pause-fill"
            } else {
                "play-fill"
            },
            if playing_here { t!(Pause) } else { t!(Play) },
            self.look(&self.nav.collection_play, window, true),
            accent,
            compact,
        )
        .track_focus(&self.nav.collection_play)
        .on_click(cx.listener(|this, _, _, cx| this.play_collection(cx)));

        // Albümün sanatçıları tıklanabilir (albüm sayfasındaki adreslerle).
        let lead_el: AnyElement = match (&id, self.album_artists(&id)) {
            (CollectionId::Album(_), Some(artists)) => {
                let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
                let uris: Rc<Vec<String>> =
                    Rc::new(artists.iter().map(|a| a.uri.clone()).collect());
                let (this, u) = (cx.entity().downgrade(), uris.clone());
                let on_click: super::link_text::SpanHandler = Rc::new(move |i, _, cx| {
                    if let Some(uri) = u.get(i).cloned() {
                        let _ = this.update(cx, |shell, cx| {
                            shell.navigate(super::history::Route::Artist(uri), cx)
                        });
                    }
                });
                let (this, u) = (cx.entity().downgrade(), uris);
                let on_hover: super::link_text::SpanHandler = Rc::new(move |i, _, cx| {
                    if let Some(uri) = u.get(i).cloned() {
                        let _ = this.update(cx, |shell, _| shell.prefetch(uri));
                    }
                });
                super::link_text::link_line("meta-lead", &names, on_click, Some(on_hover))
                    .flex()
                    .min_w_0()
                    .into_any_element()
            }
            _ => line_with_tip("meta-lead", lead)
                .flex()
                .min_w_0()
                .into_any_element(),
        };
        // Meta: öndeki ad (sahip / sanatçı) ön plan renginde, gerisi ikincil
        // renkte. Açık başlıkta adın ALTINDA, Body rolünde ayrı satır; şerit
        // hâlindeki küçük başlıkta tek satır, ad ve meta yan yana (Secondary).
        let meta_role = if compact { T_SECONDARY } else { T_BODY };
        let meta = div()
            .flex()
            .items_center()
            .min_w_0()
            .type_role(meta_role)
            // Yüzdeli üst sınır (max_w relative) burada kullanılmaz: satırın
            // eni ölçülürken kap henüz belirsiz, yüzde 0'a çözülüp yazıyı
            // "…"ya çökertiyordu. Doğal eninde durur, yer daralınca küçülür.
            // Yazı kutusu flex: tek satırlık yazı (`Line`) blok kutuda en
            // dar ölçüsüyle (0) ölçülüp geniş alanda bile "…" çiziyordu; flex
            // kutuda doğal eniyle ölçülür, yer daralınca kısalır.
            .child(
                div()
                    .flex()
                    .flex_shrink()
                    .min_w_0()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(lead_el),
            )
            // Dar pencerede kısalır: tamamı ipucunda (ayıraç ipucuna girmez).
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .text_color(muted())
                    .child(div().flex_none().child(" · "))
                    .child(
                        line_with_tip("meta-rest", rest.to_string())
                            .flex()
                            .min_w_0(),
                    ),
            );
        let title_el = div()
            .flex()
            .min_w_0()
            .font_family(font())
            .type_role(if compact { T_TITLE } else { T_HEADLINE })
            .font_weight(FontWeight::EXTRA_BOLD)
            .text_color(text())
            .child(line_with_tip("page-title", title.clone()).flex().min_w_0());

        // İçerik satırlarla aynı enle: Çal düğmesi süre sütununun sağ
        // kenarında (sağ sayfa kenarı) durur. Üstteki çizgi rafı sayfadan
        // ayırır, o yüzden içerik alanı boyu kalır. Açıkken sol blok iki
        // satır (ad, altında meta), düğmeler bloğa göre dikey ortada;
        // küçülünce tek satır.
        let identity = if compact {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(S4))
                .child(div().flex().flex_shrink().min_w_0().child(title_el))
                .child(div().flex_1().min_w_0().flex().child(meta))
        } else {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(title_el)
                .child(meta)
        };
        let content = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .gap(px(S4))
            .child(identity)
            .children(self.collection_actions(&id, &title, compact, window, cx))
            .child(play)
            // İki boy arasında geçerken yeni hâl belirerek gelir.
            .with_animation(
                if compact {
                    "header-compact"
                } else {
                    "header-open"
                },
                Animation::new(motion()).with_easing(ease_out_quint()),
                |el, d| el.opacity(d),
            );
        let header = div()
            .flex_shrink_0()
            .h(px(header_height(progress)))
            .px(px(page_margin(self.window_width)))
            .border_t_1()
            .border_color(veil(LINE))
            .child(content);

        // Fare tekerleği gpui'nin kendi kaydırmasından önce burada yakalanır:
        // raf, başlık ve liste tek bir hareket gibi kayar (bkz.
        // `scroll_list_by`), çentikler yumuşak (bkz. `smooth_scroll`). Yalnız
        // liste görünümü yenilenir (kök onunla birlikte çizilir, rafın boyu
        // orada hesaplanır).
        let this = cx.entity().downgrade();
        let wheel = super::smooth_scroll::wheel_zone(move |ev, bounds, view, window, cx| {
            let line_height = window.line_height();
            let viewport = f32::from(bounds.size.height);
            let handled = this
                .update(cx, |shell, cx| {
                    shell.wheel_list(ev.delta, line_height, viewport, cx)
                })
                .unwrap_or(false);
            if handled {
                cx.notify(view);
            }
            handled
        });

        // Liste değişince başlık ve şarkılar belirerek, hafifçe kayarak gelir
        // (sayfalarla aynı geçiş, `pages::page_enter`).
        let page = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .relative()
            .child(header)
            .children(self.render_find_bar(cx))
            .child(column_header(
                matches!(id, CollectionId::Album(_)),
                self.shows_added_column(&id),
                page_margin(self.window_width),
            ))
            .child(body)
            .child(wheel);
        super::pages::page_enter(ElementId::Name(format!("page-{}", id.key()).into()), page)
    }

    fn render_track_list(
        &self,
        id: &CollectionId,
        visible: Option<Arc<Vec<usize>>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.clone();
        let count = visible.as_ref().map_or_else(
            || self.library.tracks(&id).map_or(0, |t| t.len()),
            |v| v.len(),
        );
        // Albümde her satır aynı kapağı ve albüm adını tekrarlardı: ikisi de
        // gizlenir.
        let album = matches!(id, CollectionId::Album(_));
        // Kendi listende satırlar sürüklenerek sıralanır (süzgeç yokken).
        let reorder =
            visible.is_none() && self.library.item(&id).is_some_and(super::manage::is_own);
        let album_artists = album
            .then(|| self.library.item(&id).map(|i| i.subtitle.clone()))
            .flatten();
        let show_added = self.shows_added_column(&id);
        // Satırlar yüzen hapın altından geçer: liste pencerenin altına kadar
        // (ve hap payı kadar ötesine) uzanır, altındaki iç boşluk (`pb`) son
        // satırın hapın üstünde durabilmesini sağlar. gpui listesi alttaki iç
        // boşluğu görünen alandan düşüyor; liste o yüzden pay kadar uzun:
        // görünen alan pencerenin altına denk gelir, satırlar oraya kadar
        // çizilir.
        let list = uniform_list(
            "tracks",
            count,
            cx.processor(
                move |this: &mut Shell, range: std::ops::Range<usize>, _, cx| {
                    let Some(tracks) = this.library.tracks(&id).cloned() else {
                        return Vec::new();
                    };
                    // Çalan satır: sıradaki yeri ve şarkısı tutuyorsa orası; liste
                    // bu arada değiştiyse (satır çıkarıldı, taşındı) şarkının yeni yeri.
                    let current = this.state.track.as_ref().map(|t| t.uri.as_str());
                    let playing_index = (this.state.queue.context.as_ref() == Some(&id))
                        .then(|| {
                            let i = this.state.queue.index;
                            match current {
                                Some(uri) if tracks.get(i).is_none_or(|t| t.uri != uri) => {
                                    tracks.iter().position(|t| t.uri == uri)
                                }
                                _ => Some(i),
                            }
                        })
                        .flatten();
                    let list = ListKey::Collection(id.clone());
                    let dates = show_added
                        .then(|| this.library.added.get(&id).cloned())
                        .flatten();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64);
                    let rows = range
                        .filter_map(|pos| {
                            let ix = match &visible {
                                Some(v) => *v.get(pos)?,
                                None => pos,
                            };
                            let track = tracks.get(ix)?;
                            let spec = RowSpec {
                                list: list.clone(),
                                ix,
                                show_cover: !album,
                                show_album: !album,
                                hide_artist: album_artists
                                    .as_deref()
                                    .is_some_and(|a| same_artists(a, &track.artists)),
                                artist_column: false,
                                playing: playing_index == Some(ix),
                                selected: this.is_selected(&list, ix),
                                look: Look::Live,
                                reorder,
                                added: show_added.then(|| {
                                    dates
                                        .as_ref()
                                        .and_then(|d| d.get(&track.uri))
                                        .map(|at| super::added_date::added_label(now, *at))
                                        .unwrap_or_default()
                                        .into()
                                }),
                                plays: None,
                            };
                            Some(this.song_row(spec, track, cx))
                        })
                        .collect();
                    // Satırlar çizilirken istenen küçük kapaklar.
                    this.flush_image_requests(cx);
                    rows
                },
            ),
        )
        .track_scroll(self.library.scroll.clone())
        // Boyu içerikten değil, iki kenarından (üst ve alt) gelsin.
        .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .bottom(px(-PILL_LIST_END))
        .pb(px(PILL_LIST_END))
        .into_any_element();
        // Satırların göründüğü alan: ekolayzer katmanı bunun dışına taşmasın.
        // Ölçü listeden sonra alınır (liste o sırada yerleşmiş olur): kaydırma
        // çubuğu bu çizimde eski bir boyla ya da konumla çizildiyse (pencere
        // boyu değişti, raf kapandı, klavyeyle satıra gidildi) görünüm bir
        // kare sonra kendini yeniden çizer.
        let drawn = self
            .scroll_metrics(super::ScrollBar::List)
            .map(|m| (m.viewport, m.offset));
        let viewport = self.list_viewport.clone();
        let scroll = self.library.scroll.clone();
        let drawn_collapse = self.library.header_collapse.min(COLLAPSE_MAX);
        let measure = gpui::canvas(
            move |bounds, window, _| {
                viewport.set(Some(bounds));
                let state = scroll.0.borrow();
                let laid_out = state.last_item_size.map(|size| {
                    (
                        f32::from(size.item.height),
                        -f32::from(state.base_handle.offset().y),
                    )
                });
                if let Some((lv, lo)) = laid_out {
                    scroll_trace::state(drawn_collapse, lo, || format!("layout viewport={lv:.1}"));
                }
                let stale = match (drawn, laid_out) {
                    (Some((v, o)), Some((lv, lo))) => (v - lv).abs() > 0.5 || (o - lo).abs() > 0.5,
                    (None, Some(_)) => true,
                    _ => false,
                };
                if stale {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        // Köşesi açıkça verilmeli: yoksa mutlak öğe akıştaki yerine, listenin
        // altına konuyor ve ekolayzer bütünüyle kırpılıyordu.
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .relative()
            .group("list-scroll")
            .flex_1()
            .min_h_0()
            .child(list)
            .child(measure)
            .children(self.scrollbar(super::ScrollBar::List, cx))
            .into_any_element()
    }

    /// Vitrin için eski imza: beğenilenlerde `ix`. satır.
    pub(super) fn track_row(
        &self,
        ix: usize,
        track: &TrackRow,
        playing: bool,
        selected: bool,
        album: bool,
        look: Look,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spec = RowSpec {
            list: ListKey::Collection(CollectionId::Liked),
            ix,
            show_cover: !album,
            show_album: !album,
            hide_artist: false,
            artist_column: false,
            playing,
            selected,
            look,
            reorder: false,
            added: None,
            plays: None,
        };
        self.song_row(spec, track, cx)
    }

    /// Şarkı satırı; kütüphane listesinde de sayfalarda da aynı. Tek tık
    /// seçer (Ctrl ekler, Shift aralık), çift tık ve ▶ çalar, sağ tık menü
    /// açar; sanatçı ve albüm adları tıklanabilir; kalp beğenir; kendi
    /// listende satır sürüklenip başka bir satırın yerine bırakılır.
    pub(super) fn song_row(
        &self,
        spec: RowSpec,
        track: &TrackRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RowSpec {
            list,
            ix,
            show_cover,
            show_album,
            hide_artist,
            artist_column,
            playing,
            selected,
            look,
            reorder,
            added,
            plays,
        } = spec;
        let accent = self.accent().text;
        // Numara sütunundaki her şey (numara, çal simgesi, ekolayzer, "#")
        // 32 px'lik hücrenin ortasında: üzerine gelince beliren ▶, yerini
        // aldığı numarayla ve kendi düğme zeminiyle aynı merkezde. (Sağa
        // yaslıyken ▶ düğme zemininin sağ kenarına yapışıyordu.)
        let number: AnyElement = if playing && !super::gallery::enabled() {
            // Ekolayzeri listenin üstündeki katman çizer (her karede, liste
            // yeniden kurulmadan). Burada yalnız hücrenin ekrandaki yeri
            // ölçülür; katman aynı karede listeden sonra geldiği için yeni
            // yeri okur.
            let cell = self.playing_cell.clone();
            gpui::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
                .absolute()
                .size_full()
                .into_any_element()
        } else if playing {
            // Vitrinde birden çok "çalan" satır var: orada yerinde çizilir.
            equalizer(accent, self.state.is_playing())
        } else {
            div()
                .tabular()
                .group_hover(ROW_GROUP, |s| s.opacity(0.))
                .when(look == Look::Hover, |el| el.opacity(0.))
                .child((ix + 1).to_string())
                .into_any_element()
        };
        // Üzerine gelince numaranın yerine çal simgesi.
        let play_icon = (!playing).then(|| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .opacity(if look == Look::Hover { 1. } else { 0. })
                .group_hover(ROW_GROUP, |s| s.opacity(1.))
                .child(icon("play-fill", 14., text()))
        });
        let title_color = if playing { rgb(accent) } else { text() };
        let liked = self.is_liked(&track.uri);
        // Tıklanabilen adlar ve beğenilmemiş şarkının kalbi yalnız fare
        // üzerindeki satırda kurulur: her satırda kurulunca kaydırırken kare
        // ~0,9 ms (%17) pahalanıyordu (`OPENIFY_PROF_HIDE=rowx` ile ölçüldü).
        // Diğer satırlar düz yazı; görünüş aynı.
        let hot = look == Look::Hover
            || self
                .manage
                .hover
                .as_ref()
                .is_some_and(|(l, i)| *i == ix && *l == list);
        self.want_links(&track.uri);
        // Yerel olarak gizlenen şarkının satırı soluk görünür (bkz. `listening`).
        let hidden = self.row_hidden(&list, &track.uri);

        let row_id = ElementId::Name(format!("row-{ix}").into());
        let row = div()
            .id(row_id)
            .group(ROW_GROUP)
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(ROW_INSET))
            .rounded(px(R_MD))
            .flex()
            .items_center()
            .gap(px(S4))
            .type_role(T_SECONDARY)
            .text_color(muted());
        let click_list = list.clone();
        let menu_list = list.clone();
        let play_list = list.clone();
        let hover_list = list.clone();
        let row = row_states(row, look, selected, playing.then_some(accent))
            .when(hidden, |el| el.opacity(Self::hidden_opacity()))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.hover_row(hover_list.clone(), ix, *hovered, cx)
            }))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.click_row(
                    click_list.clone(),
                    ix,
                    event.modifiers(),
                    event.click_count(),
                    cx,
                );
            }))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_row_menu(menu_list.clone(), ix, ev.position, cx);
                }),
            )
            .child(
                div()
                    .id(("play-cell", ix))
                    .relative()
                    .flex_none()
                    .size(px(NUMBER_CELL))
                    .rounded(px(R_SM))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!playing, |el| {
                        el.hover(|s| s.bg(veil(VEIL_HOVER))).on_click(cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.play_in(&play_list, ix, cx);
                            },
                        ))
                    })
                    .child(number)
                    .children(play_icon),
            )
            .when(show_cover, |el| {
                el.child(cover_image(
                    &self.images,
                    &track.image,
                    &track.album,
                    ROW_THUMB,
                    R_SM,
                    "music",
                ))
            })
            .child(
                title_column()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(T_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(title_color)
                            .child(line_with_tip(("t-title", ix), track.name.clone())),
                    )
                    // Albümde sanatçı her satırda aynıysa tekrar yazılmaz
                    // (yalnızca farklıysa, ör. "feat.", görünür).
                    .when(!hide_artist, |el| {
                        // Ölçüm kancası: OPENIFY_PROF_HIDE="links" eski düz yazı.
                        if !hot || super::frames::hidden("links") || super::frames::hidden("rowx") {
                            el.child(line_with_tip(("t-artists", ix), track.artists.clone()))
                        } else {
                            el.child(div().flex().child(self.artist_links(
                                ("t-artists", ix).into(),
                                track,
                                cx,
                            )))
                        }
                    }),
            )
            .when(artist_column, |el| {
                if !hot || super::frames::hidden("links") || super::frames::hidden("rowx") {
                    el.child(
                        column(ALBUM_COLUMN)
                            .child(line_with_tip(("t-artists-col", ix), track.artists.clone())),
                    )
                } else {
                    el.child(column(ALBUM_COLUMN).flex().child(self.artist_links(
                        ("t-artists-col", ix).into(),
                        track,
                        cx,
                    )))
                }
            })
            .when(show_album, |el| {
                if !hot || super::frames::hidden("links") || super::frames::hidden("rowx") {
                    el.child(
                        column(ALBUM_COLUMN)
                            .child(line_with_tip(("t-album", ix), track.album.clone())),
                    )
                } else {
                    el.child(column(ALBUM_COLUMN).flex().child(self.album_link(
                        ("t-album", ix).into(),
                        track,
                        cx,
                    )))
                }
            })
            // Eklenme tarihi (listeler ve Beğenilenler; dar pencerede sütun yok).
            .when_some(added, |el, when| {
                el.child(
                    div()
                        .w(px(ADDED_COLUMN))
                        .flex_none()
                        .flex()
                        .child(line_with_tip(("t-added", ix), when).flex().min_w_0()),
                )
            })
            // Dinlenme sayısı (albüm ve sanatçı sayfaları; dar pencerede sütun yok).
            .when_some(plays, |el, n| {
                el.child(
                    div()
                        .w(px(PLAYS_COLUMN))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .type_role(T_LABEL)
                        .text_color(label())
                        .tabular()
                        .child(crate::i18n::num(n)),
                )
            })
            // Beğenilenlerde her satır beğenili: kalp yalnız üzerine gelince.
            .child(
                if !(hot || liked && list != ListKey::Collection(CollectionId::Liked))
                    || super::frames::hidden("hearts")
                    || super::frames::hidden("rowx")
                {
                    div().w(px(NUMBER_CELL)).flex_none().into_any_element()
                } else {
                    self.heart_cell(
                        ix,
                        track,
                        liked,
                        if hot { Look::Hover } else { look },
                        list == ListKey::Collection(CollectionId::Liked),
                        cx,
                    )
                },
            )
            .child(
                div()
                    .w(px(48.))
                    .flex()
                    .justify_end()
                    .tabular()
                    .child(fmt_time(track.duration_ms)),
            );

        // Kendi listende sürükleyip bırakarak sıralama. Bırakılacak yer
        // önceden görünür: aşağı taşırken satırın altında, yukarı taşırken
        // üstünde kapak ışığı renginde çizgi.
        let row = match (&list, reorder) {
            (ListKey::Collection(id), true) => {
                let from_selection = self.is_selected(&list, ix) && self.manage.sel.rows.len() > 1;
                // Seçim bitişikse bütün blok taşınır, değilse yalnız bu satır.
                let (from, len) = match from_selection
                    .then(|| contiguous(&self.manage.sel.rows))
                    .flatten()
                {
                    Some((start, n)) => (start, n),
                    None => (ix, 1),
                };
                let label: SharedString = if len == 1 {
                    track.name.clone().into()
                } else {
                    tp!(Songs, len).into()
                };
                let drag = super::manage::DragRows {
                    list: id.clone(),
                    from,
                    len,
                    label,
                };
                row.on_drag(drag, move |d, _, _, cx| {
                    cx.new(|_| super::manage::DragPreview {
                        label: d.label.clone(),
                        accent,
                    })
                })
                .drag_over::<super::manage::DragRows>(move |s, d, _, _| {
                    let s = s.bg(veil(VEIL_HOVER));
                    if d.from < ix {
                        s.border_b_2().border_color(rgb(accent))
                    } else {
                        s.border_t_2().border_color(rgb(accent))
                    }
                })
                .on_drop(cx.listener(
                    move |this, d: &super::manage::DragRows, _, cx| this.drop_rows(d, ix, cx),
                ))
            }
            _ => row,
        };

        // Liste öğeleri içerik kadar daralmasın: dış kap tam genişlikte.
        div()
            .w_full()
            .px(px(page_margin(self.window_width) - ROW_INSET))
            .child(row)
            .into_any_element()
    }

    /// Satırdaki kalp: beğenilmişse hep görünür (kapak ışığı renginde, dolu),
    /// değilse yalnız satırın üzerine gelince.
    pub(super) fn heart_cell(
        &self,
        ix: usize,
        track: &TrackRow,
        liked: bool,
        look: Look,
        quiet: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let row = track.clone();
        div()
            .id(("heart", ix))
            .flex_none()
            .size(px(NUMBER_CELL))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when((!liked || quiet) && look != Look::Hover, |el| {
                el.opacity(0.).group_hover(ROW_GROUP, |s| s.opacity(1.))
            })
            .child(super::tween::hover_fill(
                veil(VEIL_HOVER),
                veil(VEIL_PRESSED),
                0.,
                super::tween::FULL,
            ))
            .cursor_pointer()
            .tooltip(super::components::Tooltip::text(
                if liked { t!(UnlikeTip) } else { t!(LikeTip) },
                None,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let on = !this.is_liked(&row.uri);
                this.set_liked(vec![row.clone()], on, cx);
            }))
            .child(icon(
                if liked { "heart-fill" } else { "heart" },
                16.,
                if liked { rgb(accent) } else { muted() },
            ))
            .into_any_element()
    }

    /// Kütüphanedeki albümün sanatçıları (albüm sayfası önbellekteyse).
    fn album_artists(&self, id: &CollectionId) -> Option<Vec<crate::engine::ArtistCard>> {
        let CollectionId::Album(uri) = id else {
            return None;
        };
        match self.pages.albums.get(uri) {
            Some(super::pages::PageLoad::Ready(p)) if !p.artists.is_empty() => {
                Some(p.artists.clone())
            }
            _ => None,
        }
    }

    /// Liste başlığındaki ikincil eylemler: kendi listende "Yeniden adlandır"
    /// ve "Diğer", albümde kalp. Küçülmüş başlıkta yalnız "Diğer".
    fn collection_actions(
        &self,
        id: &CollectionId,
        title: &str,
        compact: bool,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let _ = window;
        let accent = self.accent().text;
        let mut out = Vec::new();
        if let CollectionId::Album(uri) = id
            && !compact
        {
            let uri = uri.clone();
            out.push(
                icon_button_tinted(
                    "collection-save",
                    "heart-fill",
                    rgb(accent),
                    Look::Live,
                    accent,
                )
                .tooltip(super::components::Tooltip::text(
                    t!(RemoveFromLibrary),
                    None,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_album_saved(uri.clone(), false, None, cx)
                }))
                .into_any_element(),
            );
        }
        if *id != CollectionId::Liked {
            let target = super::manage::CardTarget::Collection {
                id: id.clone(),
                title: title.to_string(),
            };
            let (probe, anchor) = super::components::anchor_probe();
            out.push(
                icon_button("collection-more", "ellipsis", Look::Live, accent)
                    .relative()
                    .tooltip(super::components::Tooltip::text(t!(MoreTip), None))
                    .child(probe)
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.open_card_menu(
                            target.clone(),
                            super::components::menu_below(anchor.get(), ev.position()),
                            cx,
                        );
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    /// Bul (Ctrl+F) açıksa listenin üstünde süzme kutusu.
    fn render_find_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self.manage.find.as_ref()?;
        let accent = self.accent().text;
        let empty = find.query.is_empty();
        let caret = || {
            div()
                .w(px(2.))
                .h(px(T_SECONDARY.line))
                .flex_shrink_0()
                .bg(rgb(accent))
        };
        let id = self.library.selected.clone()?;
        let total = self.library.tracks(&id).map_or(0, |t| t.len());
        let shown = self
            .visible_rows(&ListKey::Collection(id))
            .map_or(total, |v| v.len());
        Some(
            div()
                .flex_shrink_0()
                .px(px(page_margin(self.window_width)))
                .pb(px(S3))
                .flex()
                .items_center()
                .gap(px(S3))
                .child(
                    div()
                        .w(px(320.))
                        .h(px(super::SEARCH_HEIGHT))
                        .px(px(S3))
                        .rounded(px(R_LG))
                        .flex()
                        .items_center()
                        .gap(px(S2))
                        .bg(shade(SHADE_INPUT))
                        .border_1()
                        .border_color(rgb(accent))
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
                                .when(empty, |el| el.child(caret()).child(t!(FindInList)))
                                .when(!empty, |el| el.child(find.query.clone()).child(caret())),
                        ),
                )
                .child(
                    div()
                        .type_role(T_LABEL)
                        .tabular()
                        .text_color(label())
                        .child(format!("{shown} / {total}")),
                )
                .child(div().flex_1())
                .child(
                    icon_button("find-close", "x", Look::Live, accent)
                        .tooltip(super::components::Tooltip::text(t!(CloseFind), Some("Esc")))
                        .on_click(cx.listener(|this, _, _, cx| this.close_find(cx))),
                )
                .into_any_element(),
        )
    }

    /// Kapak: listenin kendisi; beğenilenler için kalp; resim yoksa ton karosu.
    pub(super) fn collection_art(
        &self,
        id: &CollectionId,
        name: &str,
        image: &str,
        side: f32,
        radius: f32,
    ) -> AnyElement {
        match id {
            CollectionId::Liked => liked_art(side, radius, self.accent().text),
            CollectionId::Album(_) => {
                cover_image(&self.images, image, name, side, radius, "disc-3")
            }
            // Takip edilen sanatçı: yuvarlak portre (köşe yarıçapı yok sayılır).
            CollectionId::Artist(_) => {
                cover_image(&self.images, image, name, side, side / 2., "user")
            }
            // Kapağı olmayan liste: ilk şarkılarının dört albüm kapağından 2x2
            // mozaik (bkz. `engine::library::with_mosaics`).
            CollectionId::Playlist(_) => match crate::engine::mosaic_urls(image) {
                Some(urls) => self.mosaic_art(&urls, side, radius),
                None => cover_image(&self.images, image, name, side, radius, "list-music"),
            },
        }
    }

    /// 2x2 mozaik kapak: aralarında 2 px, yalnız dış köşeler yuvarlak (gpui'de
    /// kabın köşesi içindeki resmi kırpmıyor; her resim kendi dış köşesini
    /// yuvarlar).
    fn mosaic_art(&self, urls: &[&str], side: f32, radius: f32) -> AnyElement {
        let tile = (side - S0) / 2.;
        let cell = |url: &str, corner: usize| {
            let mut el = div()
                .relative()
                .size(px(tile))
                .flex_none()
                .overflow_hidden()
                .bg(rgb(palette().placeholder));
            el = match corner {
                0 => el.rounded_tl(px(radius)),
                1 => el.rounded_tr(px(radius)),
                2 => el.rounded_bl(px(radius)),
                _ => el.rounded_br(px(radius)),
            };
            match self.images.get(url) {
                Some(path) => {
                    let picture = img(path).size_full();
                    let picture = match corner {
                        0 => picture.rounded_tl(px(radius)),
                        1 => picture.rounded_tr(px(radius)),
                        2 => picture.rounded_bl(px(radius)),
                        _ => picture.rounded_br(px(radius)),
                    };
                    el.child(picture)
                }
                None => el,
            }
        };
        div()
            .size(px(side))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(S0))
            .children([0, 2].map(|row| {
                div()
                    .flex()
                    .gap(px(S0))
                    .child(cell(urls[row], row))
                    .child(cell(urls[row + 1], row + 1))
            }))
            .into_any_element()
    }
}

/// Şarkı satırının durumları. Normal satırda beyaz ton katmanları (üzerine
/// gelme, seçili, basma); çalan satırda aynı basamaklar kapak ışığının
/// tonuyla (bkz. `ACCENT_PLAYING`).
fn row_states(
    el: gpui::Stateful<gpui::Div>,
    look: Look,
    selected: bool,
    playing: Option<u32>,
) -> gpui::Stateful<gpui::Div> {
    // (dururken, üzerine gelince, basınca)
    let (rest, hover, pressed) = match (playing, selected) {
        (Some(a), true) => (
            Some(accent_veil(a, ACCENT_PLAYING_SELECTED)),
            accent_veil(a, ACCENT_PLAYING_SELECTED),
            accent_veil(a, ACCENT_PLAYING_PRESSED),
        ),
        (Some(a), false) => (
            Some(accent_veil(a, ACCENT_PLAYING)),
            accent_veil(a, ACCENT_PLAYING_HOVER),
            accent_veil(a, ACCENT_PLAYING_PRESSED),
        ),
        (None, true) => (
            Some(veil(VEIL_SELECTED)),
            veil(VEIL_SELECTED),
            veil(VEIL_PRESSED),
        ),
        (None, false) => (None, veil(VEIL_HOVER), veil(VEIL_PRESSED)),
    };
    let el = match rest {
        Some(bg) => el.bg(bg),
        None => el,
    };
    match look {
        // Üzerine gelme / basma rengi 140 ms'de gelir (`tween::HoverFill`,
        // satırın ilk çocuğu): zemin yerine katman, aradaki fark kadar.
        Look::Live | Look::Focus => el.cursor_pointer().child(super::tween::hover_fill(
            hover,
            pressed,
            rest.map_or(0., |c| c.a),
            R_MD,
        )),
        Look::Hover => el.bg(hover),
        Look::Pressed => el.bg(pressed),
        Look::Disabled => el,
    }
}

/// Beğenilen şarkıların kapağı: sessiz bir ton karosu üstünde kapak
/// ışığı renginde kalp. (Mor-pembe degrade Spotify'ın kendi görseliydi ve
/// kapak dışından renk getiriyordu; bkz. DESIGN.md, Tek Işık Kuralı.)
fn liked_art(side: f32, radius: f32, accent: u32) -> AnyElement {
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(radius))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(palette().placeholder))
        .child(icon("heart-fill", (side * 0.42).max(14.), rgb(accent)))
        .into_any_element()
}

/// Ekolayzer çubuğunun en kısa boyu ve üstüne eklenebilen yükseklik (px).
const EQ_MIN: f32 = 3.;
const EQ_RANGE: f32 = 11.;

/// Çalan şarkının yanındaki üç çubuk. `levels` her çubuğun yüksekliği,
/// 0 (en kısa) – 1 (en uzun). Bugün saatten hesaplanıyor (`eq_levels`);
/// gerçek ses bantlarının düzeyleri geldiğinde aynı fonksiyon onlarla
/// beslenecek.
pub(super) fn equalizer_bars(color: u32, levels: [f32; 3]) -> AnyElement {
    let bar = |level: f32| {
        div()
            .w(px(3.))
            .h(px(EQ_MIN + EQ_RANGE * level.clamp(0., 1.)))
            .rounded(px(R_XS))
            .bg(rgb(color))
    };
    div()
        .h(px(EQ_MIN + EQ_RANGE))
        .flex()
        .items_end()
        .gap(px(S0))
        .child(bar(levels[0]))
        .child(bar(levels[1]))
        .child(bar(levels[2]))
        .into_any_element()
}

/// Ekolayzerin bu anki düzeyleri (saatten): çalarken her çubuk farklı hızda,
/// birbirinden bağımsız inip kalkar; duraklatılınca merdiven gibi durur.
pub(super) fn eq_levels(playing: bool) -> [f32; 3] {
    if !playing {
        return [1. / EQ_RANGE, 4. / EQ_RANGE, 7. / EQ_RANGE];
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f32() % 1000.0)
        .unwrap_or(0.0);
    let speed = [5.3, 7.1, 6.2];
    let phase = [0.0, 1.7, 3.1];
    std::array::from_fn(|i| 0.5 + 0.5 * (t * speed[i] + phase[i]).sin())
}

/// Çalan şarkının yanında hareket eden üç çubuk (duraklatılınca durur).
///
/// Listede bunu kendi katmanı her karede yeniden çizer (bkz.
/// `views::EqualizerLayer`): pencerenin geri kalanı önceki kareden kullanılır.
pub(super) fn equalizer(color: u32, playing: bool) -> AnyElement {
    equalizer_bars(color, eq_levels(playing))
}

fn column_header(album: bool, added: bool, margin: f32) -> AnyElement {
    div()
        .mx(px(margin - ROW_INSET))
        .px(px(ROW_INSET))
        .pb(px(S2))
        .mb(px(S1))
        .flex()
        .gap(px(S4))
        .type_role(T_LABEL)
        .font_weight(FontWeight::MEDIUM)
        .text_color(label())
        .border_b_1()
        .border_color(veil(LINE))
        .child(div().w(px(NUMBER_CELL)).flex().justify_center().child("#"))
        .when(!album, |el| el.child(div().w(px(ROW_THUMB))))
        .child(title_column().child(t!(ColTitle)))
        .when(!album, |el| el.child(column(ALBUM_COLUMN).child(t!(Album))))
        .when(added, |el| {
            el.child(
                div()
                    .w(px(ADDED_COLUMN))
                    .flex_none()
                    .child(t!(ColDateAdded)),
            )
        })
        // Kalp sütunu: başlığı yok.
        .child(div().w(px(NUMBER_CELL)).flex_none())
        .child(div().w(px(48.)).flex().justify_end().child(t!(ColDuration)))
        .into_any_element()
}

/// Sütun başlıklarının yüksekliği (etiket satırı + alt boşluk + çizgi + ara;
/// yoğunluk çıtasının sınaması için).
#[cfg(test)]
pub(super) const COLUMN_HEADER_HEIGHT: f32 = T_LABEL.line + S2 + 1. + S1;

/// Şarkı ve albüm sütunları genişliği 4'e 3 paylaşır (her pencere eninde;
/// liste içerik alanının tamamını kullandığı için şarkı sütununun eski 520
/// px üst sınırı kalktı: sınır olsaydı artan bütün yer albüm sütununa gider,
/// albüm adıyla süre arasında boş bir bant kalırdı).
const TITLE_COLUMN: f32 = 4.;
/// Şarkı satırının üzerine gelme grubu (numara → çal simgesi).
const ROW_GROUP: &str = "track-row";
const ALBUM_COLUMN: f32 = 3.;
/// "Eklenme tarihi" sütununun eni (sabit; şarkı ve albüm sütunları kalanı paylaşır)
/// ve sütunun görünmesi için içeriğin en az eni: daha darda gizlenir (900 px
/// pencere, yerleşik panel açıkken ~1300 px altı).
const ADDED_COLUMN: f32 = 132.;
pub(super) const ADDED_MIN_WIDTH: f32 = 1000.;
/// Dinlenme sayısı sütununun eni (sabit, sağa yaslı) ve görünmesi için içeriğin
/// en az eni; daha darda "Eklenme tarihi" gibi gizlenir.
const PLAYS_COLUMN: f32 = 72.;
pub(super) const PLAYS_MIN_WIDTH: f32 = 1000.;

fn column(grow: f32) -> gpui::Div {
    let mut el = div().flex_basis(gpui::relative(0.)).flex_shrink().min_w_0();
    el.style().flex_grow = Some(grow);
    el
}

/// Şarkı sütunu (albüm sayfasında albüm sütunu yok, tek başına esner).
fn title_column() -> gpui::Div {
    column(TITLE_COLUMN)
}

/// Raf + başlık + liste ekseninde `d` px (+ aşağı) git: (kapanma, liste
/// konumu). Konum tek sayı (kapanma + liste konumu); ilk `COLLAPSE_MAX`'ı
/// kapanma, kalanı liste. Aşağı: önce raf kapanır, sonra liste kayar;
/// yukarı: önce liste en üste gelir, sonra raf açılır. `max_y`: listenin en
/// çok kayabileceği yol (liste zaten ötesindeyse orada kalır).
fn move_axis(collapse: f32, y: f32, d: f32, max_y: f32) -> (f32, f32) {
    let from = collapse.min(COLLAPSE_MAX) + y.max(0.);
    let to = (from + d).clamp(0., COLLAPSE_MAX + max_y.max(y));
    let collapse = to.min(COLLAPSE_MAX);
    (collapse, to - collapse)
}

/// Değişmez bozuksa (raf tam kapanmamışken liste kaymış) liste konumunu
/// kapanmaya aktar: toplam konum aynı kalır. Bozuk değilse `None`.
fn fold_axis(collapse: f32, y: f32) -> Option<(f32, f32)> {
    let collapse = collapse.min(COLLAPSE_MAX);
    if y <= 0. || collapse >= COLLAPSE_MAX {
        return None;
    }
    let fold = y.min(COLLAPSE_MAX - collapse);
    Some((collapse + fold, y - fold))
}

/// Rafın ve başlığın ne kadar kapalı olacağı (pencere boyu kısa / uzun
/// arasında geçince): `closed` ise tamamen kapalı (raf şerit hâlinde;
/// çizerken en çoğa kırpılır), yoksa açık. Kısa pencerede ve liste kaymışken
/// kapalı. Yukarı kaydırma yine açar.
pub(super) fn initial_collapse(closed: bool) -> f32 {
    if closed { f32::MAX } else { 0. }
}

/// İki sanatçı yazısı aynı kişileri mi söylüyor? ("A, B" ile "b, a " aynı.)
pub(super) fn same_artists(a: &str, b: &str) -> bool {
    let set = |s: &str| {
        let mut v: Vec<String> = s
            .split(',')
            .map(|n| n.trim().to_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        v.sort();
        v
    };
    set(a) == set(b)
}

/// Durum görünümünü liste alanının ortasına yerleştirir: alttaki pay hapın
/// kapladığı yer, böylece görünüm hapın üstünde kalan alanın ortasında durur.
pub(super) fn centered(content: gpui::Div) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .pb(px(PILL_CLEARANCE))
        .child(content)
        .into_any_element()
}

/// Liste yüklenirken: gerçek satırla aynı ölçülerde, ton katmanından
/// çubuklar. Veri gelince hiçbir şey kaymaz.
pub(super) fn skeleton_rows(album: bool) -> AnyElement {
    skeleton_rows_at(album, PAGE_PAD)
}

/// `skeleton_rows`, verilen kenar boşluğuyla (`theme::page_margin`).
pub(super) fn skeleton_rows_at(album: bool, margin: f32) -> AnyElement {
    let bar = |w: f32, h: f32| {
        div()
            .w(px(w))
            .h(px(h))
            .rounded(px(R_XS))
            .bg(veil(VEIL_HOVER))
    };
    let mut rows = div().flex().flex_col().px(px(margin - ROW_INSET));
    for i in 0..10 {
        // Genişlikler satırdan satıra değişir, gerçek bir liste gibi dursun.
        let title = [180., 120., 210., 150., 96., 170., 140., 200., 110., 160.][i];
        let artist = [90., 70., 120., 80., 110., 60., 100., 85., 75., 95.][i];
        rows = rows.child(
            div()
                .h(px(ROW_HEIGHT))
                .px(px(ROW_INSET))
                .flex()
                .items_center()
                .gap(px(S4))
                .child(
                    div()
                        .w(px(NUMBER_CELL))
                        .flex()
                        .justify_center()
                        .child(bar(12., 10.)),
                )
                .when(!album, |el| {
                    el.child(bar(ROW_THUMB, ROW_THUMB).rounded(px(R_SM)))
                })
                .child(
                    title_column()
                        .flex()
                        .flex_col()
                        .gap(px(S2))
                        .child(bar(title, 10.))
                        .when(!album, |el| el.child(bar(artist, 8.))),
                )
                .when(!album, |el| {
                    el.child(column(ALBUM_COLUMN).child(bar(artist + 30., 8.)))
                })
                .child(div().w(px(NUMBER_CELL)).flex_none())
                .child(div().w(px(48.)).flex().justify_end().child(bar(28., 8.))),
        );
    }
    rows.into_any_element()
}

/// Kaydırma izi (geliştirme aracı). `OPENIFY_SCROLL_TRACE=<dosya>` verilirse
/// rafın kapanması ve listenin konumu her olayda (tekerlek, adım, satıra
/// gitme, kaydırma çubuğu, pencere boyu, onarım) ve çizimde değiştikçe dosyaya bir satır
/// yazılır: `t_ms,event,collapse,offset,detail`. Satırlar seyrek (yalnız
/// olaylarda), her biri hemen diske gider: süreç öldürülse de iz eksiksiz.
/// Değişken yoksa hiçbir şey yapmaz.
pub(super) mod scroll_trace {
    use std::{cell::RefCell, fs::File, io::Write, time::Instant};

    struct Trace {
        origin: Instant,
        file: File,
        /// Son yazılan durum (yalnız değişince yeni `state` satırı).
        last: Option<(f32, f32)>,
    }

    thread_local! {
        static TRACE: RefCell<Option<Trace>> = RefCell::new(
            std::env::var_os("OPENIFY_SCROLL_TRACE").and_then(|path| {
                let mut file = File::create(path).ok()?;
                let _ = writeln!(file, "t_ms,event,collapse,offset,detail\n0.0,start,0,0,collapse_max={:.2}", super::COLLAPSE_MAX);
                Some(Trace { origin: Instant::now(), file, last: None })
            })
        );
    }

    pub fn enabled() -> bool {
        TRACE.with(|t| t.borrow().is_some())
    }

    fn write(t: &mut Trace, event: &str, collapse: f32, offset: f32, detail: &str) {
        let ms = t.origin.elapsed().as_secs_f64() * 1000.;
        let _ = writeln!(t.file, "{ms:.1},{event},{collapse:.2},{offset:.2},{detail}");
    }

    /// Bir olay satırı.
    pub fn event(event: &str, collapse: f32, offset: f32, detail: impl FnOnce() -> String) {
        TRACE.with(|t| {
            if let Some(t) = t.borrow_mut().as_mut() {
                write(t, event, collapse, offset, &detail());
                t.last = Some((collapse, offset));
            }
        });
    }

    /// Çizimdeki durum: yalnız değiştiyse yazılır.
    pub fn state(collapse: f32, offset: f32, detail: impl FnOnce() -> String) {
        TRACE.with(|t| {
            if let Some(t) = t.borrow_mut().as_mut() {
                let changed = t
                    .last
                    .is_none_or(|(c, o)| (c - collapse).abs() > 0.01 || (o - offset).abs() > 0.01);
                if changed {
                    write(t, "state", collapse, offset, &detail());
                    t.last = Some((collapse, offset));
                }
            }
        });
    }
}

/// "1 sa 12 dk" / "38 dk"
pub(super) fn fmt_total(ms: u64) -> String {
    let minutes = ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, m) => tf!(DurMinutes, m),
        (h, m) => tf!(DurHoursMinutes, h, m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artist_lists_compare_loosely() {
        assert!(same_artists("Ufo361, Ezhel", "ezhel,ufo361 "));
        assert!(same_artists("Güneş", "Güneş"));
        assert!(!same_artists("Ufo361", "Ufo361, Ezhel"));
    }

    #[test]
    fn formats_total_duration() {
        assert_eq!(fmt_total(38 * 60_000 + 59_000), "38 dk");
        assert_eq!(fmt_total(72 * 60_000), "1 sa 12 dk");
    }

    /// Değişmez: raf/başlık tam kapanmamışken liste en üstte.
    fn holds(collapse: f32, y: f32) -> bool {
        y <= 0. || collapse >= COLLAPSE_MAX
    }

    #[test]
    fn wheel_axis_collapses_first_then_scrolls() {
        let m = COLLAPSE_MAX;
        // Aşağı: önce raf kapanır, kalan yol listeye.
        assert_eq!(move_axis(0., 0., 234., 1000.), (m, 234. - m));
        assert_eq!(move_axis(0., 0., 10., 1000.), (10., 0.));
        // Yukarı: önce liste en üste, sonra raf açılır.
        assert_eq!(move_axis(m, 50., -80., 1000.), (m - 30., 0.));
        // Sınırlar: en üstte ve en altta yol alınmaz.
        assert_eq!(move_axis(0., 0., -50., 1000.), (0., 0.));
        assert_eq!(move_axis(m, 1000., 50., 1000.), (m, 1000.));
        // Liste hiç çizilmediyse (max_y 0) yalnız raf kapanır.
        assert_eq!(move_axis(0., 0., 234., 0.), (m, 0.));
        // Kısa pencerede kapanma "en çok" (f32::MAX) olarak saklanıyor.
        assert_eq!(move_axis(f32::MAX, 0., 20., 1000.), (m, 20.));
        // Bozuk başlangıç (raf açık, liste 0,7 px aşağıda; eski "y < 0,5"
        // sınaması bunu "en üstte değil" sayıp bütün yolu listeye veriyordu):
        // adım değişmezi kurar.
        let (c, y) = move_axis(0., 0.7, 234., 1000.);
        assert!(holds(c, y) && (c + y - 234.7).abs() < 1e-3, "{c} {y}");
        // Her başlangıçtan her adımda değişmez korunur.
        for (c0, y0) in [(0., 0.), (40., 0.), (m, 0.), (m, 300.), (10., 120.)] {
            for d in [-500., -90., -3., 0., 3., 90., 500.] {
                let (c, y) = move_axis(c0, y0, d, 1000.);
                assert!(holds(c, y), "({c0}, {y0}) + {d} -> ({c}, {y})");
            }
        }
    }

    #[test]
    fn fold_keeps_rows_in_place() {
        let m = COLLAPSE_MAX;
        // Raf açıkken liste 224 px kaymış: raf kapanır, liste o kadar yukarı.
        assert_eq!(fold_axis(0., 224.), Some((m, 224. - m)));
        assert_eq!(fold_axis(30., 20.), Some((50., 0.)));
        // Değişmez sağlamsa dokunulmaz.
        assert_eq!(fold_axis(0., 0.), None);
        assert_eq!(fold_axis(m, 300.), None);
        assert_eq!(fold_axis(f32::MAX, 300.), None);
    }

    #[test]
    fn paused_equalizer_keeps_the_old_staircase() {
        let heights = eq_levels(false).map(|l| EQ_MIN + EQ_RANGE * l);
        for (h, want) in heights.iter().zip([4., 7., 10.]) {
            assert!((h - want).abs() < 1e-4, "{heights:?}");
        }
        assert!(eq_levels(true).iter().all(|l| (0. ..=1.).contains(l)));
    }
}
