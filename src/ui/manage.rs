//! Kütüphane yönetimi (Adım 13): beğenme, sağ tık menüsü, listeye ekleme,
//! liste oluşturma / adlandırma / silme, sürükleyip sıralama, çoklu seçim,
//! liste içinde bul (Ctrl+F), geri al (Ctrl+Z) ve eşitleme göstergesi.
//!
//! Her işlem önce ekranda (aynı karede) uygulanır, sonra motora gider; motor
//! kalıcı kuyruğa yazar ve Spotify'a arkadan gönderir (bkz.
//! `engine/wire.rs`). Ekrandaki güncelleme ile motorun kopyası aynı saf
//! fonksiyonlardan geçer (`patch_tracks`, `patch_library`): ikisi ayrışmaz.
//! Yazılamayan işlem sessizce kaybolmaz: başlık çubuğundaki eşitleme
//! simgesi kırmızıya döner, panelde "Yeniden dene" / "Bırak".

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gpui::{
    AnimationExt, AnyElement, ClickEvent, ClipboardItem, Context, FontWeight, KeyDownEvent,
    Modifiers, MouseButton, Pixels, Point, SharedString, Task, Window, deferred, div, prelude::*,
    px, rgb,
};

use super::{
    Shell,
    components::{
        Look, MENU_ITEM_HEIGHT, MENU_SEPARATOR_HEIGHT, MenuItem, Tooltip, kbd, menu_item,
        menu_panel, menu_separator, primary_button_sized, secondary_button,
    },
    fuzzy,
    history::Route,
    library::Loadable,
    pill::{PILL_BOTTOM, PILL_HEIGHT},
    shortcuts::Shortcut,
    text::line_with_tip,
    theme::*,
};
use crate::engine::{
    AddPosition, CollectionId, Command, DESCRIPTION_MAX, EditOp, FailKind, FailedEdit,
    LibraryCommand, LibraryItem, TrackRow, edit_failed_notice, folder_contains,
    folder_start_marker, new_folder_id, new_temp_id, patch_library, patch_tracks, same_edit,
    touched_collection,
};
use crate::{t, tf, tp};

/// Sağ tık menüsünün ve alt menünün eni; alt menünün en büyük boyu.
const MENU_WIDTH: f32 = 240.;
const SUBMENU_MAX_HEIGHT: f32 = 360.;
/// Geri al bildirimi bu kadar görünür.
const TOAST_FOR: Duration = Duration::from_secs(6);
const PROMPT_WIDTH: f32 = 400.;
const SYNC_PANEL_WIDTH: f32 = 360.;
/// Liste adının en çok uzunluğu (Spotify 100 karakterde kesiyor).
const NAME_MAX: usize = 100;

/// Tam pencere katmanının (menü, pencere, panel) başlık çubuğunun altını
/// kaplayan engelleyici çocuğu: katmanın görünmez zemini fareyi arkadaki
/// düğmelerden ve satırlardan ayırır, böylece menüyü açan düğme "üzerine
/// gelinmiş" görünmez ve ipucu menünün üstünde kalmaz. Katmanın kendisi
/// engellemez: o zaman başlık çubuğundaki pencere düğmeleri ve sürükleme alanı
/// da ölürdü (bkz. `palette.rs`). Üst katmanın fare dinleyicisi bu bölgede
/// çalışmaz (engelleyici onu örter): kapatma dinleyicileri bu çocuğa da konur.
pub(super) fn layer_blocker() -> gpui::Div {
    div()
        .absolute()
        .top(px(super::TITLEBAR_HEIGHT))
        .left_0()
        .right_0()
        .bottom_0()
        .occlude()
}

// ── Durum ───────────────────────────────────────────────────────────────

/// Hangi listede: kütüphane listesi ya da bir sayfadaki şarkılar
/// ("top:<sanatçı>", "album:<albüm>", "search:<sorgu>").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ListKey {
    Collection(CollectionId),
    Page(String),
}

/// Seçili satırlar (tek tık, Ctrl ile ekle / çıkar, Shift ile aralık).
#[derive(Default)]
pub struct Selection {
    pub list: Option<ListKey>,
    pub rows: BTreeSet<usize>,
    pub anchor: Option<usize>,
}

/// Bir yazma işlemi: işlem, şarkıların bilgileri, (albüm kaydında) kartı.
pub type Edit = (EditOp, Vec<TrackRow>, Option<LibraryItem>);

struct Undo {
    label: String,
    ops: Vec<Edit>,
    /// Geri alınacak işlemlerin kendisi: biri Spotify'a yazılamazsa bu kayıt
    /// düşer (yazılmamış bir işlemi geri alacak bir şey yok).
    done: Vec<EditOp>,
}

pub struct Toast {
    pub text: String,
    pub undo: bool,
    pub problem: bool,
    /// Bildirimin anlattığı yazma işlemleri. Biri kalıcı olarak yazılamazsa
    /// "yapıldı" diyen bu bildirim kalkar (hata şeridiyle çelişmesin).
    pub edits: Vec<EditOp>,
}

pub struct Find {
    pub query: String,
    /// (liste, sorgu) → görünen satırlar; liste değişince yeniden.
    cache: RefCell<Option<(usize, String, Arc<Vec<usize>>)>>,
}

impl Find {
    pub fn with_query(query: String) -> Self {
        Self {
            query,
            cache: RefCell::new(None),
        }
    }
}

#[derive(Clone)]
pub enum PromptKind {
    Create {
        rows: Vec<TrackRow>,
        open: bool,
    },
    Rename {
        playlist: String,
        previous: String,
    },
    /// Liste açıklaması (çok satır, en çok `DESCRIPTION_MAX` karakter).
    Description {
        playlist: String,
        previous: String,
    },
    /// Yeni klasör; `move`: oluşunca bu liste klasörün içine taşınır.
    FolderCreate {
        move_playlist: Option<(String, String)>,
    },
    FolderRename {
        folder: String,
        previous: String,
    },
    /// Spotify Connect aygıt adı (bkz. `settings::connect_section`).
    DeviceName,
}

pub struct Prompt {
    pub kind: PromptKind,
    pub text: String,
}

/// Kart menüsünün hedefi.
#[derive(Clone, Debug)]
pub enum CardTarget {
    Collection {
        id: CollectionId,
        title: String,
    },
    Artist(String),
    Album {
        uri: String,
        item: Option<LibraryItem>,
    },
    /// Kütüphane klasörü (Spotify'ın klasör işareti).
    Folder {
        key: String,
    },
}

#[derive(Clone)]
pub enum Act {
    Enqueue(Vec<TrackRow>),
    PlayNext(Vec<TrackRow>),
    AddTo {
        playlist: String,
        title: String,
        rows: Vec<TrackRow>,
    },
    NewPlaylist(Vec<TrackRow>),
    Like {
        rows: Vec<TrackRow>,
        on: bool,
    },
    GoAlbum(TrackRow),
    GoArtist(TrackRow, usize),
    CopyLinks(Vec<String>),
    RemoveRows {
        playlist: String,
        rows: Vec<(usize, TrackRow)>,
    },
    Open(CardTarget),
    Play(CardTarget),
    Rename {
        playlist: String,
        name: String,
    },
    /// Listenin açıklamasını düzenle (ad kutusu gibi, çok satır).
    EditDescription {
        playlist: String,
    },
    /// Ortak listeyi aç / kapa.
    SetCollaborative {
        playlist: String,
        on: bool,
    },
    /// Yeni klasör; `move` verilmişse liste oluşan klasöre taşınır.
    NewFolder {
        move_playlist: Option<(String, String)>,
    },
    RenameFolder {
        folder: String,
        name: String,
    },
    DeleteFolder {
        folder: String,
    },
    MoveToFolder {
        playlist: String,
        /// Hedef klasör; `None` köke (klasörden çıkar).
        folder: Option<String>,
        /// Eski klasör (geri alma için; `None` kökteydi).
        previous: Option<String>,
    },
    Delete {
        playlist: String,
        own: bool,
    },
    SaveAlbum {
        uri: String,
        on: bool,
        item: Option<LibraryItem>,
    },
    /// Sanatçıyı takip et / takipten çık (`item`: kütüphane kartı).
    FollowArtist {
        uri: String,
        on: bool,
        item: Option<LibraryItem>,
    },
    /// Spotify masaüstü uygulamasında (yoksa tarayıcıda) aç.
    OpenInSpotify(String),
    /// Sanatçıyı kara listeye al / listeden çıkar (bkz. `settings::set_artist_blocked`).
    BlockArtist {
        name: String,
        uri: String,
        on: bool,
    },
    /// Arkadaş etkinliği satırı (bkz. `friends`): şarkıyı bağlamıyla çal, albüme
    /// ya da sanatçıya git (adresler satırın kendisinde bilinir).
    PlayFriend(Box<crate::engine::Friend>),
    GoAlbumUri(String),
    GoArtistUri(String),
    /// Kütüphanede en üste sabitle / sabitlemeyi kaldır (yerel; bkz. `library_layout`).
    Pin {
        key: String,
        on: bool,
    },
    /// Kütüphane sıralamasını seç (sıralama menüsü).
    SetSort(super::shelf::Sort),
    /// Şarkı künyesi penceresi (bkz. `credits`).
    ShowCredits(TrackRow),
    /// Radyoyu başlat (bkz. `listening`): şarkı / sanatçı / albüm / liste adresi + görünen ad.
    StartRadio {
        seed: String,
        title: String,
    },
    /// Şarkıları gizle: `context` bu liste / albüm, `None` hiçbir yerde (yerel; bkz. `listening`).
    HideSongs {
        rows: Vec<TrackRow>,
        context: Option<String>,
    },
    /// Gizlemeyi kaldır (bu bağlamdaki ve genel kayıt).
    ShowSongs {
        rows: Vec<TrackRow>,
        context: Option<String>,
    },
}

pub enum Entry {
    Item {
        icon: &'static str,
        label: String,
        shortcut: Option<&'static str>,
        danger: bool,
        act: Option<Act>,
        sub: Vec<(String, Act)>,
    },
    Separator,
}

pub struct Menu {
    pub at: Point<Pixels>,
    pub entries: Vec<Entry>,
    /// Klavyeyle gezilen öğe.
    pub cursor: Option<usize>,
    /// Alt menüsü açık öğe ve alt menüde gezilen öğe.
    pub sub: Option<usize>,
    pub sub_cursor: Option<usize>,
}

#[derive(Default)]
pub struct Sync {
    pub pending: usize,
    pub failed: Vec<FailedEdit>,
}

pub struct ManageState {
    pub sel: Selection,
    pub menu: Option<Menu>,
    pub prompt: Option<Prompt>,
    pub toast: Option<Toast>,
    toast_task: Option<Task<()>>,
    undo: Vec<Undo>,
    pub deleted: HashMap<String, (usize, LibraryItem)>,
    pub find: Option<Find>,
    pub sync: Sync,
    pub sync_open: bool,
    liked: RefCell<Option<(usize, Rc<HashSet<String>>)>>,
    /// Farenin üzerinde olduğu şarkı satırı (yalnız onda tıklanabilir adlar
    /// ve kalp kurulur, bkz. `song_row`).
    pub hover: Option<(ListKey, usize)>,
    /// Yazılan şarkıların adları (eşitleme panelinde anmak için).
    names: HashMap<String, String>,
}

impl ManageState {
    pub fn new() -> Self {
        Self {
            sel: Selection::default(),
            menu: None,
            prompt: None,
            toast: None,
            toast_task: None,
            undo: Vec::new(),
            deleted: HashMap::new(),
            find: None,
            sync: Sync::default(),
            sync_open: false,
            liked: RefCell::new(None),
            hover: None,
            names: HashMap::new(),
        }
    }

    /// Ekranda klavyeyi kendine alan bir katman açık mı (menü, ad kutusu)?
    pub fn modal(&self) -> bool {
        self.menu.is_some() || self.prompt.is_some()
    }

    /// Kalıcı olarak yazılamayan (ya da bırakılan) işlemlerin "yapıldı"
    /// bildirimi ve geri alma kaydı kalkar: Spotify'a ulaşmamış bir işlem için
    /// "Geri al" önermek de, "Beğenilenlerden çıkarıldı" demek de yanlış olur.
    /// Bir şey kalktıysa `true`.
    pub fn retract_failed(&mut self, failed: &[EditOp]) -> bool {
        let is_failed = |op: &EditOp| failed.iter().any(|f| same_edit(op, f));
        let before = self.undo.len();
        self.undo.retain(|u| !u.done.iter().any(is_failed));
        let mut changed = self.undo.len() != before;
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.edits.iter().any(is_failed))
        {
            self.toast = None;
            self.toast_task = None;
            changed = true;
        }
        changed
    }
}

/// "spotify:track:ID" → "https://open.spotify.com/track/ID".
pub fn web_link(uri: &str) -> Option<String> {
    let mut parts = uri.split(':');
    let (Some("spotify"), Some(kind), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    Some(format!("https://open.spotify.com/{kind}/{id}"))
}

/// Kendi listen mi (adı değiştirilebilir, satırları çıkarılabilir)? Kendi
/// listelerinin açıklaması yalnız "N şarkı"; Spotify'ın ve takip edilenlerin
/// önünde ad var.
pub fn is_own(item: &LibraryItem) -> bool {
    matches!(item.id, CollectionId::Playlist(_))
        && crate::i18n::canon::plain_song_count(&item.subtitle).is_some()
}

/// "Bir değişiklik yazılamadı" şeridi, yazılamayan işlem kalmadığında (bırakıldı
/// ya da yeniden deneniyor) yanlıştır: kalkmalı mı?
fn stale_failure_notice(notice: Option<&super::player::Notice>, failed_left: usize) -> bool {
    failed_left == 0 && notice.is_some_and(|n| n.text == edit_failed_notice())
}

/// İşlemin konusu: neden metni buna ve hatanın türüne göre seçilir.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Subject {
    /// Şarkıyı beğenme / vazgeçme.
    Like,
    Album,
    /// Sanatçıyı takip etme / takipten çıkma.
    Follow,
    /// Listeye şarkı ekleme / çıkarma / sıralama.
    ListContent,
    Rename,
    Create,
    /// Liste açıklaması / ortak liste (listenin kendi ayarları).
    Meta,
    /// Klasör işlemleri (kök liste).
    Folder,
    /// Listeyi silme / geri getirme.
    Remove,
}

fn subject_of(op: &EditOp) -> Subject {
    match op {
        EditOp::Save { uri } | EditOp::Unsave { uri } if uri.starts_with("spotify:album:") => {
            Subject::Album
        }
        EditOp::Save { .. } | EditOp::Unsave { .. } => Subject::Like,
        EditOp::Follow { .. } | EditOp::Unfollow { .. } => Subject::Follow,
        EditOp::AddToPlaylist { .. }
        | EditOp::RemoveFromPlaylist { .. }
        | EditOp::MovePlaylistItems { .. } => Subject::ListContent,
        EditOp::RenamePlaylist { .. } => Subject::Rename,
        EditOp::CreatePlaylist { .. } => Subject::Create,
        EditOp::SetPlaylistDescription { .. } | EditOp::SetPlaylistCollaborative { .. } => {
            Subject::Meta
        }
        EditOp::CreateFolder { .. }
        | EditOp::RestoreFolder { .. }
        | EditOp::RenameFolder { .. }
        | EditOp::DeleteFolder { .. }
        | EditOp::MovePlaylistToFolder { .. } => Subject::Folder,
        EditOp::DeletePlaylist { .. } | EditOp::RestorePlaylist { .. } => Subject::Remove,
    }
}

/// İşlemin yazdığı mevcut liste (varsa).
fn playlist_of(op: &EditOp) -> Option<&str> {
    match op {
        EditOp::AddToPlaylist { playlist, .. }
        | EditOp::RemoveFromPlaylist { playlist, .. }
        | EditOp::RenamePlaylist { playlist, .. }
        | EditOp::SetPlaylistDescription { playlist, .. }
        | EditOp::SetPlaylistCollaborative { playlist, .. }
        | EditOp::MovePlaylistItems { playlist, .. } => Some(playlist),
        _ => None,
    }
}

