//! Kütüphane araçları sayfası (`Route::Tools`): durum, eylemler, klavye.
//! Çizim `tools_view.rs`'te.
//!
//! Tek sayfada beş araç (üstte seçenek düğmeleri): Yinelenenler, Birleştir /
//! böl, Sıralama, Yedek, İstatistik. Kütüphane araç çubuğundan ve komut
//! paletinden açılır.
//!
//! Spotify'a yazan her şey MEVCUT edit kuyruğundan geçer (`manage::commit`):
//! şarkı çıkarma (`RemoveFromPlaylist`), liste oluşturma (`CreatePlaylist`),
//! öğe taşıma (`MovePlaylistItems`); hepsi Ctrl+Z ile geri alınır. Yeni bir
//! canlı yazma türü yok. Kaynak listeler (birleştir / böl) hiç silinmez ya da
//! değişmez. `BULK_CONFIRM` (50) ve üstü değişiklik önce uyarı ve onay ister.
//!
//! Hangi satırların yinelenen olduğu, nasıl taşınacağı, nasıl bölüneceği saf
//! hesaptır (`engine::tools`); burası seçimi, önizlemeyi, onayı ve klavyeyi
//! yönetir. Yedek ve istatistik yereldir (motor `tools/handlers.rs`).

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Context, KeyDownEvent, Task, Window};

use super::{
    Shell,
    history::Route,
    library::Loadable,
    manage::{Edit, inverse_all, is_own},
};
use crate::engine::{
    self, CollectionId, Command, EditOp, Event, LibraryCommand, TrackRow, new_temp_id,
    tools::{
        BULK_CONFIRM,
        backup::Summary,
        civil,
        dupes::{self, DedupeMode, DupGroup, DupKind},
        plan::{self, Dir, Move, SortBy, SplitBy},
        stats::{PlayCounter, PlayLog, StatsRange, StatsReport, TrackMeta},
    },
};
use crate::{t, tf, tp};

/// Seçim kutusunun (ve sonuç tablosunun) bir seferde gösterdiği satır sayısı.
pub const VISIBLE_ROWS: usize = 7;
/// Yinelenen sonuç tablosunun bir seferde gösterdiği satır sayısı.
pub const RESULT_ROWS: usize = 10;
/// Bir sıralamada Openify'ın tek seferde yazdığı en çok taşıma: her taşıma
/// Spotify'a ayrı bir yazma; bundan uzun bir sıralama Spotify'ı boğardı.
pub const MAX_SORT_MOVES: usize = 500;

// ── Araçlar ve kutular ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Duplicates,
    MergeSplit,
    Sort,
    Backup,
    Stats,
}

impl Tool {
    pub const ALL: [Tool; 5] = [Tool::Duplicates, Tool::MergeSplit, Tool::Sort, Tool::Backup, Tool::Stats];

    pub fn key(self) -> &'static str {
        match self {
            Tool::Duplicates => "dupes",
            Tool::MergeSplit => "merge",
            Tool::Sort => "sort",
            Tool::Backup => "backup",
            Tool::Stats => "stats",
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        Self::ALL.into_iter().find(|t| t.key() == key || (key == "split" && *t == Tool::MergeSplit))
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Duplicates => t!(ToolsDupes),
            Tool::MergeSplit => t!(ToolsMergeSplit),
            Tool::Sort => t!(ToolsSort),
            Tool::Backup => t!(ToolsBackup),
            Tool::Stats => t!(ToolsStats),
        }
    }
}

/// Klavye odağı alabilen kaydırmalı kutular.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum BoxId {
    DupPick,
    DupRows,
    MergePick,
    SortPick,
}

impl BoxId {
    pub fn key(self) -> &'static str {
        match self {
            BoxId::DupPick => "tools-box:dup-pick",
            BoxId::DupRows => "tools-box:dup-rows",
            BoxId::MergePick => "tools-box:merge-pick",
            BoxId::SortPick => "tools-box:sort-pick",
        }
    }

    pub fn from_key(key: &str) -> Option<BoxId> {
        [BoxId::DupPick, BoxId::DupRows, BoxId::MergePick, BoxId::SortPick].into_iter().find(|b| b.key() == key)
    }

    /// Kutuda aynı anda görünen satır.
    pub fn visible(self) -> usize {
        match self {
            BoxId::DupRows => RESULT_ROWS,
            _ => VISIBLE_ROWS,
        }
    }
}

/// Kaydırmalı kutunun imleci ve ilk görünen satırı (kendi kaydırmamız: klavye
/// imleci hep görünür kalır).
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListBox {
    pub cursor: usize,
    pub offset: usize,
}

impl ListBox {
    fn fit(&mut self, len: usize, visible: usize) {
        if len == 0 {
            *self = Self::default();
            return;
        }
        self.cursor = self.cursor.min(len - 1);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + visible {
            self.offset = self.cursor + 1 - visible;
        }
        self.offset = self.offset.min(len.saturating_sub(visible));
    }

