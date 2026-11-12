//! Komut paleti (Ctrl+K).
//!
//! Müzik ve kullanıcı odaklı, sade arama (7 Kasım gecesi: "hâlâ kod
//! editörü gibi"): kutu boşken yalnız son aramalar ve son çalınanlar
//! (kapaklı); komut listesi, tuş rozetleri, alt satır ve sayaç yok. Yazdıkça
//! şarkı, sanatçı, albüm ve liste sonuçları kapaklarıyla anında güncellenir
//! (kütüphane yerelde, Spotify `SPOTIFY_DELAY` sonra); komutlar (sonraki
//! şarkı, sessize al, Keşfet...) yalnız adı yazılınca sonuç olarak çıkar.
//! Eski sorguların sonuçları sıra numarasıyla ayıklanır.
//!
//! Tuşlar: ↑/↓ seç, Enter aç / çal, Esc kapat, Ctrl+Backspace kelime sil,
//! Ctrl+Enter bütün sonuçlar (kısayol listesi "Kısayollar" komutuyla).

use std::{sync::Arc, time::Duration};

use gpui::{
    AnimationExt, AnyElement, Context, KeyDownEvent, MouseButton, ScrollHandle, SharedString, Task,
    Window, deferred, div, prelude::*, px, rgb,
};

use super::{
    Shell, components::kbd, cover_image, fuzzy, library::Loadable, player::fmt_time,
    text::line_with_tip, theme::*,
};
use crate::engine::{CollectionId, Command, SearchResults, TrackRow};
use crate::{t, tf, tp};

const SPOTIFY_DELAY: Duration = Duration::from_millis(300);
/// Panelin eni ve en büyük boyu (her yerden açılan palet penceresi de buna göre).
pub const PANEL_WIDTH: f32 = 640.;
pub const PANEL_MAX_HEIGHT: f32 = 600.;
const MAX_COMMANDS: usize = 3;
// Her bölümden birkaç sonuç: dört tür (şarkı, sanatçı, albüm, liste) kaydırmadan
// görünür olsun diye kısa tutulur; fazlası "Bütün sonuçları gör"de.
const MAX_COLLECTIONS: usize = 2;
const MAX_LOCAL_TRACKS: usize = 3;
const MAX_SPOTIFY_TRACKS: usize = 3;
const MAX_SPOTIFY_OTHER: usize = 2;
/// Kutu boşken en çok kaç son arama ve son çalınan gösterilir.
const MAX_RECENT_SEARCHES: usize = 3;
const MAX_RECENT_PLAYED: usize = 5;
/// Sonuç satırındaki kapak.
const THUMB: f32 = S7 - S2;
/// Spotify'da aramak için en az bu kadar harf.
const MIN_SPOTIFY_QUERY: usize = 2;
/// Yazı bir şarkıyla daha iyi eşleşiyorsa komut listeden çıkar: komut ancak
/// en iyi eşleşmenin bu oranına yetişirse kalır. ("dal": şarkı 88, harfleri
/// dağılmış "Çal / Duraklat" 61.)
const COMMAND_RELATIVE_SCORE: f32 = 0.85;

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