/// Yazma hatasının insan dilinde açıklaması: işleme ve hatanın türüne göre
/// (ham hata metni gösterilmez, tahmin edilmez). "Liste sana ait değil" yalnız
/// bilinen BAŞKASININ listesine yazılırken (`foreign_list`) söylenir.
pub fn explain_edit_failure(op: &EditOp, kind: FailKind, foreign_list: bool) -> &'static str {
    let subject = subject_of(op);
    match kind {
        FailKind::Forbidden => match (subject, foreign_list) {
            (Subject::Like, _) => t!(FailLikeDenied),
            (Subject::Album, _) => t!(FailAlbumDenied),
            (Subject::Follow, _) => t!(FailFollowDenied),
            (Subject::ListContent, true) => t!(FailListContentNotYours),
            (Subject::ListContent, false) => t!(FailListDenied),
            (Subject::Rename, true) => t!(FailRenameNotYours),
            (Subject::Rename, false) => t!(FailRenameDenied),
            (Subject::Create, _) => t!(FailCreateDenied),
            (Subject::Meta, _) => t!(FailMetaDenied),
            (Subject::Folder, _) => t!(FailFolderDenied),
            (Subject::Remove, _) => t!(FailRemoveDenied),
        },
        FailKind::NotFound => match subject {
            Subject::Like => t!(FailGoneSong),
            Subject::Album => t!(FailGoneAlbum),
            Subject::Follow => t!(FailGoneArtist),
            Subject::ListContent => t!(FailGoneListOrSong),
            Subject::Rename | Subject::Remove | Subject::Meta => t!(FailGoneList),
            Subject::Create => t!(FailGoneCreate),
            Subject::Folder => t!(FailGoneFolder),
        },
        FailKind::Offline => t!(FailOffline),
        FailKind::Expired => t!(FailExpired),
        FailKind::Unauthorized => t!(FailUnauthorized),
        FailKind::Conflict => t!(FailConflict),
        FailKind::Server => t!(FailServer),
        FailKind::RateLimited => t!(FailRateLimited),
        FailKind::Unsupported => t!(FailUnsupported),
        FailKind::Invalid => t!(FailInvalid),
        FailKind::ParentFailed => t!(FailParent),
        FailKind::Unknown => t!(FailUnknown),
    }
}

impl Shell {
    // ── Beğenme ─────────────────────────────────────────────────────────

    /// Beğenilen şarkıların adresleri (liste değişince yeniden kurulur).
    pub(super) fn liked_set(&self) -> Rc<HashSet<String>> {
        let tracks = self.library.tracks(&CollectionId::Liked);
        let key = tracks.map_or(0, |t| Arc::as_ptr(t) as usize);
        let mut cache = self.manage.liked.borrow_mut();
        if let Some((k, set)) = cache.as_ref()
            && *k == key
        {
            return set.clone();
        }
        let set: Rc<HashSet<String>> = Rc::new(
            tracks
                .map(|t| t.iter().map(|r| r.uri.clone()).collect())
                .unwrap_or_default(),
        );
        *cache = Some((key, set.clone()));
        set
    }

    pub(super) fn is_liked(&self, uri: &str) -> bool {
        self.liked_set().contains(uri)
    }

    /// Şarkıları beğen / beğenmekten vazgeç (geri alınabilir).
    pub(super) fn set_liked(&mut self, rows: Vec<TrackRow>, on: bool, cx: &mut Context<Self>) {
        let liked = self.liked_set();
        let rows: Vec<TrackRow> = rows
            .into_iter()
            .filter(|r| liked.contains(&r.uri) != on)
            .collect();
        if rows.is_empty() {
            return;
        }
        let ops: Vec<Edit> = rows
            .iter()
            .map(|r| {
                let uri = r.uri.clone();
                (
                    if on {
                        EditOp::Save { uri }
                    } else {
                        EditOp::Unsave { uri }
                    },
                    vec![r.clone()],
                    None,
                )
            })
            .collect();
        let label = match (rows.len(), on) {
            (1, true) => t!(ToastLiked).to_string(),
            (1, false) => t!(ToastUnliked).to_string(),
            (n, true) => tf!(ToastLikedN, n),
            (n, false) => tf!(ToastUnlikedN, n),
        };
        let undo = inverse_all(&ops);
        self.commit(ops, Some((label, undo)), cx);
    }

    /// Albümü kütüphaneye kaydet / kütüphaneden çıkar.
    pub(super) fn set_album_saved(
        &mut self,
        uri: String,
        on: bool,
        item: Option<LibraryItem>,
        cx: &mut Context<Self>,
    ) {
        let op = if on {
            EditOp::Save { uri: uri.clone() }
        } else {
            EditOp::Unsave { uri: uri.clone() }
        };
        // Geri alırken albümün kartı yerine konsun.
        let card = item.clone().or_else(|| {
            self.library
                .item(&CollectionId::Album(uri.clone()))
                .cloned()
        });
        let ops = vec![(op.clone(), Vec::new(), card.clone())];
        let undo = vec![(
            op.inverse().expect("kaydetmenin tersi var"),
            Vec::new(),
            card,
        )];
        let label = if on {
            t!(ToastAlbumAdded)
        } else {
            t!(ToastAlbumRemoved)
        };
        self.commit(ops, Some((label.into(), undo)), cx);
    }

    pub(super) fn album_saved(&self, uri: &str) -> bool {
        self.library
            .item(&CollectionId::Album(uri.to_string()))
            .is_some()
    }

    // ── Sanatçı takibi ──────────────────────────────────────────────────

    /// Sanatçı takip ediliyor mu? Ekrandaki kütüphane bekleyen (henüz
    /// yazılmamış) takip / bırakmayı da içerir: düğme anında döner.
    pub(super) fn artist_followed(&self, uri: &str) -> bool {
        self.library
            .item(&CollectionId::Artist(uri.to_string()))
            .is_some()
    }

    /// Sanatçının kartı (ad ve portre), bilinen yerlerden: kütüphane, açılmış
    /// sanatçı sayfası, benzer sanatçılar, arama sonuçları. Hiçbirinde yoksa
    /// `None` (menü "Takip et" önermez: adsız kart eklenmez).
    pub(super) fn known_artist(&self, uri: &str) -> Option<LibraryItem> {
        use super::pages::PageLoad;
        let card = |name: &str, image: &str| crate::engine::artist_item(uri, name, image);
        if let Some(item) = self.library.item(&CollectionId::Artist(uri.to_string())) {
            return Some(item.clone());
        }
        if let Some(PageLoad::Ready(p)) = self.pages.artists.get(uri) {
            return Some(card(&p.name, &p.image));
        }
        for list in self.pages.related.values() {
            if let Some(a) = list.artists.iter().find(|a| a.uri == uri) {
                return Some(card(&a.name, &a.image));
            }
        }
        for page in self.pages.artists.values() {
            if let PageLoad::Ready(p) = page
                && let Some(a) = p.related.iter().find(|a| a.uri == uri)
            {
                return Some(card(&a.name, &a.image));
            }
        }
        if let Some(PageLoad::Ready(r)) = self.pages.search.as_ref().map(|s| &s.results)
            && let Some(h) = r.artists.iter().find(|h| h.uri == uri)
        {
            return Some(card(&h.name, &h.image));
        }
        None
    }

    /// Eşitleme panelinde anılacak ad (bilinmiyorsa "bu sanatçı").
    fn artist_label(&self, uri: &str) -> String {
        match self
            .known_artist(uri)
            .map(|a| a.name)
            .filter(|n| !n.is_empty())
        {
            Some(name) => format!("“{name}”"),
            None => t!(AnArtistFallback).into(),
        }
    }

    /// Sanatçıyı takip et / takipten çık (geri alınabilir). `item`: kartı
    /// (kütüphaneye anında yerleşir; geri alırken de aynı kart döner).
    pub(super) fn set_artist_followed(
        &mut self,
        uri: String,
        on: bool,
        item: Option<LibraryItem>,
        cx: &mut Context<Self>,
    ) {
        if self.artist_followed(&uri) == on {
            return;
        }
        let card = item.or_else(|| self.known_artist(&uri));
        // Takip için kart şart (adsız kart eklenmez); bırakırken gerekmez.
        if on && card.is_none() {
            return;
        }
        let op = if on {
            EditOp::Follow { uri: uri.clone() }
        } else {
            EditOp::Unfollow { uri: uri.clone() }
        };
        let name = card.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let ops = vec![(op.clone(), Vec::new(), card.clone())];
        let undo = vec![(op.inverse().expect("takibin tersi var"), Vec::new(), card)];
        let label = if on {
            tf!(ToastArtistFollowed, name)
        } else {
            tf!(ToastArtistUnfollowed, name)
        };
        self.commit(ops, Some((label, undo)), cx);
    }

    // ── Yazma, geri alma ────────────────────────────────────────────────

    /// İşlemleri ekranda hemen uygula ve motora gönder. `undo`: geri al
    /// bildirimi ve Ctrl+Z ile uygulanacak işlemler.
    pub(super) fn commit(
        &mut self,
        ops: Vec<Edit>,
        undo: Option<(String, Vec<Edit>)>,
        cx: &mut Context<Self>,
    ) {
        let done: Vec<EditOp> = ops.iter().map(|(op, _, _)| op.clone()).collect();
        // Aynı listeye dokunan işlemler tek çalışma kopyası üzerinde uygulanır ve
        // ekrana bir kez yazılır (yüzlerce taşımalık sıralamada her işlemde listeyi
        // baştan kopyalamak arayüzü takıyordu).
        let mut work: HashMap<CollectionId, (Vec<TrackRow>, bool)> = HashMap::new();
        for (op, rows, item) in &ops {
            // Eşitleme panelinde adıyla anılsın (satır listeden çıkmış olabilir).
            for r in rows {
                self.manage.names.insert(r.uri.clone(), r.name.clone());
            }
            self.patch_screen(op, rows, item.as_ref(), &mut work);
            self.engine.send(Command::Library(LibraryCommand::Edit {
                op: op.clone(),
                rows: rows.clone(),
                item: item.clone(),
            }));
        }
        for (id, (list, changed)) in work {
            if changed {
                self.library
                    .collections
                    .insert(id, Loadable::Loaded(Arc::new(list)));
            }
        }
        // Satır sıraları değişmiş olabilir: seçim bırakılır.
        self.manage.sel = Selection::default();
        self.library.selected_row = None;
        if let Some((label, ops)) = undo {
            self.manage.undo.push(Undo {
                label: label.clone(),
                ops,
                done: done.clone(),
            });
            if self.manage.undo.len() > 50 {
                self.manage.undo.remove(0);
            }
            self.show_toast_for(label, true, false, done, cx);
        }
        if self.all.open {
            self.recompute_all();
        }
        cx.notify();
    }

    /// Ekrandaki kopyayı işleme göre güncelle (motorla aynı fonksiyonlar). Listeler
    /// `work`te (şarkılar, değişti mi) toplanır; `commit` döngü bitince ekrana yazar.
    fn patch_screen(
        &mut self,
        op: &EditOp,
        rows: &[TrackRow],
        item: Option<&LibraryItem>,
        work: &mut HashMap<CollectionId, (Vec<TrackRow>, bool)>,
    ) {
        if let Some(id) = touched_collection(op) {
            if !work.contains_key(&id) {
                let start = match self.library.collections.get(&id) {
                    Some(Loadable::Loaded(t)) => Some((**t).clone()),
                    _ if matches!(op, EditOp::CreatePlaylist { .. }) => Some(Vec::new()),
                    _ => None,
                };
                if let Some(list) = start {
                    work.insert(id.clone(), (list, false));
                }
            }
            if let Some((list, changed)) = work.get_mut(&id) {
                *changed |= patch_tracks(op, rows, list);
            }
        }
        if let Loadable::Loaded(lib) = &mut self.library.library {
            patch_library(op, item, lib, &mut self.manage.deleted);
        }
    }

