//! Arama yüzeyi (Ctrl+K ve başlık çubuğundaki arama hapı).
//!
//! Bir komut paleti değil, bir müzik arama yüzeyi (13 Kasım: "bildiğin kodlama
//! arayüzü gibi, gereksiz teknik"): başlık çubuğundaki hapın altına açılan
//! büyük, cam bir yüzey. Kutu boşken yalnız müzik: son aramalar (çip), "Tekrar
//! dinle" kapak sırası ve (önbellekte varsa) renkli tür çipleri; hiçbiri yoksa
//! kısa bir davet cümlesi. Yazınca solda büyük "En iyi sonuç" kartı, sağında
//! "Şarkılar" (4 satır); altında "Sanatçılar" (yuvarlak) ve "Albümler ve
//! listeler" (kare kapak) sıraları. Eylemler (sonraki şarkı, sessize al,
//! Keşfet...) varsayılanda yok; yazılan bir eylemin adıyla eşleşirse en altta
//! tek bir "Eylemler" bölümünde çıkar. Spotify cevabı gelene dek iskelet
//! (soluk kutular) görünür; en altta sabit "“…” için tüm sonuçlar" bağlantısı.
//!
//! Klavye: yazı ekrandaki girişe gider; ↑ ↓ ← → ızgarada gezer (kart sıralarında
//! sağ / sol, bölümler arasında yukarı / aşağı), Tab sıradaki, Enter açar ya da
//! çalar, Esc ve Ctrl+K kapatır, Ctrl+Enter bütün sonuçlar, Ctrl+Backspace
//! kelime siler. Seçili öğenin etrafında vurgu rengi halka.

use std::{sync::Arc, time::Duration};

use gpui::{
    AnimationExt, AnyElement, Context, ElementId, KeyDownEvent, MouseButton, ScrollHandle,
    SharedString, Task, Window, deferred, div, linear_color_stop, linear_gradient, prelude::*, px,
    rgb,
};

use super::{
    Shell, cover_image, fuzzy, history::Route, library::Loadable, manage::CardTarget,
    player::fmt_time, text::line_with_tip, theme::*,
};
use crate::engine::{CollectionId, Command, SearchResults, TrackRow};
use crate::{t, tf};

const SPOTIFY_DELAY: Duration = Duration::from_millis(300);
/// Yüzeyin eni ve en büyük boyu (her yerden açılan palet penceresi de buna göre).
pub const PANEL_WIDTH: f32 = 720.;
pub const PANEL_MAX_HEIGHT: f32 = 700.;
/// Spotify'da aramak için en az bu kadar harf.
const MIN_SPOTIFY_QUERY: usize = 2;

const MAX_SONGS: usize = 4;
const MAX_TILES: usize = 6;
const MAX_ACTIONS: usize = 3;
const MAX_LOCAL_SONGS: usize = 2;
const MAX_LOCAL_COLLECTIONS: usize = 3;
/// Kutu boşken: son aramalar, son çalınanlar, tür çipleri.
const MAX_RECENT_SEARCHES: usize = 6;
const MAX_GENRES: usize = 8;

/// Giriş hapının boyu (başlık çubuğundaki hapla aynı dil, biraz büyük).
const INPUT_HEIGHT: f32 = S7 - S2;
/// Şarkı satırı ve kapağı.
const ROW_HEIGHT: f32 = S7;
const ROW_COVER: f32 = S7 - S2;
/// En iyi sonuç kartı: şarkı satırlarının toplam boyu kadar (4 × 48).
const BEST_WIDTH: f32 = 248.;
const BEST_HEIGHT: f32 = ROW_HEIGHT * MAX_SONGS as f32;
const BEST_ART: f32 = S8 + S5;
/// Kare / yuvarlak kart sırasındaki görsel.
const TILE: f32 = 84.;
/// Çipin boyu (son arama, tür).
const CHIP_HEIGHT: f32 = S6;
const GENRE_HEIGHT: f32 = S6 + S2;
/// Çal düğmesinin çapı (kartta, görselin köşesinde).
const PLAY_BIG: f32 = S7 - S2;
const PLAY_SMALL: f32 = S6 + S0;
/// Sabit alt bağlantının boyu.
const FOOTER_HEIGHT: f32 = S7 - S2;
/// Komut adına yazılan en az şu kadar harf eşleşirse eylem çıkar.
const MIN_ACTION_QUERY: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaletteCommand {
    TogglePlay,
    Next,
    Previous,
    ToggleMute,
    RefreshLibrary,
    Shortcuts,
    Logout,
    /// Ana sayfa (Adım 12).
    Home,
    /// Keşfet ve Kütüphane sekmeleri.
    Explore,
    Library,
    /// Yeni, boş liste (Adım 13).
    NewPlaylist,
    /// Ayarlar sayfası (Adım 15).
    Settings,
    /// Son çalınanlar sayfası.
    Recent,
    /// Kütüphane araçları sayfası (yinelenenler, birleştir / böl, sıralama, yedek, istatistik).
    Tools,
    /// Gizli oturumu aç / kapat (bkz. `listening`).
    PrivateSession,
    /// Çalan şarkının radyosunu başlat (bkz. `listening`).
    StartRadio,
}

/// Eylem, adı, açıklaması. Çal ve sessize al eylemlerinin adı o anki duruma
/// göre değişir (bkz. `command_state`).
fn commands() -> [(PaletteCommand, &'static str, &'static str); 16] {
    [
        (PaletteCommand::TogglePlay, t!(CmdTogglePlay), ""),
        (PaletteCommand::Next, t!(CmdNext), ""),
        (PaletteCommand::Previous, t!(CmdPrevious), ""),
        (PaletteCommand::ToggleMute, t!(CmdToggleMute), ""),
        (
            PaletteCommand::RefreshLibrary,
            t!(CmdRefreshLibrary),
            t!(CmdRefreshLibraryDesc),
        ),
        (PaletteCommand::Home, t!(NavHome), t!(CmdHomeDesc)),
        (PaletteCommand::Explore, t!(NavExplore), t!(CmdExploreDesc)),
        (PaletteCommand::Library, t!(NavLibrary), t!(CmdLibraryDesc)),
        (
            PaletteCommand::Recent,
            t!(RecentPageTitle),
            t!(CmdRecentDesc),
        ),
        (
            PaletteCommand::NewPlaylist,
            t!(NewPlaylist),
            t!(CmdNewPlaylistDesc),
        ),
        (PaletteCommand::Tools, t!(ToolsTitle), t!(CmdToolsDesc)),
        (
            PaletteCommand::StartRadio,
            t!(CmdStartRadio),
            t!(CmdStartRadioDesc),
        ),
        (
            PaletteCommand::PrivateSession,
            t!(CmdPrivateSession),
            t!(CmdPrivateSessionDesc),
        ),
        (PaletteCommand::Settings, t!(Settings), t!(CmdSettingsDesc)),
        (
            PaletteCommand::Shortcuts,
            t!(Shortcuts),
            t!(CmdShortcutsDesc),
        ),
        (PaletteCommand::Logout, t!(SignOut), t!(CmdSignOutDesc)),
    ]
}

/// Arama kutusunun yer tutucusu: başlık çubuğundaki hap ve yüzeydeki giriş
/// aynı metni gösterir (ikisi aynı kutu).
pub fn search_placeholder() -> &'static str {
    t!(SearchPlaceholder)
}

#[derive(Debug, Clone)]
pub enum Action {
    Command(PaletteCommand),
    OpenCollection {
        id: CollectionId,
        title: String,
    },
    PlayCollection {
        id: CollectionId,
        title: String,
    },
    PlayTrack(String),
    /// Sanatçı sayfası, albüm (kütüphanedeyse liste, değilse sayfa), tam
    /// arama sonuçları (Adım 12) ve her birinin çalma karşılığı.
    OpenArtist(String),
    PlayArtist(String),
    OpenAlbum(String),
    PlayAlbum(String),
    /// Tür sayfası (Keşfet).
    OpenCategory(String),
    AllResults(String),
    /// Son aramalardan biri: yazıya dönüşür, sonuçlar gelir.
    Search(String),
}

/// Öğenin görseli.
#[derive(Clone)]
pub enum Art {
    /// Kapak resmi (şarkı, albüm, liste).
    Cover { url: String, icon: &'static str },
    /// Yuvarlak portre (sanatçı).
    Round { url: String },
    /// Kütüphanedeki liste (beğenilenler için kalp).
    Collection { id: CollectionId, url: String },
    /// Eylem: ton karosunda simge.
    Glyph(&'static str),
    /// Çip ve düz satırlar: görsel yok.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// "En iyi sonuç" kartı.
    Best,
    Song,
    /// Sanatçı, albüm ve liste kartları (görselin altında ad).
    Tile,
    /// Son arama çipi, tür çipi.
    Chip,
    Genre,
    Action,
    /// Sabit alt bağlantı ("tüm sonuçlar").
    More,
}

/// Izgaradaki yer: satır, sütun ve kaç satıra yayıldığı (en iyi sonuç kartı
/// şarkı satırlarının yanında, hepsini kaplar).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
    pub span: usize,
}

pub struct Item {
    pub kind: Kind,
    pub art: Art,
    /// Komutun sabit adı (sadeleşmiş): başlık duruma göre değişebilir ("Sessize
    /// al" / "Sesi aç"), yazılanın adı tutup tutmadığı bununla bakılır.
    pub name_key: String,
    pub title: String,
    pub subtitle: String,
    /// Sağda soluk gösterilen ek bilgi (süre).
    pub detail: String,
    /// Kartın türü etiketi ("Sanatçı", "Albüm", "Şarkı", "Liste").
    pub tag: &'static str,
    pub action: Action,
    /// Görselin üstündeki yuvarlak çal düğmesinin eylemi (varsa).
    pub play: Option<Action>,
    /// Aynı öğeyi iki kez göstermemek için (Spotify adresi).
    pub dedupe: String,
    /// Tür çipinin rengi (0xRRGGBB).
    pub color: Option<u32>,
    /// Geri dönüşü zor işlem (çıkış): tehlike yazısıyla, onay ister.
    pub danger: bool,
    /// Şu an yapılamaz (örneğin sıradaki şarkı yok): soluk, seçilmez.
    pub disabled: bool,
    pub cell: Cell,
}

impl Item {
    fn new(kind: Kind, title: impl Into<String>, action: Action) -> Self {
        Self {
            kind,
            art: Art::None,
            name_key: String::new(),
            title: title.into(),
            subtitle: String::new(),
            detail: String::new(),
            tag: "",
            action,
            play: None,
            dedupe: String::new(),
            color: None,
            danger: false,
            disabled: false,
            cell: Cell::default(),
        }
    }

    fn art(mut self, art: Art) -> Self {
        self.art = art;
        self
    }

    fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    fn tag(mut self, tag: &'static str) -> Self {
        self.tag = tag;
        self
    }

    fn play(mut self, play: Action) -> Self {
        self.play = Some(play);
        self
    }

    fn dedupe(mut self, key: impl Into<String>) -> Self {
        self.dedupe = key.into();
        self
    }
}

/// Yüzeyin bölümleri (yukarıdan aşağı); öğelere `items` sırasıyla başvurur.
#[derive(Debug, Clone)]
pub enum Block {
    /// Solda en iyi sonuç, sağda şarkılar.
    Top {
        best: Option<usize>,
        songs: Vec<usize>,
    },
    Chips {
        title: &'static str,
        items: Vec<usize>,
    },
    Tiles {
        title: &'static str,
        items: Vec<usize>,
    },
    Actions {
        items: Vec<usize>,
    },
    /// Spotify cevabı beklenirken soluk yer tutucular.
    Skeleton(Skel),
    /// Sakin, tek satırlık bilgi (davet, sonuç yok, Spotify'a ulaşılamadı).
    Note {
        icon: &'static str,
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skel {
    Top,
    Round,
    Square,
}

pub enum Spotify {
    Idle,
    Waiting,
    Loading,
    Done(Arc<SearchResults>),
    Failed(String),
}

impl Spotify {
    fn pending(&self) -> bool {
        matches!(self, Spotify::Waiting | Spotify::Loading)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub items: Vec<Item>,
    pub blocks: Vec<Block>,
    /// Sabit alt bağlantının sırası (`items` içinde; sorgu yokken yok).
    footer: Option<usize>,
    pub spotify: Spotify,
    /// Son gönderilen Spotify aramasının sıra numarası.
    seq: u64,
    debounce: Option<Task<()>>,
    scroll: ScrollHandle,
    /// Yerel arama dizini: kütüphanedeki şarkılar ve sadeleştirilmiş metinleri.
    tracks: Arc<Vec<TrackRow>>,
    track_keys: Vec<String>,
    /// Onay bekleyen tehlikeli öğe: ikinci Enter çalıştırır.
    confirm: Option<usize>,
    /// Kullanıcı seçimi oklarla taşıdı: sonuçlar güncellenince seçim
    /// kendiliğinden en iyi eşleşmeye atlamasın.
    moved: bool,
    /// Palet ana pencerede değil, her yerden açılan kendi penceresinde
    /// (Ctrl+Shift+Boşluk, bkz. `popups.rs`): ana pencere onu çizmez.
    pub detached: bool,
    /// Son seçilen öğe ana pencereyi gerektiriyor (liste açıldı): her
    /// yerden açılan palet kapanınca ana pencere öne gelir.
    pub wants_window: bool,
    /// Son aramalar (en yenisi önde); ayarlarda saklanır.
    pub recent_searches: Vec<String>,
    /// Kaçıncı açılış: açılma geçişi her açılışta yeniden oynasın (her
    /// yerden açılan paletin penceresi kapanınca silinmiyor, gizleniyor).
    opened: usize,
}

impl PaletteState {
    pub fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            items: Vec::new(),
            blocks: Vec::new(),
            footer: None,
            spotify: Spotify::Idle,
            seq: 0,
            debounce: None,
            scroll: ScrollHandle::new(),
            tracks: Arc::default(),
            track_keys: Vec::new(),
            confirm: None,
            moved: false,
            detached: false,
            wants_window: false,
            recent_searches: Vec::new(),
            opened: 0,
        }
    }

    /// Bir arama yapıldı (sonuç açıldı): son aramaların başına.
    pub fn note_search(&mut self, query: &str) {
        let q = query.trim();
        if q.chars().count() < MIN_SPOTIFY_QUERY {
            return;
        }
        self.recent_searches
            .retain(|s| !s.eq_ignore_ascii_case(q) && s != q);
        self.recent_searches.insert(0, q.to_string());
        self.recent_searches
            .truncate(crate::settings::RECENT_SEARCHES_MAX);
    }

    pub fn set_index(&mut self, tracks: Arc<Vec<TrackRow>>, keys: Vec<String>) {
        self.track_keys = keys;
        self.tracks = tracks;
    }

    /// Seçili öğenin etrafındaki halka çizilsin mi: yazı varsa ya da kullanıcı
    /// oklarla gezdiyse (kutu boşken ilk öğe kendiliğinden seçili görünmesin).
    fn selection_visible(&self) -> bool {
        self.moved || !self.query.trim().is_empty()
    }
}

/// Şarkıların arama anahtarları (Türkçe harfler sadeleşmiş ad, sanatçı,
/// albüm). Binlerce şarkıda ~0,1 sn sürüyor: ana iş parçacığında değil,
/// arka planda hesaplanır.
pub fn index_keys(tracks: &[TrackRow]) -> Vec<String> {
    tracks
        .iter()
        .map(|t| fuzzy::fold(&format!("{} {} {}", t.name, t.artists, t.album)))
        .collect()
}

// ── Izgarada gezinme (saf; testler Shell'siz çalışır) ─────────────────────

impl Cell {
    fn covers(&self, row: usize) -> bool {
        row >= self.row && row < self.row + self.span.max(1)
    }
}

/// Seçimi `dir` yönünde taşır; gidilecek yer yoksa `None`. Sağ / sol aynı
/// satırda yan sütuna (en iyi sonuç kartından şarkıya, şarkıdan karta); yukarı
/// / aşağı bir alttaki satırda en yakın sütuna geçer. Yayılan kart (en iyi
/// sonuç) aşağı giderken alt kenarından, yukarı giderken üst kenarından çıkar.
/// Sütunlar ayrı öğeler olduğundan satır sonunda sarılmaz; yukarı / aşağı
/// baştan sona sarar. Kullanılamayan öğeler atlanır.
pub fn neighbor(items: &[Item], from: usize, dir: Dir) -> Option<usize> {
    let cur = items.get(from)?.cell;
    let usable = |j: usize| j != from && !items[j].disabled && items[j].kind != Kind::More;
    match dir {
        Dir::Left | Dir::Right => items
            .iter()
            .enumerate()
            .filter(|(j, it)| {
                usable(*j)
                    && it.cell.covers(cur.row)
                    && match dir {
                        Dir::Left => it.cell.col < cur.col,
                        _ => it.cell.col > cur.col,
                    }
            })
            .min_by_key(|(_, it)| match dir {
                Dir::Left => usize::MAX - it.cell.col,
                _ => it.cell.col,
            })
            .map(|(j, _)| j),
        Dir::Up | Dir::Down => {
            let rows: Vec<usize> = {
                let mut r: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| usable(*j) || *j == from)
                    .flat_map(|(_, it)| it.cell.row..it.cell.row + it.cell.span.max(1))
                    .collect();
                r.sort_unstable();
                r.dedup();
                r
            };
            let target = if dir == Dir::Down {
                let edge = cur.row + cur.span.max(1);
                rows.iter()
                    .copied()
                    .find(|r| *r >= edge)
                    .or(rows.first().copied())
            } else {
                rows.iter()
                    .copied()
                    .rev()
                    .find(|r| *r < cur.row)
                    .or(rows.last().copied())
            }?;
            items
                .iter()
                .enumerate()
                .filter(|(j, it)| usable(*j) && it.cell.covers(target))
                .min_by_key(|(_, it)| (it.cell.col.abs_diff(cur.col), it.cell.col))
                .map(|(j, _)| j)
        }
    }
}

/// Okunma sırasında bir sonraki / önceki kullanılabilir öğe (Tab).
fn step_linear(items: &[Item], from: usize, delta: isize) -> usize {
    let len = items.len();
    let mut next = from;
    for _ in 0..len {
        next = (next as isize + delta).rem_euclid(len as isize) as usize;
        if !items[next].disabled {
            break;
        }
    }
    next
}

/// Hangi öğe "en iyi sonuç" olur: puan × tür ağırlığı; yazılanla başlayan ya
/// da aynen eşleşen ad öne geçer. Sanatçı ve albüm şarkıdan önce gelir
/// (şarkının resmi küçük, kartta büyük görünür).
fn pick_best(cands: &[(HitKind, u32, MatchLevel)]) -> Option<usize> {
    cands
        .iter()
        .enumerate()
        .filter(|(_, (_, score, _))| *score > 0)
        .max_by(|a, b| {
            let rank = |c: &(HitKind, u32, MatchLevel)| c.1 as f32 * c.0.weight() * c.2.weight();
            rank(a.1)
                .partial_cmp(&rank(b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
                // Eşitlikte listede önce gelen.
                .then(b.0.cmp(&a.0))
        })
        .map(|(i, _)| i)
}

/// En iyi sonuç adayının türü (ağırlık için).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitKind {
    Artist,
    Album,
    Playlist,
    Song,
}

impl HitKind {
    fn weight(self) -> f32 {
        match self {
            HitKind::Artist => 1.25,
            HitKind::Album => 1.1,
            HitKind::Playlist => 1.0,
            HitKind::Song => 0.9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchLevel {
    Fuzzy,
    Prefix,
    Exact,
}

impl MatchLevel {
    fn weight(self) -> f32 {
        match self {
            MatchLevel::Fuzzy => 1.0,
            MatchLevel::Prefix => 1.6,
            MatchLevel::Exact => 2.4,
        }
    }

    fn of(name: &str, typed: &str) -> Self {
        let name = fuzzy::fold(name);
        if name == typed {
            MatchLevel::Exact
        } else if name.starts_with(typed) {
            MatchLevel::Prefix
        } else {
            MatchLevel::Fuzzy
        }
    }
}

/// Eylem, yazılanla eşleşiyor mu: adın içinde geçiyorsa (en az 2 harf); ya da
/// açıklamasında geçiyorsa (en az 4 harf).
fn action_matches(typed: &str, name_key: &str, desc_key: &str) -> bool {
    let n = typed.chars().count();
    (n >= MIN_ACTION_QUERY && name_key.contains(typed)) || (n >= 4 && desc_key.contains(typed))
}

/// Izgara yerlerini bölümlerden yazar: bölümler yukarıdan aşağı satırlar.
fn assign_cells(items: &mut [Item], blocks: &[Block], footer: Option<usize>) {
    let mut row = 0;
    for block in blocks {
        match block {
            Block::Top { best, songs } => {
                let span = songs.len().max(1);
                let song_col = usize::from(best.is_some());
                if let Some(b) = best {
                    items[*b].cell = Cell { row, col: 0, span };
                }
                for (k, s) in songs.iter().enumerate() {
                    items[*s].cell = Cell {
                        row: row + k,
                        col: song_col,
                        span: 1,
                    };
                }
                row += span;
            }
            Block::Chips { items: ix, .. } | Block::Tiles { items: ix, .. } => {
                for (c, i) in ix.iter().enumerate() {
                    items[*i].cell = Cell {
                        row,
                        col: c,
                        span: 1,
                    };
                }
                row += 1;
            }
            Block::Actions { items: ix } => {
                for i in ix {
                    items[*i].cell = Cell {
                        row,
                        col: 0,
                        span: 1,
                    };
                    row += 1;
                }
            }
            Block::Skeleton(_) | Block::Note { .. } => {}
        }
    }
    if let Some(f) = footer {
        items[f].cell = Cell {
            row,
            col: 0,
            span: 1,
        };
    }
}

impl Shell {
    pub(super) fn open_palette(&mut self, cx: &mut Context<Self>) {
        // Açıkken yeniden açma: yazılanı ve seçimi sıfırlardı.
        if self.palette.open {
            return;
        }
        self.refocus_anchor = true;
        self.palette.open = true;
        self.palette.detached = false;
        self.palette.wants_window = false;
        self.palette.opened += 1;
        self.palette.query.clear();
        self.palette.moved = false;
        self.palette.spotify = Spotify::Idle;
        self.palette.debounce = None;
        self.recompute_palette();
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette.open = false;
        self.palette.confirm = None;
        self.palette.debounce = None;
        cx.notify();
    }

    pub(super) fn on_search_results(
        &mut self,
        seq: u64,
        results: Result<Arc<SearchResults>, String>,
    ) {
        if seq != self.palette.seq {
            return; // Eski bir sorgunun sonucu.
        }
        self.palette.spotify = match results {
            Ok(r) => Spotify::Done(r),
            Err(e) => Spotify::Failed(e),
        };
        self.recompute_palette();
    }

    pub(super) fn on_query_changed(&mut self, cx: &mut Context<Self>) {
        self.palette.selected = 0;
        self.palette.moved = false;
        self.palette.spotify = Spotify::Idle;
        self.palette.debounce = None;

        let query = self.palette.query.trim().to_string();
        if query.chars().count() >= MIN_SPOTIFY_QUERY {
            self.palette.spotify = Spotify::Waiting;
            self.palette.debounce = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SPOTIFY_DELAY).await;
                let _ = this.update(cx, |shell, cx| {
                    shell.palette.seq += 1;
                    shell.palette.spotify = Spotify::Loading;
                    // Ekran sınaması (`OPENIFY_TEST_SEARCH_JSON`): kayıtlı yanıt, Spotify'a gitmeden.
                    match crate::engine::test_palette_search() {
                        crate::engine::TestSearch::Results(saved) => {
                            shell.on_search_results(shell.palette.seq, Ok(Arc::new(saved)));
                        }
                        // İskelet / hata görüntüleri için: cevap hiç gelmez ya da hata gelir.
                        crate::engine::TestSearch::Hold => {}
                        crate::engine::TestSearch::Fail => {
                            shell.on_search_results(shell.palette.seq, Err("offline".to_string()))
                        }
                        crate::engine::TestSearch::Live => {
                            shell.engine.send(Command::Search {
                                seq: shell.palette.seq,
                                query,
                            });
                        }
                    }
                    cx.notify();
                });
            }));
        }
        self.recompute_palette();
        cx.notify();
    }

    /// Sorguya göre bölümleri ve öğeleri baştan kurar.
    pub(super) fn recompute_palette(&mut self) {
        self.palette.confirm = None;
        let query = self.palette.query.clone();
        let (mut items, blocks, footer) = match fuzzy::Fuzzy::new(&query) {
            None => {
                let (items, blocks) = self.idle_view();
                (items, blocks, None)
            }
            Some(f) => self.result_view(&query, f),
        };
        let mut footer_ix = None;
        if let Some(more) = footer {
            footer_ix = Some(items.len());
            items.push(more);
        }
        assign_cells(&mut items, &blocks, footer_ix);

        // Seçim: adı yazılan eylem (en az 3 harf, adın başı: "sessize" yazıp
        // Enter sessize alır), yoksa en iyi sonuç, yoksa ilk kullanılabilir.
        let typed = fuzzy::fold(query.trim());
        let named = (typed.chars().count() >= 3)
            .then(|| {
                items.iter().position(|i| {
                    i.kind == Kind::Action && !i.disabled && i.name_key.starts_with(&typed)
                })
            })
            .flatten();
        let first_usable = items
            .iter()
            .position(|i| !i.disabled && i.kind != Kind::More);
        let best = items.iter().position(|i| i.kind == Kind::Best);
        let default = named.or(best).or(first_usable).unwrap_or(0);

        self.palette.items = items;
        self.palette.blocks = blocks;
        self.palette.footer = footer_ix;
        if !self.palette.moved {
            self.palette.selected = default;
        }
        self.palette.selected = self
            .palette
            .selected
            .min(self.palette.items.len().saturating_sub(1));
        // Seçim kullanılamayan bir eylemde durmasın.
        if self
            .palette
            .items
            .get(self.palette.selected)
            .is_some_and(|i| i.disabled)
        {
            self.palette.selected = first_usable.unwrap_or(0);
        }
        self.scroll_to_selected();
    }

    /// Kutu boşken: son aramalar, son çalınanlar, tür çipleri (veri varsa).
    fn idle_view(&self) -> (Vec<Item>, Vec<Block>) {
        let mut items = Vec::new();
        let mut blocks = Vec::new();

        let mut searches = Vec::new();
        for q in self
            .palette
            .recent_searches
            .iter()
            .take(MAX_RECENT_SEARCHES)
        {
            searches.push(items.len());
            items.push(Item::new(Kind::Chip, q.clone(), Action::Search(q.clone())));
        }
        if !searches.is_empty() {
            blocks.push(Block::Chips {
                title: t!(PaletteRecentSearches),
                items: searches,
            });
        }

        if let Loadable::Loaded(library) = &self.library.library {
            let mut tiles = Vec::new();
            for it in crate::engine::home(library, &self.recent, MAX_TILES).recent {
                tiles.push(items.len());
                items.push(collection_tile(&it));
            }
            if !tiles.is_empty() {
                blocks.push(Block::Tiles {
                    title: t!(PaletteListenAgain),
                    items: tiles,
                });
            }
        }

        if let Some(list) = self.pages.explore.categories() {
            let mut genres = Vec::new();
            for cat in list.items.iter().take(MAX_GENRES) {
                genres.push(items.len());
                let mut item = Item::new(
                    Kind::Genre,
                    cat.name.clone(),
                    Action::OpenCategory(cat.uri.clone()),
                );
                item.color = Some(super::explore::tile_color(&cat.color));
                items.push(item);
            }
            if !genres.is_empty() {
                blocks.push(Block::Chips {
                    title: t!(PaletteBrowseGenres),
                    items: genres,
                });
            }
        }

        if blocks.is_empty() {
            blocks.push(Block::Note {
                icon: "search",
                text: t!(PaletteInvite).to_string(),
            });
        }
        (items, blocks)
    }

    /// Yazılınca: en iyi sonuç + şarkılar, sanatçılar, albümler ve listeler,
    /// (eşleşen) eylemler ve alt bağlantı. Kütüphane yerelde anında; Spotify
    /// cevabı gelene dek eksik bölümlerin yerinde iskelet.
    fn result_view(
        &self,
        query: &str,
        mut f: fuzzy::Fuzzy,
    ) -> (Vec<Item>, Vec<Block>, Option<Item>) {
        let typed = fuzzy::fold(query.trim());
        let spotify = match &self.palette.spotify {
            Spotify::Done(results) => Some(results.clone()),
            _ => None,
        };

        // ── Adaylar ──
        // Şarkılar: önce kütüphaneden birkaçı, sonra Spotify, kalanı yine
        // kütüphane (aynı şarkı bir kez).
        let local: Vec<usize> = f.top(
            self.palette.track_keys.iter().map(String::as_str),
            MAX_SONGS + 1,
        );
        let mut songs: Vec<(Item, u32)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut push_song = |songs: &mut Vec<(Item, u32)>, t: &TrackRow, f: &mut fuzzy::Fuzzy| {
            if seen.insert(t.uri.clone()) {
                let score = f
                    .score(&fuzzy::fold(&format!("{} {}", t.name, t.artists)))
                    .unwrap_or(1);
                songs.push((song_item(t), score));
            }
        };
        for i in local.iter().take(MAX_LOCAL_SONGS) {
            push_song(&mut songs, &self.palette.tracks[*i], &mut f);
        }
        if let Some(results) = &spotify {
            for t in results.tracks.iter().take(MAX_SONGS + 1) {
                push_song(&mut songs, t, &mut f);
            }
        }
        for i in local.iter().skip(MAX_LOCAL_SONGS) {
            push_song(&mut songs, &self.palette.tracks[*i], &mut f);
        }
        songs.truncate(MAX_SONGS + 1);

        // Sanatçılar (yalnız Spotify).
        let mut artists: Vec<(Item, u32)> = Vec::new();
        if let Some(results) = &spotify {
            for a in results.artists.iter().take(MAX_TILES + 1) {
                let score = f.score(&fuzzy::fold(&a.name)).unwrap_or(1);
                artists.push((
                    Item::new(
                        Kind::Tile,
                        a.name.clone(),
                        Action::OpenArtist(a.uri.clone()),
                    )
                    .art(Art::Round {
                        url: a.image.clone(),
                    })
                    .subtitle(t!(Artist))
                    .tag(t!(Artist))
                    .play(Action::PlayArtist(a.uri.clone()))
                    .dedupe(a.uri.clone()),
                    score,
                ));
            }
        }

        // Albümler ve listeler: önce kütüphanedekiler, sonra Spotify.
        let mut cards: Vec<(Item, u32)> = Vec::new();
        if let Loadable::Loaded(library) = &self.library.library {
            let mut all = vec![(
                CollectionId::Liked,
                t!(LikedSongs).to_string(),
                crate::i18n::canon::songs(library.liked_count),
                String::new(),
            )];
            all.extend(library.albums.iter().chain(&library.playlists).map(|i| {
                (
                    i.id.clone(),
                    i.name.clone(),
                    crate::i18n::canon::localize(&i.subtitle).into_owned(),
                    i.image.clone(),
                )
            }));
            let keys: Vec<String> = all
                .iter()
                .map(|(_, n, s, _)| fuzzy::fold(&format!("{n} {s}")))
                .collect();
            for i in f.top(keys.iter().map(String::as_str), MAX_LOCAL_COLLECTIONS) {
                let (id, name, subtitle, image) = all[i].clone();
                let score = f.score(&keys[i]).unwrap_or(1);
                let tag = match id {
                    CollectionId::Album(_) => t!(Album),
                    _ => t!(Playlist),
                };
                let dedupe = match &id {
                    CollectionId::Album(u) | CollectionId::Playlist(u) => u.clone(),
                    _ => String::new(),
                };
                cards.push((
                    Item::new(
                        Kind::Tile,
                        name.clone(),
                        Action::OpenCollection {
                            id: id.clone(),
                            title: name.clone(),
                        },
                    )
                    .art(Art::Collection {
                        id: id.clone(),
                        url: image,
                    })
                    .subtitle(subtitle)
                    .tag(tag)
                    .play(Action::PlayCollection { id, title: name })
                    .dedupe(dedupe),
                    score,
                ));
            }
        }
        if let Some(results) = &spotify {
            for hit in results.albums.iter().take(MAX_TILES) {
                let score = f.score(&fuzzy::fold(&hit.name)).unwrap_or(1);
                cards.push((
                    Item::new(
                        Kind::Tile,
                        hit.name.clone(),
                        Action::OpenAlbum(hit.uri.clone()),
                    )
                    .art(Art::Cover {
                        url: hit.image.clone(),
                        icon: "disc-3",
                    })
                    .subtitle(crate::i18n::canon::localize(&hit.subtitle).into_owned())
                    .tag(t!(Album))
                    .play(Action::PlayAlbum(hit.uri.clone()))
                    .dedupe(hit.uri.clone()),
                    score,
                ));
            }
            for hit in results.playlists.iter().take(MAX_TILES) {
                let score = f.score(&fuzzy::fold(&hit.name)).unwrap_or(1);
                let id = CollectionId::Playlist(hit.uri.clone());
                cards.push((
                    Item::new(
                        Kind::Tile,
                        hit.name.clone(),
                        Action::OpenCollection {
                            id: id.clone(),
                            title: hit.name.clone(),
                        },
                    )
                    .art(Art::Cover {
                        url: hit.image.clone(),
                        icon: "list-music",
                    })
                    .subtitle(crate::i18n::canon::localize(&hit.subtitle).into_owned())
                    .tag(t!(Playlist))
                    .play(Action::PlayCollection {
                        id,
                        title: hit.name.clone(),
                    })
                    .dedupe(hit.uri.clone()),
                    score,
                ));
            }
        }
        let mut seen_cards = std::collections::HashSet::new();
        cards.retain(|(i, _)| i.dedupe.is_empty() || seen_cards.insert(i.dedupe.clone()));

        // ── En iyi sonuç ──
        // Adaylar: sanatçılar, kartlar ve şarkıların başı.
        let mut cands: Vec<(HitKind, u32, MatchLevel)> = Vec::new();
        let mut cand_src: Vec<(u8, usize)> = Vec::new();
        for (i, (item, score)) in artists.iter().enumerate() {
            cands.push((HitKind::Artist, *score, MatchLevel::of(&item.title, &typed)));
            cand_src.push((0, i));
        }
        for (i, (item, score)) in cards.iter().enumerate() {
            let kind = if item.tag == t!(Album) {
                HitKind::Album
            } else {
                HitKind::Playlist
            };
            cands.push((kind, *score, MatchLevel::of(&item.title, &typed)));
            cand_src.push((1, i));
        }
        if let Some((item, score)) = songs.first() {
            cands.push((HitKind::Song, *score, MatchLevel::of(&item.title, &typed)));
            cand_src.push((2, 0));
        }
        let best = pick_best(&cands).map(|i| cand_src[i]);

        let mut items: Vec<Item> = Vec::new();
        let mut blocks: Vec<Block> = Vec::new();

        // Seçilen en iyi sonuç kendi türünün listesinden çıkar (ikinci kez
        // görünmesin); kartın kendisi `Kind::Best`.
        let mut best_item: Option<Item> = None;
        match best {
            Some((0, i)) => best_item = Some(artists.remove(i).0),
            Some((1, i)) => best_item = Some(cards.remove(i).0),
            Some((2, _)) => best_item = Some(songs.remove(0).0),
            _ => {}
        }
        if let Some(b) = best_item.as_mut() {
            b.kind = Kind::Best;
            if matches!(b.action, Action::PlayTrack(_)) {
                b.play = Some(b.action.clone());
            }
        }

        let pending = self.palette.spotify.pending();

        // ── Bölümler ──
        let best_ix = best_item.map(|b| {
            items.push(b);
            items.len() - 1
        });
        let song_ix: Vec<usize> = songs
            .into_iter()
            .take(MAX_SONGS)
            .map(|(i, _)| {
                items.push(i);
                items.len() - 1
            })
            .collect();
        if best_ix.is_some() || !song_ix.is_empty() {
            blocks.push(Block::Top {
                best: best_ix,
                songs: song_ix,
            });
        } else if pending {
            blocks.push(Block::Skeleton(Skel::Top));
        }

        let artist_ix: Vec<usize> = artists
            .into_iter()
            .take(MAX_TILES)
            .map(|(i, _)| {
                items.push(i);
                items.len() - 1
            })
            .collect();
        if !artist_ix.is_empty() {
            blocks.push(Block::Tiles {
                title: t!(SectionArtists),
                items: artist_ix,
            });
        } else if pending {
            blocks.push(Block::Skeleton(Skel::Round));
        }

        let card_ix: Vec<usize> = cards
            .into_iter()
            .take(MAX_TILES)
            .map(|(i, _)| {
                items.push(i);
                items.len() - 1
            })
            .collect();
        if !card_ix.is_empty() {
            blocks.push(Block::Tiles {
                title: t!(PaletteAlbumsLists),
                items: card_ix,
            });
        } else if pending {
            blocks.push(Block::Skeleton(Skel::Square));
        }

        // Spotify'a ulaşılamadıysa sakin bir not (kütüphane sonuçları durur).
        if let Spotify::Failed(e) = &self.palette.spotify {
            blocks.push(Block::Note {
                icon: "wifi-off",
                text: match super::components::explain_error(e).1 {
                    title if title == t!(ErrOfflineTitle) => t!(PaletteSearchOffline).into(),
                    title if title == t!(ErrBusyTitle) => t!(PaletteSearchBusy).into(),
                    _ => t!(PaletteSearchFailed).into(),
                },
            });
        }

        // Eylemler: yalnız adı yazılınca.
        let mut actions = Vec::new();
        if typed.chars().count() >= MIN_ACTION_QUERY {
            for (cmd, title, desc) in commands() {
                let name_key = fuzzy::fold(title);
                if !action_matches(&typed, &name_key, &fuzzy::fold(desc)) {
                    continue;
                }
                let mut item = action_item(cmd, title, desc);
                let (new_title, disabled) = self.command_state(cmd);
                if let Some(new_title) = new_title {
                    item.name_key = fuzzy::fold(&new_title);
                    item.title = new_title;
                }
                item.disabled = disabled;
                actions.push(item);
                if actions.len() == MAX_ACTIONS {
                    break;
                }
            }
        }
        if !actions.is_empty() {
            let ix: Vec<usize> = actions
                .into_iter()
                .map(|i| {
                    items.push(i);
                    items.len() - 1
                })
                .collect();
            blocks.push(Block::Actions { items: ix });
        }

        // Hiçbir şey bulunamadıysa (Spotify cevabı da geldi / hata verdi) en
        // üstte sakin bir cümle; tek harfte Spotify aranmadığı için "bulunamadı"
        // denmez, davet cümlesi kalır.
        if items.is_empty() && !pending {
            let text = if query.trim().chars().count() < MIN_SPOTIFY_QUERY {
                t!(PaletteInvite).to_string()
            } else {
                tf!(PaletteNoResultsFor, query.trim())
            };
            blocks.insert(
                0,
                Block::Note {
                    icon: "search",
                    text,
                },
            );
        }

        let footer = (query.trim().chars().count() >= MIN_SPOTIFY_QUERY).then(|| {
            Item::new(
                Kind::More,
                tf!(PaletteAllResultsFor, query.trim()),
                Action::AllResults(query.trim().to_string()),
            )
        });
        (items, blocks, footer)
    }

    /// Eylemin şu anki adı (duruma göre değişiyorsa) ve yapılamaz olup
    /// olmadığı: "Çal / Duraklat" yerine o an ne olacaksa o yazar.
    fn command_state(&self, cmd: PaletteCommand) -> (Option<String>, bool) {
        let has_track = self.state.track.is_some();
        match cmd {
            PaletteCommand::TogglePlay if self.state.is_playing() => {
                (Some(t!(Pause).into()), false)
            }
            PaletteCommand::TogglePlay if has_track => (Some(t!(Play).into()), false),
            PaletteCommand::TogglePlay => match self.startable_collection() {
                Some(name) => (Some(tf!(PlayNamed, name)), false),
                None => (Some(t!(Play).into()), true),
            },
            PaletteCommand::Next => (None, !self.state.queue.has_next),
            PaletteCommand::Previous => (None, !has_track),
            PaletteCommand::ToggleMute if self.state.volume == 0.0 => {
                (Some(t!(Unmute).into()), false)
            }
            PaletteCommand::ToggleMute => (Some(t!(Mute).into()), false),
            PaletteCommand::PrivateSession if self.private_session() => {
                (Some(t!(CmdPrivateOff).into()), false)
            }
            PaletteCommand::PrivateSession => (Some(t!(CmdPrivateOn).into()), false),
            // Çalan şarkı yoksa radyonun tohumu yok.
            PaletteCommand::StartRadio => (None, !has_track),
            _ => (None, false),
        }
    }

    /// Seçimi okunma sırasında taşır (Tab); kullanılamayan öğelerin üstünden atlar.
    fn move_palette_selection(&mut self, delta: isize) {
        if self.palette.items.is_empty() {
            return;
        }
        self.palette.selected = step_linear(&self.palette.items, self.palette.selected, delta);
        self.scroll_to_selected();
    }

    /// Kutu boşken seçim görünmez: ilk ok / Tab tuşu onu ilk kullanılabilir
    /// öğede görünür kılar (taşımaz). Görünür kıldıysa `true`.
    fn reveal_selection(&mut self) -> bool {
        if self.palette.selection_visible() {
            return false;
        }
        self.palette.moved = true;
        self.palette.selected = self
            .palette
            .items
            .iter()
            .position(|i| !i.disabled && i.kind != Kind::More)
            .unwrap_or(0);
        self.scroll_to_selected();
        true
    }

    /// Seçimi ızgarada bir yöne taşır.
    fn move_palette_dir(&mut self, dir: Dir) {
        if let Some(next) = neighbor(&self.palette.items, self.palette.selected, dir) {
            self.palette.selected = next;
        }
        self.scroll_to_selected();
    }

    /// Seçili öğenin bölümünü görünür kıl.
    fn scroll_to_selected(&self) {
        let sel = self.palette.selected;
        let Some(item) = self.palette.items.get(sel) else {
            return;
        };
        if item.kind == Kind::More {
            return;
        }
        let block = self.palette.blocks.iter().position(|b| match b {
            Block::Top { best, songs } => *best == Some(sel) || songs.contains(&sel),
            Block::Chips { items, .. } | Block::Tiles { items, .. } => items.contains(&sel),
            Block::Actions { items } => items.contains(&sel),
            _ => false,
        });
        if let Some(b) = block {
            // İlk bölüm hep tepede: kaydırma yerine başa dönsün.
            if b == 0 {
                self.palette.scroll.set_offset(gpui::point(px(0.), px(0.)));
            } else {
                self.palette.scroll.scroll_to_item(b);
            }
        }
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.palette.items.get(index) else {
            return;
        };
        let action = item.action.clone();
        if item.disabled {
            return;
        }
        // Tehlikeli işlem ilk Enter'da çalışmaz: satır onay ister.
        if item.danger && self.palette.confirm != Some(index) {
            self.palette.confirm = Some(index);
            cx.notify();
            return;
        }
        self.perform(action, cx);
    }

    fn perform(&mut self, action: Action, cx: &mut Context<Self>) {
        // Son arama: yazıya dönüşür, sonuçlar gelir.
        if let Action::Search(q) = action {
            self.palette.query = q;
            self.on_query_changed(cx);
            return;
        }
        // Sayfa ana pencerede açılır: her yerden açılan paletten seçildiyse
        // ana pencere öne gelsin.
        self.palette.wants_window = matches!(
            action,
            Action::OpenCollection { .. }
                | Action::PlayCollection { .. }
                | Action::OpenArtist(_)
                | Action::PlayArtist(_)
                | Action::OpenAlbum(_)
                | Action::PlayAlbum(_)
                | Action::OpenCategory(_)
                | Action::AllResults(_)
                | Action::Command(
                    PaletteCommand::Home
                        | PaletteCommand::Explore
                        | PaletteCommand::Library
                        | PaletteCommand::NewPlaylist
                        | PaletteCommand::Settings
                        | PaletteCommand::Recent
                        | PaletteCommand::Tools
                        | PaletteCommand::Shortcuts
                )
        );
        // Yazılan bir sonuca gidildiyse (eylem ve tür sayfası değil) son aramalara girer.
        if !matches!(action, Action::Command(_) | Action::OpenCategory(_))
            && !self.palette.query.trim().is_empty()
        {
            let query = self.palette.query.clone();
            self.palette.note_search(&query);
            self.save_settings();
        }
        self.close_palette(cx);
        match action {
            Action::Command(cmd) => self.run_command(cmd, cx),
            Action::OpenArtist(uri) => self.navigate(Route::Artist(uri), cx),
            Action::PlayArtist(uri) => {
                self.navigate(Route::Artist(uri.clone()), cx);
                if self.ready_to_play(cx) {
                    self.state.notice = None;
                    self.engine.send(Command::PlayContext(uri));
                }
            }
            Action::OpenAlbum(uri) => self.open_album(uri, cx),
            Action::PlayAlbum(uri) => {
                if self.ready_to_play(cx) {
                    self.state.notice = None;
                    self.play_card(CardTarget::Album { uri, item: None }, cx);
                }
            }
            Action::AllResults(query) => self.navigate(Route::Search(query), cx),
            Action::OpenCategory(uri) => self.navigate(Route::Category(uri), cx),
            Action::OpenCollection { id, title } => {
                self.library.titles.insert(id.clone(), title);
                self.select_collection(id, cx);
            }
            Action::PlayCollection { id, title } => {
                if self.ready_to_play(cx) {
                    self.state.notice = None;
                    self.play_card(CardTarget::Collection { id, title }, cx);
                }
            }
            Action::PlayTrack(uri) => {
                if let Ok(uri) = librespot::core::SpotifyUri::from_uri(&uri)
                    && self.ready_to_play(cx)
                {
                    self.state.notice = None;
                    self.engine.send(Command::Load(uri));
                }
            }
            Action::Search(_) => {}
        }
    }

    fn run_command(&mut self, cmd: PaletteCommand, cx: &mut Context<Self>) {
        match cmd {
            PaletteCommand::TogglePlay => self.toggle_play(cx),
            PaletteCommand::Next => self.next(cx),
            PaletteCommand::Previous => self.previous(cx),
            PaletteCommand::ToggleMute => self.toggle_mute(cx),
            PaletteCommand::RefreshLibrary => self.engine.send(Command::RefreshLibrary),
            PaletteCommand::Shortcuts => self.open_help(None, cx),
            PaletteCommand::Home => self.go_home(cx),
            PaletteCommand::Explore => self.go_tab(super::history::Tab::Explore, cx),
            PaletteCommand::Library => self.go_tab(super::history::Tab::Library, cx),
            PaletteCommand::NewPlaylist => self.ask_new_playlist(Vec::new(), true, cx),
            PaletteCommand::Settings => self.open_settings(cx),
            PaletteCommand::Recent => self.navigate(Route::Recent, cx),
            PaletteCommand::Tools => self.open_tools(cx),
            PaletteCommand::PrivateSession => self.toggle_private_session(cx),
            PaletteCommand::StartRadio => self.start_radio_from_current(cx),
            // Paletin kendi satır onayı (ikinci Enter) zaten geçildi.
            PaletteCommand::Logout => self.sign_out(cx),
        }
    }

    /// Palet açıkken bütün tuşlar buraya gelir.
    pub(super) fn palette_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control;
        match ks.key.as_str() {
            "escape" => self.close_palette(cx),
            "k" if ctrl => self.close_palette(cx),
            // Ctrl+Enter: yazılanın bütün Spotify sonuçları kendi sayfasında.
            "enter" if ctrl => {
                let query = self.palette.query.trim().to_string();
                if query.chars().count() >= MIN_SPOTIFY_QUERY {
                    self.palette.wants_window = true;
                    self.palette.note_search(&query);
                    self.save_settings();
                    self.close_palette(cx);
                    self.navigate(Route::Search(query), cx);
                }
            }
            "enter" => {
                if self.palette.selection_visible() {
                    self.activate(self.palette.selected, cx);
                }
            }
            "up" | "down" | "left" | "right" => {
                let dir = match ks.key.as_str() {
                    "up" => Dir::Up,
                    "down" => Dir::Down,
                    "left" => Dir::Left,
                    _ => Dir::Right,
                };
                self.palette.confirm = None;
                if !self.reveal_selection() {
                    self.palette.moved = true;
                    self.move_palette_dir(dir);
                }
                cx.notify();
            }
            "tab" => {
                if !self.reveal_selection() {
                    self.palette.moved = true;
                    self.move_palette_selection(if ks.modifiers.shift { -1 } else { 1 });
                }
                cx.notify();
            }
            "space" if !ctrl => {
                self.palette.query.push(' ');
                self.on_query_changed(cx);
            }
            "backspace" => {
                if ctrl {
                    let trimmed = self.palette.query.trim_end().len();
                    let cut = self.palette.query[..trimmed]
                        .rfind(' ')
                        .map_or(0, |i| i + 1);
                    self.palette.query.truncate(cut);
                } else {
                    self.palette.query.pop();
                }
                self.on_query_changed(cx);
            }
            "v" if ctrl => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    self.palette
                        .query
                        .push_str(text.lines().next().unwrap_or_default());
                    self.on_query_changed(cx);
                }
            }
            _ if !ctrl && !ks.modifiers.alt => {
                if let Some(ch) = ks
                    .key_char
                    .as_deref()
                    .filter(|c| !c.chars().any(char::is_control))
                {
                    self.palette.query.push_str(ch);
                    self.on_query_changed(cx);
                }
            }
            _ => {}
        }
    }

    // ── Görünüm ─────────────────────────────────────────────────────────

    /// Ana penceredeki yüzey: arkadaki içeriği karartan perdenin üstünde,
    /// başlık çubuğundaki arama hapının hemen altından açılır; hapla aynı
    /// eksende ortalı.
    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.render_palette_panel(cx);
        let scrim = div()
            .id("palette-scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(shade(SCRIM))
            // Tekerlek arkadaki listeyi kaydırmasın. Paletin kendi listesi
            // tekerleği bundan önce alır (gpui içten dışa dağıtıyor), o
            // kayar. (`occlude` değil: o, başlık çubuğundaki pencere
            // düğmelerini ve sürükleme alanını da kapatırdı.)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .flex()
            .justify_center()
            // Yüzey boyca gerilmesin, içeriği kadar olsun. Üstü başlık
            // çubuğundaki hapın üstünde başlar: giriş hapı o hapın yerine
            // oturur, "ikinci bir arama kutusu" gibi durmaz, aynı kutu büyür.
            .items_start()
            .pt(px(S1))
            .px(px(S4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _: &mut Window, cx| this.close_palette(cx)),
            )
            .child(panel);
        // Perde de yumuşakça belirir ("Animasyonları azalt"ta hiç sarmalanmaz).
        let scrim = if animations_enabled() {
            scrim
                .with_animation(
                    ("palette-scrim", self.palette.opened),
                    gpui::Animation::new(motion()).with_easing(gpui::ease_out_quint()),
                    |el, d| el.bg(shade(SCRIM * d)),
                )
                .into_any_element()
        } else {
            scrim.into_any_element()
        };
        deferred(scrim).with_priority(1).into_any_element()
    }

    /// Yüzeyin kendisi (giriş hapı, bölümler, alt bağlantı) açılma geçişiyle.
    /// Ana pencere de her yerden açılan palet penceresi de bunu çizer: ikisi
    /// birebir aynı.
    pub(super) fn render_palette_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent();
        let detached = self.palette.detached;
        let width = if detached {
            PANEL_WIDTH
        } else {
            PANEL_WIDTH.min(self.viewport_w - 2. * S4)
        };
        let max_height = if detached {
            PANEL_MAX_HEIGHT
        } else {
            (self.viewport_h - S1 - S4).clamp(S8 * 4., PANEL_MAX_HEIGHT)
        };

        // ── Giriş hapı ──
        let query: SharedString = if self.palette.query.is_empty() {
            search_placeholder().into()
        } else {
            self.palette.query.clone().into()
        };
        let empty = self.palette.query.is_empty();
        let input = div()
            .flex_none()
            .h(px(INPUT_HEIGHT))
            .px(px(S4))
            .rounded(px(R_FULL))
            .flex()
            .items_center()
            .gap(px(S3))
            .bg(rgb(search_fill(accent.glow, true)))
            .border_1()
            .border_color(rgb(accent.text))
            .child(icon("search", 18., soft()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .type_role(T_TITLE)
                    .text_color(if empty {
                        search_hint(accent.text)
                    } else {
                        text()
                    })
                    // İmleç: yazılanın sonunda (boşken yer tutucunun başında).
                    .when(empty, |el| el.child(caret(accent.text)))
                    .child(query)
                    .when(!empty, |el| el.child(caret(accent.text))),
            );

        // ── Bölümler ──
        let mut list = div()
            .id("palette-results")
            // İçerik kadar yer kaplasın; uzarsa yüzey sınırında kaydırılsın.
            .flex_shrink()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.palette.scroll)
            .flex()
            .flex_col()
            .px(px(S2))
            .pb(px(S2));
        for (n, block) in self.palette.blocks.iter().enumerate() {
            list = list.child(self.render_block(n, block, accent.text, cx));
        }

        let mut panel = div()
            .id("palette")
            .w(px(width))
            .max_h(px(max_height))
            .flex()
            .flex_col()
            .p(px(S2))
            // Cam: üstte kapağın ışığına hafifçe çalan, altta koyu (hapın camı).
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(surface_top(accent.glow)), 0.),
                linear_color_stop(rgb(PILL_GLASS_BOTTOM), 1.),
            ))
            .border_1()
            .border_color(veil(PILL_RING))
            .rounded(px(R_2XL))
            .shadow(capsule_shadow())
            .overflow_hidden()
            // Yüzeyin içine tıklamak paleti kapatmasın.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(input)
            .child(list);
        if let Some(f) = self.palette.footer {
            panel = panel.child(self.render_footer(f, accent.text, cx));
        }

        // Kapaklar: çizimde istenenler motora gider (her yerden açılan palet
        // penceresi de dahil).
        self.flush_image_requests(cx);
        if animations_enabled() {
            panel
                .with_animation(
                    ("palette-open", self.palette.opened),
                    gpui::Animation::new(motion()).with_easing(gpui::ease_out_quint()),
                    |el, d| el.opacity(d).mt(px(-S2 * (1. - d))),
                )
                .into_any_element()
        } else {
            panel.into_any_element()
        }
    }

    fn render_block(
        &self,
        n: usize,
        block: &Block,
        accent: u32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match block {
            Block::Top { best, songs } => self.render_top(*best, songs, accent, cx),
            Block::Chips { title, items } => div()
                .flex()
                .flex_col()
                .child(section_title(title))
                .child(
                    div()
                        .px(px(S2))
                        .pt(px(S1))
                        .flex()
                        .flex_wrap()
                        .gap(px(S2))
                        .children(items.iter().map(|i| self.render_chip(*i, accent, cx))),
                )
                .into_any_element(),
            Block::Tiles { title, items } => div()
                .flex()
                .flex_col()
                .child(section_title(title))
                .child(
                    div()
                        .flex()
                        .gap(px(S1))
                        .children(items.iter().map(|i| self.render_tile(*i, accent, cx))),
                )
                .into_any_element(),
            Block::Actions { items } => div()
                .flex()
                .flex_col()
                .child(section_title(t!(PaletteActions)))
                .children(items.iter().map(|i| self.render_action(*i, accent, cx)))
                .into_any_element(),
            Block::Skeleton(kind) => skeleton(n, *kind),
            Block::Note { icon: name, text } => div()
                .px(px(S2))
                .py(px(S4))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(S2))
                .type_role(T_SECONDARY)
                .text_color(muted())
                .child(icon(name, 16., label()))
                .child(text.clone())
                .into_any_element(),
        }
    }

    /// Solda en iyi sonuç kartı, sağında şarkılar.
    fn render_top(
        &self,
        best: Option<usize>,
        songs: &[usize],
        accent: u32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let left = best.map(|b| {
            div()
                .flex_none()
                .w(px(BEST_WIDTH))
                .flex()
                .flex_col()
                .child(section_title(t!(PaletteTopResult)))
                .child(self.render_best(b, accent, cx))
        });
        let right = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .when(!songs.is_empty(), |el| {
                el.child(section_title(t!(SectionSongs)))
            })
            .children(songs.iter().map(|i| self.render_song(*i, accent, cx)));
        div()
            .flex()
            .gap(px(S3))
            .children(left)
            .child(right)
            .into_any_element()
    }

    /// Halka bu öğede çizilsin mi: seçili ve seçim görünür. Alt bağlantı tek
    /// seçilebilir öğeyken (iskelet, sonuç yok) kendiliğinden halkalanmaz; yine de
    /// Enter onu açar.
    fn selected(&self, i: usize) -> bool {
        self.palette.selected == i
            && self.palette.selection_visible()
            && (self.palette.moved || self.palette.items[i].kind != Kind::More)
    }

    /// Çal düğmesi: görselin köşesinde yuvarlak, vurgu renginde; kartın
    /// üzerine gelince ya da seçiliyken görünür.
    fn play_button(
        &self,
        i: usize,
        style: PlayStyle,
        action: Action,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let PlayStyle {
            size,
            group,
            selected,
            accent,
        } = style;
        div()
            .id(("palette-play", i))
            .absolute()
            .right(px(S2))
            .bottom(px(S2))
            .size(px(size))
            .rounded(px(R_FULL))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(accent))
            .shadow(float_shadow())
            .cursor_pointer()
            .opacity(if selected { 1. } else { 0. })
            .group_hover(group, |s| s.opacity(1.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.perform(action.clone(), cx);
            }))
            .child(icon("play-fill", size * 0.4, rgb(BG)))
            .into_any_element()
    }

    /// "En iyi sonuç" kartı: büyük görsel, ad, tür etiketi.
    fn render_best(&self, i: usize, accent: u32, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.palette.items[i];
        let selected = self.selected(i);
        let group: SharedString = format!("palette-best-{i}").into();
        let round = matches!(item.art, Art::Round { .. });
        let art = self.palette_art(
            &item.art,
            &item.title,
            BEST_ART,
            if round { BEST_ART / 2. } else { R_LG },
        );
        div()
            .id(("palette-item", i))
            .group(group.clone())
            .relative()
            .flex_none()
            .w_full()
            .h(px(BEST_HEIGHT))
            .p(px(S4))
            .rounded(px(R_XL))
            .border_2()
            .border_color(ring(selected, accent))
            .bg(veil(if selected { VEIL_SELECTED } else { VEIL_HOVER }))
            .cursor_pointer()
            .hover(|s| s.bg(veil(VEIL_SELECTED)))
            .active(|s| s.bg(veil(VEIL_PRESSED)))
            .flex()
            .flex_col()
            .justify_between()
            .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            .child(art)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(S1))
                    .child(
                        div()
                            .flex()
                            .font_family(font())
                            .type_role(T_TITLE)
                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                            .text_color(text())
                            .child(
                                line_with_tip(("palette-best-title", i), item.title.clone())
                                    .flex()
                                    .min_w_0(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(S2))
                            .type_role(T_SECONDARY)
                            .text_color(muted())
                            .child(
                                div()
                                    .flex_none()
                                    .px(px(S2))
                                    .rounded(px(R_FULL))
                                    .bg(veil(VEIL_SELECTED))
                                    .type_role(T_LABEL)
                                    .text_color(soft())
                                    .child(item.tag),
                            )
                            .when(!item.subtitle.is_empty() && !round, |el| {
                                el.child(
                                    line_with_tip(("palette-best-sub", i), item.subtitle.clone())
                                        .flex()
                                        .min_w_0(),
                                )
                            }),
                    ),
            )
            .children(item.play.clone().map(|a| {
                let style = PlayStyle {
                    size: PLAY_BIG,
                    group,
                    selected,
                    accent,
                };
                self.play_button(i, style, a, cx)
            }))
            .into_any_element()
    }

    /// Şarkı satırı: kapak, ad, sanatçı, süre.
    fn render_song(&self, i: usize, accent: u32, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.palette.items[i];
        let selected = self.selected(i);
        let group: SharedString = format!("palette-song-{i}").into();
        div()
            .id(("palette-item", i))
            .group(group.clone())
            .h(px(ROW_HEIGHT))
            .px(px(S2))
            .rounded(px(R_MD))
            .border_2()
            .border_color(ring(selected, accent))
            .flex()
            .items_center()
            .gap(px(S3))
            .cursor_pointer()
            .bg(veil(if selected { VEIL_SELECTED } else { 0. }))
            .hover(|s| s.bg(veil(VEIL_HOVER)))
            .active(|s| s.bg(veil(VEIL_PRESSED)))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .size(px(ROW_COVER))
                    .child(self.palette_art(&item.art, &item.title, ROW_COVER, R_SM))
                    // Üzerine gelince kapakta çal işareti.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .rounded(px(R_SM))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(shade(SCRIM))
                            .opacity(if selected { 1. } else { 0. })
                            .group_hover(group, |s| s.opacity(1.))
                            .child(icon("play-fill", 16., text())),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(T_BODY)
                            .text_color(text())
                            .child(line_with_tip(("palette-title", i), item.title.clone())),
                    )
                    .child(
                        div()
                            .type_role(T_SECONDARY)
                            .text_color(muted())
                            .child(line_with_tip(("palette-sub", i), item.subtitle.clone())),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .type_role(T_LABEL)
                    .tabular()
                    .text_color(label())
                    .child(item.detail.clone()),
            )
            .into_any_element()
    }

    /// Sanatçı / albüm / liste kartı: görsel, altında ad (ve alt satır).
    fn render_tile(&self, i: usize, accent: u32, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.palette.items[i];
        let selected = self.selected(i);
        let round = matches!(item.art, Art::Round { .. });
        let group: SharedString = format!("palette-tile-{i}").into();
        let art = div()
            .relative()
            .flex_none()
            .size(px(TILE))
            .child(self.palette_art(
                &item.art,
                &item.title,
                TILE,
                if round { TILE / 2. } else { R_LG },
            ))
            .children(item.play.clone().map(|a| {
                let style = PlayStyle {
                    size: PLAY_SMALL,
                    group: group.clone(),
                    selected,
                    accent,
                };
                self.play_button(i, style, a, cx)
            }));
        let name = div()
            .w(px(TILE))
            .flex()
            .when(round, |el| el.justify_center())
            .type_role(T_SECONDARY)
            .text_color(text())
            .child(line_with_tip(("palette-tile-title", i), item.title.clone()).flex());
        let sub = (!item.subtitle.is_empty() && !round).then(|| {
            div()
                .w(px(TILE))
                .flex()
                .type_role(T_LABEL)
                .text_color(label())
                .child(line_with_tip(("palette-tile-sub", i), item.subtitle.clone()).flex())
        });
        div()
            .id(("palette-item", i))
            .group(group)
            .flex_none()
            .p(px(S2))
            .rounded(px(R_LG))
            .border_2()
            .border_color(ring(selected, accent))
            .bg(veil(if selected { VEIL_SELECTED } else { 0. }))
            .flex()
            .flex_col()
            .gap(px(S2))
            .cursor_pointer()
            .hover(|s| s.bg(veil(VEIL_HOVER)))
            .active(|s| s.bg(veil(VEIL_PRESSED)))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            .child(art)
            .child(div().flex().flex_col().child(name).children(sub))
            .into_any_element()
    }

    /// Çip: son arama (arama simgesi, soluk zemin) ya da tür (türün renginde).
    fn render_chip(&self, i: usize, accent: u32, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.palette.items[i];
        let selected = self.selected(i);
        let genre = item.kind == Kind::Genre;
        let fill = item.color.map(rgb);
        div()
            .id(("palette-item", i))
            .h(px(if genre { GENRE_HEIGHT } else { CHIP_HEIGHT }))
            .max_w(px(S8 * 4.))
            .px(px(S4))
            .rounded(px(R_FULL))
            .border_2()
            .border_color(ring(selected, if genre { TEXT } else { accent }))
            .flex()
            .items_center()
            .gap(px(S2))
            .cursor_pointer()
            .when(!genre, |el| {
                el.bg(veil(if selected { VEIL_SELECTED } else { VEIL_HOVER }))
                    .hover(|s| s.bg(veil(VEIL_SELECTED)))
                    .active(|s| s.bg(veil(VEIL_PRESSED)))
            })
            .when_some(fill, |el, fill| {
                el.bg(fill)
                    .hover(|s| s.opacity(0.88))
                    .active(|s| s.opacity(0.76))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            .when(!genre, |el| el.child(icon("history", 14., label())))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .type_role(if genre { T_BODY } else { T_SECONDARY })
                    .when(genre, |el| el.font_weight(gpui::FontWeight::SEMIBOLD))
                    .text_color(text())
                    .child(
                        line_with_tip(("palette-chip-title", i), item.title.clone())
                            .flex()
                            .min_w_0(),
                    ),
            )
            .into_any_element()
    }

    /// Eylem satırı: simgeli karo, insan diliyle ad, açıklama.
    fn render_action(&self, i: usize, accent: u32, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.palette.items[i];
        let selected = self.selected(i);
        let confirming = self.palette.confirm == Some(i);
        // Onay hem fareyle hem klavyeyle verilebilir: metin ikisini de söyler.
        let (title, subtitle): (String, String) = if confirming {
            (
                t!(PaletteConfirmSignOut).into(),
                t!(PaletteConfirmSignOutDesc).into(),
            )
        } else {
            (item.title.clone(), item.subtitle.clone())
        };
        let title_color = if item.disabled {
            label()
        } else if item.danger {
            rgb(DANGER_TEXT)
        } else {
            text()
        };
        let interactive = !item.disabled;
        let glyph = match item.art {
            Art::Glyph(g) => g,
            _ => "music",
        };
        div()
            .id(("palette-item", i))
            .h(px(ROW_HEIGHT))
            .px(px(S2))
            .rounded(px(R_MD))
            .border_2()
            .border_color(ring(selected && interactive, accent))
            .flex()
            .items_center()
            .gap(px(S3))
            .bg(veil(if selected && interactive {
                VEIL_SELECTED
            } else {
                0.
            }))
            .when(interactive, |el| {
                el.cursor_pointer()
                    .hover(|s| s.bg(veil(VEIL_HOVER)))
                    .active(|s| s.bg(veil(VEIL_PRESSED)))
                    .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            })
            .child(
                div()
                    .flex_none()
                    .size(px(ROW_COVER))
                    .rounded(px(R_MD))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(veil(VEIL_HOVER))
                    .child(icon(glyph, 18., muted())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .type_role(T_BODY)
                            .text_color(title_color)
                            .child(line_with_tip(("palette-action", i), title)),
                    )
                    .when(!subtitle.is_empty(), |el| {
                        el.child(
                            div()
                                .type_role(T_SECONDARY)
                                .text_color(if item.disabled { label() } else { muted() })
                                .child(line_with_tip(("palette-action-sub", i), subtitle)),
                        )
                    }),
            )
            .into_any_element()
    }

    /// Sabit alt bağlantı: "“…” için tüm sonuçlar".
    fn render_footer(&self, i: usize, accent: u32, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.palette.items[i];
        let selected = self.selected(i);
        div()
            .id(("palette-item", i))
            .flex_none()
            .mt(px(S1))
            .h(px(FOOTER_HEIGHT))
            .px(px(S4))
            .rounded(px(R_FULL))
            .border_2()
            .border_color(ring(selected, accent))
            .flex()
            .items_center()
            .gap(px(S3))
            .cursor_pointer()
            .bg(veil(if selected { VEIL_SELECTED } else { 0. }))
            .hover(|s| s.bg(veil(VEIL_HOVER)))
            .active(|s| s.bg(veil(VEIL_PRESSED)))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            .child(icon("search", 16., muted()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .type_role(T_BODY)
                    .text_color(text())
                    .child(line_with_tip(("palette-more", i), item.title.clone()).flex()),
            )
            .child(icon("chevron-right", 16., muted()))
            .into_any_element()
    }

    /// Görsel: kapak, yuvarlak portre, listenin kapağı ya da ton karosunda simge.
    fn palette_art(&self, art: &Art, name: &str, side: f32, radius: f32) -> AnyElement {
        match art {
            Art::Cover {
                url,
                icon: icon_name,
            } => cover_image(&self.images, url, name, side, radius, icon_name),
            Art::Round { url } => cover_image(&self.images, url, name, side, side / 2., "user"),
            Art::Collection { id, url } => self.collection_art(id, name, url, side, radius),
            Art::Glyph(icon_name) => div()
                .flex_none()
                .size(px(side))
                .rounded(px(radius))
                .flex()
                .items_center()
                .justify_center()
                .bg(veil(VEIL_HOVER))
                .child(icon(icon_name, 16., muted()))
                .into_any_element(),
            Art::None => div().into_any_element(),
        }
    }
}

/// Yuvarlak çal düğmesinin ölçüsü ve durumu.
struct PlayStyle {
    size: f32,
    group: SharedString,
    selected: bool,
    accent: u32,
}

/// Halka rengi: seçiliyken vurgu rengi, değilse saydam (kenar hep var:
/// seçim yer değiştirtmesin).
fn ring(selected: bool, accent: u32) -> gpui::Hsla {
    if selected {
        rgb(accent).into()
    } else {
        gpui::transparent_black()
    }
}

/// Bölüm başlığı: sade, cümle biçiminde; büyük harf ya da üst başlık yok.
fn section_title(title: &'static str) -> gpui::Div {
    div()
        .px(px(S2))
        .pt(px(S3))
        .pb(px(S1))
        .type_role(T_BODY)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(soft())
        .child(title)
}

/// Yükleniyor: yüzeyin gerçek yerleşimiyle aynı ölçüde soluk kutular. Animasyonlar
/// açıksa yumuşakça nabız atar, kapalıysa sabit durur.
fn skeleton(n: usize, kind: Skel) -> AnyElement {
    let bar = |w: f32, h: f32, r: f32| {
        div()
            .flex_none()
            .w(px(w))
            .h(px(h))
            .rounded(px(r))
            .bg(veil(VEIL_SELECTED))
    };
    let body =
        match kind {
            Skel::Top => div()
                .flex()
                .gap(px(S3))
                .child(
                    div()
                        .flex_none()
                        .w(px(BEST_WIDTH))
                        .flex()
                        .flex_col()
                        .child(div().px(px(S2)).pt(px(S3)).pb(px(S2)).child(bar(
                            S8 + S4,
                            T_BODY.line,
                            R_SM,
                        )))
                        .child(
                            div()
                                .w_full()
                                .h(px(BEST_HEIGHT))
                                .rounded(px(R_XL))
                                .bg(veil(VEIL_HOVER)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().px(px(S2)).pt(px(S3)).pb(px(S2)).child(bar(
                            S8,
                            T_BODY.line,
                            R_SM,
                        )))
                        .children((0..MAX_SONGS).map(|_| {
                            div()
                                .h(px(ROW_HEIGHT))
                                .px(px(S2))
                                .flex()
                                .items_center()
                                .gap(px(S3))
                                .child(bar(ROW_COVER, ROW_COVER, R_SM))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(S1))
                                        .child(bar(S8 + S7, S3, R_SM))
                                        .child(bar(S8 + S5, S3, R_SM)),
                                )
                        })),
                ),
            Skel::Round | Skel::Square => {
                let round = kind == Skel::Round;
                div()
                    .flex()
                    .flex_col()
                    .child(div().px(px(S2)).pt(px(S3)).pb(px(S2)).child(bar(
                        S8 + S4,
                        T_BODY.line,
                        R_SM,
                    )))
                    .child(div().flex().gap(px(S1)).children((0..MAX_TILES).map(|_| {
                        div()
                            .flex_none()
                            .p(px(S2))
                            .flex()
                            .flex_col()
                            .gap(px(S2))
                            .child(bar(TILE, TILE, if round { TILE / 2. } else { R_LG }))
                            .child(bar(S8, S3, R_SM))
                    })))
            }
        };
    if animations_enabled() {
        body.with_animation(
            ElementId::Name(format!("palette-skel-{n}").into()),
            gpui::Animation::new(Duration::from_millis(1500))
                .repeat()
                .with_easing(gpui::pulsating_between(0.45, 1.)),
            |el, d| el.opacity(d),
        )
        .into_any_element()
    } else {
        body.into_any_element()
    }
}