    pub fn move_by(&mut self, delta: isize, len: usize, visible: usize) {
        self.cursor = (self.cursor as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        self.fit(len, visible);
    }

    pub fn to(&mut self, index: usize, len: usize, visible: usize) {
        self.cursor = index;
        self.fit(len, visible);
    }

    /// Tekerlek: görünür pencereyi kaydırır, imleç pencerede kalır.
    pub fn scroll(&mut self, lines: isize, len: usize, visible: usize) {
        let max = len.saturating_sub(visible) as isize;
        self.offset = (self.offset as isize + lines).clamp(0, max) as usize;
        self.cursor = self.cursor.clamp(self.offset, (self.offset + visible).saturating_sub(1).min(len.saturating_sub(1)));
    }
}

// ── Durum ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bulk {
    Dupes,
    Merge,
    Split,
    Sort,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MergeMode {
    Merge,
    Split,
}

/// Yinelenen sonuç tablosunun satırı.
#[derive(Clone, Debug, PartialEq)]
pub enum DupRow {
    Header { kind: DupKind, removable: usize },
    Item { group: usize, item: usize },
}

/// Bir listenin yinelenen sonuçları (liste değişmedikçe yeniden hesaplanmaz).
pub struct DupView {
    pub id: CollectionId,
    key: usize,
    pub groups: Vec<DupGroup>,
    pub rows: Vec<DupRow>,
}

pub struct DupState {
    pub list: Option<CollectionId>,
    pub pick: ListBox,
    pub rows: ListBox,
    /// Kullanıcının işaretini kaldırdığı (kaldırılmayacak) satır sıraları.
    pub spared: HashSet<usize>,
}

pub struct MergeState {
    pub mode: MergeMode,
    pub pick: ListBox,
    pub picked: Vec<CollectionId>,
    pub split_from: Option<CollectionId>,
    pub dedupe: DedupeMode,
    pub split_by: SplitBy,
    /// Önizleme ve onay adımı.
    pub review: bool,
    pub name: String,
}

pub struct SortState {
    pub list: Option<CollectionId>,
    pub pick: ListBox,
    pub by: SortBy,
    pub dir: Dir,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupStatus {
    Idle,
    Choosing,
    Writing,
    Done(Summary),
    Failed,
}

pub struct StatsState {
    pub range: StatsRange,
    pub report: Option<Arc<StatsReport>>,
    /// Listeler 10 satır mı (5 yerine).
    pub more: bool,
}

/// Sıralama önizlemesi: istenen sıra ve gereken taşımalar (liste ya da ölçüt
/// değişmedikçe yeniden hesaplanmaz; her karede kurmak büyük listede takılırdı).
pub struct SortView {
    pub tracks: Arc<Vec<TrackRow>>,
    /// Hedef sıra: sıralanmış satırların şimdiki indisleri.
    pub order: Vec<usize>,
    /// Gereken taşıma sayısı (n eksi en uzun artan altdizi).
    pub count: usize,
    /// Taşımalar; yalnız `count` en çok `MAX_SORT_MOVES` ise kurulur, değilse boş.
    pub moves: Vec<Move>,
}

/// Çalma sayacı ve eşik zamanlayıcısı (dinleme istatistiği için).
pub struct Recorder {
    counter: PlayCounter,
    base: Instant,
    timer: Option<Task<()>>,
}

pub struct ToolsUi {
    pub tool: Tool,
    /// Fareyle son kullanılan kutu: odak çapadayken ↑/↓ ona gider.
    pub active: Option<BoxId>,
    pub dup: DupState,
    pub merge: MergeState,
    pub sort: SortState,
    pub backup: BackupStatus,
    pub stats: StatsState,
    pub confirm: Option<Bulk>,
    pub dup_view: RefCell<Option<Rc<DupView>>>,
    /// Önizleme hesapları: (anahtar, sonuç); anahtar kaynak listelerin içeriği ve
    /// seçenekler değişince değişir.
    merge_memo: RefCell<Option<(u64, Rc<plan::MergePlan>)>>,
    split_memo: RefCell<Option<(u64, Rc<Vec<plan::SplitGroup>>)>>,
    sort_memo: RefCell<Option<(u64, Rc<SortView>)>>,
    pub recorder: Recorder,
    /// Ekran sınaması betiği (`OPENIFY_TEST_TOOLS`), sırası gelmemiş adımlar.
    script: Vec<String>,
}

impl ToolsUi {
    pub fn new() -> Self {
        // Sınama kancası: yalnız sınama koşusunda (`OPENIFY_TEST_DISPLAY`).
        let script = std::env::var("OPENIFY_TEST_TOOLS")
            .ok()
            .filter(|_| std::env::var_os("OPENIFY_TEST_DISPLAY").is_some())
            .map(|v| v.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).rev().collect())
            .unwrap_or_default();
        Self {
            tool: Tool::Duplicates,
            active: None,
            dup: DupState { list: None, pick: ListBox::default(), rows: ListBox::default(), spared: HashSet::new() },
            merge: MergeState {
                mode: MergeMode::Merge,
                pick: ListBox::default(),
                picked: Vec::new(),
                split_from: None,
                dedupe: DedupeMode::SameTrack,
                split_by: SplitBy::Artist,
                review: false,
                name: String::new(),
            },
            sort: SortState { list: None, pick: ListBox::default(), by: SortBy::Name, dir: Dir::Asc },
            backup: BackupStatus::Idle,
            stats: StatsState { range: StatsRange::Weeks4, report: None, more: false },
            confirm: None,
            dup_view: RefCell::new(None),
            merge_memo: RefCell::new(None),
            split_memo: RefCell::new(None),
            sort_memo: RefCell::new(None),
            recorder: Recorder { counter: PlayCounter::new(), base: Instant::now(), timer: None },
            script,
        }
    }
}

/// Seçim kutusundaki bir satır.
#[derive(Clone, Debug)]
pub struct PickItem {
    pub id: CollectionId,
    pub name: String,
    pub count: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Yalnız kendi listelerin (yazılabilir).
    Own,
    /// Beğenilenler ve bütün çalma listeleri (okunabilir).
    Readable,
}

/// Kaynak listelerin durumu.
pub enum Sources {
    Ready(Vec<Arc<Vec<TrackRow>>>),
    Loading,
    Failed,
}

fn hash_of(parts: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// "A + B" (üçten fazlada "A + B + 2 liste daha" değil, ilk ikisi ve sayı).
fn default_merge_name(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} + {b}"),
        [a, b, rest @ ..] => format!("{a} + {b} + {}", rest.len()),
    }
}

/// Toplam dinleme süresi: "12 sa 34 dk" / "12 h 34 min"; bir saatten kısaysa dakika.
pub fn listened_text(ms: u64) -> String {
    let minutes = ms / 60_000;
    let (h, m) = (minutes / 60, minutes % 60);
    if h == 0 { tf!(ToolsMinutes, m) } else { tf!(ToolsHoursMinutes, h, m) }
}

impl Shell {
    // ── Açılış ──────────────────────────────────────────────────────────

    /// Sayfayı aç (Kütüphane araç çubuğu, komut paleti).
    pub(super) fn open_tools(&mut self, cx: &mut Context<Self>) {
        self.navigate(Route::Tools, cx);
    }

    /// Sayfaya girerken: istatistik için rapor istenir; sınama betiği çalışır.
    pub(super) fn enter_tools(&mut self, cx: &mut Context<Self>) {
        self.tools.confirm = None;
        self.tools.backup = match self.tools.backup {
            BackupStatus::Choosing | BackupStatus::Writing => self.tools.backup.clone(),
            _ => BackupStatus::Idle,
        };
        self.tools_want_stats();
        self.tools_run_script(cx);
        cx.notify();
    }

    fn tools_on(&self) -> bool {
        matches!(self.pages.history.current(), Route::Tools)
    }

    // ── Kaynak listeler ─────────────────────────────────────────────────

    /// Seçim kutusunun satırları.
    pub(super) fn tools_items(&self, source: Source) -> Vec<PickItem> {
        let Loadable::Loaded(lib) = &self.library.library else { return Vec::new() };
        let mut out = Vec::new();
        if source == Source::Readable {
            out.push(PickItem { id: CollectionId::Liked, name: t!(LikedSongs).to_string(), count: Some(lib.liked_count) });
        }
        for p in &lib.playlists {
            if source == Source::Own && !is_own(p) {
                continue;
            }
            out.push(PickItem {
                id: p.id.clone(),
                name: p.name.clone(),
                count: crate::i18n::canon::song_count(&p.subtitle),
            });
        }
        out
    }