    /// Ctrl+Z: son işlemi geri al.
    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.manage.undo.pop() else {
            self.show_toast(t!(ToastNothingToUndo).into(), false, false, cx);
            return;
        };
        // Silinen liste geri gelir; açık liste silindiyse geri dönülür.
        let done: Vec<EditOp> = undo.ops.iter().map(|(op, _, _)| op.clone()).collect();
        self.commit(undo.ops, None, cx);
        self.show_toast_for(
            tf!(ToastUndone, crate::i18n::lower(&undo.label)),
            false,
            false,
            done,
            cx,
        );
    }

    pub(super) fn show_toast(
        &mut self,
        text: String,
        undo: bool,
        problem: bool,
        cx: &mut Context<Self>,
    ) {
        self.show_toast_for(text, undo, problem, Vec::new(), cx);
    }

    /// `edits`: bildirimin anlattığı yazma işlemleri (yazılamazlarsa bildirim kalkar).
    fn show_toast_for(
        &mut self,
        text: String,
        undo: bool,
        problem: bool,
        edits: Vec<EditOp>,
        cx: &mut Context<Self>,
    ) {
        self.manage.toast = Some(Toast {
            text,
            undo,
            problem,
            edits,
        });
        self.manage.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            let _ = this.update(cx, |shell, cx| {
                shell.manage.toast = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Motorun yazma kuyruğu durumu.
    pub(super) fn on_edit_status(
        &mut self,
        pending: usize,
        failed: Vec<FailedEdit>,
        cx: &mut Context<Self>,
    ) {
        // Bu durumda yeni yazılamayanlar: "yapıldı" diyen bildirimleri kalkar.
        let fresh: Vec<EditOp> = failed
            .iter()
            .filter(|f| !self.manage.sync.failed.iter().any(|old| old.id == f.id))
            .map(|f| f.op.clone())
            .collect();
        self.manage.sync = Sync { pending, failed };
        if self.manage.sync.pending == 0 && self.manage.sync.failed.is_empty() {
            self.manage.sync_open = false;
        }
        if !fresh.is_empty() && self.manage.retract_failed(&fresh) {
            cx.notify();
        }
        // Yazılamayan işlem kalmadı (bırakıldı ya da yeniden deneniyor):
        // "yazılamadı, simgeden dene" şeridi artık yanlış, kalkar.
        if stale_failure_notice(self.state.notice.as_ref(), self.manage.sync.failed.len()) {
            self.state.notice = None;
            cx.notify();
        }
    }

    /// "Bırak": işlem kuyruktan gider, motor listeyi / kütüphaneyi Spotify'daki
    /// son bilinen hâle döndürüp yeniden gönderir.
    fn dismiss_failed(&mut self, id: i64, cx: &mut Context<Self>) {
        let failed: Vec<EditOp> = self
            .manage
            .sync
            .failed
            .iter()
            .filter(|f| f.id == id)
            .map(|f| f.op.clone())
            .collect();
        // Bırakılan oluşturma: o liste hiç oluşmayacak, açıksa Beğenilenler'e dön.
        for op in &failed {
            if let EditOp::CreatePlaylist { temp_id, .. } = op
                && self.library.selected.as_ref() == Some(&CollectionId::Playlist(temp_id.clone()))
            {
                self.select_collection(CollectionId::Liked, cx);
            }
        }
        self.manage.retract_failed(&failed);
        self.engine
            .send(Command::Library(LibraryCommand::DismissEdit(id)));
        self.show_toast(t!(ToastDropped).into(), false, false, cx);
    }

    /// Geçici adresli liste Spotify'da oluştu: her yerde gerçek adrese çevir.
    pub(super) fn on_created(&mut self, temp: String, uri: String, cx: &mut Context<Self>) {
        let from = CollectionId::Playlist(temp.clone());
        let to = CollectionId::Playlist(uri.clone());
        if let Loadable::Loaded(lib) = &mut self.library.library {
            for p in lib.playlists.iter_mut().filter(|p| p.id == from) {
                p.id = to.clone();
            }
        }
        if let Some(tracks) = self.library.collections.remove(&from) {
            self.library.collections.insert(to.clone(), tracks);
        }
        if let Some(t) = self.library.titles.remove(&from) {
            self.library.titles.insert(to.clone(), t);
        }
        if self.library.selected.as_ref() == Some(&from) {
            self.library.selected = Some(to.clone());
        }
        for r in self.recent.iter_mut().filter(|r| **r == temp) {
            *r = uri.clone();
        }
        self.pages
            .history
            .rename(&Route::Collection(from), Route::Collection(to));
        for undo in &mut self.manage.undo {
            for (op, _, _) in &mut undo.ops {
                rewrite_playlist(op, &temp, &uri);
            }
        }
        cx.notify();
    }

    // ── Seçim ───────────────────────────────────────────────────────────

    /// Fare bir satıra girdi / çıktı: yalnız liste görünümü yenilenir.
    pub(super) fn hover_row(
        &mut self,
        list: ListKey,
        ix: usize,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (list, ix);
        let before = self.manage.hover.clone();
        if hovered {
            self.manage.hover = Some(key);
        } else if self.manage.hover.as_ref() == Some(&key) {
            self.manage.hover = None;
        }
        if self.manage.hover != before {
            self.views.list.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn is_selected(&self, list: &ListKey, ix: usize) -> bool {
        self.manage.sel.list.as_ref() == Some(list) && self.manage.sel.rows.contains(&ix)
    }

    pub(super) fn select_single(&mut self, list: ListKey, ix: usize) {
        self.manage.sel = Selection {
            list: Some(list),
            rows: BTreeSet::from([ix]),
            anchor: Some(ix),
        };
    }

    /// Satıra tıklandı: Ctrl ekler / çıkarır, Shift aralık seçer, çift tık çalar.
    pub(super) fn click_row(
        &mut self,
        list: ListKey,
        ix: usize,
        modifiers: Modifiers,
        count: usize,
        cx: &mut Context<Self>,
    ) {
        if count >= 2 && !modifiers.control && !modifiers.shift {
            self.play_in(&list, ix, cx);
            return;
        }
        let same = self.manage.sel.list.as_ref() == Some(&list);
        if modifiers.control {
            if !same {
                self.manage.sel = Selection {
                    list: Some(list.clone()),
                    ..Default::default()
                };
            }
            let rows = &mut self.manage.sel.rows;
            if !rows.remove(&ix) {
                rows.insert(ix);
            }
            self.manage.sel.anchor = Some(ix);
        } else if modifiers.shift && same && self.manage.sel.anchor.is_some() {
            let a = self.manage.sel.anchor.unwrap_or(ix);
            let (lo, hi) = (a.min(ix), a.max(ix));
            let visible = self.visible_rows(&list);
            self.manage.sel.rows = (lo..=hi)
                .filter(|r| visible.as_ref().is_none_or(|v| v.contains(r)))
                .collect();
        } else {
            self.select_single(list.clone(), ix);
        }
        if let ListKey::Collection(_) = list {
            self.library.selected_row = Some(ix);
        }
        cx.notify();
    }

    /// Ctrl+A: açık listenin (süzülmüşse görünen) bütün satırları.
    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected.clone().filter(|_| !self.page_open()) else {
            return;
        };
        let list = ListKey::Collection(id);
        let Some(tracks) = self.list_tracks(&list) else {
            return;
        };
        let rows: BTreeSet<usize> = match self.visible_rows(&list) {
            Some(v) => v.iter().copied().collect(),
            None => (0..tracks.len()).collect(),
        };
        self.manage.sel = Selection {
            list: Some(list),
            rows,
            anchor: None,
        };
        cx.notify();
    }

    /// İşlem yapılacak satırlar: sağ tıklanan satır seçimin içindeyse bütün
    /// seçim, değilse yalnız o satır (ve seçim ona taşınır).
    fn target_rows(&mut self, list: &ListKey, ix: usize) -> Vec<usize> {
        if !self.is_selected(list, ix) {
            self.select_single(list.clone(), ix);
            if let ListKey::Collection(_) = list {
                self.library.selected_row = Some(ix);
            }
        }
        self.manage.sel.rows.iter().copied().collect()
    }

    // ── Liste içinde bul ────────────────────────────────────────────────

    pub(super) fn open_find(&mut self, cx: &mut Context<Self>) {
        if self.page_open() || self.all.open || self.library.selected.is_none() {
            return;
        }
        if self.manage.find.is_none() {
            self.manage.find = Some(Find {
                query: String::new(),
                cache: RefCell::new(None),
            });
        }
        self.refocus_anchor = true;
        cx.notify();
    }

    pub(super) fn close_find(&mut self, cx: &mut Context<Self>) {
        if self.manage.find.take().is_some() {
            cx.notify();
        }
    }

    /// Süzgeçten geçen satırlar (süzgeç yoksa `None`: hepsi).
    pub(super) fn visible_rows(&self, list: &ListKey) -> Option<Arc<Vec<usize>>> {
        let ListKey::Collection(_) = list else {
            return None;
        };
        let find = self
            .manage
            .find
            .as_ref()
            .filter(|f| !f.query.trim().is_empty())?;
        let tracks = self.list_tracks(list)?;
        let key = Arc::as_ptr(&tracks) as usize;
        if let Some((k, q, rows)) = find.cache.borrow().as_ref()
            && *k == key
            && *q == find.query
        {
            return Some(rows.clone());
        }
        // Her kelime şarkı adında, sanatçıda ya da albümde geçmeli.
        let words: Vec<String> = fuzzy::fold(&find.query)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let rows: Arc<Vec<usize>> = Arc::new(
            tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    let hay = fuzzy::fold(&format!("{} {} {}", t.name, t.artists, t.album));
                    words.iter().all(|w| hay.contains(w.as_str()))
                })
                .map(|(i, _)| i)
                .collect(),
        );
        *find.cache.borrow_mut() = Some((key, find.query.clone(), rows.clone()));
        Some(rows)
    }

    /// Bul açıkken yazılanlar. İşlendiyse `true`.
    pub(super) fn find_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(find) = self.manage.find.as_mut() else {
            return false;
        };
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control;
        match ks.key.as_str() {
            "escape" => {
                self.close_find(cx);
                return true;
            }
            "backspace" => {
                if ctrl {
                    let trimmed = find.query.trim_end().len();
                    let cut = find.query[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                    find.query.truncate(cut);
                } else {
                    find.query.pop();
                }
            }
            "v" if ctrl => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    find.query.push_str(text.lines().next().unwrap_or_default());
                }
            }
            "space" if !ctrl && !find.query.is_empty() => find.query.push(' '),
            _ if !ctrl && !ks.modifiers.alt => {
                let Some(ch) = ks
                    .key_char
                    .as_deref()
                    .filter(|c| !c.is_empty() && !c.chars().any(char::is_control))
                else {
                    return false;
                };
                find.query.push_str(ch);
            }
            _ => return false,
        }
        // Süzgeç değişti: seçim ve kaydırma baştan.
        self.manage.sel = Selection::default();
        self.library.selected_row = None;
        self.library.list_glide.cancel();
        let x = self.library.scroll.0.borrow().base_handle.offset().x;
        self.library
            .scroll
            .0
            .borrow()
            .base_handle
            .set_offset(gpui::point(x, px(0.)));
        cx.notify();
        true
    }

    // ── Sürükleyip sıralama ─────────────────────────────────────────────

    /// Kendi listende satır(lar) bırakıldı: `target` satırının yerine
    /// (aşağı taşırken arkasına, yukarı taşırken önüne).
    pub(super) fn drop_rows(&mut self, drag: &DragRows, target: usize, cx: &mut Context<Self>) {
        let CollectionId::Playlist(playlist) = &drag.list else {
            return;
        };
        let Some(tracks) = self.library.tracks(&drag.list).cloned() else {
            return;
        };
        let (from, len) = (drag.from, drag.len);
        if (target >= from && target < from + len)
            || from + len > tracks.len()
            || target >= tracks.len()
        {
            return;
        }
        let to = if target > from {
            target + 1 - len
        } else {
            target
        };
        let uris: Vec<String> = tracks[from..from + len]
            .iter()
            .map(|t| t.uri.clone())
            .collect();
        let op = EditOp::MovePlaylistItems {
            playlist: playlist.clone(),
            from_index: from as u32,
            length: len as u32,
            to_index: to as u32,
            uris,
        };
        let label = if len == 1 {
            t!(ToastMoved1).to_string()
        } else {
            tf!(ToastMovedN, len)
        };
        let ops = vec![(op, Vec::new(), None)];
        let undo = inverse_all(&ops);
        self.commit(ops, Some((label, undo)), cx);
        // Taşınan blok seçili kalsın.
        self.manage.sel = Selection {
            list: Some(ListKey::Collection(drag.list.clone())),
            rows: (to..to + len).collect(),
            anchor: Some(to),
        };
    }

    // ── Listeler: oluştur, adlandır, sil, ekle, çıkar ───────────────────

    /// Kendi listelerin (listeye ekle menüsü), kütüphane sırasıyla.
    pub(super) fn own_playlists(&self) -> Vec<LibraryItem> {
        match &self.library.library {
            Loadable::Loaded(lib) => lib
                .playlists
                .iter()
                .filter(|p| is_own(p))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn add_to_playlist(
        &mut self,
        playlist: String,
        title: String,
        rows: Vec<TrackRow>,
        cx: &mut Context<Self>,
    ) {
        if rows.is_empty() {
            return;
        }
        let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
        let id = CollectionId::Playlist(playlist.clone());
        let len = self.library.tracks(&id).map(|t| t.len());
        let op = EditOp::AddToPlaylist {
            playlist: playlist.clone(),
            uris: uris.clone(),
            position: AddPosition::Last,
        };
        // Geri alma: tam eklenen satırlar (aynı şarkının önceki kopyaları kalsın).
        let undo_op = EditOp::RemoveFromPlaylist {
            playlist,
            uris,
            indices: len
                .map(|l| (l..l + rows.len()).map(|i| i as u32).collect())
                .unwrap_or_default(),
        };
        let label = if rows.len() == 1 {
            tf!(ToastAddedToList, title)
        } else {
            tf!(ToastAddedToListN, rows.len(), title)
        };
        self.commit(
            vec![(op, rows, None)],
            Some((label, vec![(undo_op, Vec::new(), None)])),
            cx,
        );
    }

    /// Kendi listenden satırları çıkar (tek satır: aynı şarkının öbür
    /// kopyaları kalır). Büyük sıradan küçüğe: her işlemin sırası geçerli.
    pub(super) fn remove_rows(
        &mut self,
        playlist: String,
        mut rows: Vec<(usize, TrackRow)>,
        cx: &mut Context<Self>,
    ) {
        if rows.is_empty() {
            return;
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        let ops: Vec<Edit> = rows
            .iter()
            .map(|(ix, r)| {
                (
                    EditOp::RemoveFromPlaylist {
                        playlist: playlist.clone(),
                        uris: vec![r.uri.clone()],
                        indices: vec![*ix as u32],
                    },
                    Vec::new(),
                    None,
                )
            })
            .collect();
        // Tersi küçükten büyüğe yerlerine eklenir.
        let undo: Vec<Edit> = rows
            .iter()
            .rev()
            .map(|(ix, r)| {
                (
                    EditOp::AddToPlaylist {
                        playlist: playlist.clone(),
                        uris: vec![r.uri.clone()],
                        position: AddPosition::At(*ix as u32),
                    },
                    vec![r.clone()],
                    None,
                )
            })
            .collect();
        let label = if rows.len() == 1 {
            t!(ToastRemovedFromList).to_string()
        } else {
            tf!(ToastRemovedFromListN, rows.len())
        };
        self.commit(ops, Some((label, undo)), cx);
    }

    /// Delete: açık kendi listende seçili satırları çıkar.
    pub(super) fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected.clone().filter(|_| !self.page_open()) else {
            return;
        };
        let CollectionId::Playlist(playlist) = &id else {
            return;
        };
        if !self.library.item(&id).is_some_and(is_own) {
            return;
        }
        let list = ListKey::Collection(id.clone());
        if self.manage.sel.list.as_ref() != Some(&list) {
            return;
        }
        let Some(tracks) = self.list_tracks(&list) else {
            return;
        };
        let rows: Vec<(usize, TrackRow)> = self
            .manage
            .sel
            .rows
            .iter()
            .filter_map(|&i| Some((i, tracks.get(i)?.clone())))
            .collect();
        self.remove_rows(playlist.clone(), rows, cx);
    }

    pub(super) fn ask_new_playlist(
        &mut self,
        rows: Vec<TrackRow>,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        self.manage.menu = None;
        self.manage.prompt = Some(Prompt {
            kind: PromptKind::Create { rows, open },
            text: String::new(),
        });
        self.refocus_anchor = true;
        cx.notify();
    }

    pub(super) fn ask_rename(
        &mut self,
        playlist: String,
        previous: String,
        cx: &mut Context<Self>,
    ) {
        self.manage.menu = None;
        self.manage.prompt = Some(Prompt {
            kind: PromptKind::Rename {
                playlist,
                previous: previous.clone(),
            },
            text: previous,
        });
        self.refocus_anchor = true;
        cx.notify();
    }

    /// Liste açıklamasını düzenle (ad kutusu gibi; metin çok satırlı olabilir).
    pub(super) fn ask_description(&mut self, playlist: String, cx: &mut Context<Self>) {
        let id = CollectionId::Playlist(playlist.clone());
        let previous = self.list_description(&id).unwrap_or_default().to_string();
        self.manage.menu = None;
        self.manage.prompt = Some(Prompt {
            kind: PromptKind::Description {
                playlist,
                previous: previous.clone(),
            },
            text: previous,
        });
        self.refocus_anchor = true;
        cx.notify();
    }

    /// Ortak listeyi aç / kapa (geri alınabilir).
    pub(super) fn set_collaborative(&mut self, playlist: String, on: bool, cx: &mut Context<Self>) {
        let id = CollectionId::Playlist(playlist.clone());
        if self.list_is_collaborative(&id) == on {
            return;
        }
        let op = EditOp::SetPlaylistCollaborative {
            playlist,
            collaborative: on,
            previous: Some(!on),
        };
        let ops = vec![(op, Vec::new(), None)];
        let undo = inverse_all(&ops);
        let label = if on {
            t!(ToastCollaborativeOn)
        } else {
            t!(ToastCollaborativeOff)
        };
        self.commit(ops, Some((label.into(), undo)), cx);
    }

    // ── Klasörler ───────────────────────────────────────────────────────

    /// Yeni klasör adı sor (`move_playlist`: oluşunca bu liste içine taşınır).
    pub(super) fn ask_new_folder(
        &mut self,
        move_playlist: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        self.manage.menu = None;
        self.manage.prompt = Some(Prompt {
            kind: PromptKind::FolderCreate { move_playlist },
            text: String::new(),
        });
        self.refocus_anchor = true;
        cx.notify();
    }

    pub(super) fn ask_folder_rename(
        &mut self,
        folder: String,
        previous: String,
        cx: &mut Context<Self>,
    ) {
        self.manage.menu = None;
        self.manage.prompt = Some(Prompt {
            kind: PromptKind::FolderRename {
                folder,
                previous: previous.clone(),
            },
            text: previous,
        });
        self.refocus_anchor = true;
        cx.notify();
    }

    /// Klasörü sil: iki işaret kalkar, içindekiler kök seviyeye çıkar. Geri
    /// alma klasörü (listeleriyle) eski yerine koyar.
    pub(super) fn delete_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        let (name, before, contents, parent) = match &self.library.library {
            Loadable::Loaded(lib) => {
                let Some(f) = lib.meta.folders.iter().find(|f| f.key == folder) else {
                    return;
                };
                // İçindeki listeler (doğrudan çocuklar), kütüphane sırasıyla.
                let contents: Vec<String> = lib
                    .playlists
                    .iter()
                    .filter(|p| {
                        lib.meta.parents.get(p.id.key()).map(String::as_str)
                            == Some(folder.as_str())
                    })
                    .map(|p| p.id.key().to_string())
                    .collect();
                // Klasörden sonra gelen liste: geri alırken yerini bulmak için.
                let inside = lib
                    .playlists
                    .iter()
                    .filter(|p| folder_contains(&lib.meta, p.id.key(), &folder))
                    .count();
                let before = lib
                    .playlists
                    .get(f.at + inside)
                    .map(|p| p.id.key().to_string());
                (f.name.clone(), before, contents, f.parent.clone())
            }
            _ => return,
        };
        let op = EditOp::DeleteFolder {
            folder: folder.clone(),
            name: name.clone(),
            before: before.clone(),
            contents: contents.clone(),
        };
        let undo = vec![(
            EditOp::RestoreFolder {
                folder: folder.clone(),
                name: name.clone(),
                before,
                contents,
            },
            Vec::new(),
            None,
        )];
        // Açık klasör kapansın; ızgarada içindeysek bir üst klasöre çıkılır.
        self.set_folder_open(&folder, false, cx);
        if self.all.folder.as_deref() == Some(folder.as_str()) {
            self.go_to_folder(parent, cx);
        }
        self.commit(
            vec![(op, Vec::new(), None)],
            Some((t!(ToastFolderDeleted).into(), undo)),
            cx,
        );
    }

    /// Listeyi bir klasöre taşı (`folder` boşsa köke çıkar).
    pub(super) fn move_to_folder(
        &mut self,
        playlist: String,
        folder: Option<String>,
        previous: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if previous == folder {
            return;
        }
        let op = EditOp::MovePlaylistToFolder {
            playlist,
            folder: folder.clone(),
            previous,
        };
        let ops = vec![(op, Vec::new(), None)];
        let undo = inverse_all(&ops);
        let label = match folder.and_then(|key| self.folder_name(&key).map(str::to_string)) {
            Some(name) => tf!(ToastMovedToFolder, name),
            None => t!(ToastMovedToRoot).to_string(),
        };
        self.commit(ops, Some((label, undo)), cx);
    }

    /// F2: açık kendi listeni yeniden adlandır.
    pub(super) fn rename_current(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected.clone().filter(|_| !self.page_open()) else {
            return;
        };
        if let (CollectionId::Playlist(uri), Some(item)) =
            (&id, self.library.item(&id).filter(|i| is_own(i)))
        {
            let (uri, name) = (uri.clone(), item.name.clone());
            self.ask_rename(uri, name, cx);
        }
    }

    fn confirm_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.manage.prompt.take() else {
            return;
        };
        let description = matches!(&prompt.kind, PromptKind::Description { .. });
        let text = prompt.text.trim().to_string();
        // Açıklama boşaltılabilir; ötekilerde boş ad kabul edilmez.
        if text.is_empty() && !description {
            self.manage.prompt = Some(prompt);
            return;
        }
        match prompt.kind {
            PromptKind::Description { playlist, previous } => {
                if text == previous {
                    cx.notify();
                    return;
                }
                let op = EditOp::SetPlaylistDescription {
                    playlist,
                    description: text,
                    previous: Some(previous),
                };
                let ops = vec![(op, Vec::new(), None)];
                let undo = inverse_all(&ops);
                self.commit(ops, Some((t!(ToastDescriptionSaved).into(), undo)), cx);
            }
            PromptKind::FolderCreate { move_playlist } => {
                let key = folder_start_marker(&new_folder_id(), &text);
                let create = EditOp::CreateFolder {
                    key: key.clone(),
                    name: text.clone(),
                };
                let mut ops = vec![(create, Vec::new(), None)];
                if let Some((playlist, _)) = move_playlist {
                    let previous = self
                        .folder_of(&CollectionId::Playlist(playlist.clone()))
                        .map(str::to_string);
                    ops.push((
                        EditOp::MovePlaylistToFolder {
                            playlist,
                            folder: Some(key),
                            previous,
                        },
                        Vec::new(),
                        None,
                    ));
                }
                let undo = inverse_all(&ops);
                self.commit(ops, Some((tf!(ToastFolderCreated, text), undo)), cx);
            }
            PromptKind::FolderRename { folder, previous } => {
                if text == previous {
                    cx.notify();
                    return;
                }
                let op = EditOp::RenameFolder {
                    folder,
                    name: text,
                    previous_name: Some(previous),
                };
                let ops = vec![(op, Vec::new(), None)];
                let undo = inverse_all(&ops);
                self.commit(ops, Some((t!(ToastFolderRenamed).into(), undo)), cx);
            }
            PromptKind::Create { rows, open } => {
                let temp_id = new_temp_id();
                let uris: Vec<String> = rows.iter().map(|r| r.uri.clone()).collect();
                let n = rows.len();
                let op = EditOp::CreatePlaylist {
                    temp_id: temp_id.clone(),
                    name: text.clone(),
                    uris,
                    created: None,
                };
                let undo = vec![(
                    EditOp::DeletePlaylist {
                        playlist: temp_id.clone(),
                    },
                    Vec::new(),
                    None,
                )];
                let label = match n {
                    0 => tf!(ToastCreated, text),
                    1 => tf!(ToastCreatedWith1, text),
                    n => tf!(ToastCreatedWithN, text, n),
                };
                self.commit(vec![(op, rows, None)], Some((label, undo)), cx);
                if open {
                    self.library
                        .titles
                        .insert(CollectionId::Playlist(temp_id.clone()), text);
                    self.select_collection(CollectionId::Playlist(temp_id), cx);
                }
            }
            PromptKind::Rename { playlist, previous } => {
                if text == previous {
                    cx.notify();
                    return;
                }
                let op = EditOp::RenamePlaylist {
                    playlist,
                    name: text,
                    previous_name: Some(previous),
                };
                let ops = vec![(op, Vec::new(), None)];
                let undo = inverse_all(&ops);
                self.commit(ops, Some((t!(ToastRenamed).into(), undo)), cx);
            }
            PromptKind::DeviceName => self.set_device_name(text, cx),
        }
    }

    /// Listeyi sil (kendi listen) ya da kütüphaneden kaldır (takip edilen):
    /// ikisi de geri alınabilir (Spotify bir süre geri getirmeye izin verir).
    pub(super) fn delete_playlist(&mut self, playlist: String, own: bool, cx: &mut Context<Self>) {
        let id = CollectionId::Playlist(playlist.clone());
        let ops = vec![(EditOp::DeletePlaylist { playlist }, Vec::new(), None)];
        let undo = inverse_all(&ops);
        let label = if own {
            t!(ToastListDeleted)
        } else {
            t!(ToastListRemoved)
        };
        let open_here = self.library.selected.as_ref() == Some(&id) && !self.page_open();
        self.commit(ops, Some((label.into(), undo)), cx);
        if open_here {
            self.select_collection(CollectionId::Liked, cx);
        }
    }

    pub(super) fn prompt_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(prompt) = self.manage.prompt.as_mut() else {
            return;
        };
        let description = matches!(&prompt.kind, PromptKind::Description { .. });
        let limit = if description {
            DESCRIPTION_MAX
        } else {
            NAME_MAX
        };
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control;
        match ks.key.as_str() {
            "escape" => self.manage.prompt = None,
            // Açıklamada Enter yeni satır açar; kaydetmek için Ctrl+Enter
            // (öteki kutularda Enter kaydeder).
            "enter" if description && !ctrl => prompt.text.push('\n'),
            "enter" => self.confirm_prompt(cx),
            "backspace" if ctrl => {
                let trimmed = prompt.text.trim_end().len();
                let cut = prompt.text[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                prompt.text.truncate(cut);
            }
            "backspace" => {
                prompt.text.pop();
            }
            "v" if ctrl => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    if description {
                        prompt
                            .text
                            .push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
                    } else {
                        prompt
                            .text
                            .push_str(text.lines().next().unwrap_or_default());
                    }
                }
            }
            "space" => prompt.text.push(' '),
            _ if !ctrl && !ks.modifiers.alt => {
                if let Some(ch) = ks
                    .key_char
                    .as_deref()
                    .filter(|c| !c.chars().any(char::is_control))
                {
                    prompt.text.push_str(ch);
                }
            }
            _ => {}
        }
        // Yeni satır düz Enter ile gelir; kutu 300 karakterde kesilir.
        if let Some(p) = self.manage.prompt.as_mut()
            && p.text.chars().count() > limit
        {
            p.text = p.text.chars().take(limit).collect();
        }
        cx.notify();
    }

    // ── Sağ tık menüsü ──────────────────────────────────────────────────

    /// Şarkı satırının (ya da seçimin) menüsü.
    pub(super) fn open_row_menu(
        &mut self,
        list: ListKey,
        ix: usize,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(tracks) = self.list_tracks(&list) else {
            return;
        };
        let picked = self.target_rows(&list, ix);
        let rows: Vec<TrackRow> = picked
            .iter()
            .filter_map(|&i| tracks.get(i).cloned())
            .collect();
        if rows.is_empty() {
            return;
        }
        let single = rows.len() == 1;
        let count = rows.len();
        // Gruplar (aralarında ayırıcı, boş grup ayırıcı bırakmaz, bkz. `join_groups`):
        // 1 sıra · 2 beğen, radyo, albüm, sanatçı, katkılar · 3 hiç çalma, gizle ·
        // 4 bağlantı, Spotify'da aç · 5 listeden çıkar.
        let mut queue = vec![
            item(
                "list-end",
                if single {
                    t!(MenuAddToQueue).into()
                } else {
                    tf!(MenuAddToQueueN, count)
                },
                Act::Enqueue(rows.clone()),
            ),
            item(
                "list-start",
                if single {
                    t!(MenuPlayNext).into()
                } else {
                    tf!(MenuPlayNextN, count)
                },
                Act::PlayNext(rows.clone()),
            ),
        ];
        let mut add = vec![(
            t!(MenuNewPlaylistDots).to_string(),
            Act::NewPlaylist(rows.clone()),
        )];
        for p in self.own_playlists() {
            if let CollectionId::Playlist(uri) = &p.id
                && ListKey::Collection(p.id.clone()) != list
            {
                add.push((
                    p.name.clone(),
                    Act::AddTo {
                        playlist: uri.clone(),
                        title: p.name.clone(),
                        rows: rows.clone(),
                    },
                ));
            }
        }
        queue.push(Entry::Item {
            icon: "list-plus",
            label: t!(MenuAddToPlaylist).into(),
            shortcut: None,
            danger: false,
            act: None,
            sub: add,
        });
        let mut entries = Vec::new();
        let mut never = Vec::new();
        let liked = self.liked_set();
        let all_liked = rows.iter().all(|r| liked.contains(&r.uri));
        entries.push(item(
            if all_liked { "heart-fill" } else { "heart" },
            if all_liked {
                t!(UnlikeTip).into()
            } else {
                t!(LikeTip).into()
            },
            Act::Like {
                rows: rows.clone(),
                on: !all_liked,
            },
        ));
        if single {
            let row = rows[0].clone();
            // Radyo: bu şarkıdan başlayan sonsuz benzer şarkı akışı (bkz. `listening`).
            entries.extend(Self::radio_entry(&row.uri, &row.name));
            // Albüm sayfasında "Albüme git" zaten burası.
            let on_album_page = matches!(&list, ListKey::Page(k) if k.starts_with("album:"));
            if !row.album.is_empty() && !on_album_page {
                entries.push(item(
                    "disc-3",
                    t!(MenuGoAlbum).into(),
                    Act::GoAlbum(row.clone()),
                ));
            }
            let names = super::link_text::split_names(&row.artists);
            match names.len() {
                0 => {}
                1 => entries.push(item(
                    "user",
                    t!(MenuGoArtist).into(),
                    Act::GoArtist(row.clone(), 0),
                )),
                _ => entries.push(Entry::Item {
                    icon: "user",
                    label: t!(MenuGoArtist).into(),
                    shortcut: None,
                    danger: false,
                    act: None,
                    sub: names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (n.to_string(), Act::GoArtist(row.clone(), i)))
                        .collect(),
                }),
            }
            if row.uri.starts_with("spotify:track:") {
                entries.push(item(
                    "info",
                    t!(MenuShowCredits).into(),
                    Act::ShowCredits(row.clone()),
                ));
            }
            // Karıştırma kuralları: "Bu sanatçıyı hiç çalma" (bkz. `settings::never_play_entries`).
            never = self.never_play_entries(&names, "", false);
        }
        // Yerel gizleme: "Bu şarkıyı gizle" / "Hiçbir yerde çalma" / "Göster" (bkz. `listening`).
        never.extend(self.hide_entries(&list, &rows));
        let mut link = vec![item(
            "link",
            if single {
                t!(MenuCopyLink).into()
            } else {
                tf!(MenuCopyLinkN, count)
            },
            Act::CopyLinks(rows.iter().map(|r| r.uri.clone()).collect()),
        )];
        if single && crate::link::app_target(&rows[0].uri).is_some() {
            link.push(item(
                "external-link",
                t!(MenuOpenInSpotify).into(),
                Act::OpenInSpotify(rows[0].uri.clone()),
            ));
        }
        // Kendi listende: satırları çıkar.
        let mut remove = Vec::new();
        if let ListKey::Collection(id @ CollectionId::Playlist(uri)) = &list
            && self.library.item(id).is_some_and(is_own)
        {
            let pairs: Vec<(usize, TrackRow)> = picked
                .iter()
                .filter_map(|&i| Some((i, tracks.get(i)?.clone())))
                .collect();
            remove.push(Entry::Item {
                icon: "trash-2",
                label: if single {
                    t!(MenuRemoveFromList).into()
                } else {
                    tf!(MenuRemoveFromListN, count)
                },
                shortcut: Some("Del"),
                danger: true,
                act: Some(Act::RemoveRows {
                    playlist: uri.clone(),
                    rows: pairs,
                }),
                sub: Vec::new(),
            });
        }
        self.show_menu(
            at,
            join_groups(vec![queue, entries, never, link, remove]),
            cx,
        );
    }

    /// Kart (liste, albüm, sanatçı) menüsü.
    pub(super) fn open_card_menu(
        &mut self,
        target: CardTarget,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        // Klasörde "Aç" liste görünümünde "Genişlet" / "Daralt" olur (klasör yalnız okunur).
        let open_label = match &target {
            CardTarget::Folder { key, .. } if self.all.list => {
                if self.is_folder_open(key) {
                    t!(MenuCollapseFolder)
                } else {
                    t!(MenuExpandFolder)
                }
            }
            _ => t!(MenuOpen),
        };
        // Hedef zaten ekrandaysa "Aç" yok (açmak hiçbir şey yapmazdı).
        let on_screen = target_on_screen(
            self.pages.history.current(),
            self.all.open,
            self.library.selected.as_ref(),
            &target,
        );
        let mut entries = Vec::new();
        if !on_screen {
            entries.push(item(
                "chevron-right",
                open_label.into(),
                Act::Open(target.clone()),
            ));
        }
        if !matches!(target, CardTarget::Artist(_) | CardTarget::Folder { .. }) {
            entries.push(item("play", t!(Play).into(), Act::Play(target.clone())));
        }
        // Radyo: sanatçı, albüm ve listeden (Beğenilenler'in tohumu yok).
        if let Some((seed, title)) = self.card_radio_seed(&target) {
            entries.extend(Self::radio_entry(&seed, &title));
        }
        // Sabitleme (yerel): kütüphanedeki liste, albüm, takip edilen sanatçı, Beğenilenler.
        let pin_id = match &target {
            CardTarget::Collection { id, .. } => Some(id.clone()),
            CardTarget::Artist(uri) => Some(CollectionId::Artist(uri.clone())),
            CardTarget::Album { uri, .. } => Some(CollectionId::Album(uri.clone())),
            CardTarget::Folder { .. } => None,
        };
        if let Some(entry) = pin_id.and_then(|id| self.pin_entry(&id)) {
            entries.push(entry);
        }
        let uri = match &target {
            CardTarget::Collection { id, .. } => {
                (id != &CollectionId::Liked).then(|| id.key().to_string())
            }
            CardTarget::Artist(uri) | CardTarget::Album { uri, .. } => Some(uri.clone()),
            CardTarget::Folder { .. } => None,
        };
        if let Some(uri) = uri.filter(|u| u.starts_with("spotify:")) {
            entries.push(item(
                "link",
                t!(MenuCopyLink).into(),
                Act::CopyLinks(vec![uri.clone()]),
            ));
            if crate::link::app_target(&uri).is_some() {
                entries.push(item(
                    "external-link",
                    t!(MenuOpenInSpotify).into(),
                    Act::OpenInSpotify(uri),
                ));
            }
        }
        match &target {
            CardTarget::Collection {
                id: id @ CollectionId::Playlist(uri),
                title,
            } => {
                // Henüz oluşmakta olan (geçici adresli) liste de senin.
                let own = self.library.item(id).is_some_and(is_own) || uri.starts_with("local:");
                let in_library = self.library.item(id).is_some();
                if own {
                    entries.push(Entry::Separator);
                    entries.push(Entry::Item {
                        icon: "pencil",
                        label: t!(MenuRename).into(),
                        shortcut: Some("F2"),
                        danger: false,
                        act: Some(Act::Rename {
                            playlist: uri.clone(),
                            name: title.clone(),
                        }),
                        sub: Vec::new(),
                    });
                    // Açıklama ve ortak liste: listenin kendi ayarları.
                    let description = self.list_description(id).unwrap_or_default().to_string();
                    entries.push(Entry::Item {
                        icon: "pencil",
                        label: if description.is_empty() {
                            t!(MenuAddDescription).into()
                        } else {
                            t!(MenuEditDescription).into()
                        },
                        shortcut: None,
                        danger: false,
                        act: Some(Act::EditDescription {
                            playlist: uri.clone(),
                        }),
                        sub: Vec::new(),
                    });
                    let shared = self.list_is_collaborative(id);
                    entries.push(item(
                        "users",
                        if shared {
                            t!(MenuMakePrivate).into()
                        } else {
                            t!(MenuMakeCollaborative).into()
                        },
                        Act::SetCollaborative {
                            playlist: uri.clone(),
                            on: !shared,
                        },
                    ));
                }
                // Klasör: yalnız kök listede gerçekten duran listeler (henüz
                // oluşmakta olan geçici adresli liste kök listede yok).
                if in_library && uri.starts_with("spotify:playlist:") {
                    let current = self.folder_of(id).map(str::to_string);
                    let mut folders: Vec<(String, Act)> = vec![(
                        t!(MenuNewFolderDots).to_string(),
                        Act::NewFolder {
                            move_playlist: Some((uri.clone(), title.clone())),
                        },
                    )];
                    if current.is_some() {
                        folders.push((
                            t!(MenuMoveToRoot).to_string(),
                            Act::MoveToFolder {
                                playlist: uri.clone(),
                                folder: None,
                                previous: current.clone(),
                            },
                        ));
                    }
                    for (key, name) in self.folder_choices() {
                        if Some(&key) == current.as_ref() {
                            continue;
                        }
                        folders.push((
                            name,
                            Act::MoveToFolder {
                                playlist: uri.clone(),
                                folder: Some(key),
                                previous: current.clone(),
                            },
                        ));
                    }
                    entries.push(Entry::Item {
                        icon: "folder",
                        label: t!(MenuMoveToFolder).into(),
                        shortcut: None,
                        danger: false,
                        act: None,
                        sub: folders,
                    });
                }
                if in_library {
                    entries.push(Entry::Separator);
                    entries.push(Entry::Item {
                        icon: "trash-2",
                        label: if own {
                            t!(MenuDeleteList).into()
                        } else {
                            t!(MenuRemoveFromLibrary).into()
                        },
                        shortcut: None,
                        danger: true,
                        act: Some(Act::Delete {
                            playlist: uri.clone(),
                            own,
                        }),
                        sub: Vec::new(),
                    });
                }
            }
            CardTarget::Collection {
                id: CollectionId::Album(uri),
                ..
            } => {
                entries.push(Entry::Separator);
                entries.push(item(
                    "heart-fill",
                    t!(RemoveFromLibrary).into(),
                    Act::SaveAlbum {
                        uri: uri.clone(),
                        on: false,
                        item: None,
                    },
                ));
            }
            CardTarget::Album { uri, item: card } => {
                entries.push(Entry::Separator);
                let saved = self.album_saved(uri);
                entries.push(item(
                    if saved { "heart-fill" } else { "heart" },
                    if saved {
                        t!(RemoveFromLibrary).into()
                    } else {
                        t!(AddToLibrary).into()
                    },
                    Act::SaveAlbum {
                        uri: uri.clone(),
                        on: !saved,
                        item: card.clone(),
                    },
                ));
            }
            CardTarget::Artist(uri) => {
                // Takip: kartı (ad, portre) bilinen yerlerden; bilinmiyorsa
                // adsız kart eklememek için önerilmez (takipteyse bırakmak her zaman var).
                let followed = self.artist_followed(uri);
                let card = self.known_artist(uri);
                if followed || card.is_some() {
                    entries.push(Entry::Separator);
                    entries.push(item(
                        if followed { "user-check" } else { "user-plus" },
                        if followed {
                            t!(MenuUnfollowArtist).into()
                        } else {
                            t!(MenuFollowArtist).into()
                        },
                        Act::FollowArtist {
                            uri: uri.clone(),
                            on: !followed,
                            item: card,
                        },
                    ));
                }
                // Karıştırma kuralları: "Bu sanatçıyı hiç çalma" (ad, kartın bilindiği yerlerden).
                if let Some(artist) = self.known_artist(uri) {
                    if !entries.iter().any(|e| matches!(e, Entry::Separator)) {
                        entries.push(Entry::Separator);
                    }
                    entries.extend(self.never_play_entries(&[artist.name.as_str()], uri, false));
                }
            }
            CardTarget::Folder { key } => {
                if let Some(name) = self.folder_name(key).map(str::to_string) {
                    entries.push(Entry::Separator);
                    entries.push(Entry::Item {
                        icon: "pencil",
                        label: t!(MenuRenameFolder).into(),
                        shortcut: None,
                        danger: false,
                        act: Some(Act::RenameFolder {
                            folder: key.clone(),
                            name,
                        }),
                        sub: Vec::new(),
                    });
                    entries.push(Entry::Item {
                        icon: "trash-2",
                        label: t!(MenuDeleteFolder).into(),
                        shortcut: None,
                        danger: true,
                        act: Some(Act::DeleteFolder {
                            folder: key.clone(),
                        }),
                        sub: Vec::new(),
                    });
                }
            }
            _ => {}
        }
        self.show_menu(at, entries, cx);
    }

    pub(super) fn show_menu(
        &mut self,
        at: Point<Pixels>,
        entries: Vec<Entry>,
        cx: &mut Context<Self>,
    ) {
        self.manage.menu = Some(Menu {
            at,
            entries,
            cursor: None,
            sub: None,
            sub_cursor: None,
        });
        self.manage.sync_open = false;
        self.refocus_anchor = true;
        cx.notify();
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.manage.menu.take().is_some() {
            cx.notify();
        }
    }

    /// Shift+F10: seçili satırın menüsü, satırın yanında.
    pub(super) fn menu_for_selection(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(id) = self.library.selected.clone().filter(|_| !self.page_open()) else {
            return;
        };
        let Some(ix) = self.library.selected_row else {
            return;
        };
        let list = ListKey::Collection(id);
        let y = self
            .list_viewport
            .get()
            .map(|b| {
                f32::from(b.origin.y)
                    + (ix as f32 * super::library::ROW_HEIGHT - self.list_offset()).max(0.)
                    + S6
            })
            .unwrap_or(f32::from(window.viewport_size().height) / 2.);
        self.open_row_menu(
            list,
            ix,
            gpui::point(px(super::library::PAGE_PAD * 4.), px(y)),
            cx,
        );
    }

    fn run_act(&mut self, act: Act, cx: &mut Context<Self>) {
        self.manage.menu = None;
        match act {
            Act::Enqueue(rows) => {
                let n = rows.len();
                self.engine.send(Command::Enqueue(rows));
                self.show_toast(
                    if n == 1 {
                        t!(ToastQueued).into()
                    } else {
                        tf!(ToastQueuedN, n)
                    },
                    false,
                    false,
                    cx,
                );
            }
            Act::PlayNext(rows) => {
                let n = rows.len();
                self.engine.send(Command::PlayNext(rows));
                self.show_toast(
                    if n == 1 {
                        t!(ToastPlayNext).into()
                    } else {
                        tf!(ToastPlayNextN, n)
                    },
                    false,
                    false,
                    cx,
                );
            }
            Act::AddTo {
                playlist,
                title,
                rows,
            } => self.add_to_playlist(playlist, title, rows, cx),
            Act::NewPlaylist(rows) => self.ask_new_playlist(rows, false, cx),
            Act::Like { rows, on } => self.set_liked(rows, on, cx),
            Act::GoAlbum(row) => self.open_album_of(&row, cx),
            Act::GoArtist(row, i) => self.open_artist_of(&row, i, cx),
            Act::CopyLinks(uris) => {
                let links: Vec<String> = uris.iter().filter_map(|u| web_link(u)).collect();
                if !links.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(links.join("\n")));
                    self.show_toast(
                        if links.len() == 1 {
                            t!(ToastLinkCopied).into()
                        } else {
                            tf!(ToastLinksCopied, links.len())
                        },
                        false,
                        false,
                        cx,
                    );
                }
            }
            Act::RemoveRows { playlist, rows } => self.remove_rows(playlist, rows, cx),
            Act::Open(target) => self.open_card(target, cx),
            Act::Play(target) => self.play_card(target, cx),
            Act::Rename { playlist, name } => self.ask_rename(playlist, name, cx),
            Act::EditDescription { playlist } => self.ask_description(playlist, cx),
            Act::SetCollaborative { playlist, on } => self.set_collaborative(playlist, on, cx),
            Act::NewFolder { move_playlist } => self.ask_new_folder(move_playlist, cx),
            Act::RenameFolder { folder, name } => self.ask_folder_rename(folder, name, cx),
            Act::DeleteFolder { folder } => self.delete_folder(folder, cx),
            Act::MoveToFolder {
                playlist,
                folder,
                previous,
            } => self.move_to_folder(playlist, folder, previous, cx),
            Act::Delete { playlist, own } => self.delete_playlist(playlist, own, cx),
            Act::SaveAlbum { uri, on, item } => self.set_album_saved(uri, on, item, cx),
            Act::FollowArtist { uri, on, item } => self.set_artist_followed(uri, on, item, cx),
            Act::OpenInSpotify(uri) => self.open_in_spotify(&uri, cx),
            Act::BlockArtist { name, uri, on } => self.set_artist_blocked(name, uri, on, cx),
            Act::PlayFriend(friend) => self.play_friend(&friend),
            Act::GoAlbumUri(uri) => self.go_friend_album(uri, cx),
            Act::GoArtistUri(uri) => self.go_friend_artist(uri, cx),
            Act::Pin { key, on } => self.set_pinned(key, on, cx),
            Act::SetSort(sort) => self.set_all_sort(sort, cx),
            Act::ShowCredits(row) => self.open_credits(row.uri, row.name, row.artists, None, cx),
            Act::StartRadio { seed, title } => self.start_radio(seed, title, cx),
            Act::HideSongs { rows, context } => self.hide_songs(rows, context, cx),
            Act::ShowSongs { rows, context } => self.show_songs(rows, context, cx),
        }
        cx.notify();
    }

    /// "Spotify'da aç": uygulama kuruluysa `spotify:` adresiyle (Windows kabuğu
    /// uygulamayı ilgili sayfada açar), değilse aynı yerin web bağlantısıyla
    /// tarayıcıda. Sınama kopyası dış uygulama açmaz.
    pub(super) fn open_in_spotify(&mut self, uri: &str, cx: &mut Context<Self>) {
        let Some(target) = crate::link::app_target(uri) else {
            return;
        };
        let test = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some();
        let (url, toast) = if test || crate::link::spotify_app_registered() {
            (target, t!(ToastOpeningInSpotify))
        } else if let Some(web) = web_link(&target) {
            (web, t!(ToastOpeningInBrowser))
        } else {
            return;
        };
        if !test {
            cx.open_url(&url);
        }
        self.show_toast(toast.into(), false, false, cx);
    }

    /// Kartı aç.
    pub(super) fn open_card(&mut self, target: CardTarget, cx: &mut Context<Self>) {
        match target {
            CardTarget::Collection { id, title } => {
                self.library.titles.entry(id.clone()).or_insert(title);
                self.select_collection(id, cx);
            }
            CardTarget::Artist(uri) => self.navigate(Route::Artist(uri), cx),
            CardTarget::Album { uri, .. } => self.open_album(uri, cx),
            CardTarget::Folder { key, .. } => self.open_folder(key, cx),
        }
    }

    /// Kartı çal: liste / albüm açılır ve baştan çalar.
    pub(super) fn play_card(&mut self, target: CardTarget, cx: &mut Context<Self>) {
        match target {
            CardTarget::Collection { id, title } => {
                self.library.titles.entry(id.clone()).or_insert(title);
                self.select_collection(id.clone(), cx);
                if self.library.tracks(&id).is_some_and(|t| !t.is_empty()) {
                    self.play_from_start(cx);
                } else {
                    self.pages.play_when_loaded = Some(id);
                }
            }
            CardTarget::Album { uri, .. } => {
                self.open_album(uri.clone(), cx);
                self.play_in(&ListKey::Page(format!("album:{uri}")), 0, cx);
            }
            CardTarget::Artist(_) | CardTarget::Folder { .. } => {}
        }
    }

    /// Menü açıkken tuşlar.
    pub(super) fn menu_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(menu) = self.manage.menu.as_mut() else {
            return;
        };
        let items: Vec<usize> = menu
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Entry::Item { .. }))
            .map(|(i, _)| i)
            .collect();
        let step = |cur: Option<usize>, len: usize, d: isize| -> Option<usize> {
            if len == 0 {
                return None;
            }
            Some(match cur {
                None if d > 0 => 0,
                None => len - 1,
                Some(c) => (c as isize + d).rem_euclid(len as isize) as usize,
            })
        };
        if let Some(sub) = menu.sub {
            let Entry::Item { sub: list, .. } = &menu.entries[sub] else {
                return;
            };
            match key {
                "up" | "down" => {
                    menu.sub_cursor = step(
                        menu.sub_cursor,
                        list.len(),
                        if key == "up" { -1 } else { 1 },
                    )
                }
                "left" | "escape" => {
                    menu.sub = None;
                    menu.sub_cursor = None;
                }
                "enter" | "space" => {
                    if let Some(act) = menu
                        .sub_cursor
                        .and_then(|c| list.get(c))
                        .map(|(_, a)| a.clone())
                    {
                        self.run_act(act, cx);
                        return;
                    }
                }
                _ => {}
            }
            cx.notify();
            return;
        }
        match key {
            "up" | "down" => {
                let pos = menu.cursor.and_then(|c| items.iter().position(|&i| i == c));
                menu.cursor =
                    step(pos, items.len(), if key == "up" { -1 } else { 1 }).map(|p| items[p]);
            }
            "escape" => {
                self.manage.menu = None;
            }
            "right" | "enter" | "space" => {
                let Some(c) = menu.cursor else { return };
                if let Entry::Item { act, sub, .. } = &menu.entries[c] {
                    if !sub.is_empty() {
                        menu.sub = Some(c);
                        menu.sub_cursor = Some(0);
                    } else if key != "right"
                        && let Some(act) = act.clone()
                    {
                        self.run_act(act, cx);
                        return;
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Gezinme ve yönetim kısayolları. İşlendiyse `true`.
    ///
    /// Alt+← / → geri / ileri, Alt+Home ana sayfa, Ctrl+F bul, Ctrl+Z geri
    /// al, Ctrl+A hepsini seç, Delete seçili satırları listeden çıkar, F2
    /// listeyi yeniden adlandır, Shift+F10 seçili satırın menüsü.
    pub(super) fn manage_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.manage.prompt.is_some() {
            self.prompt_key(event, cx);
            return true;
        }
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        if self.manage.menu.is_some() {
            self.menu_key(key, cx);
            return true;
        }
        if self.manage.sync_open && key == "escape" {
            self.manage.sync_open = false;
            cx.notify();
            return true;
        }
        match Shortcut::from_keystroke(ks) {
            Some(Shortcut::Back) => self.go_back(cx),
            Some(Shortcut::Forward) => self.go_forward(cx),
            Some(Shortcut::GoHome) => self.go_home(cx),
            // Ctrl 1 / 2 / 3: Ana sayfa, Keşfet, Kütüphane sekmeleri.
            Some(Shortcut::TabHome) => self.go_tab(super::history::Tab::Home, cx),
            Some(Shortcut::TabExplore) => self.go_tab(super::history::Tab::Explore, cx),
            Some(Shortcut::TabLibrary) => self.go_tab(super::history::Tab::Library, cx),
            // Ctrl+F; F3: Windows'ta da "bul" (bul açıkken F3 kutuya gider).
            Some(Shortcut::Find) if key != "f3" || self.manage.find.is_none() => self.open_find(cx),
            Some(Shortcut::Undo) => self.undo(cx),
            Some(Shortcut::SelectAll) if !self.all.open => self.select_all(cx),
            Some(Shortcut::Delete) if !self.all.open => self.remove_selected(cx),
            Some(Shortcut::Rename) => self.rename_current(cx),
            Some(Shortcut::RowMenu) => self.menu_for_selection(window, cx),
            _ => return self.manage.find.is_some() && !self.all.open && self.find_key(event, cx),
        }
        true
    }

    // ── Görünüm: menü, ad kutusu, bildirim, eşitleme ────────────────────

    /// Pencerenin en üstündeki katmanlar (kökte, paletten önce çizilir).
    pub(super) fn render_manage_overlays(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(menu) = &self.manage.menu {
            out.push(self.render_menu(menu, window, cx));
        }
        if self.manage.sync_open {
            out.push(self.render_sync_panel(window, cx));
        }
        if let Some(toast) = &self.manage.toast {
            out.push(self.render_toast(toast, cx));
        }
        if let Some(prompt) = &self.manage.prompt {
            out.push(self.render_prompt(prompt, cx));
        }
        out
    }

    fn render_menu(&self, menu: &Menu, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let viewport = window.viewport_size();
        let (win_w, win_h) = (f32::from(viewport.width), f32::from(viewport.height));
        let heights: Vec<f32> = menu
            .entries
            .iter()
            .map(|e| {
                if matches!(e, Entry::Separator) {
                    MENU_SEPARATOR_HEIGHT
                } else {
                    MENU_ITEM_HEIGHT
                }
            })
            .collect();
        let menu_h: f32 = heights.iter().sum::<f32>() + 2. * S1 + 2.;
        // Pencereye sığsın: sağa / alta taşarsa içeri kayar.
        let x = f32::from(menu.at.x).min(win_w - MENU_WIDTH - S2).max(S2);
        let y = f32::from(menu.at.y).min(win_h - menu_h - S2).max(S2);

        let mut panel = menu_panel("row-menu").w(px(MENU_WIDTH)).occlude();
        for (i, entry) in menu.entries.iter().enumerate() {
            match entry {
                Entry::Separator => panel = panel.child(menu_separator()),
                Entry::Item {
                    icon: icon_name,
                    label: text_label,
                    shortcut,
                    danger,
                    act,
                    sub,
                } => {
                    let mut mi = MenuItem::new(text_label.clone());
                    // Boş simge adı: simge yeri boş kalır (sıralama menüsünde yalnız seçili olan işaretli).
                    if !icon_name.is_empty() {
                        mi = mi.icon(icon_name);
                    }
                    if let Some(s) = shortcut {
                        mi = mi.shortcut(s);
                    }
                    if *danger {
                        mi = mi.danger();
                    }
                    let highlighted = menu.cursor == Some(i) || menu.sub == Some(i);
                    mi = mi.look(if highlighted { Look::Hover } else { Look::Live });
                    let has_sub = !sub.is_empty();
                    let act = act.clone();
                    let el = menu_item(i, mi, accent)
                        .when(has_sub, |el| el.child(icon("chevron-right", 14., label())))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && let Some(m) = this.manage.menu.as_mut() {
                                let changed = m.cursor != Some(i) || m.sub != has_sub.then_some(i);
                                m.cursor = Some(i);
                                m.sub = has_sub.then_some(i);
                                m.sub_cursor = None;
                                if changed {
                                    cx.notify();
                                }
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(act) = act.clone() {
                                this.run_act(act, cx);
                            } else if let Some(m) = this.manage.menu.as_mut() {
                                m.sub = Some(i);
                                cx.notify();
                            }
                        }));
                    panel = panel.child(el);
                }
            }
        }

        // Dışarı tıklamak menüyü kapatır (arkadaki satır seçilmez); tekerlek de
        // kapatır. Dinleyiciler hem katmanda (başlık çubuğu) hem engelleyicide.
        let mut layer =
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_menu(cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_menu(cx);
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, _, _, cx| this.close_menu(cx)))
                .child(
                    layer_blocker()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_menu(cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_menu(cx);
                            }),
                        )
                        .on_scroll_wheel(cx.listener(|this, _, _, cx| this.close_menu(cx))),
                )
                .child(div().absolute().left(px(x)).top(px(y)).child(
                    panel.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                ));

        // Alt menü: öğenin sağında (sığmazsa solunda), en çok 360 px, kayar.
        if let Some(s) = menu.sub
            && let Some(Entry::Item { sub, .. }) = menu.entries.get(s)
        {
            let top = y + 1. + S1 + heights[..s].iter().sum::<f32>();
            let sub_h =
                (sub.len() as f32 * MENU_ITEM_HEIGHT + 2. * S1 + 2.).min(SUBMENU_MAX_HEIGHT);
            let sx = if x + 2. * MENU_WIDTH - S1 <= win_w - S2 {
                x + MENU_WIDTH - S1
            } else {
                x - MENU_WIDTH + S1
            };
            let sy = (top - S1 - 1.).min(win_h - sub_h - S2).max(S2);
            let mut list = div()
                .id("submenu-list")
                .flex()
                .flex_col()
                .max_h(px(SUBMENU_MAX_HEIGHT - 2. * S1 - 2.))
                .overflow_y_scroll();
            for (k, (label_text, act)) in sub.iter().enumerate() {
                let act = act.clone();
                let mi = MenuItem::new(label_text.clone()).look(if menu.sub_cursor == Some(k) {
                    Look::Hover
                } else {
                    Look::Live
                });
                let mi = if k == 0 && matches!(act, Act::NewPlaylist(_)) {
                    mi.icon("plus")
                } else {
                    mi
                };
                list = list.child(
                    // Kaymayan öğe: kayan sütunda öğeler küçülüp 32 yerine
                    // ~29 px'e sıkışıyordu (kaymak yerine).
                    menu_item(1000 + k, mi, accent)
                        .flex_shrink_0()
                        .on_click(cx.listener(move |this, _, _, cx| this.run_act(act.clone(), cx))),
                );
            }
            let subpanel = menu_panel("row-submenu")
                .w(px(MENU_WIDTH))
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(list);
            layer = layer.child(div().absolute().left(px(sx)).top(px(sy)).child(subpanel));
        }
        deferred(layer).with_priority(2).into_any_element()
    }

    fn render_toast(&self, toast: &Toast, cx: &mut Context<Self>) -> AnyElement {
        let body = self.toast_body(toast, cx);
        // Hapın üstünde bildirim şeridi varsa onun da üstünde (dar pencerede
        // ikisi yan yana sığmıyor).
        let lift = if self
            .state
            .visible_notice(std::time::Instant::now())
            .is_some()
        {
            S7 + S3
        } else {
            0.
        };
        // Sağ altta, hapın üstünde: işi bölmeden köşede.
        div()
            .absolute()
            // Yerleşik "Şimdi çalıyor" paneli varsa onun solunda.
            .right(px(self.margin() + self.dock_inset()))
            .bottom(px(PILL_BOTTOM + PILL_HEIGHT + S3 + lift))
            .child(body.with_animation(
                SharedString::from(format!("toast-{}", toast.text)),
                gpui::Animation::new(motion()).with_easing(gpui::ease_out_quint()),
                |el, d| el.opacity(d).mb(px(-S2 * (1. - d))),
            ))
            .into_any_element()
    }

    /// Bildirimin kendisi: simge, metin, varsa "Geri al · Ctrl Z".
    fn toast_body(&self, toast: &Toast, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let accent = self.accent().text;
        div()
            .id("toast")
            .occlude()
            .flex()
            .items_center()
            .gap(px(S3))
            .h(px(S7))
            .pl(px(S4))
            .pr(px(if toast.undo { S2 } else { S4 }))
            .rounded(px(R_XL))
            .bg(rgb(palette().bar))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow(float_shadow())
            .type_role(T_SECONDARY)
            .text_color(text())
            .child(icon(
                if toast.problem {
                    "circle-alert"
                } else {
                    "check"
                },
                16.,
                if toast.problem {
                    rgb(palette().danger_text)
                } else {
                    rgb(accent)
                },
            ))
            .child(
                div().max_w(px(320.)).min_w_0().flex().child(
                    line_with_tip("toast-text", toast.text.clone())
                        .flex()
                        .min_w_0(),
                ),
            )
            .when(toast.undo, |el| {
                el.child(
                    div()
                        .id("toast-undo")
                        .h(px(32.))
                        .px(px(S3))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .gap(px(S2))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(accent))
                        .cursor_pointer()
                        .child(super::tween::hover_fill(
                            veil(VEIL_HOVER),
                            veil(VEIL_PRESSED),
                            0.,
                            super::tween::FULL,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.undo(cx)))
                        .child(t!(Undo))
                        .child(kbd("Ctrl Z")),
                )
            })
    }

    fn render_prompt(&self, prompt: &Prompt, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (title, action, hint, description) = match &prompt.kind {
            PromptKind::Create { rows, .. } if rows.is_empty() => (
                t!(NewPlaylist).to_string(),
                t!(PromptCreate),
                t!(PromptNameHint),
                false,
            ),
            PromptKind::Create { rows, .. } => (
                tp!(PromptNewPlaylistSongs, rows.len()),
                t!(PromptCreate),
                t!(PromptNameHint),
                false,
            ),
            PromptKind::Rename { .. } => (
                t!(PromptRenameTitle).to_string(),
                t!(Save),
                t!(PromptNameHint),
                false,
            ),
            PromptKind::Description { .. } => (
                t!(PromptDescriptionTitle).to_string(),
                t!(Save),
                t!(PromptDescriptionHint),
                true,
            ),
            PromptKind::FolderCreate { .. } => (
                t!(PromptNewFolderTitle).to_string(),
                t!(PromptCreate),
                t!(PromptFolderHint),
                false,
            ),
            PromptKind::FolderRename { .. } => (
                t!(PromptRenameFolderTitle).to_string(),
                t!(Save),
                t!(PromptFolderHint),
                false,
            ),
            PromptKind::DeviceName => (
                t!(PromptDeviceNameTitle).to_string(),
                t!(Save),
                t!(PromptNameHint),
                false,
            ),
        };
        let empty = prompt.text.is_empty();
        let caret = || {
            div()
                .w(px(2.))
                .h(px(T_BODY.line))
                .flex_shrink_0()
                .bg(rgb(accent))
        };
        // Açıklama kutusu çok satır: yazı sarar, kutu dört satırdan sonra keser.
        let field = if description {
            div()
                .min_h(px(T_BODY.line * 4.))
                .max_h(px(T_BODY.line * 4.))
                .px(px(S3))
                .py(px(S2))
                .rounded(px(R_LG))
                .flex()
                .items_end()
                .overflow_hidden()
                .bg(shade(SHADE_INPUT))
                .border_1()
                .border_color(rgb(accent))
                .type_role(T_BODY)
                .text_color(if empty { muted() } else { text() })
                .when(empty, |el| el.child(caret()).child(hint))
                .when(!empty, |el| {
                    el.child(div().flex_1().min_w_0().child(prompt.text.clone()))
                        .child(caret())
                })
        } else {
            div()
                .h(px(40.))
                .px(px(S3))
                .rounded(px(R_LG))
                .flex()
                .items_center()
                .bg(shade(SHADE_INPUT))
                .border_1()
                .border_color(rgb(accent))
                .overflow_hidden()
                .whitespace_nowrap()
                .type_role(T_BODY)
                .text_color(if empty { muted() } else { text() })
                .when(empty, |el| el.child(caret()).child(hint))
                .when(!empty, |el| el.child(prompt.text.clone()).child(caret()))
        };
        let ok = description || !prompt.text.trim().is_empty();
        let counter = description.then(|| {
            div()
                .type_role(T_LABEL)
                .tabular()
                .text_color(label())
                .child(tf!(
                    PromptChars,
                    prompt.text.chars().count(),
                    crate::engine::DESCRIPTION_MAX
                ))
        });
        let panel = div()
            .id("prompt")
            .occlude()
            .w(px(PROMPT_WIDTH))
            .p(px(S5))
            .flex()
            .flex_col()
            .gap(px(S4))
            .rounded(px(R_XL))
            .bg(rgb(palette().bar))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow(float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(S3))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(font())
                            .type_role(T_TITLE)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(text())
                            .child(title),
                    )
                    .children(counter),
            )
            .child(field)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(S2))
                    .child(
                        secondary_button("prompt-cancel", "x", t!(Cancel), Look::Live, accent)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.manage.prompt = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        primary_button_sized(
                            "prompt-ok",
                            "check",
                            action,
                            if ok { Look::Live } else { Look::Disabled },
                            accent,
                            true,
                        )
                        .when(ok, |el| {
                            el.on_click(cx.listener(|this, _, _, cx| this.confirm_prompt(cx)))
                        }),
                    ),
            );
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(shade(SCRIM))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.manage.prompt = None;
                        cx.notify();
                    }),
                )
                .child(layer_blocker().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.manage.prompt = None;
                        cx.notify();
                    }),
                ))
                .child(panel),
        )
        .with_priority(3)
        .into_any_element()
    }

    /// Başlık çubuğundaki eşitleme göstergesi: yalnız bekleyen ya da
    /// yazılamayan değişiklik varken görünür.
    pub(super) fn render_sync_badge(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sync = &self.manage.sync;
        self.sync_badge(
            "sync-badge",
            sync.pending,
            sync.failed.len(),
            self.manage.sync_open,
            cx,
        )
    }

    fn sync_badge(
        &self,
        id: &'static str,
        pending: usize,
        failed: usize,
        open: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if failed == 0 && pending == 0 {
            return None;
        }
        let (icon_name, color, count, tip) = if failed > 0 {
            (
                "circle-alert",
                rgb(palette().danger_text),
                failed,
                tp!(SyncFailed, failed),
            )
        } else {
            ("cloud-upload", muted(), pending, tp!(SyncWriting, pending))
        };
        Some(
            div()
                .id(id)
                .h(px(32.))
                .px(px(S2))
                .mr(px(S2))
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(S1))
                .type_role(T_LABEL)
                .tabular()
                .text_color(color)
                .cursor_pointer()
                .when(open, |el| el.bg(veil(VEIL_SELECTED)))
                .child(super::tween::hover_fill(
                    veil(VEIL_HOVER),
                    veil(VEIL_PRESSED),
                    0.,
                    super::tween::FULL,
                ))
                // Panel açıkken ipucu yok: panelin üstünde kalırdı.
                .when(!open, |el| el.tooltip(Tooltip::text(tip, None)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.manage.sync_open = !this.manage.sync_open;
                    this.manage.menu = None;
                    cx.notify();
                }))
                .child(icon(icon_name, 14., color))
                .child(count.to_string())
                .into_any_element(),
        )
    }

    fn render_sync_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let sync = &self.manage.sync;
        let mut panel = div()
            .id("sync-panel")
            .occlude()
            .w(px(SYNC_PANEL_WIDTH))
            .p(px(S2))
            .flex()
            .flex_col()
            .gap(px(S1))
            .rounded(px(R_XL))
            .bg(rgb(palette().bar))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow(float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(S2))
                    .pt(px(S2))
                    .pb(px(S1))
                    .type_role(T_BODY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text())
                    .child(t!(SyncTitle)),
            );
        if sync.pending > 0 {
            let offline = matches!(self.state.connection, super::player::Connection::Failed(_));
            panel = panel.child(
                div()
                    .px(px(S2))
                    .py(px(S1))
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(if offline {
                        tp!(SyncWaiting, sync.pending)
                    } else {
                        tp!(SyncSaving, sync.pending)
                    }),
            );
        }
        for (k, f) in sync.failed.iter().enumerate() {
            let id = f.id;
            // "Bu liste sana ait değil" yalnız bilinen başkasının listesine yazılırken.
            let foreign = playlist_of(&f.op).is_some_and(|uri| {
                self.library
                    .item(&CollectionId::Playlist(uri.to_string()))
                    .is_some_and(|i| !is_own(i))
            });
            panel =
                panel.child(
                    div()
                        .px(px(S2))
                        .py(px(S2))
                        .rounded(px(R_MD))
                        .bg(veil(VEIL_HOVER))
                        .flex()
                        .flex_col()
                        .gap(px(S2))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(S0))
                                .child(div().type_role(T_BODY).text_color(text()).child(
                                    line_with_tip(("sync-what", k), self.describe_edit(&f.op)),
                                ))
                                .child(
                                    div()
                                        .type_role(T_SECONDARY)
                                        .text_color(rgb(palette().danger_text))
                                        .child(explain_edit_failure(&f.op, f.kind, foreign)),
                                )
                                // Değişiklik ekranda hâlâ uygulanmış görünüyor (örn. çıkarılan
                                // şarkı listede yok): bunu ve "Bırak"ın ne yaptığını söyle.
                                .child(
                                    div()
                                        .type_role(T_SECONDARY)
                                        .text_color(muted())
                                        .child(t!(SyncStillShown)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(S2))
                                .child(
                                    secondary_button(
                                        ("sync-retry", k),
                                        "refresh-cw",
                                        t!(Retry),
                                        Look::Live,
                                        accent,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.engine.send(Command::Library(
                                                LibraryCommand::RetryEdit(id),
                                            ));
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    secondary_button(
                                        ("sync-dismiss", k),
                                        "x",
                                        t!(SyncDrop),
                                        Look::Live,
                                        accent,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| this.dismiss_failed(id, cx),
                                    )),
                                ),
                        ),
                );
        }
        // Panelin sağ kenarı göstergenin sağ kenarında: gösterge üç başlık
        // düğmesinin solunda, aralarında S2 (`sync_badge`). Önce tahmini
        // 140 px'ti, panel göstergeden 6 px sağa taşıyordu.
        let right = f32::from(window.viewport_size().width) - 3. * super::CAPTION_BUTTON_WIDTH - S2;
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.manage.sync_open = false;
                        cx.notify();
                    }),
                )
                // Tekerlek arkadaki listeye geçer (`block_mouse_except_scroll`).
                .child(layer_blocker().block_mouse_except_scroll().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.manage.sync_open = false;
                        cx.notify();
                    }),
                ))
                .child(
                    div()
                        .absolute()
                        .top(px(super::TITLEBAR_HEIGHT))
                        .left(px((right - SYNC_PANEL_WIDTH).max(S2)))
                        .child(panel),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }

    /// Yazma işleminin insan dilinde adı (eşitleme panelinde).
    fn describe_edit(&self, op: &EditOp) -> String {
        let track = |uri: &str| {
            self.track_name(uri)
                .map(|n| format!("“{n}”"))
                .unwrap_or_else(|| t!(ASong).into())
        };
        let list = |uri: &str| {
            let t = self.library.title(&CollectionId::Playlist(uri.to_string()));
            if t.is_empty() {
                t!(AListFallback).to_string()
            } else {
                format!("“{t}”")
            }
        };
        match op {
            EditOp::Save { uri } if uri.starts_with("spotify:album:") => t!(FailedAlbumAdd).into(),
            EditOp::Unsave { uri } if uri.starts_with("spotify:album:") => {
                t!(FailedAlbumRemove).into()
            }
            EditOp::Save { uri } => tf!(FailedLike, track(uri)),
            EditOp::Unsave { uri } => tf!(FailedUnlike, track(uri)),
            EditOp::Follow { uri } => tf!(FailedFollow, self.artist_label(uri)),
            EditOp::Unfollow { uri } => tf!(FailedUnfollow, self.artist_label(uri)),
            EditOp::AddToPlaylist { playlist, uris, .. } => {
                tp!(FailedAddToList, uris.len(), list(playlist))
            }
            EditOp::RemoveFromPlaylist { playlist, uris, .. } => {
                tp!(FailedRemoveFromList, uris.len(), list(playlist))
            }
            EditOp::CreatePlaylist { name, .. } => tf!(FailedCreate, name),
            EditOp::RenamePlaylist { name, .. } => tf!(FailedRename, name),
            EditOp::DeletePlaylist { playlist } => tf!(FailedDelete, list(playlist)),
            EditOp::RestorePlaylist { playlist } => tf!(FailedRestore, list(playlist)),
            EditOp::MovePlaylistItems { playlist, .. } => tf!(FailedReorder, list(playlist)),
            EditOp::SetPlaylistDescription { playlist, .. } => {
                tf!(FailedDescription, list(playlist))
            }
            EditOp::SetPlaylistCollaborative { playlist, .. } => {
                tf!(FailedCollaborative, list(playlist))
            }
            EditOp::CreateFolder { name, .. } => tf!(FailedFolderCreate, name),
            EditOp::RestoreFolder { name, .. } => tf!(FailedFolderRestore, name),
            EditOp::RenameFolder { name, .. } => tf!(FailedFolderRename, name),
            EditOp::DeleteFolder { name, .. } => tf!(FailedFolderDelete, name),
            EditOp::MovePlaylistToFolder { playlist, .. } => {
                tf!(FailedFolderMove, list(playlist))
            }
        }
    }

    // i18n-skip-start: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez
    /// Bileşen vitrini (`OPENIFY_GALLERY=1`): kalp, sağ tık menüsü, geri al
    /// bildirimi, eşitleme göstergesi, ad kutusu, sürükleme etiketi, çoklu seçim.
    pub(super) fn manage_gallery(&self, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let caption = |t: &'static str| div().type_role(T_LABEL).text_color(label()).child(t);
        let sample = TrackRow {
            uri: "spotify:track:vitrin".into(),
            name: "Gülümse".into(),
            artists: "Sezen Aksu, Onno Tunç".into(),
            album: "Gülümse".into(),
            duration_ms: 231_000,
            image: String::new(),
        };
        let mut hearts = div().flex().items_center().gap(px(S5));
        for (i, (liked, look, name)) in [
            (false, Look::Hover, "Beğenilmemiş (satırın üzerinde)"),
            (true, Look::Live, "Beğenilmiş"),
            (true, Look::Hover, "Beğenilmiş, üzerine gelince"),
        ]
        .into_iter()
        .enumerate()
        {
            hearts = hearts.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S2))
                    .child(self.heart_cell(900 + i, &sample, liked, look, false, cx))
                    .child(div().type_role(T_LABEL).text_color(label()).child(name)),
            );
        }
        let menu = super::components::menu(
            "g-row-menu",
            vec![
                super::components::MenuEntry::Item(MenuItem::new("Sıraya ekle").icon("list-end")),
                super::components::MenuEntry::Item(MenuItem::new("Sonra çal").icon("list-start")),
                super::components::MenuEntry::Item(
                    MenuItem::new("Listeye ekle")
                        .icon("list-plus")
                        .look(Look::Hover),
                ),
                super::components::MenuEntry::Separator,
                super::components::MenuEntry::Item(MenuItem::new("Beğen").icon("heart")),
                super::components::MenuEntry::Item(MenuItem::new("Albüme git").icon("disc-3")),
                super::components::MenuEntry::Item(MenuItem::new("Sanatçıya git").icon("user")),
                super::components::MenuEntry::Item(
                    MenuItem::new("Bağlantıyı kopyala").icon("link"),
                ),
                super::components::MenuEntry::Separator,
                super::components::MenuEntry::Item(
                    MenuItem::new("Listeden çıkar")
                        .icon("trash-2")
                        .shortcut("Del")
                        .danger(),
                ),
            ],
            accent,
        )
        .w(px(MENU_WIDTH));
        let toasts = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(S3))
            .child(self.toast_body(
                &Toast {
                    text: "Listeden çıkarıldı".into(),
                    undo: true,
                    problem: false,
                    edits: Vec::new(),
                },
                cx,
            ))
            .child(self.toast_body(
                &Toast {
                    text: "Bağlantı kopyalandı".into(),
                    undo: false,
                    problem: false,
                    edits: Vec::new(),
                },
                cx,
            ))
            .child(self.toast_body(
                &Toast {
                    text: "Bir değişiklik Spotify'a yazılamadı".into(),
                    undo: false,
                    problem: true,
                    edits: Vec::new(),
                },
                cx,
            ));
        let badges = div()
            .flex()
            .items_center()
            .gap(px(S4))
            .children(self.sync_badge("g-sync-1", 2, 0, false, cx))
            .children(self.sync_badge("g-sync-2", 0, 1, false, cx))
            .children(self.sync_badge("g-sync-3", 0, 1, true, cx));
        let preview = div().flex().child(
            DragPreview {
                label: "Gülümse".into(),
                accent,
            }
            .render_static(),
        );
        let mut rows = div().flex().flex_col().w(px(720.));
        for i in 0..3 {
            rows = rows.child(self.track_row(i, &sample, false, i < 2, false, Look::Live, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(S5))
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Kütüphane yönetimi (Adım 13)"),
            )
            .child(caption("Satırdaki kalp"))
            .child(hearts)
            .child(caption(
                "Sağ tık menüsü (satır); \"Listeye ekle\" alt menü açar",
            ))
            .child(div().flex().child(menu))
            .child(caption("Geri al bildirimi (sağ alt köşe, 6 sn)"))
            .child(toasts)
            .child(caption(
                "Eşitleme göstergesi: yazılıyor, yazılamadı, panel açık",
            ))
            .child(badges)
            .child(caption("Sürüklerken imlecin yanındaki etiket"))
            .child(preview)
            .child(caption("Çoklu seçim: iki satır seçili"))
            .child(rows)
            .into_any_element()
    }
    // i18n-skip-end: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez

    /// Bilinen bir şarkının satırı (açık listeler, sayfalardaki listeler).
    fn find_row(&self, uri: &str) -> Option<TrackRow> {
        self.library
            .collections
            .values()
            .filter_map(|l| match l {
                Loadable::Loaded(t) => Some(t),
                _ => None,
            })
            .chain(self.pages.lists.values())
            .find_map(|t| t.iter().find(|r| r.uri == uri).cloned())
    }

    /// Haptaki çalan şarkının satırı (bilinmiyorsa adıyla kurulur).
    pub(super) fn pill_row(&self, uri: &str, name: &str, artists: &str) -> TrackRow {
        self.find_row(uri).unwrap_or_else(|| TrackRow {
            uri: uri.to_string(),
            name: name.to_string(),
            artists: artists.to_string(),
            album: String::new(),
            duration_ms: self.state.duration_ms(),
            image: String::new(),
        })
    }

    /// Haptaki sanatçı adları: her biri sanatçı sayfasına gider (Adım 12). Sağ
    /// tık: "Bu sanatçıyı hiç çalma" (çalan şarkının sanatçıları).
    pub(super) fn pill_artists(
        &self,
        uri: &str,
        name: &str,
        artists: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = self.pill_row(uri, name, artists);
        let links = self.artist_links("np-artists".into(), &row, cx);
        let artists = artists.to_string();
        div()
            .flex()
            .min_w_0()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_playing_artist_menu(&artists, ev.position, cx);
                }),
            )
            .child(links)
            .into_any_element()
    }

    /// Çalan şarkının sanatçı adlarında sağ tık menüsü: her sanatçı için
    /// "Bu sanatçıyı hiç çalma" (kara listedeyse "yeniden izin ver").
    fn open_playing_artist_menu(
        &mut self,
        artists: &str,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let mut entries = Vec::new();
        let all = super::link_text::split_names(artists);
        let named = all.len() > 1;
        for name in all {
            entries.extend(self.never_play_entries(&[name], "", named));
        }
        if !entries.is_empty() {
            self.show_menu(at, entries, cx);
        }
    }

    /// Haptaki kalp: çalan şarkıyı beğen / beğenmekten vazgeç (Adım 13).
    pub(super) fn pill_heart(
        &self,
        uri: &str,
        name: &str,
        artists: &str,
        stop: Option<(&'static str, isize, &Window)>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !uri.starts_with("spotify:track:") {
            return None;
        }
        let accent = self.accent().text;
        let liked = self.is_liked(uri);
        let row = self.pill_row(uri, name, artists);
        let (handle, look) = match stop {
            Some((key, index, window)) => self.stop_look(key, index, true, window, cx),
            None => (None, Look::Live),
        };
        Some(
            div()
                .id("np-heart")
                // Haptaki kalp: hapın boş yer tıklamasına (panel) gitmesin.
                .block_mouse_except_scroll()
                .flex_none()
                .size(px(32.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                // Klavye odağı: hep duran 2 px'lik kenar, odakta vurgu renginde.
                .border_2()
                .border_color(if look == Look::Focus {
                    rgb(accent).into()
                } else {
                    gpui::transparent_black()
                })
                .when_some(handle, |el, h| el.track_focus(&h))
                .child(super::tween::hover_fill(
                    veil(VEIL_HOVER),
                    veil(VEIL_PRESSED),
                    0.,
                    super::tween::FULL,
                ))
                .tooltip(Tooltip::text(
                    if liked { t!(UnlikeTip) } else { t!(LikeTip) },
                    None,
                ))
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    // Fareyle tıklayınca odak bırakılır; klavyeyle (Enter /
                    // Boşluk) düğmede kalır, halka görünür kalsın.
                    if !ev.is_keyboard() {
                        window.focus(&this.focus);
                    }
                    let on = !this.is_liked(&row.uri);
                    this.set_liked(vec![row.clone()], on, cx);
                }))
                .child(icon(
                    if liked { "heart-fill" } else { "heart" },
                    16.,
                    if liked { rgb(accent) } else { muted() },
                ))
                .into_any_element(),
        )
    }

    /// Bilinen bir şarkının adı (açık listeler ve arama dizini).
    fn track_name(&self, uri: &str) -> Option<String> {
        if let Some(name) = self.manage.names.get(uri) {
            return Some(name.clone());
        }
        self.library
            .collections
            .values()
            .filter_map(|l| match l {
                Loadable::Loaded(t) => Some(t),
                _ => None,
            })
            .find_map(|t| t.iter().find(|r| r.uri == uri).map(|r| r.name.clone()))
    }
}