/// Yazı kutusunun imleci.
fn caret(accent: u32) -> gpui::Div {
    div()
        .w(px(2.))
        .h(px(T_TITLE.size + S1))
        .flex_shrink_0()
        .bg(rgb(accent))
}

/// Kütüphane listesi kartı (kutu boşken "Tekrar dinle").
fn collection_tile(it: &crate::engine::LibraryItem) -> Item {
    let (title, tag) = match it.id {
        CollectionId::Liked => (t!(LikedSongs).to_string(), t!(Playlist)),
        CollectionId::Album(_) => (it.name.clone(), t!(Album)),
        _ => (it.name.clone(), t!(Playlist)),
    };
    Item::new(
        Kind::Tile,
        title.clone(),
        Action::OpenCollection {
            id: it.id.clone(),
            title: title.clone(),
        },
    )
    .art(Art::Collection {
        id: it.id.clone(),
        url: it.image.clone(),
    })
    .subtitle(crate::i18n::canon::localize(&it.subtitle).into_owned())
    .tag(tag)
    .play(Action::PlayCollection {
        id: it.id.clone(),
        title,
    })
}

fn action_item(cmd: PaletteCommand, title: &str, desc: &str) -> Item {
    let danger = matches!(cmd, PaletteCommand::Logout);
    let mut item = Item::new(Kind::Action, title, Action::Command(cmd))
        .art(Art::Glyph(command_icon(cmd)))
        .subtitle(desc);
    item.name_key = fuzzy::fold(title);
    item.danger = danger;
    item
}