/// Komut, adı, açıklaması, kısayolu. Çal ve sessize al komutlarının adı
/// o anki duruma göre değişir (bkz. `command_state`).
fn commands() -> [(
    PaletteCommand,
    &'static str,
    &'static str,
    Option<&'static str>,
); 16] {
    [
        (
            PaletteCommand::TogglePlay,
            t!(CmdTogglePlay),
            "",
            Some(t!(KeySpace)),
        ),
        (PaletteCommand::Next, t!(CmdNext), "", Some("Ctrl →")),
        (
            PaletteCommand::Previous,
            t!(CmdPrevious),
            "",
            Some("Ctrl ←"),
        ),
        (
            PaletteCommand::ToggleMute,
            t!(CmdToggleMute),
            "",
            Some("Ctrl Shift ↓"),
        ),
        (
            PaletteCommand::RefreshLibrary,
            t!(CmdRefreshLibrary),
            t!(CmdRefreshLibraryDesc),
            None,
        ),
        (
            PaletteCommand::Home,
            t!(NavHome),
            t!(CmdHomeDesc),
            Some("Ctrl 1"),
        ),
        (
            PaletteCommand::Explore,
            t!(NavExplore),
            t!(CmdExploreDesc),
            Some("Ctrl 2"),
        ),
        (
            PaletteCommand::Library,
            t!(NavLibrary),
            t!(CmdLibraryDesc),
            Some("Ctrl 3"),
        ),
        (
            PaletteCommand::Recent,
            t!(RecentPageTitle),
            t!(CmdRecentDesc),
            None,
        ),
        (
            PaletteCommand::NewPlaylist,
            t!(NewPlaylist),
            t!(CmdNewPlaylistDesc),
            None,
        ),
        (
            PaletteCommand::Tools,
            t!(ToolsTitle),
            t!(CmdToolsDesc),
            None,
        ),
        (
            PaletteCommand::StartRadio,
            t!(CmdStartRadio),
            t!(CmdStartRadioDesc),
            None,
        ),
        (
            PaletteCommand::PrivateSession,
            t!(CmdPrivateSession),
            t!(CmdPrivateSessionDesc),
            None,
        ),
        (
            PaletteCommand::Settings,
            t!(Settings),
            t!(CmdSettingsDesc),
            Some("Ctrl ,"),
        ),
        (
            PaletteCommand::Shortcuts,
            t!(Shortcuts),
            t!(CmdShortcutsDesc),
            Some("?"),
        ),
        (
            PaletteCommand::Logout,
            t!(SignOut),
            t!(CmdSignOutDesc),
            None,
        ),
    ]
}

/// Arama kutusunun yer tutucusu: başlık çubuğundaki kutu ve palet aynı
/// metni gösterir (ikisi aynı kutu). Kısa: dar pencerede de kesilmez;
/// komutlar palet açılınca zaten listede.
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
    PlayTrack(String),
    /// Sanatçı sayfası, albüm (kütüphanedeyse liste, değilse sayfa), tam
    /// arama sonuçları (Adım 12).
    OpenArtist(String),
    OpenAlbum(String),
    AllResults(String),
    /// Son aramalardan biri: yazıya dönüşür, sonuçlar gelir.
    Search(String),
}