fn item(icon: &'static str, label: String, act: Act) -> Entry {
    Entry::Item {
        icon,
        label,
        shortcut: None,
        danger: false,
        act: Some(act),
        sub: Vec::new(),
    }
}

/// Menü gruplarını art arda koyar, boş olmayan her iki grubun arasına tek
/// ayırıcı: boş grup baştan, sondan ya da ortadan çift ayırıcı bırakmaz.
fn join_groups(groups: Vec<Vec<Entry>>) -> Vec<Entry> {
    let mut out = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        if !out.is_empty() {
            out.push(Entry::Separator);
        }
        out.extend(group);
    }
    out
}

/// Kartın hedefi şu an ekranda mı (menüde "Aç" gereksiz)? Liste / albüm:
/// kütüphane listesi olarak açık (`Route::Collection`, "tümü" görünümü kapalı,
/// seçili liste bu); kütüphanede olmayan albüm ve sanatçı: kendi sayfası.
/// `selected` sayfa gezilince de kalır, bu yüzden yol da bakılır (kütüphane
/// ızgarasında seçili liste "ekranda" sayılmaz).
fn target_on_screen(
    route: &Route,
    all_open: bool,
    selected: Option<&CollectionId>,
    target: &CardTarget,
) -> bool {
    let collection_shown = |id: &CollectionId| {
        !all_open && selected == Some(id) && matches!(route, Route::Collection(c) if c == id)
    };
    match target {
        CardTarget::Collection { id, .. } => collection_shown(id),
        CardTarget::Album { uri, .. } => {
            matches!(route, Route::Album(a) if a == uri)
                || collection_shown(&CollectionId::Album(uri.clone()))
        }
        CardTarget::Artist(uri) => matches!(route, Route::Artist(a) if a == uri),
        CardTarget::Folder { .. } => false,
    }
}