    /// Listenin şarkıları yüklü değilse iste (önbellekten gelir; yoksa Spotify'dan).
    fn tools_ensure_loaded(&mut self, id: &CollectionId, cx: &mut Context<Self>) {
        if matches!(self.library.collections.get(id), Some(Loadable::Loaded(_) | Loadable::Loading)) {
            return;
        }
        self.library.collections.insert(id.clone(), Loadable::Loading);
        self.engine.send(Command::OpenCollection(id.clone()));
        self.request_added_dates(id);
        cx.notify();
    }

    /// Eklenme anları (sıralama ve yıla göre bölme için) istenmemişse iste.
    fn tools_ensure_dates(&self, id: &CollectionId) {
        if !self.library.added.contains_key(id) {
            self.request_added_dates(id);
        }
    }

    pub(super) fn tools_sources(&self, ids: &[CollectionId]) -> Sources {
        let mut lists = Vec::new();
        for id in ids {
            match self.library.collections.get(id) {
                Some(Loadable::Loaded(t)) => lists.push(t.clone()),
                Some(Loadable::Failed(_)) => return Sources::Failed,
                _ => return Sources::Loading,
            }
        }
        Sources::Ready(lists)
    }

    pub(super) fn tools_title(&self, id: &CollectionId) -> String {
        self.library.title(id)
    }

    // ── Araç seçimi ─────────────────────────────────────────────────────

