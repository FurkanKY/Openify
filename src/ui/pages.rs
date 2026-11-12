//! Sayfalar ve gezinme (Adım 12): sanatçı, albüm, ana sayfa, tam arama
//! sonuçları; geri / ileri.
//!
//! Sayfalar listenin yerinde açılır (raf gizlenir, "Tümü" gibi). Önbellekteki
//! sayfa tıklandığı karede çizilir: motorun sayfa önbelleği arayüzden de
//! okunur (`engine::cached_artist`). Önbellekte yoksa aynı ölçülerde iskelet
//! görünür (veri gelince hiçbir şey zıplamaz); bayatsa gösterilir ve arkadan
//! tazelenir.
//!
//! Bütün sayfalar aynı başlık bloğunu (kapak / portre, ad, meta, eylemler)
//! ve aynı kart ızgarasını kullanır. Izgara her pencere eninde tam sütun:
//! kart eni sütun sayısına göre esner (yarım kart yok).

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::Instant,
};

use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, MouseButton, MouseDownEvent, Pixels,
    ScrollDelta, SharedString, UniformListScrollHandle, Window, div, prelude::*, px, rgb,
};

use super::{
    ScrollBar, Shell,
    components::{
        Look, PRIMARY_HEIGHT, Tooltip, anchor_probe, chip, explain_error, icon_button,
        icon_button_tinted, menu_below, primary_button, secondary_button, state_view,
    },
    cover_image,
    history::{History, Route},
    library::{RowSpec, fmt_total, skeleton_rows_at},
    link_text::{link_line, split_names},
    manage::{CardTarget, ListKey},
    pill::PILL_CLEARANCE,
    player::PendingTrack,
    smooth_scroll::{SmoothScroll, Wheel},
    text::line_with_tip,
    theme::*,
};
use crate::engine::{
    AlbumCard, AlbumPage, ArtistCard, ArtistPage, CollectionId, Command, LibraryCommand,
    LibraryEvent, LibraryItem, RelatedList, ReleaseKind, SearchResults, TrackLinks, TrackRow,
};
use crate::{t, tf, tp};

/// Başlık bloğundaki kapak / portre.
const HEADER_ART: f32 = 160.;
const HEADER_ART_COMPACT: f32 = 128.;
/// Albüm sayfasının büyük kapağı (Apple Music gibi başlıkta kahraman).
const ALBUM_ART: f32 = 224.;
const ALBUM_ART_COMPACT: f32 = 160.;
/// Albüm listesi sayfa kenarından kenarına uzar (kütüphane listesi gibi,
/// `theme::page_margin`; 7 Kasım: 1040 px üst sınırı sağda boş alan
/// bırakıyordu). İçerik bu enden geniş olunca, albümde albüm sanatçısından
/// farklı sanatçılı şarkı varsa sanatçılar başlığın altından ayrı bir
/// sütuna geçer (ortadaki boşluk anlamlı veriyle dolar); yoksa başlık solda,
/// süre sağda.
const ALBUM_ARTIST_COLUMN_FROM: f32 = 1100.;
/// Kartın en dar hâli: sütun sayısı buna göre, sonra kartlar esner.
pub(super) const CARD_MIN: f32 = 152.;
pub(super) const CARD_GAP: f32 = S4;
/// Popüler şarkılar: önce 5, "Daha fazla" ile hepsi.
const TOP_FEW: usize = 5;
/// Tam arama sayfasında şarkılar: önce 5.
const SEARCH_FEW: usize = 5;
/// Başlık çubuğundaki gezinme düğmelerinin eni: sol boşluk + geri / ileri (ana
/// sayfa artık "Ana sayfa" sekmesi).
pub(super) const NAV_WIDTH: f32 = S5 + 2. * 36.;
/// Tam arama sıra numaraları paletinkilerle karışmasın.
const SEARCH_SEQ_BASE: u64 = 1 << 40;

pub enum PageLoad<T> {
    Loading,
    Ready(Arc<T>),
    Failed(String),
}

pub struct SearchPage {
    pub query: String,
    pub seq: u64,
    pub results: PageLoad<SearchResults>,
    /// Seçili çip (Hepsi · Şarkılar · …); yeni aramada "Hepsi".
    pub kind: super::search_page::SearchKind,
}

/// Bağı henüz bilinmeyen bir ada tıklandı: bağ gelince oraya gidilir.
#[derive(Clone)]
enum LinkTarget {
    Artist(usize),
    Album,
}

pub struct PageState {
    pub history: History,
    pub artists: HashMap<String, PageLoad<ArtistPage>>,
    pub albums: HashMap<String, PageLoad<AlbumPage>>,
    pub search: Option<SearchPage>,
    /// Sayfalardaki şarkı listeleri ("top:…", "album:…", "search:…").
    pub lists: HashMap<String, Arc<Vec<TrackRow>>>,
    pub scroll: UniformListScrollHandle,
    pub glide: Rc<SmoothScroll>,
    /// Açılmış bölümler ("<sayfa>#<bölüm>").
    pub expanded: HashSet<String>,
    /// Şarkı → albüm ve sanatçı adresleri.
    pub links: HashMap<String, TrackLinks>,
    /// Çizim sırasında bağı istenen şarkılar (çizimden sonra motora gider).
    want_links: RefCell<Vec<String>>,
    asked_links: RefCell<HashSet<String>>,
    /// Üzerine gelinince önceden istenen sayfalar (bir kez).
    prefetched: HashSet<String>,
    /// Benzer sanatçılar (sayfadan ayrı gelir; bkz. `engine::related`) ve
    /// bu oturumda benzerleri sorulmuş sanatçılar (bir kez).
    pub related: HashMap<String, Arc<RelatedList>>,
    /// Keşfet, tür sayfaları ve Ana sayfanın Spotify akışı (bkz. `explore`).
    pub explore: super::explore::ExploreUi,
    asked_related: HashSet<String>,
    /// Adresi aranan Deezer kartı (bulununca sayfasına gidilir).
    resolving: Option<String>,
    pending_link: Option<(String, LinkTarget)>,
    seq: u64,
    /// Sayfa çizilince uygulanacak kaydırma (geri / ileri).
    restore: Option<f32>,
    /// Kart menüsünden "Çal": liste gelince baştan çalınır.
    pub play_when_loaded: Option<CollectionId>,
    /// Ölçüm: sayfaya gidildiği an ve veri ilk kez çizildi mi (kayda
    /// "sayfa çizildi: N ms" düşer; çıta önbellekte bir kare, değilse < 300 ms).
    opened_at: Option<(Route, Instant)>,
}

impl PageState {
    pub fn new() -> Self {
        Self {
            history: History::new(Route::Home),
            artists: HashMap::new(),
            albums: HashMap::new(),
            search: None,
            lists: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
            glide: Rc::default(),
            expanded: HashSet::new(),
            links: HashMap::new(),
            want_links: RefCell::new(Vec::new()),
            asked_links: RefCell::new(HashSet::new()),
            prefetched: HashSet::new(),
            related: HashMap::new(),
            explore: Default::default(),
            asked_related: HashSet::new(),
            resolving: None,
            pending_link: None,
            seq: SEARCH_SEQ_BASE,
            restore: None,
            play_when_loaded: None,
            opened_at: None,
        }
    }
}

/// Kapaksız başlık bloğunun üst boşluğu (`page_header(None, ..)`).
const PLAIN_HEADER_TOP: f32 = S7;

/// Sayfa başlığının yazı rolü: dar pencerede Headline, aksi halde Display.
fn title_role(compact: bool) -> TypeRole {
    if compact { T_HEADLINE } else { T_DISPLAY }
}

/// Kapaksız başlık bloğunun dikey ölçüleri (sayfanın üstünden başlık ve
/// meta satırının üst kenarına, satır yükseklikleri, bloğun toplamı).
/// Gerçek başlık (`page_header`) ve iskelet (`page_skeleton_plain`) aynı
/// sayılardan kurulur; sınanır.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlainHeader {
    title_top: f32,
    title_h: f32,
    meta_top: f32,
    meta_h: f32,
    total: f32,
}

fn plain_header_geometry(compact: bool) -> PlainHeader {
    let title_h = title_role(compact).line;
    let title_top = PLAIN_HEADER_TOP;
    let meta_top = title_top + title_h + S2;
    let meta_h = T_BODY.line;
    PlainHeader {
        title_top,
        title_h,
        meta_top,
        meta_h,
        total: meta_top + meta_h + S6,
    }
}

/// İskelet çubuğu (yazı ya da düğme yerinde ton katmanı).
fn skeleton_bar(w: f32, h: f32) -> Div {
    div()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .rounded(px(R_XS))
        .bg(veil(VEIL_HOVER))
}

/// Yerel saat (selamlama için); Windows dışında öğle sayılır.
pub(super) fn local_hour() -> u16 {
    #[cfg(windows)]
    {
        // SAFETY: yalnız geçerli bir SYSTEMTIME'a yazar.
        unsafe { windows::Win32::System::SystemInformation::GetLocalTime().wHour }
    }
    #[cfg(not(windows))]
    {
        12
    }
}

/// Kartın kapağı.
#[derive(Clone)]
pub(super) enum Art {
    Image {
        url: String,
        icon: &'static str,
    },
    Collection {
        id: CollectionId,
        image: String,
    },
    /// Birbirinin ardına dizilmiş birkaç kapak (ana sayfada günlük karışımlar
    /// tek kart): en çok üç, öndeki ilk.
    Stack(Vec<String>),
}

#[derive(Clone)]
pub(super) struct CardSpec {
    pub(super) key: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) art: Art,
    pub(super) round: bool,
    pub(super) target: CardTarget,
}