/// Eylemin ton karosundaki simgesi.
fn command_icon(cmd: PaletteCommand) -> &'static str {
    match cmd {
        PaletteCommand::TogglePlay => "play",
        PaletteCommand::Next => "skip-forward",
        PaletteCommand::Previous => "skip-back",
        PaletteCommand::ToggleMute => "volume-x",
        PaletteCommand::RefreshLibrary => "refresh-cw",
        PaletteCommand::Shortcuts => "list",
        PaletteCommand::Logout => "log-out",
        PaletteCommand::Home => "house",
        PaletteCommand::Explore => "compass",
        PaletteCommand::Library => "library",
        PaletteCommand::NewPlaylist => "plus",
        PaletteCommand::Settings => "settings",
        PaletteCommand::Recent => "list-music",
        PaletteCommand::Tools => "wrench",
        PaletteCommand::PrivateSession => "eye-off",
        PaletteCommand::StartRadio => "radio",
    }
}

fn song_item(t: &TrackRow) -> Item {
    let mut item = Item::new(Kind::Song, t.name.clone(), Action::PlayTrack(t.uri.clone()))
        .art(Art::Cover {
            url: t.image.clone(),
            icon: "music",
        })
        .subtitle(t.artists.clone())
        .tag(t!(KindSong))
        .dedupe(t.uri.clone());
    item.detail = fmt_time(t.duration_ms);
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(kind: Kind, row: usize, col: usize, span: usize) -> Item {
        let mut item = Item::new(kind, "x", Action::Search(String::new()));
        item.cell = Cell { row, col, span };
        item
    }

    /// En iyi sonuç (4 satıra yayılır), 4 şarkı, 3 sanatçı, 2 albüm, 1 eylem.
    fn sample() -> Vec<Item> {
        vec![
            cell(Kind::Best, 0, 0, 4),   // 0
            cell(Kind::Song, 0, 1, 1),   // 1
            cell(Kind::Song, 1, 1, 1),   // 2
            cell(Kind::Song, 2, 1, 1),   // 3
            cell(Kind::Song, 3, 1, 1),   // 4
            cell(Kind::Tile, 4, 0, 1),   // 5
            cell(Kind::Tile, 4, 1, 1),   // 6
            cell(Kind::Tile, 4, 2, 1),   // 7
            cell(Kind::Tile, 5, 0, 1),   // 8
            cell(Kind::Tile, 5, 1, 1),   // 9
            cell(Kind::Action, 6, 0, 1), // 10
        ]
    }

    #[test]
    fn arrows_walk_the_grid_naturally() {
        let items = sample();
        // En iyi sonuç → sağ → ilk şarkı; şarkıdan sol → kart.
        assert_eq!(neighbor(&items, 0, Dir::Right), Some(1));
        assert_eq!(neighbor(&items, 3, Dir::Left), Some(0));
        // Şarkılarda aşağı / yukarı.
        assert_eq!(neighbor(&items, 1, Dir::Down), Some(2));
        assert_eq!(neighbor(&items, 3, Dir::Up), Some(2));
        // Karttan aşağı: şarkılar atlanır, alttaki sanatçı sırasının ilk kartı.
        assert_eq!(neighbor(&items, 0, Dir::Down), Some(5));
        // Son şarkıdan aşağı: sütuna en yakın sanatçı.
        assert_eq!(neighbor(&items, 4, Dir::Down), Some(6));
        // Sanatçı sırasında sağ / sol, satır sonunda sarmaz.
        assert_eq!(neighbor(&items, 5, Dir::Right), Some(6));
        assert_eq!(neighbor(&items, 7, Dir::Right), None);
        assert_eq!(neighbor(&items, 5, Dir::Left), None);
        // Sanatçıdan yukarı: ilk sütun karta, ikinci sütun son şarkıya.
        assert_eq!(neighbor(&items, 5, Dir::Up), Some(0));
        assert_eq!(neighbor(&items, 6, Dir::Up), Some(4));
        // Albümden aşağı eyleme, eylemden aşağı baştan sona sarar (kart).
        assert_eq!(neighbor(&items, 9, Dir::Down), Some(10));
        assert_eq!(neighbor(&items, 10, Dir::Down), Some(0));
        assert_eq!(neighbor(&items, 0, Dir::Up), Some(10));
    }

    #[test]
    fn navigation_skips_disabled_and_the_footer() {
        let mut items = sample();
        items[10].disabled = true;
        // Eylem kullanılamaz: aşağı baştan sarar (sütuna en yakın: şarkı).
        assert_eq!(neighbor(&items, 9, Dir::Down), Some(1));
        let mut more = cell(Kind::More, 7, 0, 1);
        more.disabled = false;
        items.push(more);
        // Alt bağlantıya yalnız Tab ile gidilir, ok tuşları onu atlar.
        assert_eq!(neighbor(&items, 9, Dir::Down), Some(1));
        assert_eq!(step_linear(&items, 9, 1), 11);
        assert_eq!(step_linear(&items, 11, 1), 0);
        assert_eq!(step_linear(&items, 0, -1), 11);
    }

    #[test]
    fn best_result_prefers_an_exact_artist_over_a_loose_song() {
        let cands = [
            (HitKind::Artist, 90, MatchLevel::Exact),
            (HitKind::Album, 100, MatchLevel::Fuzzy),
            (HitKind::Song, 140, MatchLevel::Fuzzy),
        ];
        assert_eq!(pick_best(&cands), Some(0));
        // Eşit puanda sanatçı şarkıdan, ad başı eşleşmesi gevşek eşleşmeden önde.
        let cands = [
            (HitKind::Song, 100, MatchLevel::Fuzzy),
            (HitKind::Artist, 100, MatchLevel::Fuzzy),
        ];
        assert_eq!(pick_best(&cands), Some(1));
        let cands = [
            (HitKind::Album, 100, MatchLevel::Fuzzy),
            (HitKind::Playlist, 100, MatchLevel::Prefix),
        ];
        assert_eq!(pick_best(&cands), Some(1));
        assert_eq!(pick_best(&[]), None);
        // Hiç eşleşmeyen aday en iyi olmaz.
        assert_eq!(pick_best(&[(HitKind::Song, 0, MatchLevel::Fuzzy)]), None);
    }

    #[test]
    fn match_level_reads_exact_and_prefix_with_turkish_folding() {
        assert_eq!(
            MatchLevel::of("Sezen Aksu", "sezen aksu"),
            MatchLevel::Exact
        );
        assert_eq!(MatchLevel::of("Şebnem Ferah", "seb"), MatchLevel::Prefix);
        assert_eq!(MatchLevel::of("Kara Şimşek", "sim"), MatchLevel::Fuzzy);
    }

    #[test]
    fn actions_show_only_when_their_name_is_typed() {
        // "sessize" eylemin adında: çıkar. Kısa yazı (1 harf) çıkarmaz.
        assert!(action_matches("sessize", "sessize al", "x"));
        assert!(!action_matches("s", "sessize al", "x"));
        // Açıklamada geçen yazı ancak 4+ harfle eşleşir ("yeni" açıklamada, ama
        // "ye" çıkarmaz).
        assert!(action_matches("yeni", "ana sayfa", "yeni cikanlar"));
        assert!(!action_matches("ye", "ana sayfa", "yeni cikanlar"));
        // Şarkı adı gibi yazı: eylem yok.
        assert!(!action_matches("dal", "cal / duraklat", ""));
    }

    #[test]
    fn cells_follow_the_block_order() {
        let mut items: Vec<Item> = (0..9)
            .map(|_| Item::new(Kind::Song, "x", Action::Search(String::new())))
            .collect();
        let blocks = vec![
            Block::Top {
                best: Some(0),
                songs: vec![1, 2, 3],
            },
            Block::Tiles {
                title: "t",
                items: vec![4, 5],
            },
            Block::Actions { items: vec![6, 7] },
        ];
        assign_cells(&mut items, &blocks, Some(8));
        assert_eq!(
            items[0].cell,
            Cell {
                row: 0,
                col: 0,
                span: 3
            }
        );
        assert_eq!(
            items[3].cell,
            Cell {
                row: 2,
                col: 1,
                span: 1
            }
        );
        assert_eq!(
            items[5].cell,
            Cell {
                row: 3,
                col: 1,
                span: 1
            }
        );
        assert_eq!(
            items[7].cell,
            Cell {
                row: 5,
                col: 0,
                span: 1
            }
        );
        assert_eq!(
            items[8].cell,
            Cell {
                row: 6,
                col: 0,
                span: 1
            }
        );
        // En iyi sonuç yoksa şarkılar ilk sütunda.
        let blocks = vec![Block::Top {
            best: None,
            songs: vec![1, 2],
        }];
        assign_cells(&mut items, &blocks, None);
        assert_eq!(
            items[2].cell,
            Cell {
                row: 1,
                col: 0,
                span: 1
            }
        );
    }

    #[test]
    fn recent_searches_are_deduped_capped_and_ignore_short_ones() {
        let mut p = PaletteState::new();
        p.note_search("a");
        p.note_search("  perma ");
        p.note_search("Sezen");
        p.note_search("perma");
        assert_eq!(p.recent_searches, ["perma", "Sezen"]);
        for i in 0..20 {
            p.note_search(&format!("sorgu {i}"));
        }
        assert_eq!(
            p.recent_searches.len(),
            crate::settings::RECENT_SEARCHES_MAX
        );
        assert_eq!(p.recent_searches[0], "sorgu 19");
    }

    #[test]
    fn every_command_has_a_title() {
        assert!(commands().iter().all(|c| !c.1.is_empty()));
        crate::i18n::with_lang(crate::i18n::Lang::En, || {
            assert!(commands().iter().all(|c| !c.1.is_empty()));
        });
    }

    #[test]
    fn search_pill_width_shrinks_with_the_window_and_leaves_room() {
        use super::super::search_width_for;
        let widths: Vec<f32> = [760., 999., 1000., 1179., 1180., 1499., 1500., 1920.]
            .map(search_width_for)
            .to_vec();
        // Pencere daraldıkça hap küçülür, hiç büyümez; 420-520 aralığı ve dar 280.
        assert!(widths.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(widths, [280., 280., 420., 420., 480., 480., 520., 520.]);
        // Hap ortada: sol yarıda (pencere - hap) / 2 en az 240 px kalır (profil
        // hapı, gezinme düğmeleri, sekmeler).
        for w in [760., 1000., 1180., 1500., 1920.] {
            assert!((w - search_width_for(w)) / 2. >= 240.);
        }
    }

    #[test]
    fn selection_is_hidden_until_the_user_moves_in_an_empty_box() {
        let mut p = PaletteState::new();
        assert!(!p.selection_visible());
        p.moved = true;
        assert!(p.selection_visible());
        p.moved = false;
        p.query = "x".into();
        assert!(p.selection_visible());
    }
}