/// Satırın solundaki küçük görsel.
#[derive(Clone)]
pub enum Thumb {
    /// Kapak resmi (şarkı, albüm, liste).
    Cover { url: String, icon: &'static str },
    /// Yuvarlak portre (sanatçı).
    Artist { url: String },
    /// Kütüphanedeki liste (beğenilenler için kalp).
    Collection { id: CollectionId, url: String },
    /// Komut: ton karosunda simge.
    Icon(&'static str),
    /// Son arama: karosuz, yalnız soluk simge (satır hafif kalır).
    Glyph(&'static str),
}

pub struct Item {
    pub thumb: Thumb,
    /// Komutun sabit adı (sadeleşmiş): başlık duruma göre değişebilir ("Sessize
    /// al" / "Sesi aç"), yazılanın adı tutup tutmadığı bununla bakılır.
    pub name_key: String,
    pub section: &'static str,
    pub title: String,
    pub subtitle: String,
    /// Sağda soluk gösterilen ek bilgi (süre, kısayol).
    pub detail: String,
    pub action: Action,
    /// Geri dönüşü zor işlem (çıkış): tehlike yazısıyla, onay ister.
    pub danger: bool,
    /// Sağda kısayol etiketi olarak gösterilen tuş.
    pub shortcut: Option<&'static str>,
    /// Şu an yapılamaz (örneğin sıradaki şarkı yok): soluk, seçilmez.
    pub disabled: bool,
}

impl Item {
    fn new(
        section: &'static str,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        action: Action,
        thumb: Thumb,
    ) -> Self {
        Self {
            thumb,
            name_key: String::new(),
            section,
            title: title.into(),
            subtitle: subtitle.into(),
            detail: String::new(),
            action,
            danger: false,
            shortcut: None,
            disabled: false,
        }
    }
}

pub enum Spotify {
    Idle,
    Waiting,
    Loading,
    Done(Arc<SearchResults>),
    Failed(String),
}

pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub items: Vec<Item>,
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
        self.recompute_palette();

        let query = self.palette.query.trim().to_string();
        if query.chars().count() >= MIN_SPOTIFY_QUERY {
            self.palette.spotify = Spotify::Waiting;
            self.palette.debounce = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SPOTIFY_DELAY).await;
                let _ = this.update(cx, |shell, cx| {
                    shell.palette.seq += 1;
                    shell.palette.spotify = Spotify::Loading;
                    shell.engine.send(Command::Search {
                        seq: shell.palette.seq,
                        query,
                    });
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }

    /// Sorguya göre listeyi baştan kurar.
    pub(super) fn recompute_palette(&mut self) {
        self.palette.confirm = None;
        let query = self.palette.query.clone();
        let mut items = Vec::new();

        // Kutu boşken yalnız son aramalar ve son çalınanlar: komut listesi yok.
        let Some(mut f) = fuzzy::Fuzzy::new(&query) else {
            self.palette.items = self.palette_idle_items();
            self.palette.selected = 0;
            return;
        };

        // Komutlar yalnız adı yazılınca (adın içinde geçen yazı) çıkar.
        let typed = fuzzy::fold(query.trim());
        if typed.chars().count() >= MIN_SPOTIFY_QUERY {
            let all_commands = commands();
            let keys: Vec<String> = all_commands.iter().map(|c| fuzzy::fold(c.1)).collect();
            let named: Vec<usize> = (0..keys.len())
                .filter(|i| keys[*i].contains(&typed))
                .collect();
            for i in f.top(named.iter().map(|i| keys[*i].as_str()), MAX_COMMANDS) {
                let (cmd, title, detail, _) = all_commands[named[i]];
                items.push(command_item(cmd, title, detail, None));
            }
        }

        // Kütüphanedeki listeler ve albümler.
        if let Loadable::Loaded(library) = &self.library.library {
            let mut all = vec![(
                CollectionId::Liked,
                t!(LikedSongs).to_string(),
                tp!(Songs, library.liked_count),
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
            for i in f.top(keys.iter().map(String::as_str), MAX_COLLECTIONS) {
                let (id, name, subtitle, image) = all[i].clone();
                items.push(Item::new(
                    t!(LibraryYours),
                    name.clone(),
                    subtitle,
                    Action::OpenCollection {
                        id: id.clone(),
                        title: name,
                    },
                    Thumb::Collection { id, url: image },
                ));
            }
        }

        // Kütüphanedeki şarkılar.
        let tracks = self.palette.tracks.clone();
        let mut local_uris = std::collections::HashSet::new();
        for i in f.top(
            self.palette.track_keys.iter().map(String::as_str),
            MAX_LOCAL_TRACKS,
        ) {
            local_uris.insert(tracks[i].uri.clone());
            items.push(track_item(t!(PaletteYourSongs), &tracks[i]));
        }

        // Spotify'ın tamamı (kütüphanedekiyle aynı şarkı iki kez çıkmaz).
        if let Spotify::Done(results) = &self.palette.spotify {
            for t in results
                .tracks
                .iter()
                .filter(|t| !local_uris.contains(&t.uri))
                .take(MAX_SPOTIFY_TRACKS)
            {
                items.push(track_item(t!(SectionSongs), t));
            }
            // Sanatçı ve albüm kendi sayfasında açılır (Adım 12).
            for a in results.artists.iter().take(MAX_SPOTIFY_OTHER) {
                items.push(Item::new(
                    t!(SectionArtists),
                    a.name.clone(),
                    t!(Artist),
                    Action::OpenArtist(a.uri.clone()),
                    Thumb::Artist {
                        url: a.image.clone(),
                    },
                ));
            }
            for hit in results.albums.iter().take(MAX_SPOTIFY_OTHER) {
                items.push(Item::new(
                    t!(KindAlbums),
                    hit.name.clone(),
                    crate::i18n::canon::localize(&hit.subtitle).into_owned(),
                    Action::OpenAlbum(hit.uri.clone()),
                    Thumb::Cover {
                        url: hit.image.clone(),
                        icon: "disc-3",
                    },
                ));
            }
            for hit in results.playlists.iter().take(MAX_SPOTIFY_OTHER) {
                items.push(Item::new(
                    t!(KindLists),
                    hit.name.clone(),
                    crate::i18n::canon::localize(&hit.subtitle).into_owned(),
                    Action::OpenCollection {
                        id: CollectionId::Playlist(hit.uri.clone()),
                        title: hit.name.clone(),
                    },
                    Thumb::Cover {
                        url: hit.image.clone(),
                        icon: "list-music",
                    },
                ));
            }
        }

        for item in &mut items {
            if let Action::Command(cmd) = item.action {
                let (title, disabled) = self.command_state(cmd);
                if let Some(title) = title {
                    item.title = title;
                }
                item.disabled = disabled;
            }
        }

        // Bütün bölümler tek ölçüyle: yazılanla en iyi eşleşen öğe seçili
        // gelir (Enter onu çalıştırır). Komutlar ancak en iyi eşleşmeye
        // yakınsa listede: "dal" yazınca harfleri "Çal / Duraklat"a dağıldığı
        // için Enter şarkı yerine müziği duraklatıyordu.
        let (ranked, best) = rank_items(&query, items);
        items = ranked;
        if let Some(best) = best.filter(|_| !self.palette.moved) {
            self.palette.selected = best;
        }
        // Tam sonuç sayfası en sonda: sıralamaya girmez (yazılanı içerdiği
        // için en iyi eşleşme sayılıp Enter'ı şarkıdan çalardı). Ctrl+Enter
        // her an doğrudan açar.
        let trimmed = query.trim();
        if trimmed.chars().count() >= MIN_SPOTIFY_QUERY {
            items.push(Item::new(
                t!(PaletteMore),
                t!(PaletteAllResults),
                tf!(PaletteAllResultsDesc, trimmed),
                Action::AllResults(trimmed.to_string()),
                Thumb::Glyph("search"),
            ));
        }

        self.palette.items = items;
        self.palette.selected = self
            .palette
            .selected
            .min(self.palette.items.len().saturating_sub(1));
        // Seçim kullanılamayan bir komutta durmasın.
        if self
            .palette
            .items
            .get(self.palette.selected)
            .is_some_and(|i| i.disabled)
        {
            self.move_palette_selection(1);
        }
    }

    /// Kutu boşken gösterilenler: son aramalar (varsa) ve son çalınan listeler
    /// (kapaklı). Hiçbiri yoksa boş: palet yalnız giriş satırı.
    fn palette_idle_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for q in self
            .palette
            .recent_searches
            .iter()
            .take(MAX_RECENT_SEARCHES)
        {
            items.push(Item::new(
                t!(PaletteRecentSearches),
                q.clone(),
                "",
                Action::Search(q.clone()),
                Thumb::Glyph("search"),
            ));
        }
        if let Loadable::Loaded(library) = &self.library.library {
            for it in crate::engine::home(library, &self.recent, MAX_RECENT_PLAYED).recent {
                items.push(Item::new(
                    t!(NowRecentTitle),
                    it.name.clone(),
                    crate::i18n::canon::localize(&it.subtitle).into_owned(),
                    Action::OpenCollection {
                        id: it.id.clone(),
                        title: it.name.clone(),
                    },
                    Thumb::Collection {
                        id: it.id,
                        url: it.image,
                    },
                ));
            }
        }
        items
    }

    /// Komutun şu anki adı (duruma göre değişiyorsa) ve yapılamaz olup
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

    /// Seçimi taşır; kullanılamayan öğelerin üstünden atlar.
    fn move_palette_selection(&mut self, delta: isize) {
        let len = self.palette.items.len();
        if len == 0 {
            return;
        }
        let mut next = self.palette.selected;
        for _ in 0..len {
            next = (next as isize + delta).rem_euclid(len as isize) as usize;
            if !self.palette.items[next].disabled {
                break;
            }
        }
        self.palette.selected = next;
        self.palette.scroll.scroll_to_item(next);
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.palette.items.get(index) else {
            return;
        };
        let action = item.action.clone();
        if item.disabled {
            return;
        }
        match action {
            // Son arama: yazıya dönüşür, sonuçlar gelir.
            Action::Search(q) => {
                self.palette.query = q;
                self.on_query_changed(cx);
                return;
            }
            _ => {}
        }
        // Tehlikeli işlem ilk Enter'da çalışmaz: satır onay ister.
        if item.danger && self.palette.confirm != Some(index) {
            self.palette.confirm = Some(index);
            cx.notify();
            return;
        }
        // Liste ana pencerede açılır: her yerden açılan paletten seçildiyse
        // ana pencere öne gelsin.
        self.palette.wants_window = matches!(
            action,
            Action::OpenCollection { .. }
                | Action::OpenArtist(_)
                | Action::OpenAlbum(_)
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
        // Yazılan bir sonuca gidildiyse (komut değil) son aramalara girer.
        if !matches!(action, Action::Command(_)) && !self.palette.query.trim().is_empty() {
            let query = self.palette.query.clone();
            self.palette.note_search(&query);
            self.save_settings();
        }
        self.close_palette(cx);
        match action {
            Action::Command(cmd) => self.run_command(cmd, cx),
            Action::OpenArtist(uri) => self.navigate(super::history::Route::Artist(uri), cx),
            Action::OpenAlbum(uri) => self.open_album(uri, cx),
            Action::AllResults(query) => self.navigate(super::history::Route::Search(query), cx),
            Action::OpenCollection { id, title } => {
                self.library.titles.insert(id.clone(), title);
                self.select_collection(id, cx);
            }
            Action::PlayTrack(uri) => {
                if let Ok(uri) = librespot::core::SpotifyUri::from_uri(&uri) {
                    if self.ready_to_play(cx) {
                        self.state.notice = None;
                        self.engine.send(Command::Load(uri));
                    }
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
            PaletteCommand::Recent => self.navigate(super::history::Route::Recent, cx),
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
                    self.navigate(super::history::Route::Search(query), cx);
                }
            }
            "enter" => self.activate(self.palette.selected, cx),
            "up" => {
                self.palette.moved = true;
                self.move_palette_selection(-1);
                cx.notify();
            }
            "down" | "tab" => {
                self.palette.moved = true;
                self.move_palette_selection(1);
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

    /// Ana penceredeki palet: paneli karartılmış zeminin üstünde, başlık
    /// çubuğundaki arama kutusunun yerinde açar.
    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = self.render_palette_panel(cx);
        deferred(
            div()
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
                // Panel boyca gerilmesin, içeriği kadar olsun. Başlık
                // çubuğundaki arama kutusunun üstüne açılır: giriş satırı o
                // kutunun yerine oturur, palet "ikinci bir arama kutusu"
                // gibi durmaz, aynı kutu büyür.
                .items_start()
                .pt(px(S0))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _: &mut Window, cx| this.close_palette(cx)),
                )
                .child(panel),
        )
        .with_priority(1)
        .into_any_element()
    }

    /// Paletin paneli (giriş ve sonuçlar; alt satır yok) açılma geçişiyle. Ana
    /// pencere de her yerden açılan palet penceresi de bunu çizer: ikisi
    /// birebir aynı.
    pub(super) fn render_palette_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let query: SharedString = if self.palette.query.is_empty() {
            search_placeholder().into()
        } else {
            self.palette.query.clone().into()
        };
        // Paletin bütün içeriği aynı sol kenarda: giriş, bölüm adları ve
        // satır içeriği (S4 + S2). Altında gösterecek bir şey yoksa (kutu boş,
        // son arama ve son çalınan yok) panel yalnız giriş satırı: çizgi yok.
        let input = div()
            .px(px(S4))
            .py(px(S2))
            .flex()
            .items_center()
            .gap(px(S3))
            .when(
                !self.palette.items.is_empty() || !self.palette.query.is_empty(),
                |el| el.border_b_1().border_color(rgb(BORDER)),
            )
            .child(icon("search", 18., muted()))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .type_role(T_TITLE)
                    .text_color(if self.palette.query.is_empty() {
                        muted()
                    } else {
                        text()
                    })
                    // İmleç: yazılanın sonunda (boşken yer tutucunun başında).
                    .when(self.palette.query.is_empty(), |el| {
                        el.child(caret(self.accent().text))
                    })
                    .child(query)
                    .when(!self.palette.query.is_empty(), |el| {
                        el.child(caret(self.accent().text))
                    }),
            )
            .child(
                div()
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child(spotify_status(&self.palette.spotify)),
            );

        let mut list = div()
            .id("palette-results")
            // İçerik kadar yer kaplasın; uzarsa panel sınırında kaydırılsın.
            .flex_shrink()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.palette.scroll)
            .px(px(S2))
            .pb(px(S2));
        let mut last_section = "";
        for (i, item) in self.palette.items.iter().enumerate() {
            let header = (item.section != last_section).then_some(item.section);
            last_section = item.section;
            list = list.child(self.palette_row(i, item, header, cx));
        }
        if self.palette.items.is_empty() && !self.palette.query.is_empty() {
            list = list.child(
                div()
                    .px(px(S2))
                    .py(px(S3))
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    // Spotify araması sürerken "sonuç yok" demek erken.
                    .child(
                        if matches!(self.palette.spotify, Spotify::Waiting | Spotify::Loading) {
                            t!(PaletteSearching)
                        } else {
                            t!(PaletteNoResults)
                        },
                    ),
            );
        }

        let panel = div()
            .id("palette")
            .w(px(PANEL_WIDTH))
            .max_h(px(PANEL_MAX_HEIGHT))
            .flex()
            .flex_col()
            .bg(rgb(BAR))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded(px(R_XL))
            .shadow(float_shadow())
            .overflow_hidden()
            // Panelin içine tıklamak paleti kapatmasın.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(input)
            .child(list);

        // Kapaklar: çizimde istenenler motora gider (her yerden açılan palet
        // penceresi de dahil).
        self.flush_image_requests(cx);
        panel
            .with_animation(
                ("palette-open", self.palette.opened),
                gpui::Animation::new(motion()).with_easing(gpui::ease_out_quint()),
                |el, d| el.opacity(d).mt(px(-S2 * (1. - d))),
            )
            .into_any_element()
    }

    fn palette_row(
        &self,
        i: usize,
        item: &Item,
        header: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = i == self.palette.selected;
        let confirming = self.palette.confirm == Some(i);
        // Onay hem fareyle hem klavyeyle verilebilir: metin ikisini de söyler.
        let (title, subtitle, detail): (String, String, String) = if confirming {
            (
                t!(PaletteConfirmSignOut).into(),
                t!(PaletteConfirmSignOutDesc).into(),
                String::new(),
            )
        } else {
            (
                item.title.clone(),
                item.subtitle.clone(),
                item.detail.clone(),
            )
        };
        let title_color = if item.disabled {
            label()
        } else if item.danger {
            rgb(DANGER_TEXT)
        } else {
            text()
        };
        let interactive = !item.disabled;
        let thumb = self.palette_thumb(&item.thumb, &item.title);
        let row = div()
            .id(("palette-item", i))
            .px(px(S2))
            .py(px(S1))
            .rounded(px(R_MD))
            .flex()
            .items_center()
            .gap(px(S3))
            .when(interactive, |el| {
                el.cursor_pointer()
                    .when(selected, |el| el.bg(veil(VEIL_SELECTED)))
                    .when(!selected, |el| el.hover(|s| s.bg(veil(VEIL_HOVER))))
                    .active(|s| s.bg(veil(VEIL_PRESSED)))
                    .on_click(cx.listener(move |this, _, _, cx| this.activate(i, cx)))
            })
            .children(thumb)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S0))
                    .child(
                        div()
                            .type_role(T_BODY)
                            .text_color(title_color)
                            .child(line_with_tip(("palette-title", i), title)),
                    )
                    .when(!subtitle.is_empty(), |el| {
                        el.child(
                            div()
                                .type_role(T_SECONDARY)
                                .text_color(if item.disabled { label() } else { muted() })
                                .child(line_with_tip(("palette-subtitle", i), subtitle)),
                        )
                    }),
            )
            // Sağda yalnız kısa bilgi: şarkının süresi (kısayol etiketi yalnız
            // "Kısayollar" listesinde).
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S2))
                    .when(!detail.is_empty(), |el| {
                        el.child(
                            div()
                                .type_role(T_LABEL)
                                .tabular()
                                .text_color(label())
                                .child(detail),
                        )
                    })
                    .when(!confirming, |el| el.children(item.shortcut.map(kbd))),
            );

        div()
            .flex()
            .flex_col()
            .when_some(header, |el, h| {
                // Tek satırlık "Daha fazlası" bölümünün adı yer kaplamasın: yalnız
                // ince bir çizgi (satırın kendisi zaten ne olduğunu söylüyor).
                if is_divider_header(h) {
                    el.child(
                        div()
                            .mx(px(S2))
                            .mt(px(S2))
                            .mb(px(S1))
                            .h(px(1.))
                            .bg(veil(LINE)),
                    )
                } else {
                    el.child(
                        div()
                            .px(px(S2))
                            .pt(px(S3))
                            .pb(px(S1))
                            .type_role(T_LABEL)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(label())
                            .child(h),
                    )
                }
            })
            .child(row)
            .into_any_element()
    }

    /// Satırın solundaki görsel: kapak, yuvarlak portre, listenin kapağı ya da
    /// ton karosunda simge (komut, son arama).
    fn palette_thumb(&self, thumb: &Thumb, name: &str) -> Option<AnyElement> {
        let quiet = |icon_name: &'static str| {
            div()
                .flex_none()
                .size(px(THUMB))
                .rounded(px(R_MD))
                .flex()
                .items_center()
                .justify_center()
                .bg(veil(VEIL_HOVER))
                .child(icon(icon_name, 16., muted()))
                .into_any_element()
        };
        match thumb {
            Thumb::Cover {
                url,
                icon: icon_name,
            } => Some(cover_image(&self.images, url, name, THUMB, R_SM, icon_name)),
            Thumb::Artist { url } => Some(cover_image(
                &self.images,
                url,
                name,
                THUMB,
                THUMB / 2.,
                "user",
            )),
            Thumb::Collection { id, url } => Some(self.collection_art(id, name, url, THUMB, R_SM)),
            Thumb::Icon(icon_name) => Some(quiet(icon_name)),
            Thumb::Glyph(icon_name) => Some(
                div()
                    .flex_none()
                    .size(px(THUMB))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(icon_name, 16., label()))
                    .into_any_element(),
            ),
        }
    }
}