fn kind_label(kind: ReleaseKind) -> &'static str {
    match kind {
        ReleaseKind::Album => t!(Album),
        ReleaseKind::Single => t!(ReleaseSingle),
        ReleaseKind::Ep => "EP",
        ReleaseKind::Compilation => t!(ReleaseCompilation),
        ReleaseKind::AppearsOn => t!(Album),
    }
}

fn album_card(a: &AlbumCard) -> CardSpec {
    let subtitle = match (a.year, a.kind) {
        (0, k) => kind_label(k).to_string(),
        (y, ReleaseKind::AppearsOn) => format!("{y} · {}", a.artists),
        (y, k) => format!("{y} · {}", kind_label(k)),
    };
    CardSpec {
        key: a.uri.clone(),
        title: a.name.clone(),
        subtitle,
        art: Art::Image {
            url: a.image.clone(),
            icon: "disc-3",
        },
        round: false,
        target: CardTarget::Album {
            uri: a.uri.clone(),
            item: Some(LibraryItem {
                id: CollectionId::Album(a.uri.clone()),
                name: a.name.clone(),
                subtitle: a.artists.clone(),
                image: a.image.clone(),
            }),
        },
    }
}

pub(super) fn artist_card(a: &ArtistCard) -> CardSpec {
    CardSpec {
        key: a.uri.clone(),
        title: a.name.clone(),
        subtitle: t!(Artist).into(),
        art: Art::Image {
            url: a.image.clone(),
            icon: "user",
        },
        round: true,
        target: CardTarget::Artist(a.uri.clone()),
    }
}

/// Sayfa geçişi (7 Kasım kararı): yeni yerin içeriği belirerek ve 8 px
/// yükselerek gelir, 220 ms'de yavaşlayarak biter (`motion`; liste görünümü
/// de aynı hareketle). Geçiş bittikten sonra kare istemez; girdiyi
/// bekletmez (içerik ilk karede yerinde ve tıklanabilir). "Animasyonları
/// azalt"ta hiç sarmalanmaz: 1 ms'lik hareket bile ilk karede içeriği
/// saydam çizerdi.
pub(super) fn page_enter(id: ElementId, content: Div) -> AnyElement {
    use gpui::AnimationExt;
    if !animations_enabled() {
        return content.into_any_element();
    }
    content
        .with_animation(
            id,
            gpui::Animation::new(motion()).with_easing(gpui::ease_out_quint()),
            |el, d| el.opacity(d).mt(px(S2 * (1. - d))),
        )
        .into_any_element()
}

pub(super) fn collection_card(i: &LibraryItem) -> CardSpec {
    let title = if i.id == CollectionId::Liked {
        t!(LikedSongs).to_string()
    } else {
        i.name.clone()
    };
    CardSpec {
        key: i.id.key().to_string(),
        title: title.clone(),
        subtitle: crate::i18n::canon::localize(&i.subtitle).into_owned(),
        art: Art::Collection {
            id: i.id.clone(),
            image: i.image.clone(),
        },
        round: false,
        target: CardTarget::Collection {
            id: i.id.clone(),
            title,
        },
    }
}

impl Shell {
    // ── Gezinme ─────────────────────────────────────────────────────────

    /// Liste yerine bir sayfa mı açık?
    pub(super) fn page_open(&self) -> bool {
        !self.pages.history.current().is_collection()
    }

    /// Şimdiki yerin kaydırma konumu (geri gelince oraya dönülür).
    fn current_scroll(&self) -> f32 {
        if self.all.open {
            (-f32::from(self.all.scroll.0.borrow().base_handle.offset().y)).max(0.)
        } else if self.page_open() {
            (-f32::from(self.pages.scroll.0.borrow().base_handle.offset().y)).max(0.)
        } else {
            self.list_position()
        }
    }