/// İşlemlerin tersleri, ters sırayla (geri almada son yapılan önce döner).
pub fn inverse_all(ops: &[Edit]) -> Vec<Edit> {
    ops.iter()
        .rev()
        .filter_map(|(op, rows, item)| Some((op.inverse()?, rows.clone(), item.clone())))
        .collect()
}

/// Geçici liste adresi gerçeğine döndü: bekleyen geri alma işlemlerinde de.
fn rewrite_playlist(op: &mut EditOp, temp: &str, real: &str) {
    match op {
        EditOp::AddToPlaylist { playlist, .. }
        | EditOp::RemoveFromPlaylist { playlist, .. }
        | EditOp::RenamePlaylist { playlist, .. }
        | EditOp::DeletePlaylist { playlist }
        | EditOp::RestorePlaylist { playlist }
        | EditOp::SetPlaylistDescription { playlist, .. }
        | EditOp::SetPlaylistCollaborative { playlist, .. }
        | EditOp::MovePlaylistToFolder { playlist, .. }
        | EditOp::MovePlaylistItems { playlist, .. }
            if playlist == temp =>
        {
            *playlist = real.to_string();
        }
        _ => {}
    }
}

/// Sürüklenen satırlar (kendi listende sıralama).
#[derive(Clone)]
pub struct DragRows {
    pub list: CollectionId,
    pub from: usize,
    pub len: usize,
    pub label: SharedString,
}