/// Bölüm adı yerine ince çizgi çizilen bölüm: tek satırlık "Daha fazlası"
/// (tam sonuç sayfası). Diğer bölümler adlarını korur.
fn is_divider_header(header: &str) -> bool {
    header == t!(PaletteMore)
}

/// Bütün bölümleri tek ölçüyle değerlendirir: komutlar yalnız en iyi
/// eşleşmeye yakınsa ya da adı olduğu gibi yazılmışsa (adın içinde geçiyorsa)
/// kalır; ikinci değer yazılanla en iyi eşleşen (kullanılabilir) öğenin
/// sırası. Adı yazılan komut (en az 3 harf, adın başı) seçili gelir: "sessize"
/// yazınca Enter sessize alır; şarkı adına benzeyen yazıda komut seçilmez.
/// Boş sorguda dokunmaz.
fn rank_items(query: &str, items: Vec<Item>) -> (Vec<Item>, Option<usize>) {
    let Some(mut f) = fuzzy::Fuzzy::new(query) else {
        return (items, None);
    };
    let typed = fuzzy::fold(query.trim());
    let scores: Vec<u32> = items
        .iter()
        .map(|i| {
            f.score(&fuzzy::fold(&format!("{} {}", i.title, i.subtitle)))
                .unwrap_or(0)
        })
        .collect();
    let best = scores.iter().copied().max().unwrap_or(0);
    let cutoff = (best as f32 * COMMAND_RELATIVE_SCORE).ceil() as u32;
    let (kept, scores): (Vec<Item>, Vec<u32>) = items
        .into_iter()
        .zip(scores)
        .filter(|(i, s)| {
            !matches!(i.action, Action::Command(_))
                || (typed.chars().count() >= 2 && i.name_key.contains(&typed))
                || (*s > 0 && *s >= cutoff)
        })
        .unzip();
    let named = (typed.chars().count() >= 3)
        .then(|| {
            kept.iter().position(|i| {
                matches!(i.action, Action::Command(_))
                    && !i.disabled
                    && i.name_key.starts_with(&typed)
            })
        })
        .flatten();
    let selected = named.or_else(|| {
        scores
            .iter()
            .enumerate()
            .filter(|(i, _)| !kept[*i].disabled)
            // En yüksek puan; eşitlikte listede önce gelen.
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))
            .map(|(i, _)| i)
    });
    (kept, selected)
}