    /// Bir yere git (geçmişe eklenir).
    pub(super) fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        // Deezer'dan gelen benzer sanatçı kartı: önce Spotify adresi bulunur.
        if let Route::Artist(uri) = &route
            && uri.starts_with("deezer:")
        {
            self.open_deezer_artist(uri.clone());
            return;
        }
        let scroll = self.current_scroll();
        let moved = self.pages.history.push(route.clone(), scroll);
        let shown = match &route {
            Route::Collection(id) => self.library.selected.as_ref() == Some(id) && !self.all.open,
            Route::Library => self.all.open,
            _ => true,
        };
        if !moved && shown {
            return;
        }
        self.show_route(route, None, cx);
    }

    pub(super) fn go_back(&mut self, cx: &mut Context<Self>) {
        let scroll = self.current_scroll();
        if let Some(entry) = self.pages.history.back(scroll) {
            self.show_route(entry.route, Some(entry.scroll), cx);
        }
    }

    pub(super) fn go_forward(&mut self, cx: &mut Context<Self>) {
        let scroll = self.current_scroll();
        if let Some(entry) = self.pages.history.forward(scroll) {
            self.show_route(entry.route, Some(entry.scroll), cx);
        }
    }

    pub(super) fn go_home(&mut self, cx: &mut Context<Self>) {
        self.navigate(Route::Home, cx);
    }

    /// Yeri ekrana getir: veri önbellekteyse bu karede, değilse iskelet.
    pub(super) fn show_route(&mut self, route: Route, scroll: Option<f32>, cx: &mut Context<Self>) {
        self.pages.opened_at = Some((route.clone(), Instant::now()));
        self.close_all(cx);
        self.manage.menu = None;
        self.manage.find = None;
        self.manage.sel = Default::default();
        self.pages.glide.cancel();
        self.pages.pending_link = None;
        if !matches!(route, Route::Settings) {
            self.eq_leave();
        }
        match &route {
            Route::Collection(id) => {
                self.show_collection(id.clone(), scroll, cx);
                return;
            }
            Route::Library => {
                self.open_library(scroll, cx);
                return;
            }
            Route::Artist(uri) => {
                // Önbellek okuması UI iş parçacığında yapılmaz: motor önbellekteki
                // sayfayı hemen olay olarak yollar, bayatsa / yoksa arkadan
                // indirir (`wire::open_page`). Bellekte hazır olan sayfa gösterilir.
                if !matches!(self.pages.artists.get(uri), Some(PageLoad::Ready(_))) {
                    self.pages.artists.insert(uri.clone(), PageLoad::Loading);
                }
                self.engine
                    .send(Command::Library(LibraryCommand::OpenArtist(uri.clone())));
                // Sayfa bellekte hazırsa (önbelleğe yazılmamış olsa da) benzerleri.
                if let Some(PageLoad::Ready(page)) = self.pages.artists.get(uri) {
                    let page = page.clone();
                    self.want_related(uri, &page);
                }
            }
            Route::Album(uri) => {
                // Sanatçı sayfasıyla aynı: okuma ve tazelik kararı motorda.
                if !matches!(self.pages.albums.get(uri), Some(PageLoad::Ready(_))) {
                    self.pages.albums.insert(uri.clone(), PageLoad::Loading);
                }
                self.engine
                    .send(Command::Library(LibraryCommand::OpenAlbum(uri.clone())));
            }
            Route::Settings => self.enter_settings(cx),
            Route::Tools => self.enter_tools(cx),
            Route::Home => self.ask_explore(&[crate::engine::ExploreKey::Home]),
            Route::Explore => self.enter_explore(cx),
            Route::Category(uri) => {
                self.ask_explore(&[crate::engine::ExploreKey::Category(uri.clone())])
            }
            Route::Search(query) => {
                let have = self.pages.search.as_ref().is_some_and(|s| {
                    &s.query == query && !matches!(s.results, PageLoad::Failed(_))
                });
                if !have {
                    self.pages.seq += 1;
                    let seq = self.pages.seq;
                    self.pages.search = Some(SearchPage {
                        query: query.clone(),
                        seq,
                        results: PageLoad::Loading,
                        kind: Default::default(),
                    });
                    // Ekran sınaması (`OPENIFY_TEST_SEARCH_JSON`): kayıtlı yanıt, Spotify'a gitmeden.
                    if let Some(saved) = crate::engine::test_search_results() {
                        self.on_page_search(seq, Ok(Arc::new(saved)));
                    } else {
                        self.engine
                            .send(Command::Library(LibraryCommand::SearchAll {
                                seq,
                                query: query.clone(),
                            }));
                    }
                }
            }
            Route::Recent => {}
        }
        // Sayfa en üstten (ya da geri gelince bırakıldığı yerden) açılır.
        let y = scroll.unwrap_or(0.);
        self.pages.restore = Some(y);
        let base = self.pages.scroll.0.borrow().base_handle.clone();
        base.set_offset(gpui::point(px(0.), px(-y)));
        cx.notify();
    }

    pub(super) fn store_artist(&mut self, uri: String, page: Arc<ArtistPage>) {
        self.pages
            .lists
            .insert(format!("top:{uri}"), Arc::new(page.top_tracks.clone()));
        self.want_related(&uri, &page);
        self.pages.artists.insert(uri, PageLoad::Ready(page));
    }

    /// Benzer sanatçılar: yalnız ekrandaki sanatçı için ve oturumda bir kez
    /// (önceden alınan / yandaki panelde gösterilen sayfalar için değil:
    /// Spotify'a giden istek az kalsın). Önbellekteyse bu karede gösterilir;
    /// taze değilse motor arkadan ister. Sayfa ilk ekranı beklemez.
    fn want_related(&mut self, uri: &str, page: &ArtistPage) {
        if !page.related.is_empty()
            || !matches!(self.pages.history.current(), Route::Artist(u) if u == uri)
        {
            return;
        }
        if !self.pages.asked_related.insert(uri.to_string()) {
            return;
        }
        // Önbellekteki benzerleri motor hemen yollar, taze değilse arkadan da ister.
        self.engine
            .send(Command::Library(LibraryCommand::OpenRelated {
                uri: uri.to_string(),
                name: page.name.clone(),
                top_track: page.top_tracks.first().map(|t| t.uri.clone()),
            }));
    }

    /// Deezer kartına tıklandı: Spotify adresi biliniyorsa oraya, değilse
    /// motor bir aramayla bulur (`LibraryEvent::ArtistResolved`).
    fn open_deezer_artist(&mut self, from: String) {
        // Daha önce bulunmuşsa motor önbellekten hemen yanıtlar.
        let name = self
            .pages
            .related
            .values()
            .flat_map(|l| l.artists.iter())
            .find(|a| a.uri == from)
            .map(|a| a.name.clone());
        if let Some(name) = name {
            self.pages.resolving = Some(from.clone());
            self.engine
                .send(Command::Library(LibraryCommand::ResolveArtist {
                    from,
                    name,
                }));
        }
    }

    fn store_album(&mut self, uri: String, page: Arc<AlbumPage>) {
        self.pages
            .lists
            .insert(format!("album:{uri}"), Arc::new(page.tracks.clone()));
        // Albümdeki şarkıların bağı bilinir: albüm burası, sanatçılar başlıktaki.
        for t in &page.tracks {
            self.pages
                .links
                .entry(t.uri.clone())
                .or_insert_with(|| TrackLinks {
                    uri: t.uri.clone(),
                    album: Some((uri.clone(), page.name.clone())),
                    artists: Vec::new(),
                });
        }
        // Albümün sanatçısının diğer albümleri sayfanın altında görünür:
        // sanatçı sayfası (bir kez, oturumda) arkadan istenir.
        if matches!(self.pages.history.current(), Route::Album(u) if *u == uri)
            && let Some(a) = page.artists.first()
            && !matches!(self.pages.artists.get(&a.uri), Some(PageLoad::Ready(_)))
        {
            let artist = a.uri.clone();
            self.prefetch(artist);
        }
        self.pages.albums.insert(uri, PageLoad::Ready(page));
    }

    /// Albüme git: kütüphanendeyse liste görünümü (rafta yeri belli), değilse
    /// albüm sayfası.
    pub(super) fn open_album(&mut self, uri: String, cx: &mut Context<Self>) {
        let id = CollectionId::Album(uri.clone());
        if self.library.item(&id).is_some() {
            self.select_collection(id, cx);
        } else {
            self.navigate(Route::Album(uri), cx);
        }
    }

    /// Satırdaki `i`. sanatçıya git (bağ bilinmiyorsa önce istenir).
    pub(super) fn open_artist_of(&mut self, row: &TrackRow, i: usize, cx: &mut Context<Self>) {
        match self
            .pages
            .links
            .get(&row.uri)
            .filter(|l| !l.artists.is_empty())
        {
            Some(l) => {
                let uri = l
                    .artists
                    .get(i)
                    .or(l.artists.first())
                    .map(|a| a.0.clone())
                    .unwrap_or_default();
                self.navigate(Route::Artist(uri), cx);
            }
            None => {
                self.pages.pending_link = Some((row.uri.clone(), LinkTarget::Artist(i)));
                self.force_links(row.uri.clone());
            }
        }
    }

    pub(super) fn open_album_of(&mut self, row: &TrackRow, cx: &mut Context<Self>) {
        match self.pages.links.get(&row.uri).and_then(|l| l.album.clone()) {
            Some((uri, _)) => self.open_album(uri, cx),
            None => {
                self.pages.pending_link = Some((row.uri.clone(), LinkTarget::Album));
                self.force_links(row.uri.clone());
            }
        }
    }

    /// Tıklanan adın bağı: daha önce sorulmuş olsa da yeniden iste (önceki
    /// soru bağlantıdan önce kalmış ya da cevapsız kalmış olabilir).
    fn force_links(&self, uri: String) {
        self.pages.asked_links.borrow_mut().insert(uri.clone());
        self.engine
            .send(Command::Library(LibraryCommand::ResolveLinks(vec![uri])));
    }

    /// Adın üzerine gelindi: sayfasını şimdiden iste (bir kez).
    pub(super) fn prefetch_artist_of(&mut self, row: &TrackRow, i: usize) {
        match self
            .pages
            .links
            .get(&row.uri)
            .and_then(|l| l.artists.get(i).or(l.artists.first()).cloned())
        {
            Some((uri, _)) => self.prefetch(uri),
            None => self.ask_links(vec![row.uri.clone()]),
        }
    }

    pub(super) fn prefetch_album_of(&mut self, row: &TrackRow) {
        match self.pages.links.get(&row.uri).and_then(|l| l.album.clone()) {
            Some((uri, _))
                if self
                    .library
                    .item(&CollectionId::Album(uri.clone()))
                    .is_none() =>
            {
                self.prefetch(uri)
            }
            Some(_) => {}
            None => self.ask_links(vec![row.uri.clone()]),
        }
    }

    pub(super) fn prefetch(&mut self, uri: String) {
        if self.pages.prefetched.insert(uri.clone()) {
            self.engine
                .send(Command::Library(LibraryCommand::Prefetch(uri)));
        }
    }

    fn ask_links(&self, uris: Vec<String>) {
        let mut asked = self.pages.asked_links.borrow_mut();
        let uris: Vec<String> = uris
            .into_iter()
            .filter(|u| asked.insert(u.clone()))
            .collect();
        if !uris.is_empty() {
            self.engine
                .send(Command::Library(LibraryCommand::ResolveLinks(uris)));
        }
    }

    /// Çizilen satırın bağı bilinmiyorsa iste (çizimden sonra toplu gider).
    pub(super) fn want_links(&self, uri: &str) {
        if !self.pages.links.contains_key(uri) && !self.pages.asked_links.borrow().contains(uri) {
            self.pages.want_links.borrow_mut().push(uri.to_string());
        }
    }

    /// Çizimde istenen bağları motora gönder.
    pub(super) fn flush_link_requests(&self) {
        let wanted = std::mem::take(&mut *self.pages.want_links.borrow_mut());
        if !wanted.is_empty() {
            self.ask_links(wanted);
        }
    }

    /// Bir listenin şarkıları (kütüphane listesi ya da sayfadaki liste).
    pub(super) fn list_tracks(&self, list: &ListKey) -> Option<Arc<Vec<TrackRow>>> {
        match list {
            ListKey::Collection(id) => self.library.tracks(id).cloned(),
            ListKey::Page(key) => self.pages.lists.get(key).cloned(),
        }
    }

    /// Listenin `ix`. şarkısından çal. Sayfadaki listeler sırayı motora
    /// verir (albüm sayfası albümü "çalan liste" olarak işaretler).
    pub(super) fn play_in(&mut self, list: &ListKey, ix: usize, cx: &mut Context<Self>) {
        match list {
            ListKey::Collection(id) => {
                if self.library.selected.as_ref() == Some(id) {
                    self.play_row(ix, cx);
                }
            }
            ListKey::Page(key) => {
                let Some(tracks) = self.pages.lists.get(key).cloned() else {
                    return;
                };
                let Some(track) = tracks.get(ix) else { return };
                if !self.ready_to_play(cx) {
                    return;
                }
                let context = key
                    .strip_prefix("album:")
                    .map(|uri| CollectionId::Album(uri.to_string()));
                if let Some(id) = &context {
                    self.note_played(id);
                }
                self.state.notice = None;
                self.state.pending = Some(PendingTrack {
                    name: track.name.clone(),
                    image: track.image.clone(),
                });
                self.engine.send(Command::PlayTracks {
                    context,
                    tracks,
                    index: ix,
                });
                cx.notify();
            }
        }
    }

    // ── Motordan gelenler ───────────────────────────────────────────────

    pub(super) fn on_library_event(&mut self, event: LibraryEvent, cx: &mut Context<Self>) {
        match event {
            LibraryEvent::Artist { uri, page } => self.store_artist(uri, page),
            LibraryEvent::Album { uri, page } => self.store_album(uri, page),
            LibraryEvent::PageFailed { uri, error } => {
                // Gösterilen (bayat) sayfa hata yüzünden silinmez.
                if uri.starts_with("spotify:artist:") {
                    if !matches!(self.pages.artists.get(&uri), Some(PageLoad::Ready(_))) {
                        self.pages.artists.insert(uri, PageLoad::Failed(error));
                    }
                } else if !matches!(self.pages.albums.get(&uri), Some(PageLoad::Ready(_))) {
                    self.pages.albums.insert(uri, PageLoad::Failed(error));
                }
            }
            LibraryEvent::Links(links) => {
                for l in links {
                    // Albüm sayfasından bilinen bağda sanatçılar eksik olabilir.
                    self.pages.links.insert(l.uri.clone(), l);
                }
                if let Some((track, target)) = self.pages.pending_link.clone()
                    && let Some(l) = self.pages.links.get(&track).cloned()
                {
                    self.pages.pending_link = None;
                    match target {
                        LinkTarget::Artist(i) => {
                            if let Some((uri, _)) = l.artists.get(i).or(l.artists.first()) {
                                self.navigate(Route::Artist(uri.clone()), cx);
                            }
                        }
                        LinkTarget::Album => {
                            if let Some((uri, _)) = l.album {
                                self.open_album(uri, cx);
                            }
                        }
                    }
                }
            }
            LibraryEvent::Related { uri, list } => {
                if self.pages.related.get(&uri).is_some_and(|old| *old == list) {
                    return;
                }
                self.pages.related.insert(uri, list);
            }
            LibraryEvent::ArtistResolved { from, uri } => {
                if self.pages.resolving.as_ref() == Some(&from) {
                    self.pages.resolving = None;
                    if let Some(uri) = uri {
                        self.navigate(Route::Artist(uri), cx);
                    }
                }
            }
            LibraryEvent::Explore { key, data } => self.on_explore(key, data),
            LibraryEvent::ExploreFailed { key, error } => self.on_explore_failed(key, error),
            LibraryEvent::Friends(list) => self.on_friends(list, cx),
            LibraryEvent::FriendsFailed(why) => self.on_friends_failed(why, cx),
            LibraryEvent::Credits { uri, result } => self.on_credits(uri, result, cx),
            LibraryEvent::EditStatus { pending, failed } => {
                self.on_edit_status(pending, failed, cx)
            }
            LibraryEvent::Created { temp, uri } => self.on_created(temp, uri, cx),
            LibraryEvent::AddedDates { id, dates } => {
                self.library.added.insert(id, dates);
                self.tools_on_data(cx);
            }
            LibraryEvent::PlayStats(report) => self.on_play_stats(report, cx),
            LibraryEvent::BackupDone(summary) => self.on_backup_done(summary, cx),
            LibraryEvent::BackupFailed => self.on_backup_failed(cx),
            LibraryEvent::Hidden(list) => self.on_hidden(list),
        }
        cx.notify();
    }

    /// Tam arama sonucu geldi mi (paletinki değil)? Geldiyse `true`.
    pub(super) fn on_page_search(
        &mut self,
        seq: u64,
        result: Result<Arc<SearchResults>, String>,
    ) -> bool {
        let Some(search) = self.pages.search.as_mut().filter(|s| s.seq == seq) else {
            return seq >= SEARCH_SEQ_BASE;
        };
        search.results = match result {
            Ok(r) => {
                self.pages.lists.insert(
                    format!("search:{}", search.query),
                    Arc::new(r.tracks.clone()),
                );
                PageLoad::Ready(r)
            }
            Err(e) => PageLoad::Failed(e),
        };
        true
    }

    /// Kart menüsünden "Çal" bekleyen liste geldi.
    pub(super) fn play_if_waiting(&mut self, id: &CollectionId, cx: &mut Context<Self>) {
        if self.pages.play_when_loaded.as_ref() == Some(id)
            && self.library.selected.as_ref() == Some(id)
            && self.library.tracks(id).is_some_and(|t| !t.is_empty())
        {
            self.pages.play_when_loaded = None;
            self.play_from_start(cx);
        }
    }

    // ── Kaydırma ────────────────────────────────────────────────────────

    /// Sayfanın görünen yüksekliği ve içeriğin boyu (çizilmediyse `None`).
    pub(super) fn page_extent(&self) -> Option<(f32, f32)> {
        let state = self.pages.scroll.0.borrow();
        let h = f32::from(state.base_handle.bounds().size.height);
        if h <= 0. {
            return None;
        }
        Some((h, h + f32::from(state.base_handle.max_offset().height)))
    }

    fn scroll_page_by(&self, d: f32) -> f32 {
        let max = self.page_extent().map_or(0., |(v, c)| (c - v).max(0.));
        let handle = self.pages.scroll.0.borrow().base_handle.clone();
        let offset = handle.offset();
        let from = (-f32::from(offset.y)).max(0.);
        let to = (from + d).clamp(0., max);
        if to != from {
            handle.set_offset(gpui::point(offset.x, px(-to)));
        }
        to - from
    }

    fn wheel_page(
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
        let glide = self.pages.glide.clone();
        match glide.classify(delta, line_height, viewport, now) {
            Some(Wheel::Glide(d)) => glide.push(d, viewport, now),
            Some(Wheel::Now(d)) => {
                glide.cancel();
                self.scroll_page_by(d);
            }
            None => return false,
        }
        true
    }

    /// Sayfanın tekerlek hareketinin bu karelik adımı; sürüyorsa `true`.
    pub(super) fn tick_page_glide(&mut self) -> bool {
        let glide = self.pages.glide.clone();
        if !glide.is_active() {
            return false;
        }
        let position = (-f32::from(self.pages.scroll.0.borrow().base_handle.offset().y)).max(0.);
        glide.tick(Instant::now(), position, |d| self.scroll_page_by(d))
    }

    // ── Başlık çubuğu: geri / ileri / ana sayfa ─────────────────────────

    pub(super) fn render_nav_buttons(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let h = &self.pages.history;
        let back = h.can_back();
        let forward = h.can_forward();
        let (back_stop, back_look) = self.stop_look("nav-back", super::tab::BACK, back, window, cx);
        let (forward_stop, forward_look) =
            self.stop_look("nav-forward", super::tab::FORWARD, forward, window, cx);
        div()
            .flex_none()
            .h_full()
            .flex()
            .items_center()
            // Geri simgesi sayfanın sol kenarında (32 px) başlar.
            .pl(px(S5))
            .child(
                icon_button("nav-back", "arrow-left", back_look, accent)
                    .tooltip(Tooltip::text(t!(Back), Some("Alt ←")))
                    .when_some(back_stop, |el, h| el.track_focus(&h))
                    .when(back, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.go_back(cx)))
                    }),
            )
            .child(
                icon_button("nav-forward", "arrow-right", forward_look, accent)
                    .tooltip(Tooltip::text(t!(Forward), Some("Alt →")))
                    .when_some(forward_stop, |el, h| el.track_focus(&h))
                    .when(forward, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.go_forward(cx)))
                    }),
            )
            .into_any_element()
    }

    // ── Sayfaların çizimi ───────────────────────────────────────────────

    pub(super) fn render_page(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // Profil menüsündeki "Hakkında": Ayarlar'ın en altına (gpui konumu
        // sayfanın sonuna kırpar).
        if matches!(self.pages.history.current(), Route::Settings)
            && std::mem::take(&mut self.sui.jump_about)
        {
            self.pages.restore = Some(100_000.);
        }
        // Geri / ileride bırakılan konum: içerik çizildikten sonra (sınırlar
        // gerçek içerikle hesaplansın) bir kez daha uygulanır.
        if let Some(y) = self.pages.restore.take() {
            let base = self.pages.scroll.0.borrow().base_handle.clone();
            base.set_offset(gpui::point(px(0.), px(-y)));
            // İskelet kısa: gpui konumu ona göre kırpar. İçerik gelince yeniden.
            if !self.page_ready(&self.pages.history.current().clone()) {
                self.pages.restore = Some(y);
            }
        }
        let route = self.pages.history.current().clone();
        if let Some((r, at)) = &self.pages.opened_at
            && *r == route
            && self.page_ready(&route)
        {
            log::debug!(
                "sayfa çizildi: {:.1} ms ({route:?})",
                at.elapsed().as_secs_f64() * 1000.
            );
            self.pages.opened_at = None;
        }
        let content = match &route {
            Route::Artist(uri) => self.artist_body(uri, window, cx),
            Route::Album(uri) => self.album_body(uri, window, cx),
            Route::Home => self.home_body(window, cx),
            Route::Explore => self.explore_body(window, cx),
            Route::Category(uri) => self.category_body(uri, window, cx),
            Route::Search(q) => self.search_body(q, window, cx),
            Route::Settings => self.settings_body(window, cx),
            Route::Recent => self.recent_body(window, cx),
            Route::Tools => self.tools_body(window, cx),
            Route::Collection(_) | Route::Library => div(),
        };
        self.page_frame(content, &route, cx)
    }

    /// Sayfanın verisi elde mi (iskelet değil içerik çizilecek mi)?
    fn page_ready(&self, route: &Route) -> bool {
        match route {
            Route::Artist(u) => matches!(self.pages.artists.get(u), Some(PageLoad::Ready(_))),
            Route::Album(u) => matches!(self.pages.albums.get(u), Some(PageLoad::Ready(_))),
            Route::Search(_) => self
                .pages
                .search
                .as_ref()
                .is_some_and(|s| matches!(s.results, PageLoad::Ready(_))),
            Route::Category(u) => self
                .pages
                .explore
                .ready(&crate::engine::ExploreKey::Category(u.clone())),
            _ => true,
        }
    }

    /// Sayfanın kaydırılan alanı: tekerlek yumuşak (listedeki gibi),
    /// kaydırma çubuğu, satırlar hapın altından geçer.
    fn page_frame(&self, content: Div, route: &Route, cx: &mut Context<Self>) -> AnyElement {
        let base = self.pages.scroll.0.borrow().base_handle.clone();
        // Sayfaya girerken içerik belirerek yükselir (bkz. `page_enter`).
        // Anahtar yer: iskeletten içeriğe geçerken yeniden oynamaz.
        let content = content.pt(px(S2)).pb(px(PILL_CLEARANCE + S5));
        let scroller = div()
            .id("page-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&base)
            .child(page_enter(
                ElementId::Name(format!("page-enter-{route:?}").into()),
                content,
            ));
        let this = cx.entity().downgrade();
        let wheel = super::smooth_scroll::wheel_zone(move |ev, bounds, view, window, cx| {
            let line_height = window.line_height();
            let viewport = f32::from(bounds.size.height);
            let handled = this
                .update(cx, |shell, cx| {
                    shell.wheel_page(ev.delta, line_height, viewport, cx)
                })
                .unwrap_or(false);
            if handled {
                cx.notify(view);
            }
            handled
        });
        // Ekolayzer katmanı sayfanın dışına taşmasın.
        let viewport = self.list_viewport.clone();
        let measure = gpui::canvas(
            move |bounds, _, _| viewport.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .relative()
            .group("page-scroll")
            .flex_1()
            .min_h_0()
            .h_full()
            .child(scroller)
            .child(measure)
            .child(wheel)
            .children(self.scrollbar(ScrollBar::Page, cx))
            .into_any_element()
    }

    /// Kart ızgarasının sütun sayısı ve kart eni (tam sütun). Izgara içerik
    /// alanının bütün enini kullanır, şarkı satırları gibi (7 Kasım; önce
    /// 1280 px'te durup geniş pencerede sağda boşluk bırakıyordu).
    pub(super) fn grid(&self) -> (usize, f32) {
        let room = (self.window_width - 2. * self.margin()).max(CARD_MIN);
        let cols = (((room + CARD_GAP) / (CARD_MIN + CARD_GAP)).floor() as usize).max(2);
        let width = (room - CARD_GAP * (cols as f32 - 1.)) / cols as f32;
        (cols, width)
    }

    /// Sayfanın sol / sağ kenar boşluğu (liste satırları, raf ve başlıklarla
    /// aynı: `theme::page_margin`).
    pub(super) fn margin(&self) -> f32 {
        page_margin(self.window_width)
    }

    pub(super) fn compact(&self) -> bool {
        self.window_width < super::COMPACT_BELOW
    }

    /// Başlık bloğu: kapak, ad, meta, eylemler. Bütün sayfalarda aynı.
    pub(super) fn page_header(
        &self,
        art: Option<AnyElement>,
        title: &str,
        meta: AnyElement,
        actions: Vec<AnyElement>,
    ) -> Div {
        div()
            .px(px(self.margin()))
            .pt(px(if art.is_some() { S5 } else { PLAIN_HEADER_TOP }))
            .pb(px(S6))
            .flex()
            .items_end()
            .gap(px(S5))
            .children(art)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .font_family(font())
                            .type_role(title_role(self.compact()))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(text())
                            .child(
                                line_with_tip("page-heading", title.to_string())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .type_role(T_BODY)
                            .text_color(muted())
                            .child(meta),
                    )
                    .when(!actions.is_empty(), |el| {
                        el.child(
                            div()
                                .pt(px(S2))
                                .flex()
                                .items_center()
                                .gap(px(S3))
                                .children(actions),
                        )
                    }),
            )
    }

    fn header_art_side(&self) -> f32 {
        if self.compact() {
            HEADER_ART_COMPACT
        } else {
            HEADER_ART
        }
    }

    /// Bölüm başlığı; uzun bölümde "Tümünü göster" / "Daha az".
    pub(super) fn section_title(
        &self,
        key: &str,
        title: &str,
        toggle: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        self.section_title_hint(key, title, None, toggle, cx)
    }

    /// Aynısı, istenirse başlığın altında kısa bir açıklama satırıyla (ana
    /// sayfa; açıklamalı başlık içeriği kadar uzar, açıklamasızın 48 px'i
    /// sabit).
    pub(super) fn section_title_hint(
        &self,
        key: &str,
        title: &str,
        hint: Option<&str>,
        toggle: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let expanded = self.pages.expanded.contains(key);
        let accent = self.accent().text;
        let key_owned = key.to_string();
        let heading = div()
            .font_family(font())
            .type_role(T_TITLE)
            .font_weight(FontWeight::EXTRA_BOLD)
            .text_color(text())
            .child(title.to_string());
        let heading = match hint {
            None => div().child(heading),
            Some(hint) => div().flex().flex_col().gap(px(S0)).child(heading).child(
                div()
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(hint.to_string()),
            ),
        };
        // Ana sayfada "Çok dinlediklerin" rafının sağında, son çalınanlar
        // sayfasına geçen "Son çalınanlar" çipi ("hepsini gör").
        let see_all = key == "home#often" && !self.recent.is_empty();
        let toggle_chip = toggle.then(|| {
            chip(
                ElementId::Name(format!("more-{key}").into()),
                if expanded { t!(ShowLess) } else { t!(ShowAll) },
                false,
                Look::Live,
                accent,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.pages.expanded.remove(&key_owned) {
                    this.pages.expanded.insert(key_owned.clone());
                }
                cx.notify();
            }))
        });
        let recent_chip = see_all.then(|| {
            chip(
                ElementId::Name("see-all-recent".into()),
                t!(RecentPageTitle),
                false,
                Look::Live,
                accent,
            )
            .on_click(cx.listener(|this, _, _, cx| this.navigate(Route::Recent, cx)))
        });
        div()
            .px(px(self.margin()))
            .pb(px(S3))
            .when(hint.is_none(), |el| el.h(px(S7)))
            .when(hint.is_some(), |el| el.pt(px(S3)))
            .flex()
            .items_end()
            .justify_between()
            .child(heading)
            .when(toggle || see_all, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(S2))
                        .children(toggle_chip)
                        .children(recent_chip),
                )
            })
    }

    /// Kart bölümü: başlık ve bir satır kart (açılınca hepsi). Boşsa hiç yok.
    pub(super) fn card_section(
        &self,
        key: String,
        title: &str,
        cards: Vec<CardSpec>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if cards.is_empty() {
            return None;
        }
        let (cols, width) = self.grid();
        let expanded = self.pages.expanded.contains(&key);
        let shown = if expanded {
            cards.len()
        } else {
            cards.len().min(cols)
        };
        let mut grid = div().px(px(self.margin())).flex().flex_col().gap(px(S5));
        for row in cards[..shown].chunks(cols) {
            let mut line = div().flex().gap(px(CARD_GAP));
            for card in row {
                line = line.child(self.page_card(card, width, cx));
            }
            grid = grid.child(line);
        }
        Some(
            div()
                .pb(px(S6))
                .child(self.section_title(&key, title, cards.len() > cols, cx))
                .child(grid)
                .into_any_element(),
        )
    }

    pub(super) fn page_card(
        &self,
        card: &CardSpec,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let radius = if card.round { width / 2. } else { R_LG };
        let art = match &card.art {
            Art::Image { url, icon } => {
                cover_image(&self.images, url, &card.title, width, radius, icon)
            }
            Art::Collection { id, image } => {
                self.collection_art(id, &card.title, image, width, radius)
            }
            Art::Stack(urls) => self.stack_art(urls, &card.title, width, radius),
        };
        let group: SharedString = "page-card".into();
        let open = card.target.clone();
        let menu = card.target.clone();
        let hover_uri = match &card.target {
            CardTarget::Artist(uri) => Some(uri.clone()),
            CardTarget::Album { uri, .. }
                if self
                    .library
                    .item(&CollectionId::Album(uri.clone()))
                    .is_none() =>
            {
                Some(uri.clone())
            }
            _ => None,
        };
        let playing = matches!(&card.target, CardTarget::Collection { id, .. } if self.state.queue.context.as_ref() == Some(id) && self.state.is_playing());
        div()
            .id(ElementId::Name(format!("card-{}", card.key).into()))
            .group(group.clone())
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(S2))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.open_card(open.clone(), cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_card_menu(menu.clone(), ev.position, cx);
                }),
            )
            .when_some(hover_uri, |el, uri| {
                el.on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                    if *hovered {
                        this.prefetch(uri.clone());
                    }
                }))
            })
            .child(
                div()
                    .relative()
                    .size(px(width))
                    .child(art)
                    // Kapağın üstünde ton katmanı, 140 ms geçişle (`tween`).
                    .child(super::tween::hover_fill(
                        veil(VEIL_HOVER),
                        veil(VEIL_PRESSED),
                        0.,
                        radius,
                    ))
                    .when(playing, |el| {
                        el.child(
                            div()
                                .absolute()
                                .right(px(S2))
                                .bottom(px(S2))
                                .size(px(S5))
                                .rounded_full()
                                .bg(shade(SCRIM))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("volume-2", 14., rgb(self.accent().text))),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S0))
                    .child(
                        div()
                            .flex()
                            .type_role(T_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text())
                            .child(
                                line_with_tip(
                                    ElementId::Name(format!("ct-{}", card.key).into()),
                                    card.title.clone(),
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
                                    ElementId::Name(format!("cs-{}", card.key).into()),
                                    card.subtitle.clone(),
                                )
                                .flex()
                                .min_w_0(),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Birbirinin ardına dizilmiş kapaklar: öndeki ilk kapak, ardındakiler
    /// sağ üste doğru 12 px kayar ve soluklaşır (günlük karışımlar tek kart).
    fn stack_art(&self, urls: &[String], name: &str, side: f32, radius: f32) -> AnyElement {
        let layers = urls.len().clamp(1, 3);
        let inner = side - (layers as f32 - 1.) * S3;
        let mut stack = div().relative().size(px(side)).flex_none();
        // Arkadan öne: en arkadaki önce çizilir.
        for depth in (0..layers).rev() {
            let url = urls.get(depth).map(String::as_str).unwrap_or_default();
            let opacity = [1., 0.7, 0.45][depth];
            stack = stack.child(
                div()
                    .absolute()
                    .left(px(depth as f32 * S3))
                    .top(px((layers - 1 - depth) as f32 * S3))
                    .opacity(opacity)
                    .child(cover_image(
                        &self.images,
                        url,
                        name,
                        inner,
                        radius,
                        "list-music",
                    )),
            );
        }
        stack.into_any_element()
    }

    /// Sayfadaki şarkı listesi: başlık ve satırlar (`limit`: kısa hâli).
    #[allow(clippy::too_many_arguments)]
    fn track_section(
        &self,
        list: &str,
        title: Option<&str>,
        tracks: &[TrackRow],
        limit: Option<usize>,
        album_artists: Option<&str>,
        playing_ix: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = ListKey::Page(list.to_string());
        let expanded = self.pages.expanded.contains(list);
        let shown = match limit {
            Some(n) if !expanded => tracks.len().min(n),
            _ => tracks.len(),
        };
        let album = album_artists.is_some();
        // Geniş albüm sayfası: farklı sanatçılı şarkı varsa sanatçılar sütunda.
        let artist_column = album_artists.is_some_and(|a| {
            self.window_width >= ALBUM_ARTIST_COLUMN_FROM
                && tracks
                    .iter()
                    .any(|t| !super::library::same_artists(a, &t.artists))
        });
        let playing_uri = self.state.track.as_ref().map(|t| t.uri.as_str());
        let mut rows = div().flex().flex_col();
        for (ix, track) in tracks[..shown].iter().enumerate() {
            let playing = match playing_ix {
                Some(p) => p == ix,
                None => playing_uri == Some(track.uri.as_str()),
            };
            rows = rows.child(
                self.song_row(
                    RowSpec {
                        list: key.clone(),
                        ix,
                        show_cover: !album,
                        show_album: !album,
                        hide_artist: artist_column
                            || album_artists
                                .is_some_and(|a| super::library::same_artists(a, &track.artists)),
                        artist_column,
                        playing,
                        selected: self.is_selected(&key, ix),
                        look: Look::Live,
                        reorder: false,
                        added: None,
                    },
                    track,
                    cx,
                ),
            );
        }
        let toggle = limit.is_some_and(|n| tracks.len() > n);
        div()
            .pb(px(S6))
            .when_some(title, |el, t| {
                el.child(self.section_title(list, t, toggle, cx))
            })
            .child(rows)
            .into_any_element()
    }

    /// Sayfa alınamadı: durum görünümü, listedeki ve "Tümü"deki gibi hapın
    /// üstünde kalan alanın ortasında. (Önce sabit 420 px'lik kutunun
    /// ortasındaydı: 760 px'lik pencerede üst kenara yapışık duruyordu.)
    pub(super) fn page_error(
        &self,
        error: &str,
        retry: Route,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (icon_name, title, message) = explain_error(error);
        let accent = self.accent().text;
        // Kutu kaydırma alanı (başlık çubuğunun altı) boyunda, kaydırma yok;
        // `page_frame` bu kutuya üstte S2, altta hap payı + S5 iç boşluk
        // veriyor: ortalama hapın üstünde kalan alanda olur.
        let height = f32::from(window.viewport_size().height) - super::TITLEBAR_HEIGHT;
        div()
            .h(px(height))
            .flex()
            .items_center()
            .justify_center()
            .child(
                state_view(icon_name, title, message).child(
                    secondary_button("page-retry", "refresh-cw", t!(Retry), Look::Live, accent)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_route(retry.clone(), None, cx)
                        })),
                ),
            )
    }

    /// Yüklenirken: başlık, satırlar ve bir sıra kart aynı ölçülerde.
    pub(super) fn page_skeleton(&self, round: bool, rows: bool) -> Div {
        self.page_skeleton_sized(round, rows, self.header_art_side(), false)
    }

    /// Kapaksız sayfaların (tür sayfası, Son çalınanlar) iskeleti: kapak
    /// karesi de düğme de yok, çünkü gerçek sayfada da yok; başlık çubuğu
    /// `page_header(None, ..)`daki başlıkla aynı yerde (içerik gelince
    /// başlık sola zıplamaz). `section_title`: tür sayfasında kartlardan önce
    /// gelen 48 px'lik bölüm başlığı satırı.
    pub(super) fn page_skeleton_plain(&self, section_title: bool) -> Div {
        let g = plain_header_geometry(self.compact());
        let header = div()
            .px(px(self.margin()))
            .pt(px(PLAIN_HEADER_TOP))
            .pb(px(S6))
            .flex()
            .items_end()
            .gap(px(S5))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(div().h(px(g.title_h)).child(skeleton_bar(280., g.title_h)))
                    // Meta satırı 20 px'lik yazı satırı; çubuk onun ortasında.
                    .child(
                        div()
                            .h(px(g.meta_h))
                            .flex()
                            .items_center()
                            .child(skeleton_bar(180., 10.)),
                    ),
            );
        // Gerçek `section_title` (ipucusuz): 48 px, başlık altta (24 px'lik satır).
        let section = section_title.then(|| {
            div()
                .px(px(self.margin()))
                .pb(px(S3))
                .h(px(S7))
                .flex()
                .items_end()
                .child(
                    div()
                        .h(px(T_TITLE.line))
                        .flex()
                        .items_center()
                        .child(skeleton_bar(140., 18.)),
                )
        });
        div()
            .flex()
            .flex_col()
            .child(header)
            .children(section)
            .child(self.skeleton_cards())
    }

    /// Bir sıra kart iskeleti (kapak karesi, iki satır yazı).
    fn skeleton_cards(&self) -> Div {
        let (cols, width) = self.grid();
        let mut cards = div().px(px(self.margin())).flex().gap(px(CARD_GAP));
        for _ in 0..cols {
            cards = cards.child(
                div()
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(div().size(px(width)).rounded(px(R_LG)).bg(veil(VEIL_HOVER)))
                    .child(skeleton_bar(width * 0.7, 10.))
                    .child(skeleton_bar(width * 0.4, 8.)),
            );
        }
        cards
    }

    /// `page_skeleton`, kapak boyu ve albüm satırlarıyla
    /// (albüm sayfası: gerçek içerikle aynı ölçüler).
    fn page_skeleton_sized(&self, round: bool, rows: bool, side: f32, album: bool) -> Div {
        let bar = skeleton_bar;
        let header = div()
            .px(px(self.margin()))
            .pt(px(if round || album { S5 } else { S7 }))
            .pb(px(S6))
            .flex()
            .items_end()
            .gap(px(S5))
            .child(
                div()
                    .size(px(side))
                    .flex_none()
                    .rounded(px(if round { side / 2. } else { R_LG }))
                    .bg(veil(VEIL_HOVER)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S3))
                    .child(bar(
                        280.,
                        if self.compact() {
                            T_HEADLINE.size
                        } else {
                            T_DISPLAY.size
                        },
                    ))
                    .child(bar(180., 10.))
                    .child(div().pt(px(S2)).child(bar(96., 40.).rounded_full())),
            );
        let cards = self.skeleton_cards();
        div()
            .flex()
            .flex_col()
            .child(header)
            .when(rows, |el| {
                el.child(
                    div()
                        .pb(px(S6))
                        .child(skeleton_rows_at(album, self.margin())),
                )
            })
            .when(!album, |el| el.child(cards))
    }

    fn artist_body(&self, uri: &str, window: &Window, cx: &mut Context<Self>) -> Div {
        let _ = window;
        let page = match self.pages.artists.get(uri) {
            Some(PageLoad::Ready(p)) => p.clone(),
            Some(PageLoad::Failed(e)) => {
                return self.page_error(e, Route::Artist(uri.to_string()), window, cx);
            }
            _ => return self.page_skeleton(true, true),
        };
        let side = self.header_art_side();
        let art = cover_image(
            &self.images,
            &page.image,
            &page.name,
            side,
            side / 2.,
            "user",
        );
        let mut meta_parts = Vec::new();
        if !page.albums.is_empty() {
            meta_parts.push(tp!(ArtistAlbumCount, page.albums.len()));
        }
        if !page.singles.is_empty() {
            meta_parts.push(tp!(ArtistSingleCount, page.singles.len()));
        }
        let meta = div()
            .flex()
            .min_w_0()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(t!(Artist)),
            )
            .when(!meta_parts.is_empty(), |el| {
                el.child(format!(" · {}", meta_parts.join(" · ")))
            })
            .into_any_element();
        let list = format!("top:{uri}");
        let accent = self.accent().text;
        // Çal: sanatçının Spotify'daki bağlamı (popüler şarkılarla başlar,
        // sürer); satırlar ise listedeki sırayla çalar.
        let artist_uri = uri.to_string();
        // Takip: durum ekrandaki kütüphaneden (bekleyen yazma da sayılır), tıklayınca
        // anında döner; Spotify'a arkadan yazılır, geri alınabilir.
        let followed = self.artist_followed(uri);
        let follow_card = crate::engine::artist_item(uri, &page.name, &page.image);
        let follow_uri = uri.to_string();
        let menu_target = CardTarget::Artist(uri.to_string());
        let (more_probe, more_anchor) = anchor_probe();
        let actions = vec![
            primary_button("artist-play", "play-fill", t!(Play), Look::Live, accent)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.ready_to_play(cx) {
                        this.state.notice = None;
                        this.engine.send(Command::PlayContext(artist_uri.clone()));
                    }
                }))
                .into_any_element(),
            secondary_button(
                "artist-follow",
                if followed { "user-check" } else { "user-plus" },
                if followed {
                    t!(ArtistFollowing)
                } else {
                    t!(ArtistFollow)
                },
                Look::Live,
                accent,
            )
            // Çal ile aynı yükseklik ve yazı rolü (ikincil düğme 32 px'ti, yan yana tutarsız duruyordu).
            .h(px(PRIMARY_HEIGHT))
            .px(px(S4))
            .type_role(T_BODY)
            .when(followed, |el| el.border_color(rgb(accent)))
            .on_click(cx.listener(move |this, _, _, cx| {
                let on = !this.artist_followed(&follow_uri);
                this.set_artist_followed(follow_uri.clone(), on, Some(follow_card.clone()), cx);
            }))
            .into_any_element(),
            icon_button("artist-more", "ellipsis", Look::Live, accent)
                .relative()
                .tooltip(Tooltip::text(t!(MoreTip), None))
                .child(more_probe)
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                    this.open_card_menu(
                        menu_target.clone(),
                        menu_below(more_anchor.get(), ev.position()),
                        cx,
                    );
                }))
                .into_any_element(),
        ];
        let mut body =
            div()
                .flex()
                .flex_col()
                .child(self.page_header(Some(art), &page.name, meta, actions));
        if !page.top_tracks.is_empty() {
            body = body.child(self.track_section(
                &list,
                Some(t!(PagePopular)),
                &page.top_tracks,
                Some(TOP_FEW),
                None,
                None,
                cx,
            ));
        }
        let sections: [(&str, &str, &Vec<AlbumCard>); 4] = [
            ("albums", t!(KindAlbums), &page.albums),
            ("singles", t!(PageSingles), &page.singles),
            ("compilations", t!(PageCompilations), &page.compilations),
            ("appears", t!(PageAppearsOn), &page.appears_on),
        ];
        for (k, title, cards) in sections {
            let cards: Vec<CardSpec> = cards.iter().map(album_card).collect();
            body = body.children(self.card_section(format!("{uri}#{k}"), title, cards, cx));
        }
        // Benzer sanatçılar sayfadan ayrı gelir (ARTIST_V4'te boş; bkz.
        // `engine::related`): gelene kadar ve boşsa bölüm yok.
        let related = match self.pages.related.get(uri) {
            Some(list) if page.related.is_empty() => &list.artists,
            _ => &page.related,
        };
        let related: Vec<CardSpec> = related.iter().map(artist_card).collect();
        body.children(self.card_section(format!("{uri}#related"), t!(PageRelated), related, cx))
    }

    fn album_body(&self, uri: &str, window: &Window, cx: &mut Context<Self>) -> Div {
        let compact = self.compact();
        let side = if compact {
            ALBUM_ART_COMPACT
        } else {
            ALBUM_ART
        };
        let page = match self.pages.albums.get(uri) {
            Some(PageLoad::Ready(p)) => p.clone(),
            Some(PageLoad::Failed(e)) => {
                return self.page_error(e, Route::Album(uri.to_string()), window, cx);
            }
            _ => return self.page_skeleton_sized(false, true, side, true),
        };
        let art = cover_image(&self.images, &page.image, &page.name, side, R_LG, "disc-3");
        let names: Vec<&str> = page.artists.iter().map(|a| a.name.as_str()).collect();
        let artist_uris: Rc<Vec<String>> =
            Rc::new(page.artists.iter().map(|a| a.uri.clone()).collect());
        let this = cx.entity().downgrade();
        let uris = artist_uris.clone();
        let on_click: super::link_text::SpanHandler = Rc::new(move |i, _, cx| {
            let uri = uris.get(i).cloned();
            let _ = this.update(cx, |shell, cx| {
                if let Some(uri) = uri {
                    shell.navigate(Route::Artist(uri), cx);
                }
            });
        });
        let this = cx.entity().downgrade();
        let uris = artist_uris.clone();
        let on_hover: super::link_text::SpanHandler = Rc::new(move |i, _, cx| {
            let uri = uris.get(i).cloned();
            let _ = this.update(cx, |shell, _| {
                if let Some(uri) = uri {
                    shell.prefetch(uri);
                }
            });
        });
        // Başlık bloğu: ad (Display), sanatçılar (tıklanır), tür · yıl · şarkı
        // sayısı · süre, eylemler.
        let mut rest = vec![kind_label(page.kind).to_string()];
        if page.year > 0 {
            rest.push(page.year.to_string());
        }
        rest.push(tp!(Songs, page.tracks.len()));
        rest.push(fmt_total(page.total_ms));
        let artists_line = div()
            .flex()
            .min_w_0()
            .type_role(if compact { T_BODY } else { T_TITLE })
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(text())
            .child(
                link_line("album-artists", &names, on_click, Some(on_hover))
                    .flex()
                    .min_w_0(),
            );
        let meta = div()
            .flex()
            .min_w_0()
            .type_role(T_SECONDARY)
            .text_color(muted())
            .child(rest.join(" · "));
        let accent = self.accent().text;
        let list = format!("album:{uri}");
        let list_key = ListKey::Page(list.clone());
        let saved = self.album_saved(uri);
        let card = LibraryItem {
            id: CollectionId::Album(uri.to_string()),
            name: page.name.clone(),
            subtitle: names.join(", "),
            image: page.image.clone(),
        };
        let uri_owned = uri.to_string();
        let menu_target = CardTarget::Album {
            uri: uri.to_string(),
            item: Some(card.clone()),
        };
        let (more_probe, more_anchor) = anchor_probe();
        let actions = vec![
            primary_button("album-play", "play-fill", t!(Play), Look::Live, accent)
                .on_click(cx.listener(move |this, _, _, cx| this.play_in(&list_key, 0, cx)))
                .into_any_element(),
            icon_button_tinted(
                "album-save",
                if saved { "heart-fill" } else { "heart" },
                if saved { rgb(accent) } else { soft() },
                Look::Live,
                accent,
            )
            .tooltip(Tooltip::text(
                if saved {
                    t!(RemoveFromLibrary)
                } else {
                    t!(AddToLibrary)
                },
                None,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                let on = !this.album_saved(&uri_owned);
                this.set_album_saved(uri_owned.clone(), on, Some(card.clone()), cx);
            }))
            .into_any_element(),
            icon_button("album-more", "ellipsis", Look::Live, accent)
                .relative()
                .tooltip(Tooltip::text(t!(MoreTip), None))
                .child(more_probe)
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                    this.open_card_menu(
                        menu_target.clone(),
                        menu_below(more_anchor.get(), ev.position()),
                        cx,
                    );
                }))
                .into_any_element(),
        ];
        let header = div()
            .px(px(self.margin()))
            .pb(px(S6))
            .flex()
            .items_end()
            .gap(px(S5))
            .child(art)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .child(
                        div()
                            .flex()
                            .min_w_0()
                            .font_family(font())
                            .type_role(if compact { T_HEADLINE } else { T_DISPLAY })
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(text())
                            .child(
                                line_with_tip("page-heading", page.name.clone())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(artists_line)
                    .child(meta)
                    .child(
                        div()
                            .pt(px(S2))
                            .flex()
                            .items_center()
                            .gap(px(S3))
                            .children(actions),
                    ),
            );
        // Albüm sayfasından çalınca albüm "çalan liste" olur: satır vurgusu sırayla.
        let playing_ix = (self.state.queue.context.as_ref()
            == Some(&CollectionId::Album(uri.to_string())))
        .then_some(self.state.queue.index);
        let artists_text = names.join(", ");
        // Sanatçının diğer albümleri: sanatçı sayfası bellekteyse (albüm
        // gelince arkadan istenir, bkz. `store_album`).
        let more = page
            .artists
            .first()
            .and_then(|a| match self.pages.artists.get(&a.uri) {
                Some(PageLoad::Ready(p)) => Some((a.name.clone(), p.clone())),
                _ => None,
            })
            .map(|(name, p)| {
                let cards: Vec<CardSpec> = p
                    .albums
                    .iter()
                    .chain(&p.singles)
                    .filter(|c| c.uri != uri)
                    .map(album_card)
                    .collect();
                (name, cards)
            });
        let mut body = div()
            .flex()
            .flex_col()
            .child(header)
            .child(div().w_full().flex().flex_col().child(self.track_section(
                &list,
                None,
                &page.tracks,
                None,
                Some(&artists_text),
                playing_ix,
                cx,
            )))
            .when(!page.label.is_empty(), |el| {
                el.child(
                    div()
                        .px(px(self.margin()))
                        .pb(px(S6))
                        .type_role(T_LABEL)
                        .text_color(label())
                        .child(format!("© {}", page.label)),
                )
            });
        if let Some((name, cards)) = more {
            body = body.children(self.card_section(
                format!("{uri}#more"),
                &tf!(PageMoreFrom, name),
                cards,
                cx,
            ));
        }
        body
    }

    fn search_body(&self, query: &str, window: &Window, cx: &mut Context<Self>) -> Div {
        let Some(search) = self.pages.search.as_ref().filter(|s| s.query == query) else {
            return self.page_skeleton(false, true);
        };
        let results = match &search.results {
            PageLoad::Ready(r) => r.clone(),
            PageLoad::Failed(e) => {
                return self.page_error(e, Route::Search(query.to_string()), window, cx);
            }
            PageLoad::Loading => {
                let meta = div().child(t!(SearchSearching)).into_any_element();
                return div()
                    .flex()
                    .flex_col()
                    .child(self.page_header(None, &format!("“{query}”"), meta, Vec::new()))
                    .child(
                        div()
                            .pb(px(S6))
                            .child(skeleton_rows_at(false, self.margin())),
                    );
            }
        };
        let kind = search.kind;
        let total = results.total();
        let meta = div()
            .child(if total == 0 {
                t!(SearchNoResults).to_string()
            } else {
                tp!(SearchResults, total)
            })
            .into_any_element();
        let mut body = div().flex().flex_col().child(self.page_header(
            None,
            &format!("“{query}”"),
            meta,
            Vec::new(),
        ));
        // Hiç sonuç yoksa süzülecek bir şey de yok: çip sırası görünmez.
        if total == 0 {
            return body;
        }
        body = body.child(self.search_chips(kind, window, cx));
        if kind.count(&results) == 0 {
            return body.child(self.search_empty_kind(kind));
        }

        use super::search_page::{Section, Show, episode_cards, podcast_cards, profile_cards};
        // Şarkılar: "Hepsi"de ilk 5 ve "Tümünü göster"; Şarkılar çipinde hepsi.
        match kind.show(Section::Songs) {
            Show::Hidden => {}
            Show::Few if !results.tracks.is_empty() => {
                body = body.child(self.track_section(
                    &format!("search:{query}"),
                    Some(t!(SectionSongs)),
                    &results.tracks,
                    Some(SEARCH_FEW),
                    None,
                    None,
                    cx,
                ));
            }
            Show::Full if !results.tracks.is_empty() => {
                body = body.child(self.track_section(
                    &format!("search:{query}"),
                    None,
                    &results.tracks,
                    None,
                    None,
                    None,
                    cx,
                ));
            }
            Show::Few | Show::Full => {}
        }
        let hit_card = |h: &crate::engine::SearchHit,
                        round: bool,
                        target: CardTarget,
                        icon: &'static str| CardSpec {
            key: h.uri.clone(),
            title: h.name.clone(),
            subtitle: crate::i18n::canon::localize(&h.subtitle).into_owned(),
            art: Art::Image {
                url: h.image.clone(),
                icon,
            },
            round,
            target,
        };
        let artists: Vec<CardSpec> = results
            .artists
            .iter()
            .map(|h| hit_card(h, true, CardTarget::Artist(h.uri.clone()), "user"))
            .collect();
        let albums: Vec<CardSpec> = results
            .albums
            .iter()
            .map(|h| {
                hit_card(
                    h,
                    false,
                    CardTarget::Album {
                        uri: h.uri.clone(),
                        item: None,
                    },
                    "disc-3",
                )
            })
            .collect();
        let playlists: Vec<CardSpec> = results
            .playlists
            .iter()
            .map(|h| {
                hit_card(
                    h,
                    false,
                    CardTarget::Collection {
                        id: CollectionId::Playlist(h.uri.clone()),
                        title: h.name.clone(),
                    },
                    "list-music",
                )
            })
            .collect();
        let titled = |title: &'static str| kind.titled().then_some(title);
        body = body
            .children(self.kind_section(
                kind,
                Section::Artists,
                format!("search:{query}#artists"),
                t!(SectionArtists),
                artists,
                cx,
            ))
            .children(self.kind_section(
                kind,
                Section::Albums,
                format!("search:{query}#albums"),
                t!(KindAlbums),
                albums,
                cx,
            ))
            .children(self.kind_section(
                kind,
                Section::Playlists,
                format!("search:{query}#playlists"),
                t!(KindLists),
                playlists,
                cx,
            ));
        // Podcast, bölüm ve profil: uygulamada sayfa yok, kart Spotify'da açılır.
        body.children(self.ext_section(
            format!("search:{query}#podcasts"),
            titled(t!(SearchChipPodcasts)),
            podcast_cards(&results.podcasts),
            kind.show(Section::Podcasts),
            cx,
        ))
        .children(self.ext_section(
            format!("search:{query}#episodes"),
            titled(t!(SearchSectionEpisodes)),
            episode_cards(&results.episodes),
            kind.show(Section::Episodes),
            cx,
        ))
        .children(self.ext_section(
            format!("search:{query}#profiles"),
            titled(t!(SearchChipProfiles)),
            profile_cards(&results.profiles),
            kind.show(Section::Profiles),
            cx,
        ))
    }

    // i18n-skip-start: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez
    /// Bileşen vitrini (`OPENIFY_GALLERY=1`): gezinme düğmeleri, sayfa
    /// kartı, başlık bloğu, iskelet.
    pub(super) fn pages_gallery(&self, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let nav = {
            let mut row = div().flex().items_center().gap(px(S4));
            for look in Look::ALL {
                row = row.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(S1))
                        .child(
                            div()
                                .flex()
                                .child(
                                    icon_button(
                                        ("g-nav-back", look as usize),
                                        "arrow-left",
                                        look,
                                        accent,
                                    )
                                    .tooltip(Tooltip::text("Geri", Some("Alt ←"))),
                                )
                                .child(
                                    icon_button(
                                        ("g-nav-home", look as usize),
                                        "house",
                                        look,
                                        accent,
                                    )
                                    .tooltip(Tooltip::text("Ana sayfa", Some("Alt Home"))),
                                ),
                        )
                        .child(
                            div()
                                .type_role(T_LABEL)
                                .text_color(label())
                                .child(look.name()),
                        ),
                );
            }
            row
        };
        let (_, width) = self.grid();
        let sample = |n: usize, round: bool| CardSpec {
            key: format!("g-card-{n}"),
            title: if round {
                "Sezen Aksu".into()
            } else {
                "Deniz ve Mehtap".into()
            },
            subtitle: if round {
                "Sanatçı".into()
            } else {
                "1999 · Albüm".into()
            },
            art: Art::Image {
                url: String::new(),
                icon: if round { "user" } else { "disc-3" },
            },
            round,
            target: CardTarget::Artist(String::new()),
        };
        let cards = div()
            .flex()
            .gap(px(CARD_GAP))
            .child(self.page_card(&sample(1, false), width, cx))
            .child(self.page_card(&sample(2, true), width, cx));
        let meta = div()
            .child("Sanatçı · 12 albüm · 30 tekli ve EP")
            .into_any_element();
        let header = self.page_header(
            Some(cover_image(
                &self.images,
                "",
                "",
                self.header_art_side(),
                self.header_art_side() / 2.,
                "user",
            )),
            "Sezen Aksu",
            meta,
            vec![
                primary_button("g-page-play", "play-fill", "Çal", Look::Live, accent)
                    .into_any_element(),
            ],
        );
        div()
            .flex()
            .flex_col()
            .gap(px(S5))
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Gezinme ve sayfalar (Adım 12)"),
            )
            .child(
                div()
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child("Başlık çubuğu: geri / ileri / ana sayfa, beş durum"),
            )
            .child(nav)
            .child(
                div()
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child("Sayfa başlık bloğu"),
            )
            .child(div().mx(px(-self.margin())).child(header))
            .child(
                div()
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child("Sayfa kartı (albüm, sanatçı); üzerine gelince ton katmanı"),
            )
            .child(cards)
            .child(
                div()
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child("Yükleniyor: sayfa iskeleti (aynı ölçüler)"),
            )
            .child(
                div()
                    .mx(px(-self.margin()))
                    .h(px(420.))
                    .overflow_hidden()
                    .child(self.page_skeleton(true, false)),
            )
            .into_any_element()
    }
    // i18n-skip-end: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez

    /// Satırdaki sanatçı adları: her biri tıklanabilir (sayfasına gider).
    pub(super) fn artist_links(
        &self,
        id: ElementId,
        track: &TrackRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let names = split_names(&track.artists);
        let row = track.clone();
        let this = cx.entity().downgrade();
        let on_click: super::link_text::SpanHandler = Rc::new(move |i, _, cx| {
            let _ = this.update(cx, |shell, cx| shell.open_artist_of(&row, i, cx));
        });
        let row = track.clone();
        let this = cx.entity().downgrade();
        let on_hover: super::link_text::SpanHandler = Rc::new(move |i, _, cx| {
            let _ = this.update(cx, |shell, _| shell.prefetch_artist_of(&row, i));
        });
        self.want_links(&track.uri);
        link_line(id, &names, on_click, Some(on_hover)).into_any_element()
    }

    /// Satırdaki albüm adı: tıklanabilir.
    pub(super) fn album_link(
        &self,
        id: ElementId,
        track: &TrackRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = track.clone();
        let this = cx.entity().downgrade();
        let on_click: super::link_text::SpanHandler = Rc::new(move |_, _, cx| {
            let _ = this.update(cx, |shell, cx| shell.open_album_of(&row, cx));
        });
        let row = track.clone();
        let this = cx.entity().downgrade();
        let on_hover: super::link_text::SpanHandler = Rc::new(move |_, _, cx| {
            let _ = this.update(cx, |shell, _| shell.prefetch_album_of(&row));
        });
        link_line(id, &[track.album.as_str()], on_click, Some(on_hover)).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kapaksız iskeletin başlık çubuğu ile gerçek başlık aynı kutuda:
    /// üst boşluk, başlık satırı, aradaki boşluk, meta satırı.
    #[test]
    fn plain_skeleton_header_matches_page_header_offsets() {
        let wide = plain_header_geometry(false);
        assert_eq!(wide.title_top, S7);
        assert_eq!(wide.title_h, T_DISPLAY.line);
        assert_eq!(wide.meta_top, S7 + T_DISPLAY.line + S2);
        assert_eq!(wide.meta_h, T_BODY.line);
        assert_eq!(wide.total, S7 + T_DISPLAY.line + S2 + T_BODY.line + S6);

        let narrow = plain_header_geometry(true);
        assert_eq!(narrow.title_h, T_HEADLINE.line);
        assert_eq!(narrow.total, S7 + T_HEADLINE.line + S2 + T_BODY.line + S6);
        // Kapaksız başlık kapaklıdan (S5) daha aşağıdan başlar; iskelet de.
        assert!(PLAIN_HEADER_TOP > S5);
    }

    /// Kapak karesi yok: bölüm başlığı satırı gerçek `section_title` kadar
    /// (48 px) ve başlık satırı yazı rolünden.
    #[test]
    fn skeleton_section_row_matches_section_title() {
        assert_eq!(S7, 48.);
        assert_eq!(T_TITLE.line, 24.);
    }
}