/// Sürüklenirken imlecin yanında görünen etiket.
pub struct DragPreview {
    pub label: SharedString,
    pub accent: u32,
}

impl gpui::Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.render_static()
    }
}

impl DragPreview {
    /// Etiketin kendisi (vitrinde görünüm olmadan da çizilebilsin).
    pub fn render_static(&self) -> gpui::Div {
        div()
            .font_family(UI_FONT)
            .h(px(32.))
            .px(px(S3))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(S2))
            .bg(rgb(palette().bar))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow(float_shadow())
            .type_role(T_SECONDARY)
            .font_weight(FontWeight::MEDIUM)
            .text_color(text())
            .child(icon("music", 14., rgb(self.accent)))
            .child(self.label.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_links_and_ownership() {
        assert_eq!(
            web_link("spotify:track:abc").as_deref(),
            Some("https://open.spotify.com/track/abc")
        );
        // Paylaşma bağlantısı her türde `open.spotify.com/<tür>/<kimlik>`.
        for kind in ["track", "album", "artist", "playlist"] {
            assert_eq!(
                web_link(&format!("spotify:{kind}:abc")).as_deref(),
                Some(format!("https://open.spotify.com/{kind}/abc").as_str())
            );
        }
        assert_eq!(web_link("liked"), None);
        let own = LibraryItem {
            id: CollectionId::Playlist("spotify:playlist:x".into()),
            name: "A".into(),
            subtitle: "12 şarkı".into(),
            image: String::new(),
        };
        assert!(is_own(&own));
        assert!(!is_own(&LibraryItem {
            subtitle: "Takip edilen · 12 şarkı".into(),
            ..own.clone()
        }));
        assert!(!is_own(&LibraryItem {
            subtitle: "Spotify · 50 şarkı".into(),
            ..own
        }));
    }

    /// Menü girdilerinin kısa gösterimi: "-" ayırıcı, "o" öğe.
    fn shape(entries: &[Entry]) -> String {
        entries
            .iter()
            .map(|e| {
                if matches!(e, Entry::Separator) {
                    '-'
                } else {
                    'o'
                }
            })
            .collect()
    }

    fn group(n: usize) -> Vec<Entry> {
        (0..n)
            .map(|_| item("x", "a".into(), Act::Enqueue(Vec::new())))
            .collect()
    }

    #[test]
    fn menu_groups_get_one_separator_between_filled_groups() {
        // Şarkı menüsü: sıra · beğen… · hiç çalma, gizle · bağlantı · çıkar.
        assert_eq!(
            shape(&join_groups(vec![
                group(3),
                group(5),
                group(3),
                group(2),
                group(1)
            ])),
            "ooo-ooooo-ooo-oo-o"
        );
        // Boş grup ayırıcıyı ikiye katlamaz: ortada, başta, sonda.
        assert_eq!(
            shape(&join_groups(vec![
                group(3),
                group(2),
                group(0),
                group(2),
                group(0)
            ])),
            "ooo-oo-oo"
        );
        assert_eq!(
            shape(&join_groups(vec![group(0), group(2), group(1)])),
            "oo-o"
        );
        assert_eq!(
            shape(&join_groups(vec![
                group(2),
                group(0),
                group(0),
                group(0),
                group(0)
            ])),
            "oo"
        );
        assert_eq!(shape(&join_groups(vec![group(0), group(0)])), "");
        // Başta ya da sonda ayırıcı kalmaz.
        let all = join_groups(vec![group(1), group(0), group(1), group(0), group(1)]);
        assert!(
            !matches!(all.first(), Some(Entry::Separator))
                && !matches!(all.last(), Some(Entry::Separator))
        );
    }

    #[test]
    fn open_is_dropped_when_the_target_is_already_on_screen() {
        let pl = |n: u32| CollectionId::Playlist(list(n));
        let card = |id: CollectionId| CardTarget::Collection {
            id,
            title: "A".into(),
        };
        let sel = pl(1);
        // Seçili liste açık: "Aç" yok. Başka liste, "tümü" görünümü ya da sayfa: var.
        assert!(target_on_screen(
            &Route::Collection(pl(1)),
            false,
            Some(&sel),
            &card(pl(1))
        ));
        assert!(!target_on_screen(
            &Route::Collection(pl(1)),
            false,
            Some(&sel),
            &card(pl(2))
        ));
        assert!(!target_on_screen(
            &Route::Collection(pl(1)),
            true,
            Some(&sel),
            &card(pl(1))
        ));
        // Kütüphane ızgarasında `selected` eski kalır: yine de "Aç" var.
        assert!(!target_on_screen(
            &Route::Library,
            false,
            Some(&sel),
            &card(pl(1))
        ));
        assert!(!target_on_screen(
            &Route::Home,
            false,
            Some(&sel),
            &card(pl(1))
        ));
        assert!(!target_on_screen(
            &Route::Collection(pl(1)),
            false,
            None,
            &card(pl(1))
        ));
        // Kütüphanedeki albüm kartı (liste olarak açık) ve kütüphane dışı albüm sayfası.
        let album = CollectionId::Album("spotify:album:a".into());
        assert!(target_on_screen(
            &Route::Collection(album.clone()),
            false,
            Some(&album),
            &card(album.clone())
        ));
        let page = CardTarget::Album {
            uri: "spotify:album:b".into(),
            item: None,
        };
        assert!(target_on_screen(
            &Route::Album("spotify:album:b".into()),
            false,
            None,
            &page
        ));
        assert!(!target_on_screen(
            &Route::Album("spotify:album:c".into()),
            false,
            None,
            &page
        ));
        assert!(!target_on_screen(&Route::Home, false, None, &page));
        // Sanatçı: kendi sayfası.
        let artist = CardTarget::Artist("spotify:artist:a".into());
        assert!(target_on_screen(
            &Route::Artist("spotify:artist:a".into()),
            false,
            None,
            &artist
        ));
        assert!(!target_on_screen(
            &Route::Artist("spotify:artist:z".into()),
            false,
            None,
            &artist
        ));
        assert!(!target_on_screen(&Route::Home, false, None, &artist));
        // Klasör: hep "Aç" / "Genişlet".
        assert!(!target_on_screen(
            &Route::Library,
            true,
            None,
            &CardTarget::Folder { key: "k".into() }
        ));
    }

    fn uri(n: u32) -> String {
        format!("spotify:track:{n:022}")
    }

    fn list(n: u32) -> String {
        format!("spotify:playlist:{n:022}")
    }

    /// Her işlem türü × her hata türü için metin var ve ham metin değil.
    fn every_op() -> Vec<EditOp> {
        vec![
            EditOp::Save { uri: uri(1) },
            EditOp::Unsave { uri: uri(1) },
            EditOp::Save {
                uri: "spotify:album:x".into(),
            },
            EditOp::AddToPlaylist {
                playlist: list(1),
                uris: vec![uri(1)],
                position: AddPosition::Last,
            },
            EditOp::RemoveFromPlaylist {
                playlist: list(1),
                uris: vec![uri(1)],
                indices: vec![],
            },
            EditOp::MovePlaylistItems {
                playlist: list(1),
                from_index: 0,
                length: 1,
                to_index: 2,
                uris: vec![],
            },
            EditOp::RenamePlaylist {
                playlist: list(1),
                name: "A".into(),
                previous_name: None,
            },
            EditOp::CreatePlaylist {
                temp_id: "local:playlist:1".into(),
                name: "A".into(),
                uris: vec![],
                created: None,
            },
            EditOp::DeletePlaylist { playlist: list(1) },
            EditOp::RestorePlaylist { playlist: list(1) },
            EditOp::Follow {
                uri: "spotify:artist:x".into(),
            },
            EditOp::Unfollow {
                uri: "spotify:artist:x".into(),
            },
        ]
    }

    const KINDS: [FailKind; 12] = [
        FailKind::Offline,
        FailKind::RateLimited,
        FailKind::Unauthorized,
        FailKind::Forbidden,
        FailKind::NotFound,
        FailKind::Conflict,
        FailKind::Invalid,
        FailKind::Unsupported,
        FailKind::Server,
        FailKind::Expired,
        FailKind::ParentFailed,
        FailKind::Unknown,
    ];

    #[test]
    fn failure_reason_follows_the_op_and_the_error_kind() {
        let unsave = EditOp::Unsave { uri: uri(1) };
        // Beğenilenler'den çıkarmada "liste senin değil" ASLA geçmez (ekran
        // görüntüsündeki hata): izin yoksa beğeniyle ilgili söylenir.
        for foreign in [false, true] {
            let text = explain_edit_failure(&unsave, FailKind::Forbidden, foreign);
            assert!(!text.to_lowercase().contains("liste"), "{text}");
            assert!(text.contains("beğeni"), "{text}");
        }
        // Yalnız BAŞKASININ listesine yazarken "sana ait değil".
        let add = EditOp::AddToPlaylist {
            playlist: list(1),
            uris: vec![uri(1)],
            position: AddPosition::Last,
        };
        assert!(explain_edit_failure(&add, FailKind::Forbidden, true).contains("sana ait değil"));
        assert!(!explain_edit_failure(&add, FailKind::Forbidden, false).contains("sana ait değil"));
        let rename = EditOp::RenamePlaylist {
            playlist: list(1),
            name: "A".into(),
            previous_name: None,
        };
        assert!(
            explain_edit_failure(&rename, FailKind::Forbidden, true).contains("sana ait değil")
        );
        // Aynı işlem, başka tür: başka neden.
        assert!(explain_edit_failure(&unsave, FailKind::NotFound, false).contains("Şarkı"));
        assert!(explain_edit_failure(&unsave, FailKind::Offline, false).contains("ulaşılamadı"));
        assert!(explain_edit_failure(&unsave, FailKind::Server, false).contains("Biraz sonra"));
        assert!(
            explain_edit_failure(&add, FailKind::NotFound, false).contains("Liste ya da şarkı")
        );
        assert!(explain_edit_failure(&add, FailKind::Conflict, false).contains("başka bir yerde"));
        assert!(
            explain_edit_failure(&unsave, FailKind::Unauthorized, false).contains("yeniden aç")
        );
        let album = EditOp::Unsave {
            uri: "spotify:album:x".into(),
        };
        assert!(explain_edit_failure(&album, FailKind::NotFound, false).contains("Albüm"));
        assert!(explain_edit_failure(&album, FailKind::Forbidden, false).contains("albüm"));
        // Çevrimdışı süren hata beğeniye "liste" demez.
        assert!(
            !explain_edit_failure(&unsave, FailKind::NotFound, false)
                .to_lowercase()
                .contains("liste")
        );
        // Takip: ne beğeniden ne listeden söz eder; neden işleme ve türe göre.
        for op in [
            EditOp::Follow {
                uri: "spotify:artist:x".into(),
            },
            EditOp::Unfollow {
                uri: "spotify:artist:x".into(),
            },
        ] {
            for foreign in [false, true] {
                let denied = explain_edit_failure(&op, FailKind::Forbidden, foreign);
                assert!(
                    denied.contains("takip") && !denied.to_lowercase().contains("liste"),
                    "{denied}"
                );
                assert!(!denied.contains("beğeni"), "{denied}");
            }
            assert!(explain_edit_failure(&op, FailKind::NotFound, false).contains("Sanatçı"));
            assert!(explain_edit_failure(&op, FailKind::Offline, false).contains("ulaşılamadı"));
        }
    }

    #[test]
    fn every_op_and_kind_has_a_plain_sentence() {
        for op in every_op() {
            for kind in KINDS {
                for foreign in [false, true] {
                    let text = explain_edit_failure(&op, kind, foreign);
                    assert!(
                        text.ends_with('.') || text.ends_with(')'),
                        "{op:?} {kind:?}: {text}"
                    );
                    // Ham hata gövdesi / teknik söz sızmaz.
                    for raw in ["403", "500", "HTTP", "status", "Err(", "sınama", "{"] {
                        assert!(!text.contains(raw), "{op:?} {kind:?}: {text}");
                    }
                }
            }
        }
        // Neden "liste sana ait değil" yalnız başkasının listesine yazan işlemlerde.
        for op in every_op() {
            for kind in KINDS {
                for foreign in [false, true] {
                    let text = explain_edit_failure(&op, kind, foreign);
                    if text.contains("sana ait değil") {
                        assert!(
                            foreign
                                && kind == FailKind::Forbidden
                                && matches!(
                                    op,
                                    EditOp::AddToPlaylist { .. }
                                        | EditOp::RemoveFromPlaylist { .. }
                                        | EditOp::MovePlaylistItems { .. }
                                        | EditOp::RenamePlaylist { .. }
                                ),
                            "{op:?} {kind:?} {foreign}: {text}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn failure_strip_goes_away_with_the_last_failed_edit() {
        use super::super::player::Notice;
        use crate::engine::NoticeKind;
        let strip = Notice {
            kind: NoticeKind::Problem,
            text: edit_failed_notice().into(),
            at: std::time::Instant::now(),
        };
        let other = Notice {
            kind: NoticeKind::Problem,
            text: "Kütüphane tazelenemedi.".into(),
            at: strip.at,
        };
        // Yazılamayan hâlâ var: şerit durur. Kalmadı: kalkar.
        assert!(!stale_failure_notice(Some(&strip), 1));
        assert!(stale_failure_notice(Some(&strip), 0));
        // Başka bir sorun şeridine ve boş duruma dokunulmaz.
        assert!(!stale_failure_notice(Some(&other), 0));
        assert!(!stale_failure_notice(None, 0));
    }

    #[test]
    fn playlist_of_names_only_existing_list_writes() {
        let ops = every_op();
        let named: Vec<bool> = ops.iter().map(|o| playlist_of(o).is_some()).collect();
        // Save, Unsave, albüm: yok; ekle, çıkar, taşı, adlandır: var; oluştur, sil, geri getir: yok.
        assert_eq!(
            named,
            [
                false, false, false, true, true, true, true, false, false, false, false, false
            ]
        );
    }

    fn undo_entry(done: Vec<EditOp>) -> Undo {
        Undo {
            label: "x".into(),
            ops: Vec::new(),
            done,
        }
    }

    fn toast_for(edits: Vec<EditOp>) -> Toast {
        Toast {
            text: "Beğenilenlerden çıkarıldı".into(),
            undo: true,
            problem: false,
            edits,
        }
    }

    #[test]
    fn failed_write_retracts_its_success_toast_and_undo_entry() {
        let unsave = EditOp::Unsave { uri: uri(1) };
        let other = EditOp::Unsave { uri: uri(2) };
        let mut m = ManageState::new();
        m.undo.push(undo_entry(vec![other.clone()]));
        m.undo.push(undo_entry(vec![unsave.clone()]));
        m.toast = Some(toast_for(vec![unsave.clone()]));
        // Başka bir işlem yazılamadı: ne bildirim ne geri alma etkilenir.
        let unrelated = EditOp::Save { uri: uri(9) };
        assert!(!m.retract_failed(&[unrelated]));
        assert!(m.toast.is_some());
        assert_eq!(m.undo.len(), 2);
        // Bu işlem yazılamadı: "Beğenilenlerden çıkarıldı" bildirimi ve onun geri
        // alması kalkar; öbür işlemin geri alması durur. Aynı anda iki çelişen
        // bildirim kalmaz.
        assert!(m.retract_failed(&[unsave]));
        assert!(m.toast.is_none());
        assert_eq!(m.undo.len(), 1);
        assert_eq!(m.undo[0].done, [other]);
    }

    #[test]
    fn failed_follow_retracts_its_toast_and_undo_entry_but_not_other_follows() {
        let follow = EditOp::Follow {
            uri: "spotify:artist:a".into(),
        };
        let other = EditOp::Follow {
            uri: "spotify:artist:b".into(),
        };
        let mut m = ManageState::new();
        m.undo.push(undo_entry(vec![other.clone()]));
        m.undo.push(undo_entry(vec![follow.clone()]));
        m.toast = Some(Toast {
            text: "Tarkan takip ediliyor".into(),
            undo: true,
            problem: false,
            edits: vec![follow.clone()],
        });
        // Yazılamayan takip: "takip ediliyor" bildirimi ve geri alması kalkar; öteki sanatçının kalır.
        assert!(m.retract_failed(std::slice::from_ref(&follow)));
        assert!(m.toast.is_none());
        assert_eq!(m.undo.len(), 1);
        assert_eq!(m.undo[0].done, [other]);
        // Aynı sanatçıyı bırakmak başka işlem: onun hatası takip bildirimine dokunmaz.
        let mut m = ManageState::new();
        m.toast = Some(Toast {
            text: "x".into(),
            undo: true,
            problem: false,
            edits: vec![follow.clone()],
        });
        assert!(!m.retract_failed(&[EditOp::Unfollow {
            uri: "spotify:artist:a".into()
        }]));
        assert!(m.toast.is_some());
    }

    #[test]
    fn retract_handles_toast_without_undo_and_undo_without_toast() {
        let op = EditOp::Save { uri: uri(1) };
        // "Geri alındı: ..." bildirimi (geri alma kaydı yok) da kalkar.
        let mut m = ManageState::new();
        m.toast = Some(Toast {
            text: "Geri alındı".into(),
            undo: false,
            problem: false,
            edits: vec![op.clone()],
        });
        assert!(m.retract_failed(std::slice::from_ref(&op)));
        assert!(m.toast.is_none());
        // Bildirim süresi dolmuş olsa da geri alma kaydı düşer (Ctrl+Z yazılmamış
        // işi geri almaya kalkmasın).
        let mut m = ManageState::new();
        m.undo.push(undo_entry(vec![op.clone()]));
        assert!(m.retract_failed(std::slice::from_ref(&op)));
        assert!(m.undo.is_empty());
        // Bilgi bildirimlerine (işlemsiz) dokunulmaz.
        let mut m = ManageState::new();
        m.toast = Some(Toast {
            text: "Bağlantı kopyalandı".into(),
            undo: false,
            problem: false,
            edits: Vec::new(),
        });
        assert!(!m.retract_failed(&[op]));
        assert!(m.toast.is_some());
    }

    #[test]
    fn half_created_create_still_matches_its_failed_record() {
        // Kuyruktan dönen oluşturma kaydı `created` adresini taşıyabilir.
        let sent = EditOp::CreatePlaylist {
            temp_id: "local:playlist:7".into(),
            name: "A".into(),
            uris: vec![uri(1)],
            created: None,
        };
        let back = EditOp::CreatePlaylist {
            temp_id: "local:playlist:7".into(),
            name: "A".into(),
            uris: vec![uri(1)],
            created: Some(list(3)),
        };
        let mut m = ManageState::new();
        m.toast = Some(toast_for(vec![sent]));
        assert!(m.retract_failed(&[back]));
        assert!(m.toast.is_none());
    }

    #[test]
    fn undo_reverses_in_reverse_order() {
        let p = "spotify:playlist:x".to_string();
        let ops: Vec<Edit> = vec![
            (
                EditOp::RenamePlaylist {
                    playlist: p.clone(),
                    name: "B".into(),
                    previous_name: Some("A".into()),
                },
                vec![],
                None,
            ),
            (
                EditOp::DeletePlaylist {
                    playlist: p.clone(),
                },
                vec![],
                None,
            ),
        ];
        let inv = inverse_all(&ops);
        assert!(matches!(inv[0].0, EditOp::RestorePlaylist { .. }));
        assert!(matches!(&inv[1].0, EditOp::RenamePlaylist { name, .. } if name == "A"));
    }
}