/// Yazı kutusunun imleci.
fn caret(accent: u32) -> gpui::Div {
    div()
        .w(px(2.))
        .h(px(T_TITLE.size + S1))
        .flex_shrink_0()
        .bg(rgb(accent))
}

fn command_item(
    cmd: PaletteCommand,
    title: &str,
    detail: &str,
    shortcut: Option<&'static str>,
) -> Item {
    let danger = matches!(cmd, PaletteCommand::Logout);
    Item {
        thumb: Thumb::Icon(command_icon(cmd)),
        name_key: fuzzy::fold(title),
        // Geri dönüşü zor işlem kendi bölümünde, en altta.
        section: if danger {
            t!(PaletteSectionAccount)
        } else {
            t!(PaletteSectionCommands)
        },
        title: title.into(),
        subtitle: detail.into(),
        detail: String::new(),
        action: Action::Command(cmd),
        danger,
        shortcut,
        disabled: false,
    }
}

/// Komutun ton karosundaki simgesi.
fn command_icon(cmd: PaletteCommand) -> &'static str {
    match cmd {
        PaletteCommand::TogglePlay => "play",
        PaletteCommand::Next => "skip-forward",
        PaletteCommand::Previous => "skip-back",
        PaletteCommand::ToggleMute => "volume-x",
        PaletteCommand::RefreshLibrary => "refresh-cw",
        PaletteCommand::Shortcuts => "list",
        PaletteCommand::Logout => "x",
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

fn track_item(section: &'static str, t: &TrackRow) -> Item {
    Item {
        thumb: Thumb::Cover {
            url: t.image.clone(),
            icon: "music",
        },
        name_key: String::new(),
        section,
        title: t.name.clone(),
        subtitle: if t.album.is_empty() {
            t.artists.clone()
        } else {
            format!("{} · {}", t.artists, t.album)
        },
        detail: fmt_time(t.duration_ms),
        action: Action::PlayTrack(t.uri.clone()),
        danger: false,
        shortcut: None,
        disabled: false,
    }
}

fn spotify_status(s: &Spotify) -> SharedString {
    match s {
        Spotify::Idle | Spotify::Done(_) => "".into(),
        Spotify::Waiting | Spotify::Loading => t!(SearchSearching).into(),
        // Hata metni kullanıcıya gösterilmez (günlükte var); ne olduğu ve
        // ne yapılabileceği söylenir.
        Spotify::Failed(e) => match super::components::explain_error(e).1 {
            title if title == t!(ErrOfflineTitle) => t!(PaletteSearchOffline).into(),
            title if title == t!(ErrBusyTitle) => t!(PaletteSearchBusy).into(),
            _ => t!(PaletteSearchFailed).into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_single_row_more_section_becomes_a_divider() {
        assert!(is_divider_header(t!(PaletteMore)));
        assert!(!is_divider_header("Kütüphanendeki şarkılar"));
        assert!(!is_divider_header(""));
    }

    fn track(name: &str, artists: &str) -> Item {
        track_item(
            "Kütüphanendeki şarkılar",
            &TrackRow {
                uri: format!("spotify:track:{name}"),
                name: name.into(),
                artists: artists.into(),
                album: String::new(),
                duration_ms: 1000,
                image: String::new(),
            },
        )
    }

    #[test]
    fn typed_song_beats_a_loosely_matching_command() {
        // "dal" harfleri "Çal / Duraklat"a dağılıyor; Enter şarkıyı çalmalı.
        let items = vec![
            command_item(
                PaletteCommand::TogglePlay,
                "Çal / Duraklat",
                "",
                Some("Boşluk"),
            ),
            track("Dal", "Adamlar"),
            track("Dalgalar", "Başka Biri"),
        ];
        let (items, selected) = rank_items("dal", items);
        assert!(
            items
                .iter()
                .all(|i| !matches!(i.action, Action::Command(_)))
        );
        assert_eq!(items[selected.unwrap()].title, "Dal");
    }

    #[test]
    fn a_command_typed_on_purpose_stays_first() {
        let items = vec![
            command_item(PaletteCommand::Next, "Sonraki şarkı", "", None),
            track("Sonbahar", "Biri"),
        ];
        let (items, selected) = rank_items("sonraki", items);
        assert_eq!(items[selected.unwrap()].title, "Sonraki şarkı");
    }

    #[test]
    fn a_typed_command_name_is_listed_and_selected() {
        // "sessize" şarkı adına (Sessizlik) de benzer; ama komutun adı olduğu
        // gibi yazılmış: listede kalır ve seçili gelir.
        let items = vec![
            command_item(PaletteCommand::ToggleMute, "Sessize al", "", None),
            track("Sessizlik", "Ceza"),
        ];
        let (items, selected) = rank_items("sessize", items);
        assert!(
            items
                .iter()
                .any(|i| matches!(i.action, Action::Command(PaletteCommand::ToggleMute)))
        );
        assert_eq!(items[selected.unwrap()].title, "Sessize al");
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
}