    pub(super) fn tools_select_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.tools.tool = tool;
        self.tools.confirm = None;
        self.tools.active = None;
        self.tools_want_stats();
        cx.notify();
    }

    // ── Yinelenenler ────────────────────────────────────────────────────

    pub(super) fn tools_dup_pick(&mut self, index: usize, cx: &mut Context<Self>) {
        let items = self.tools_items(Source::Own);
        let Some(item) = items.get(index) else { return };
        let len = items.len();
        self.tools.dup.pick.to(index, len, VISIBLE_ROWS);
        self.tools.dup.list = Some(item.id.clone());
        self.tools.dup.rows = ListBox::default();
        self.tools.dup.spared.clear();
        self.tools.confirm = None;
        self.tools_ensure_loaded(&item.id.clone(), cx);
        cx.notify();
    }

    /// Seçili listenin yinelenenleri (önbellekli; liste değişince yeniden).
    pub(super) fn tools_dup_view(&self) -> Option<Rc<DupView>> {
        let id = self.tools.dup.list.clone()?;
        let tracks = self.library.tracks(&id)?;
        let key = Arc::as_ptr(tracks) as usize;
        if let Some(view) = self.tools.dup_view.borrow().as_ref()
            && view.id == id
            && view.key == key
        {
            return Some(view.clone());
        }
        let groups = dupes::find(tracks);
        let mut rows = Vec::new();
        for (g, group) in groups.iter().enumerate() {
            rows.push(DupRow::Header { kind: group.kind, removable: group.removable() });
            rows.extend((0..group.items.len()).map(|item| DupRow::Item { group: g, item }));
        }
        let view = Rc::new(DupView { id, key, groups, rows });
        *self.tools.dup_view.borrow_mut() = Some(view.clone());
        Some(view)
    }

    /// Kaldırılacak satırlar (işareti kaldırılmayan kalmayanlar).
    pub(super) fn tools_dup_removals(&self) -> Vec<(usize, TrackRow)> {
        self.tools_dup_view()
            .map(|v| dupes::removals(&v.groups, &self.tools.dup.spared))
            .unwrap_or_default()
    }

    /// Yinelenen satırın "kaldır" işaretini çevir (kalan satırda işe yaramaz).
    pub(super) fn tools_dup_toggle(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(view) = self.tools_dup_view() else { return };
        let Some(DupRow::Item { group, item }) = view.rows.get(row) else { return };
        let it = &view.groups[*group].items[*item];
        if it.keep {
            return;
        }
        if !self.tools.dup.spared.remove(&it.index) {
            self.tools.dup.spared.insert(it.index);
        }
        cx.notify();
    }

    pub(super) fn tools_dup_toggle_group(&mut self, group: usize, cx: &mut Context<Self>) {
        let Some(view) = self.tools_dup_view() else { return };
        let Some(g) = view.groups.get(group) else { return };
        let all_removed = g.items.iter().filter(|i| !i.keep).all(|i| !self.tools.dup.spared.contains(&i.index));
        for i in g.items.iter().filter(|i| !i.keep) {
            if all_removed {
                self.tools.dup.spared.insert(i.index);
            } else {
                self.tools.dup.spared.remove(&i.index);
            }
        }
        cx.notify();
    }

    fn tools_apply_dupes(&mut self, cx: &mut Context<Self>) {
        let Some(CollectionId::Playlist(playlist)) = self.tools.dup.list.clone() else { return };
        let rows = self.tools_dup_removals();
        if rows.is_empty() {
            return;
        }
        self.tools.dup.spared.clear();
        self.tools.dup.rows = ListBox::default();
        self.remove_rows(playlist, rows, cx);
    }

    // ── Birleştir / böl ─────────────────────────────────────────────────

    pub(super) fn tools_set_mode(&mut self, mode: MergeMode, cx: &mut Context<Self>) {
        self.tools.merge.mode = mode;
        self.tools.merge.review = false;
        self.tools.confirm = None;
        cx.notify();
    }

    pub(super) fn tools_merge_pick(&mut self, index: usize, cx: &mut Context<Self>) {
        let items = self.tools_items(Source::Readable);
        let Some(item) = items.get(index).cloned() else { return };
        self.tools.merge.pick.to(index, items.len(), VISIBLE_ROWS);
        self.tools.merge.review = false;
        self.tools.confirm = None;
        match self.tools.merge.mode {
            MergeMode::Merge => {
                let picked = &mut self.tools.merge.picked;
                match picked.iter().position(|p| *p == item.id) {
                    Some(at) => {
                        picked.remove(at);
                    }
                    None => picked.push(item.id.clone()),
                }
            }
            MergeMode::Split => self.tools.merge.split_from = Some(item.id.clone()),
        }
        self.tools_ensure_loaded(&item.id, cx);
        cx.notify();
    }

    pub(super) fn tools_merge_selection(&self) -> Vec<CollectionId> {
        match self.tools.merge.mode {
            MergeMode::Merge => self.tools.merge.picked.clone(),
            MergeMode::Split => self.tools.merge.split_from.iter().cloned().collect(),
        }
    }

    pub(super) fn tools_merge_ready(&self) -> bool {
        let n = self.tools_merge_selection().len();
        match self.tools.merge.mode {
            MergeMode::Merge => n >= 2,
            MergeMode::Split => n == 1,
        }
    }

    /// "Önizle": ön koşullar tamamsa onay adımına geç, varsayılan adı koy.
    pub(super) fn tools_review(&mut self, cx: &mut Context<Self>) {
        if !self.tools_merge_ready() {
            return;
        }
        let ids = self.tools_merge_selection();
        if !matches!(self.tools_sources(&ids), Sources::Ready(_)) {
            return;
        }
        if self.tools.merge.split_by == SplitBy::AddedYear {
            for id in &ids {
                self.tools_ensure_dates(id);
            }
        }
        let names: Vec<String> = ids.iter().map(|id| self.tools_title(id)).collect();
        self.tools.merge.name = match self.tools.merge.mode {
            MergeMode::Merge => default_merge_name(&names),
            MergeMode::Split => names.first().cloned().unwrap_or_default(),
        };
        self.tools.merge.review = true;
        self.tools.confirm = None;
        self.tools.active = None;
        cx.notify();
    }

    pub(super) fn tools_back_from_review(&mut self, cx: &mut Context<Self>) {
        self.tools.merge.review = false;
        self.tools.confirm = None;
        cx.notify();
    }

    /// Birleştirmenin sonucu (önizleme ve yazma aynı hesap).
    pub(super) fn tools_merge_plan(&self) -> Option<Rc<plan::MergePlan>> {
        let Sources::Ready(lists) = self.tools_sources(&self.tools.merge.picked) else { return None };
        let key = hash_of((lists.iter().map(|l| Arc::as_ptr(l) as usize).collect::<Vec<_>>(), self.tools.merge.dedupe));
        if let Some((k, plan)) = self.tools.merge_memo.borrow().as_ref()
            && *k == key
        {
            return Some(plan.clone());
        }
        let slices: Vec<&[TrackRow]> = lists.iter().map(|l| l.as_slice()).collect();
        let plan = Rc::new(plan::merge(&slices, self.tools.merge.dedupe));
        *self.tools.merge_memo.borrow_mut() = Some((key, plan.clone()));
        Some(plan)
    }

    /// Bölmenin grupları (önbellekli).
    pub(super) fn tools_split_groups(&self) -> Option<Rc<Vec<plan::SplitGroup>>> {
        let id = self.tools.merge.split_from.clone()?;
        let tracks = self.library.tracks(&id)?;
        let dates = self.library.added.get(&id);
        let key = hash_of((
            Arc::as_ptr(tracks) as usize,
            dates.map_or(0, |d| Arc::as_ptr(d) as usize),
            self.tools.merge.split_by,
        ));
        if let Some((k, groups)) = self.tools.split_memo.borrow().as_ref()
            && *k == key
        {
            return Some(groups.clone());
        }
        let empty = HashMap::new();
        let added = dates.map(|a| a.as_ref()).unwrap_or(&empty);
        let groups = Rc::new(plan::split(tracks, self.tools.merge.split_by, added, civil::local_offset_minutes()));
        *self.tools.split_memo.borrow_mut() = Some((key, groups.clone()));
        Some(groups)
    }

    /// Bölünen listenin adı: ön ad + grup ("Gece sürüşü · Kuzey"); "diğer" grup.
    pub(super) fn tools_split_name(&self, group: &plan::SplitGroup) -> String {
        let label = if group.other { t!(ToolsSplitOther).to_string() } else { group.label.clone() };
        format!("{} · {}", self.tools.merge.name.trim(), label)
    }

    /// Yazılacak değişiklik sayısı (onay eşiği için): açılan listeler + yazılan şarkılar.
    fn tools_change_count(&self, bulk: &Bulk) -> usize {
        match bulk {
            Bulk::Dupes => self.tools_dup_removals().len(),
            Bulk::Merge => self.tools_merge_plan().map_or(0, |p| p.rows.len() + 1),
            Bulk::Split => self
                .tools_split_groups()
                .map_or(0, |g| g.iter().map(|g| g.rows.len() + 1).sum()),
            Bulk::Sort => self.tools_sort_view().map_or(0, |v| if v.count > MAX_SORT_MOVES { 0 } else { v.count }),
        }
    }

    /// Eylem düğmesi: eşiği aşıyorsa önce onay, değilse hemen uygula.
    pub(super) fn tools_request(&mut self, bulk: Bulk, cx: &mut Context<Self>) {
        let count = self.tools_change_count(&bulk);
        if count == 0 {
            return;
        }
        if count >= BULK_CONFIRM && self.tools.confirm.as_ref() != Some(&bulk) {
            self.tools.confirm = Some(bulk);
            self.tools.active = None;
            cx.notify();
            return;
        }
        self.tools.confirm = None;
        match bulk {
            Bulk::Dupes => self.tools_apply_dupes(cx),
            Bulk::Merge => self.tools_apply_merge(cx),
            Bulk::Split => self.tools_apply_split(cx),
            Bulk::Sort => self.tools_apply_sort(cx),
        }
        cx.notify();
    }

    pub(super) fn tools_cancel_confirm(&mut self, cx: &mut Context<Self>) {
        self.tools.confirm = None;
        cx.notify();
    }

    fn tools_apply_merge(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.tools_merge_plan() else { return };
        let rows = plan.rows.clone();
        let name = self.tools.merge.name.trim().to_string();
        if name.is_empty() || plan.rows.is_empty() {
            return;
        }
        let temp_id = new_temp_id();
        let n = plan.rows.len();
        let uris: Vec<String> = plan.rows.iter().map(|r| r.uri.clone()).collect();
        let op = EditOp::CreatePlaylist { temp_id: temp_id.clone(), name: name.clone(), uris, created: None };
        let undo = vec![(EditOp::DeletePlaylist { playlist: temp_id.clone() }, Vec::new(), None)];
        let label = match n {
            1 => tf!(ToastCreatedWith1, name),
            n => tf!(ToastCreatedWithN, name, n),
        };
        self.library.titles.insert(CollectionId::Playlist(temp_id), name);
        self.commit(vec![(op, rows, None)], Some((label, undo)), cx);
        self.tools.merge.review = false;
        self.tools.merge.picked.clear();
    }

    fn tools_apply_split(&mut self, cx: &mut Context<Self>) {
        let Some(groups) = self.tools_split_groups() else { return };
        if groups.is_empty() || self.tools.merge.name.trim().is_empty() {
            return;
        }
        let mut ops: Vec<Edit> = Vec::new();
        let mut undo: Vec<Edit> = Vec::new();
        // Büyükten küçüğe önizlendi; yeni liste kütüphanenin başına girdiği için ters sırayla
        // açılır (en büyük grup en üstte kalır).
        for g in groups.iter().rev() {
            let temp_id = new_temp_id();
            let name = self.tools_split_name(g);
            let uris: Vec<String> = g.rows.iter().map(|r| r.uri.clone()).collect();
            self.library.titles.insert(CollectionId::Playlist(temp_id.clone()), name.clone());
            ops.push((
                EditOp::CreatePlaylist { temp_id: temp_id.clone(), name, uris, created: None },
                g.rows.clone(),
                None,
            ));
            undo.push((EditOp::DeletePlaylist { playlist: temp_id }, Vec::new(), None));
        }
        undo.reverse();
        let label = tp!(ToolsSplitDone, ops.len());
        self.commit(ops, Some((label, undo)), cx);
        self.tools.merge.review = false;
        self.tools.merge.split_from = None;
    }

    // ── Sıralama ────────────────────────────────────────────────────────

    pub(super) fn tools_sort_pick(&mut self, index: usize, cx: &mut Context<Self>) {
        let items = self.tools_items(Source::Own);
        let Some(item) = items.get(index).cloned() else { return };
        self.tools.sort.pick.to(index, items.len(), VISIBLE_ROWS);
        self.tools.sort.list = Some(item.id.clone());
        self.tools.confirm = None;
        self.tools_ensure_loaded(&item.id, cx);
        self.tools_ensure_dates(&item.id);
        cx.notify();
    }

    pub(super) fn tools_set_sort(&mut self, by: SortBy, cx: &mut Context<Self>) {
        self.tools.sort.by = by;
        self.tools.confirm = None;
        if let Some(id) = self.tools.sort.list.clone()
            && by == SortBy::Added
        {
            self.tools_ensure_dates(&id);
        }
        cx.notify();
    }

    pub(super) fn tools_set_dir(&mut self, dir: Dir, cx: &mut Context<Self>) {
        self.tools.sort.dir = dir;
        self.tools.confirm = None;
        cx.notify();
    }

    /// Seçili listenin sıralama önizlemesi (önbellekli): istenen sıra, taşıma sayısı,
    /// taşımalar. Liste, ölçüt, yön ya da eklenme anları değişince yeniden kurulur.
    pub(super) fn tools_sort_view(&self) -> Option<Rc<SortView>> {
        let id = self.tools.sort.list.clone()?;
        let tracks = self.library.tracks(&id)?.clone();
        let dates = self.library.added.get(&id);
        let key = hash_of((
            Arc::as_ptr(&tracks) as usize,
            dates.map_or(0, |d| Arc::as_ptr(d) as usize),
            self.tools.sort.by,
            self.tools.sort.dir,
        ));
        if let Some((k, view)) = self.tools.sort_memo.borrow().as_ref()
            && *k == key
        {
            return Some(view.clone());
        }
        let empty = HashMap::new();
        let added = dates.map(|a| a.as_ref()).unwrap_or(&empty);
        let order = plan::target_order(&tracks, self.tools.sort.by, self.tools.sort.dir, added);
        let count = plan::count_moves(&order);
        let moves = if count > 0 && count <= MAX_SORT_MOVES { plan::plan_moves(&order) } else { Vec::new() };
        let view = Rc::new(SortView { tracks, order, count, moves });
        *self.tools.sort_memo.borrow_mut() = Some((key, view.clone()));
        Some(view)
    }

    fn tools_apply_sort(&mut self, cx: &mut Context<Self>) {
        let Some(CollectionId::Playlist(playlist)) = self.tools.sort.list.clone() else { return };
        let Some(view) = self.tools_sort_view() else { return };
        let (tracks, moves) = (&view.tracks, &view.moves);
        if moves.is_empty() {
            return;
        }
        // Taşınan şarkının adresi, o anki sıraya göre (her taşıma öncekinin sonucuna).
        let mut uris: Vec<String> = tracks.iter().map(|t| t.uri.clone()).collect();
        let mut ops: Vec<Edit> = Vec::with_capacity(moves.len());
        for m in moves.iter() {
            let uri = uris[m.from].clone();
            plan::simulate_move(&mut uris, m.from, m.to);
            ops.push((
                EditOp::MovePlaylistItems {
                    playlist: playlist.clone(),
                    from_index: m.from as u32,
                    length: 1,
                    to_index: m.to as u32,
                    uris: vec![uri],
                },
                Vec::new(),
                None,
            ));
        }
        let label = tp!(ToolsSortDone, moves.len());
        let undo = inverse_all(&ops);
        self.commit(ops, Some((label, undo)), cx);
    }

    // ── Yedek ───────────────────────────────────────────────────────────

    /// "Yedeği kaydet": Windows kaydet iletişim kutusu (ayrı iş parçacığında);
    /// sınama koşusunda iletişim kutusu AÇILMAZ, geçici klasöre yazılır.
    pub(super) fn tools_backup(&mut self, cx: &mut Context<Self>) {
        if matches!(self.tools.backup, BackupStatus::Choosing | BackupStatus::Writing) {
            return;
        }
        let now = now_ms();
        let offset = civil::local_offset_minutes();
        let name = engine::tools::backup::suggested_name(now, offset);
        if let Some(path) = self.tools_backup_test_path(&name) {
            self.tools_write_backup(path, cx);
            return;
        }
        self.tools.backup = BackupStatus::Choosing;
        let (tx, rx) = async_channel::bounded::<Option<std::path::PathBuf>>(1);
        let title = t!(ToolsBackupDialogTitle).to_string();
        let fallback = engine::tools::backup::fallback_path(now, offset);
        std::thread::Builder::new()
            .name("openify-save-dialog".into())
            .spawn(move || {
                let _ = tx.send_blocking(crate::system::save_file_dialog(&title, &name, &fallback));
            })
            .ok();
        cx.spawn(async move |this, cx| {
            let chosen = rx.recv().await.ok().flatten();
            let _ = this.update(cx, |shell, cx| {
                match chosen {
                    Some(path) => shell.tools_write_backup(path, cx),
                    None => {
                        shell.tools.backup = BackupStatus::Idle;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Sınama koşusunda yedek yolu (iletişim kutusu yok): `OPENIFY_TEST_BACKUP_PATH`
    /// ya da geçici klasör. Normal koşuda `None`.
    fn tools_backup_test_path(&self, name: &str) -> Option<std::path::PathBuf> {
        let testing = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some();
        if !testing {
            return None;
        }
        if let Some(path) = std::env::var_os("OPENIFY_TEST_BACKUP_PATH") {
            return Some(path.into());
        }
        Some(std::env::temp_dir().join("openify-sandbox").join("backup").join(name))
    }

    fn tools_write_backup(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.tools.backup = BackupStatus::Writing;
        self.engine.send(Command::Library(LibraryCommand::ExportBackup(path)));
        cx.notify();
    }

    pub(super) fn on_backup_done(&mut self, summary: Summary, cx: &mut Context<Self>) {
        self.tools.backup = BackupStatus::Done(summary);
        cx.notify();
    }

    pub(super) fn on_backup_failed(&mut self, cx: &mut Context<Self>) {
        self.tools.backup = BackupStatus::Failed;
        cx.notify();
    }

    /// Yedek klasörünü Explorer'da göster (sınamada açılmaz).
    pub(super) fn tools_open_backup_folder(&mut self, path: &std::path::Path) {
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
            return;
        }
        let _ = std::process::Command::new("explorer").arg("/select,").arg(path).spawn();
    }

    // ── İstatistik ──────────────────────────────────────────────────────

    fn tools_want_stats(&mut self) {
        if self.tools_on() && self.tools.tool == Tool::Stats {
            self.engine.send(Command::Library(LibraryCommand::PlayStats(self.tools.stats.range)));
        }
    }

    pub(super) fn tools_set_range(&mut self, range: StatsRange, cx: &mut Context<Self>) {
        self.tools.stats.range = range;
        self.tools.stats.report = None;
        self.tools_want_stats();
        cx.notify();
    }

    pub(super) fn tools_more_stats(&mut self, cx: &mut Context<Self>) {
        self.tools.stats.more = !self.tools.stats.more;
        cx.notify();
    }

    pub(super) fn on_play_stats(&mut self, report: Arc<StatsReport>, cx: &mut Context<Self>) {
        if report.range == self.tools.stats.range {
            self.tools.stats.report = Some(report);
            cx.notify();
        }
    }

    // ── Çalma sayma (istatistiğin kaynağı) ──────────────────────────────

    /// Motor olaylarından çalma geçmişini besler: şarkı değişimi, çalma,
    /// durma. Gizli oturumda hiçbir şey yazılmaz.
    pub(super) fn tools_note_playback(&mut self, event: &Event, cx: &mut Context<Self>) {
        let mono = self.tools.recorder.base.elapsed().as_millis() as u64;
        let wall = now_ms();
        let logs = match event {
            Event::TrackChanged(info) => {
                let meta = TrackMeta {
                    uri: info.uri.clone(),
                    name: info.name.clone(),
                    artists: info.artists.clone(),
                    album: self.tools_album_of(&info.uri),
                    duration_ms: info.duration_ms,
                };
                self.tools.recorder.counter.track_changed(meta, mono, wall)
            }
            Event::Playing { .. } => self.tools.recorder.counter.playing(mono, wall),
            Event::Paused { .. } | Event::Loading | Event::Ended | Event::Unavailable => {
                self.tools.recorder.counter.stopped(mono, wall)
            }
            _ => return,
        };
        self.tools_send_logs(logs);
        self.tools_arm_timer(cx);
    }

    /// Şarkının albüm adı: bellekteki listelerden (çalma olayı albüm taşımıyor).
    fn tools_album_of(&self, uri: &str) -> String {
        let find = |rows: &Vec<TrackRow>| rows.iter().find(|r| r.uri == uri).map(|r| r.album.clone());
        self.library
            .collections
            .values()
            .filter_map(|l| match l {
                Loadable::Loaded(t) => find(t),
                _ => None,
            })
            .next()
            .or_else(|| self.pages.lists.values().find_map(|l| find(l)))
            .unwrap_or_default()
    }

    fn tools_send_logs(&self, logs: Vec<PlayLog>) {
        if engine::tools::stats::private_session() {
            return;
        }
        for log in logs {
            self.engine.send(Command::Library(LibraryCommand::RecordPlay(log)));
        }
    }

    /// Eşiğe kalan süre için zamanlayıcıyı kur (bir önceki iptal olur).
    fn tools_arm_timer(&mut self, cx: &mut Context<Self>) {
        let mono = self.tools.recorder.base.elapsed().as_millis() as u64;
        self.tools.recorder.timer = self.tools.recorder.counter.deadline_ms(mono).map(|ms| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(ms + 50)).await;
                let _ = this.update(cx, |shell, cx| shell.tools_tick(cx));
            })
        });
    }

    fn tools_tick(&mut self, cx: &mut Context<Self>) {
        let mono = self.tools.recorder.base.elapsed().as_millis() as u64;
        let logs = self.tools.recorder.counter.tick(mono, now_ms());
        self.tools_send_logs(logs);
        self.tools_arm_timer(cx);
    }

    // ── Klavye ──────────────────────────────────────────────────────────

    /// Sayfadaki kutular ve ad alanı için tuşlar. İşlendiyse `true`.
    pub(super) fn tools_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.tools_on() {
            return false;
        }
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        // Onay kartı: Enter onaylar, Esc vazgeçer.
        if let Some(bulk) = self.tools.confirm.clone() {
            match key {
                "escape" => self.tools_cancel_confirm(cx),
                // Düğmede Enter düğmenin kendi tıklamasıdır (Vazgeç'e basılmışken onaylanmasın).
                "enter" if self.nav.focused(window).is_none() => self.tools_request(bulk, cx),
                _ => return false,
            }
            return true;
        }
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return false;
        }
        let focused = self.nav.extra_focused(window).map(|(k, _)| k.to_string());
        // Birleştir / böl: ad alanına yazı.
        if focused.as_deref() == Some("tools-name") {
            return self.tools_name_key(event, cx);
        }
        let boxed = focused.as_deref().and_then(BoxId::from_key);
        // Odak çapadayken (fareyle tıklandıktan sonra) son kullanılan kutu.
        let target = match (boxed, focused.is_none() && self.nav.focused(window).is_none()) {
            (Some(b), _) => Some((b, true)),
            (None, true) => self.tools.active.map(|b| (b, false)),
            _ => None,
        };
        let Some((box_id, has_focus)) = target else { return false };
        let len = self.tools_box_len(box_id);
        let visible = box_id.visible();
        let lb = self.tools_box(box_id);
        match key {
            "up" => lb.move_by(-1, len, visible),
            "down" => lb.move_by(1, len, visible),
            "pageup" => lb.move_by(-(visible as isize), len, visible),
            "pagedown" => lb.move_by(visible as isize, len, visible),
            "home" => lb.to(0, len, visible),
            "end" => lb.to(len.saturating_sub(1), len, visible),
            // Odaktaki kutuda Enter / Boşluk'u gpui tıklamaya çevirir (öğe işler);
            // odak çapadayken burada işlenir.
            "enter" | "space" if !has_focus => {
                let at = lb.cursor;
                self.tools_box_activate(box_id, at, cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn tools_name_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let ks = &event.keystroke;
        let name = &mut self.tools.merge.name;
        match ks.key.as_str() {
            "backspace" => {
                name.pop();
            }
            "space" => name.push(' '),
            "enter" => {}
            "escape" | "tab" | "up" | "down" => return false,
            _ => match ks.key_char.as_deref().filter(|c| !c.is_empty() && !c.chars().any(char::is_control)) {
                Some(ch) => name.push_str(ch),
                None => return false,
            },
        }
        if name.chars().count() > 100 {
            *name = name.chars().take(100).collect();
        }
        cx.notify();
        true
    }

    /// Ctrl+V / Ctrl+Backspace ad alanında (genel tuşlardan önce alınır).
    pub(super) fn tools_name_paste(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.tools_on() || self.nav.extra_focused(window).is_none_or(|(k, _)| k != "tools-name") {
            return false;
        }
        let ks = &event.keystroke;
        if !ks.modifiers.control {
            return false;
        }
        match ks.key.as_str() {
            "v" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    let line = text.lines().next().unwrap_or_default();
                    self.tools.merge.name.push_str(line);
                }
            }
            "backspace" => {
                let name = &mut self.tools.merge.name;
                let trimmed = name.trim_end().len();
                let cut = name[..trimmed].rfind(' ').map_or(0, |i| i + 1);
                name.truncate(cut);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn tools_box_value(&self, id: BoxId) -> ListBox {
        match id {
            BoxId::DupPick => self.tools.dup.pick,
            BoxId::DupRows => self.tools.dup.rows,
            BoxId::MergePick => self.tools.merge.pick,
            BoxId::SortPick => self.tools.sort.pick,
        }
    }

    fn tools_box(&mut self, id: BoxId) -> &mut ListBox {
        match id {
            BoxId::DupPick => &mut self.tools.dup.pick,
            BoxId::DupRows => &mut self.tools.dup.rows,
            BoxId::MergePick => &mut self.tools.merge.pick,
            BoxId::SortPick => &mut self.tools.sort.pick,
        }
    }

    pub(super) fn tools_box_len(&self, id: BoxId) -> usize {
        match id {
            BoxId::DupPick | BoxId::SortPick => self.tools_items(Source::Own).len(),
            BoxId::MergePick => self.tools_items(Source::Readable).len(),
            BoxId::DupRows => self.tools_dup_view().map_or(0, |v| v.rows.len()),
        }
    }

    /// Kutudaki satır etkinleştirildi (tık, Enter, Boşluk).
    pub(super) fn tools_box_activate(&mut self, id: BoxId, index: usize, cx: &mut Context<Self>) {
        self.tools.active = Some(id);
        match id {
            BoxId::DupPick => self.tools_dup_pick(index, cx),
            BoxId::SortPick => self.tools_sort_pick(index, cx),
            BoxId::MergePick => self.tools_merge_pick(index, cx),
            BoxId::DupRows => {
                let len = self.tools_box_len(id);
                self.tools.dup.rows.to(index, len, RESULT_ROWS);
                // Başlık satırı: grubun hepsini çevirir.
                if let Some(view) = self.tools_dup_view()
                    && let Some(DupRow::Header { .. }) = view.rows.get(index)
                {
                    let group = view.rows[..index].iter().filter(|r| matches!(r, DupRow::Header { .. })).count();
                    self.tools_dup_toggle_group(group.saturating_sub(1), cx);
                } else {
                    self.tools_dup_toggle(index, cx);
                }
            }
        }
    }

    /// Tekerlek kutunun üstünde: kutu kayar (uzunsa).
    pub(super) fn tools_box_wheel(&mut self, id: BoxId, lines: isize, cx: &mut Context<Self>) {
        let len = self.tools_box_len(id);
        let visible = id.visible();
        self.tools_box(id).scroll(lines, len, visible);
        self.tools.active = Some(id);
        cx.notify();
    }

    // ── Motordan gelenler ve sınama betiği ──────────────────────────────

    /// Veri geldi (liste, kütüphane, eklenme anları): bekleyen sınama adımları sürer.
    pub(super) fn tools_on_data(&mut self, cx: &mut Context<Self>) {
        if self.tools_on() && !self.tools.script.is_empty() {
            self.tools_run_script(cx);
        }
    }

    fn tools_run_script(&mut self, cx: &mut Context<Self>) {
        while let Some(step) = self.tools.script.last().cloned() {
            if !self.tools_script_step(&step, cx) {
                return;
            }
            self.tools.script.pop();
        }
    }

    /// Bir sınama adımı: hazırsa uygular ve `true`. Adımlar tıklamayla aynı yolları kullanır.
    fn tools_script_step(&mut self, step: &str, cx: &mut Context<Self>) -> bool {
        let (name, arg) = step.split_once('=').unwrap_or((step, ""));
        let indices = |arg: &str| -> Vec<usize> { arg.split(',').filter_map(|n| n.trim().parse().ok()).collect() };
        match name {
            "tool" => {
                if let Some(tool) = Tool::from_key(arg) {
                    self.tools_select_tool(tool, cx);
                    if arg == "split" {
                        self.tools_set_mode(MergeMode::Split, cx);
                    }
                }
            }
            "mode" => self.tools_set_mode(if arg == "split" { MergeMode::Split } else { MergeMode::Merge }, cx),
            // Kütüphane gelmeden seçim yapılamaz.
            "pick" | "review" | "confirm" | "cursor" | "apply" if !matches!(self.library.library, Loadable::Loaded(_)) => return false,
            "pick" => {
                for i in indices(arg) {
                    match self.tools.tool {
                        Tool::Duplicates => self.tools_dup_pick(i, cx),
                        Tool::Sort => self.tools_sort_pick(i, cx),
                        _ => self.tools_merge_pick(i, cx),
                    }
                }
            }
            "by" => match arg {
                "artist" if self.tools.tool == Tool::Sort => self.tools_set_sort(SortBy::Artist, cx),
                "artist" => self.tools.merge.split_by = SplitBy::Artist,
                "album" if self.tools.tool == Tool::Sort => self.tools_set_sort(SortBy::Album, cx),
                "album" => self.tools.merge.split_by = SplitBy::Album,
                "year" => self.tools.merge.split_by = SplitBy::AddedYear,
                "name" => self.tools_set_sort(SortBy::Name, cx),
                "added" => self.tools_set_sort(SortBy::Added, cx),
                "duration" => self.tools_set_sort(SortBy::Duration, cx),
                _ => {}
            },
            "dir" => self.tools_set_dir(if arg == "desc" { Dir::Desc } else { Dir::Asc }, cx),
            "dedupe" => {
                self.tools.merge.dedupe = match arg {
                    "same" => DedupeMode::SameTrack,
                    "versions" => DedupeMode::Versions,
                    _ => DedupeMode::Off,
                };
            }
            "review" => {
                let ids = self.tools_merge_selection();
                match self.tools_sources(&ids) {
                    Sources::Loading => return false,
                    _ => self.tools_review(cx),
                }
            }
            "confirm" => {
                let bulk = match arg {
                    "split" => Bulk::Split,
                    "merge" => Bulk::Merge,
                    "sort" => Bulk::Sort,
                    _ => Bulk::Dupes,
                };
                self.tools.confirm = Some(bulk);
            }
            // Eylem düğmesine basmak gibi (eşik aşılırsa onay kartı da onaylanır).
            "apply" => {
                let bulk = match arg {
                    "split" => Bulk::Split,
                    "merge" => Bulk::Merge,
                    "sort" => Bulk::Sort,
                    _ => Bulk::Dupes,
                };
                self.tools_request(bulk.clone(), cx);
                if self.tools.confirm.is_some() {
                    self.tools_request(bulk, cx);
                }
            }
            "cursor" => {
                let id = match self.tools.tool {
                    Tool::Duplicates => BoxId::DupRows,
                    Tool::Sort => BoxId::SortPick,
                    _ => BoxId::MergePick,
                };
                let at = indices(arg).first().copied().unwrap_or(0);
                let len = self.tools_box_len(id);
                self.tools_box(id).to(at, len, id.visible());
            }
            "spare" => {
                for i in indices(arg) {
                    self.tools_dup_toggle(i, cx);
                }
            }
            "range" => {
                let range = match arg {
                    "6m" => StatsRange::Months6,
                    "all" => StatsRange::All,
                    _ => StatsRange::Weeks4,
                };
                self.tools_set_range(range, cx);
            }
            "more" => self.tools.stats.more = true,
            "backup" => self.tools_backup(cx),
            "wait" => {
                // Önceki adımın verisi gelsin diye: ilgili liste yüklüyse geç.
                let ids: Vec<CollectionId> = match self.tools.tool {
                    Tool::Duplicates => self.tools.dup.list.iter().cloned().collect(),
                    Tool::Sort => self.tools.sort.list.iter().cloned().collect(),
                    _ => self.tools_merge_selection(),
                };
                if matches!(self.tools_sources(&ids), Sources::Loading) {
                    return false;
                }
            }
            _ => log::warn!("OPENIFY_TEST_TOOLS: bilinmeyen adım: {step}"),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, with_lang};

    #[test]
    fn list_box_keeps_the_cursor_visible_and_clamps() {
        let mut lb = ListBox::default();
        lb.move_by(-1, 20, 7);
        assert_eq!((lb.cursor, lb.offset), (0, 0));
        for _ in 0..9 {
            lb.move_by(1, 20, 7);
        }
        assert_eq!((lb.cursor, lb.offset), (9, 3), "imleç altta: pencere kayar");
        lb.to(19, 20, 7);
        assert_eq!((lb.cursor, lb.offset), (19, 13));
        lb.move_by(5, 20, 7);
        assert_eq!(lb.cursor, 19, "sona yapışır");
        lb.to(0, 20, 7);
        assert_eq!((lb.cursor, lb.offset), (0, 0));
        // Kısa liste: kaydırma yok, imleç içeride.
        let mut short = ListBox { cursor: 5, offset: 2 };
        short.to(9, 3, 7);
        assert_eq!((short.cursor, short.offset), (2, 0));
        let mut empty = ListBox { cursor: 4, offset: 4 };
        empty.move_by(1, 0, 7);
        assert_eq!(empty, ListBox::default());
    }

    #[test]
    fn wheel_scrolls_the_window_and_pulls_the_cursor_in() {
        let mut lb = ListBox::default();
        lb.scroll(3, 20, 7);
        assert_eq!((lb.offset, lb.cursor), (3, 3));
        lb.scroll(100, 20, 7);
        assert_eq!(lb.offset, 13);
        assert!((13..20).contains(&lb.cursor));
        lb.scroll(-100, 20, 7);
        assert_eq!((lb.offset, lb.cursor), (0, 6));
        // Kısa listede kaydırma yok.
        let mut short = ListBox::default();
        short.scroll(4, 3, 7);
        assert_eq!(short.offset, 0);
    }

    #[test]
    fn keys_round_trip_and_every_tool_has_a_label_in_both_languages() {
        for tool in Tool::ALL {
            assert_eq!(Tool::from_key(tool.key()), Some(tool));
            for lang in [Lang::En, Lang::Tr] {
                assert!(!with_lang(lang, || tool.label().to_string()).is_empty());
            }
        }
        assert_eq!(Tool::from_key("split"), Some(Tool::MergeSplit));
        assert_eq!(Tool::from_key("nope"), None);
        for b in [BoxId::DupPick, BoxId::DupRows, BoxId::MergePick, BoxId::SortPick] {
            assert_eq!(BoxId::from_key(b.key()), Some(b));
        }
        assert_eq!(BoxId::from_key("seg:x:y"), None);
    }

    #[test]
    fn default_names_and_listening_time_read_naturally() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(default_merge_name(&n(&["A"])), "A");
        assert_eq!(default_merge_name(&n(&["A", "B"])), "A + B");
        assert_eq!(default_merge_name(&n(&["A", "B", "C", "D"])), "A + B + 2");
        assert_eq!(default_merge_name(&[]), "");
        assert_eq!(with_lang(Lang::En, || listened_text(45 * 60_000)), "45 min");
        assert_eq!(with_lang(Lang::En, || listened_text(754 * 60_000)), "12 h 34 min");
        assert_eq!(with_lang(Lang::Tr, || listened_text(754 * 60_000)), "12 sa 34 dk");
        assert_eq!(with_lang(Lang::Tr, || listened_text(0)), "0 dk");
    }

    /// Onay eşiği motorun sabitiyle aynı yerden gelir ve sıralama sınırı onun üstünde.
    #[test]
    fn bulk_thresholds_are_ordered() {
        assert_eq!(BULK_CONFIRM, 50);
        assert!(MAX_SORT_MOVES > BULK_CONFIRM);
    }
}
