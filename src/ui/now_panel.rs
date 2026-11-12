//! "Şimdi çalıyor" paneli (7 Kasım kararı).
//!
//! Sağdaki panelin ilk sekmesi: büyük kapak, şarkı ve sanatçı (adlar
//! tıklanabilir), sıradaki şarkı, sözlerden okunan satırın çevresi ve elde
//! olan veriyle sanatçı hakkında. Hiçbir şey yüklü değilken son çalınan
//! listeler (tıklayınca çalar).
//!
//! Yerleşim: pencere `DOCK_MIN_WIDTH`'ten genişse panel listenin sağına
//! yerleşir (listenin üstüne binmez; liste, raf ve sayfalar panelin solunda
//! kalan alanın tamamını kullanır). Hap hep pencerenin ortasında (panel
//! açılıp kapanınca kaymaz), panele yetişirse daralır. Daha dar pencerede
//! panel eskisi gibi içeriğin üstünde yüzer. Geniş pencerede panelin açık /
//! kapalı oluşu ayarlarda saklanır; gösterecek hiçbir şey yokken (şarkı yok,
//! son çalınan yok) panel açılmaz.
//!
//! Çizim yükü: panel kendi önbellekli görünümünde (`SideView`); yalnız
//! `Shell` değişince, söz satırı değişince (zamanlayıcıyla) ya da kayma
//! hareketi sürerken çizilir. Sanatçı sayfası yalnız önbellekten okunur
//! (bellek ya da disk, sanatçı başına bir kez); şarkı değişince ağa sayfa
//! isteği gitmez (sanatçı adının üzerine gelince sayfa zaten önceden istenir).

use std::{collections::HashSet, ops::Range, sync::Arc, time::Instant};

use gpui::{
    AnyElement, Context, Div, FontWeight, ScrollHandle, Window, div, img, linear_color_stop,
    linear_gradient, prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{Look, Tooltip, state_view},
    cover_image, framed,
    history::Route,
    library::{Loadable, PAGE_PAD, ROW_HEIGHT},
    now_playing::{LYRIC_LEAD_MS, LyricsStatus, Panel},
    pages::PageLoad,
    pill::pill_width,
    player::fmt_time,
    queue_panel::PANEL_WIDTH,
    text::line_with_tip,
    theme::*,
};
use crate::engine::{self, ArtistPage, CollectionId, LibraryItem, SyncKind};
use crate::t;

/// Yerleşik panelin eni (7 Kasım, kullanıcı geri bildirimi): sabit 400 px.
/// Liste artık panelin solundaki bütün yeri kullandığı için panelin pencereyle
/// büyümesine gerek yok (eskiden 360–440 arası esneyip artan yeri liste ile
/// paylaşıyordu, sonra iki yana bölüyordu: sağda ölü boşluk kalıyordu).
/// Pencere boyutlanınca yalnız liste esner, tek şey değişir.
/// - Kapak panelin iç eni: 400 - 2·16 - 2 = 366 px. Kaynak 640 px: %150
///   ölçekte 549, %175'te 640 fiziksel px; hiçbir yaygın ölçekte büyütülmez.
/// - Paneldeki en uzun düz yazı (sanatçı tanıtımı, Secondary 13 px) 368 px
///   satırda ~57 harf: rahat okuma aralığının (45–75) ortası.
/// - Hap (en çok 800) pencerenin ortasında dururken panele 24 px'ten fazla
///   yaklaşmaz: 1680 px'ten geniş her pencerede tam enli kalır (eşikte,
///   1720'de, arada 44 px).
pub(super) const DOCK_PANEL_WIDTH: f32 = 400.;
/// Panel yerleşikken listenin (satır içeriği) eşikteki eni.
const DOCK_LIST_MIN: f32 = 1240.;
/// Panelin listenin yanına yerleştiği en dar pencere: sol kenar 32, liste
/// en az 1240 (eskiden sabit 1280'di; 1240'ta şarkı ~576, albüm ~432 px),
/// liste ile panel arasında listenin sağ sayfa kenarı (32), panel (400) ve
/// sağda yüzen paneldeki gibi 16 px: 32 + 1240 + 32 + 400 + 16 = 1720 px
/// (eşik kullanıcının alıştığı yerde kalır). Panelin içeriği de pencerenin
/// sağ kenarından 16 + 16 = 32 px içeride: sol sayfa kenarının aynası.
pub(super) const DOCK_MIN_WIDTH: f32 = PAGE_PAD + DOCK_LIST_MIN + PAGE_PAD + DOCK_PANEL_WIDTH + S4;
/// Arkadaş etkinliği panelinin yerleşik eni (yüzen panelle aynı).
pub(super) const FRIENDS_PANEL_WIDTH: f32 = PANEL_WIDTH;
/// Arkadaş paneli, yanında içerik alanı uygulamanın en dar eninin (pencerenin
/// en küçük eni) altına inmedikçe yerleşir: 760 + 360 + sağ boşluk 16 = 1136.
/// (Şimdi çalıyor paneli 1240 px'lik geniş listeyi korumak için 1720'de
/// yerleşir; arkadaş paneli liste dostu değil, kısa ve dar: 1180×760 pencere
/// de dahil.) Daha darda (900×700) içeriğin üstünde yüzer.
pub(super) const FRIENDS_DOCK_MIN_WIDTH: f32 = super::MIN_WINDOW_WIDTH + FRIENDS_PANEL_WIDTH + S4;
/// "Son çalınanlar"da en çok bu kadar liste.
const RECENT_LIMIT: usize = 8;
/// Kısa panelde kapağın en küçük hâli (sayfa başlığındaki kapak kadar).
const NOW_COVER_MIN: f32 = 160.;
/// "Sıradaki" için kapak bunun altına inmez (altında kaydırmaya bırakılır).
const NOW_COVER_NEXT_MIN: f32 = 224.;
/// Sıradaki şarkının ve son çalınanların küçük kapağı.
const THUMB: f32 = 40.;
/// Ad (Headline) ve sanatçı (Title) bloğu, aralarında 4 px.
const TITLE_BLOCK: f32 = T_HEADLINE.line + S1 + T_TITLE.line;
/// Söz kesitinde bir satırın payı (Title satırı ve satır arası).
const EXCERPT_STEP: f32 = T_TITLE.line + S2;
/// Söz kesitinin kenarlarındaki solma.
const EXCERPT_FADE: f32 = S6;
/// Söz kesitinin en kısa hâli: okunan satır ve iki yanında solmanın dışında
/// kalan birer tam satır, üstte başlıktan 8 px (kısa panelde 3 okunur satır,
/// gerisi kayar).
pub(super) const EXCERPT_MIN: f32 = S2 + T_TITLE.line + 2. * (EXCERPT_FADE + EXCERPT_STEP);
/// Sanatçı tanıtım yazısından gösterilen en çok harf.
const BIO_CHARS: usize = 220;

/// Pencerenin bu karedeki yerleşimi: içerik alanının (raf, liste, sayfalar)
/// eni (hep pencerenin solundan başlar); panel listenin yanında mı; panelin
/// eni ve pencerenin sağ kenarına uzaklığı (yüzen panelde 360 ve 16).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct DockLayout {
    pub docked: bool,
    pub main_width: f32,
    pub panel_width: f32,
    pub panel_right: f32,
    /// Panel hapın yanında, pencerenin altına kadar mı iniyor (hap ona göre
    /// daralır)? Arkadaş paneli hapın ÜSTÜNDE biter, hap daralmaz.
    pub beside_pill: bool,
}

impl DockLayout {
    /// Hapın solu ve eni: hep pencerenin ortasında (panel açılıp kapanınca
    /// kaymaz). Yerleşik panele 24 px'ten fazla yaklaşacaksa daralır (dar
    /// hapın kuralları, ör. ses çubuğunun çekilmesi, bu ene göre işler).
    pub(super) fn pill_frame(&self, window_w: f32) -> (f32, f32) {
        let mut width = pill_width(window_w);
        if self.docked && self.beside_pill {
            let panel_left = window_w - self.panel_right - self.panel_width;
            width = width.min(2. * (panel_left - S5) - window_w).max(0.);
        }
        (((window_w - width) / 2.).round(), width)
    }
}

/// Yerleşim: geniş pencerede panel açıksa içerik alanı (raf, liste,
/// sayfalar) panelin solundaki bütün yer, panel sağda 16 px içeride. Gerisinde
/// içerik pencerenin tamamı (dar pencerede panel üstte yüzer).
pub(super) fn dock_layout(window_w: f32, panel_open: bool) -> DockLayout {
    dock_layout_at(window_w, panel_open, if panel_open { 1. } else { 0. })
}

/// `dock_layout`, panelin kayma ilerlemesiyle (7 Kasım akşam): `slide` 0 (kapalı)
/// – 1 (tam açık). Geniş pencerede içerik alanı panel kadar daralır ve panel
/// sağdan içeri kayar (kenarı içeriğin kenarına yapışık gelir: panel içeriği
/// iter); dar pencerede panel yerinde belirir (`render_side_panel`).
pub(super) fn dock_layout_at(window_w: f32, panel_open: bool, slide: f32) -> DockLayout {
    dock_layout_for(DockKind::NOW, window_w, panel_open, slide)
}

/// Panel türüne göre yerleşim kuralı: hangi pencere eninden yerleşir, eni
/// kaç, hapın yanına mı iner (bkz. `dock_kind`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct DockKind {
    pub min_width: f32,
    pub panel_width: f32,
    pub beside_pill: bool,
}

impl DockKind {
    /// Şimdi çalıyor / Sıra / Sözler: geniş liste korunur (1720'den), panel
    /// pencerenin altına kadar iner.
    pub const NOW: DockKind = DockKind {
        min_width: DOCK_MIN_WIDTH,
        panel_width: DOCK_PANEL_WIDTH,
        beside_pill: true,
    };
    /// Arkadaş etkinliği: içerik alanı en dar pencere eninin altına inmedikçe
    /// yerleşir; kısa panel hapın üstünde biter.
    pub const FRIENDS: DockKind = DockKind {
        min_width: FRIENDS_DOCK_MIN_WIDTH,
        panel_width: FRIENDS_PANEL_WIDTH,
        beside_pill: false,
    };
}

/// Gösterilen panelin yerleşim kuralı.
pub(super) fn dock_kind(panel: Option<Panel>) -> DockKind {
    if panel == Some(Panel::Friends) {
        DockKind::FRIENDS
    } else {
        DockKind::NOW
    }
}

/// `dock_layout_at`, panel türünün kuralıyla.
pub(super) fn dock_layout_for(
    kind: DockKind,
    window_w: f32,
    panel_open: bool,
    slide: f32,
) -> DockLayout {
    if window_w >= kind.min_width && (panel_open || slide > 0.) {
        let travel = kind.panel_width + S4;
        DockLayout {
            docked: true,
            main_width: if slide > 0. && slide < 1. {
                (window_w - travel * slide).round()
            } else {
                window_w - travel * slide
            },
            panel_width: kind.panel_width,
            panel_right: S4 - travel * (1. - slide),
            beside_pill: kind.beside_pill,
        }
    } else {
        DockLayout {
            docked: false,
            main_width: window_w,
            panel_width: PANEL_WIDTH,
            panel_right: S4,
            beside_pill: kind.beside_pill,
        }
    }
}

/// Pencere eni ya da gösterilecek içerik değişince panelin ne olacağı.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DockChange {
    Open,
    Hide,
    Keep,
}

/// `was_wide`: önceki karede pencere genişti (ilk karede `None`).
/// `prefer_open`: kullanıcının geniş penceredeki tercihi (ayarlarda).
pub(super) fn dock_change(
    was_wide: Option<bool>,
    wide: bool,
    had_content: bool,
    has_content: bool,
    prefer_open: bool,
    panel_open: bool,
) -> DockChange {
    if !wide {
        // Daralınca yerleşik panel kalkar (dar pencerede yüzen panel
        // içeriği örter; kendiliğinden açılmaz). Tercih değişmez.
        return if was_wide == Some(true) && panel_open {
            DockChange::Hide
        } else {
            DockChange::Keep
        };
    }
    if panel_open || !prefer_open || !has_content {
        return DockChange::Keep;
    }
    // Genişledi, açılış, ya da ilk kez gösterilecek bir şey oldu.
    if was_wide != Some(true) || !had_content {
        DockChange::Open
    } else {
        DockChange::Keep
    }
}

/// "Şimdi çalıyor" sekmesinin hâli.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NowBody {
    /// Çalan (ya da yüklenen) şarkı.
    Track,
    /// Hiçbir şey yüklü değil: son çalınan listeler.
    Recent,
    /// Son çalınan da yok.
    Empty,
}

pub(super) fn now_body_kind(has_track: bool, recent: usize) -> NowBody {
    if has_track {
        NowBody::Track
    } else if recent > 0 {
        NowBody::Recent
    } else {
        NowBody::Empty
    }
}

/// Büyük kapağın en büyük hâli: panelin iç eni (iki yanda 16 px, 1 px
/// kenarlık): yüzen panelde 326, yerleşikte 366.
pub(super) fn now_cover_max(panel_w: f32) -> f32 {
    panel_w - 2. * S4 - 2.
}

/// Büyük kapağın kenarı: panelin iç eni; kısa panelde (dar ve kısa pencere)
/// ad ve sanatçı kapağın altında kaydırmadan görünecek kadar küçülür, ama
/// sayfa başlığındaki kapaktan (160 px) küçük olmaz. Sıradaki şarkı varsa
/// ("Sıradaki" başlığı ve bir satır) kapak, bunlar da kaydırmadan görünecek
/// ve altta 12 px kalacak kadar küçülür; ama bu kapağı 224 px'in altına
/// indiriyorsa "Sıradaki" kaydırmaya bırakılır (kapak büyük kalır).
pub(super) fn now_cover_side(panel_w: f32, panel_h: f32, has_next: bool) -> f32 {
    let base = panel_h - ROW_HEIGHT - 2. - S1 - S4 - TITLE_BLOCK;
    let with_next = base - (S5 + T_BODY.line + S2 + ROW_HEIGHT) - S3;
    let room = if has_next && with_next >= NOW_COVER_NEXT_MIN {
        with_next
    } else {
        base - S4
    };
    room.clamp(NOW_COVER_MIN, now_cover_max(panel_w)).floor()
}

/// Söz kesitine kalan yükseklik (yaklaşık; sanatçı bölümü sayılmaz, o
/// varsa kesit esneyerek küçülür): panelin gövdesi (başlık şeridi ve
/// kenarlık dışında) eksi kapak, ad / sanatçı, varsa "Sıradaki" ve "Sözler"
/// başlığı. En az `EXCERPT_MIN`.
pub(super) fn excerpt_room(panel_h: f32, cover: f32, has_next: bool) -> f32 {
    let body = panel_h - 2. - ROW_HEIGHT;
    let heading = S5 + T_BODY.line + S2;
    let fixed = S1 + cover + S4 + TITLE_BLOCK + heading + S4;
    let next = if has_next { heading + ROW_HEIGHT } else { 0. };
    (body - fixed - next).max(EXCERPT_MIN)
}

/// Okunan satırın üstünde en çok kaç önceki satır: kesitin üçte biri
/// (sözler sekmesindeki odak yüksekliği), en az bir.
pub(super) fn excerpt_above(room: f32) -> usize {
    ((room / 3. / EXCERPT_STEP).round() as usize).max(1)
}

/// Okunan satırın altında en çok kaç satır kurulur: panelin boyunu
/// dolduracak kadar (fazlası kırpılır).
pub(super) fn excerpt_below(panel_h: f32) -> usize {
    (panel_h / EXCERPT_STEP).ceil().max(1.) as usize
}

/// Söz kesitinin satırları: okunan satırdan öncekiler (en çok `above`) ve
/// sonrakiler (en çok `below`). Okunan satır yoksa (ilk satırdan önce,
/// eşzamansız söz) baştan.
pub(super) fn excerpt_split(
    active: Option<usize>,
    len: usize,
    above: usize,
    below: usize,
) -> (Range<usize>, Range<usize>) {
    match active {
        Some(a) => (
            a.saturating_sub(above)..a,
            (a + 1).min(len)..(a + 1 + below).min(len),
        ),
        None => (0..0, 0..below.min(len)),
    }
}

/// Sanatçı yazısından kısa bir parça: harf sınırında, mümkünse kelime
/// sonunda kesilir, "…" eklenir.
pub(super) fn short_bio(bio: &str, max_chars: usize) -> String {
    let bio = bio.split_whitespace().collect::<Vec<_>>().join(" ");
    if bio.chars().count() <= max_chars {
        return bio;
    }
    let cut: String = bio.chars().take(max_chars).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max_chars / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end_matches([',', '.', ';', ':', ' ']))
}

/// Panelin içeri / dışarı kayma hareketi (7 Kasım akşam): açılırken `motion`
/// (220 ms, ease-out quint), kapanırken `motion_exit` (160 ms, ease-in). Kayma
/// sürerken kök her karede çizilir ve biter bitmez durur (`animating`).
/// İlk karede, pencerenin geniş / dar durumu değişince ve "animasyonları
/// azalt"ta hareket yok: panel anında yerine geçer.
pub(super) struct Slide {
    /// 0 (kapalı) – 1 (tam açık), son karede.
    pub value: f32,
    from: f32,
    to: f32,
    at: Instant,
    started: bool,
    wide: bool,
    /// Kapanırken hâlâ çizilen panel (kullanıcı kapattı, hareket sürüyor).
    pub exiting: Option<Panel>,
    last: Panel,
}

impl Slide {
    fn new() -> Self {
        Self {
            value: 0.,
            from: 0.,
            to: 0.,
            at: Instant::now(),
            started: false,
            wide: false,
            exiting: None,
            last: Panel::Now,
        }
    }

    /// Bu kare: panel `open` (hangi sekme), pencere `wide` mı. Hareket
    /// sürüyorsa `true`.
    pub(super) fn advance(
        &mut self,
        now: Instant,
        wide: bool,
        open: Option<Panel>,
        animate: bool,
    ) -> bool {
        let target = if open.is_some() { 1. } else { 0. };
        if let Some(p) = open {
            self.last = p;
        }
        if !self.started || self.wide != wide || !animate {
            self.started = true;
            self.wide = wide;
            (self.value, self.from, self.to, self.exiting) = (target, target, target, None);
            return false;
        }
        if target != self.to {
            self.from = self.value;
            self.to = target;
            self.at = now;
            self.exiting = open.is_none().then_some(self.last);
        }
        if self.value == self.to {
            return false;
        }
        let entering = self.to > self.from;
        // Yarıda dönülürse süre kalan yolla orantılı (en az %30).
        let span = (self.to - self.from).abs().max(0.3);
        let full = if entering {
            super::theme::motion()
        } else {
            super::theme::motion_exit()
        };
        let t = now.duration_since(self.at).as_secs_f32() / (full.as_secs_f32() * span);
        let eased = if entering {
            super::theme::quint_out(t)
        } else {
            super::theme::cubic_in(t)
        };
        if t >= 1. {
            (self.value, self.exiting) = (self.to, None);
            return false;
        }
        self.value = self.from + (self.to - self.from) * eased;
        true
    }
}
/// Yerleşik panelin durumu (`NowState::dock`).
pub(super) struct DockState {
    /// Kullanıcının geniş penceredeki tercihi (ayarlarda `now_panel`).
    pub open: bool,
    /// Son karede pencere `DOCK_MIN_WIDTH`'ten geniş miydi.
    pub wide: bool,
    /// Son karede gösterilecek bir şey var mıydı (şarkı ya da son çalınan).
    pub(super) has_content: bool,
    /// İlk kare çizildi mi (`wide` / `has_content` geçerli mi).
    synced: bool,
    /// Panel kendiliğinden açılınca hangi sekmede (son seçilen).
    pub tab: Panel,
    /// Bu karede panelin eni ve pencerenin sağ kenarına uzaklığı (yerleşik
    /// panelde 400 ve 16, yüzen panelde 360 ve 16).
    pub panel_box: (f32, f32),
    /// Açılma / kapanma hareketi (bkz. `Slide`).
    pub slide: Slide,
    /// Bu karede panel yerleşik çiziliyor mu (kapanma hareketi dahil).
    pub visual_docked: bool,
    /// Kayma hareketi sürüyor: kök bir sonraki kareyi ister.
    pub sliding: bool,
    /// Önbellekte sayfası arandı (sanatçı başına diske bir kez bakılır).
    artist_checked: HashSet<String>,
    pub scroll: ScrollHandle,
}

impl Default for DockState {
    fn default() -> Self {
        Self {
            open: true,
            wide: false,
            has_content: false,
            synced: false,
            tab: Panel::Now,
            panel_box: (PANEL_WIDTH, S4),
            slide: Slide::new(),
            visual_docked: false,
            sliding: false,
            artist_checked: HashSet::new(),
            scroll: ScrollHandle::new(),
        }
    }
}

impl Shell {
    // ── Yerleşim ────────────────────────────────────────────────────────

    /// Panelde gösterilecek bir şey var mı: şarkı ya da çözülebilen bir son
    /// çalınan liste (her karede: ucuz tutulur).
    fn panel_has_content(&self) -> bool {
        if self.state.track.is_some() || self.state.pending.is_some() {
            return true;
        }
        if !matches!(self.library.library, Loadable::Loaded(_)) {
            return false;
        }
        self.recent
            .iter()
            .any(|key| match CollectionId::from_key(key) {
                Some(CollectionId::Liked) => true,
                Some(id) => self.library.item(&id).is_some(),
                None => false,
            })
    }

    /// Kökün her karesinde, parçalar çizilmeden önce: panelin pencere enine
    /// göre hâli ve içerik alanının eni. Panel açılıp kapanınca içerik aynı
    /// karede yeni enine geçer (kayan bir şey yok).
    pub(super) fn dock_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> DockLayout {
        let window_w = f32::from(window.viewport_size().width);
        // Ölçüm kancası `OPENIFY_PROF_HIDE=dock`: panel hiç yerleşmez ve
        // kendiliğinden açılmaz (kare A/B'sinde "panelsiz" taraf).
        if super::frames::hidden("dock") {
            self.now.dock.panel_box = (PANEL_WIDTH, S4);
            return dock_layout(window_w, false);
        }
        let wide = window_w >= DOCK_MIN_WIDTH;
        let content = self.panel_has_content();
        let d = &self.now.dock;
        let change = dock_change(
            d.synced.then_some(d.wide),
            wide,
            d.has_content,
            content,
            d.open,
            self.now.panel.is_some(),
        );
        // Arkadaş paneli kendi eşiğinde (1136) yerleşir: Şimdi çalıyor
        // eşiğinden (1720) daralınca kapanmaz; yalnız kendi eşiğinin altına
        // inince (yerleşikken yüzen olacaksa) kalkar.
        let friends_open = self.now.panel == Some(Panel::Friends);
        let change = match change {
            DockChange::Hide if friends_open => DockChange::Keep,
            DockChange::Keep
                if friends_open && d.visual_docked && window_w < FRIENDS_DOCK_MIN_WIDTH =>
            {
                DockChange::Hide
            }
            other => other,
        };
        self.now.dock.wide = wide;
        self.now.dock.has_content = content;
        self.now.dock.synced = true;
        match change {
            DockChange::Open => {
                self.show_panel(self.now.dock.tab);
                self.after_view_change(cx);
            }
            DockChange::Hide => {
                self.hide_panel();
                self.after_view_change(cx);
            }
            DockChange::Keep => {}
        }

        let open = self.now.panel;
        let sliding = self
            .now
            .dock
            .slide
            .advance(Instant::now(), wide, open, animations_enabled());
        self.now.dock.sliding = sliding;
        let kind = dock_kind(self.now.shown_panel());
        let layout = dock_layout_for(kind, window_w, open.is_some(), self.now.dock.slide.value);
        self.now.dock.panel_box = (layout.panel_width, layout.panel_right);
        self.now.dock.visual_docked = layout.docked;
        layout
    }

    /// Yerleşik panelin kapladığı sağ şerit (panel ve sağ boşluğu); panel
    /// yerleşik değilse 0. Sağ alttaki bildirim bunun solunda durur.
    pub(super) fn dock_inset(&self) -> f32 {
        if self.now.dock.visual_docked {
            self.now.dock.panel_box.0 + self.now.dock.panel_box.1
        } else {
            0.
        }
    }

    // ── "Şimdi çalıyor" sekmesi ─────────────────────────────────────────

    /// Son çalınan listeler (ana sayfadaki sırayla), en çok `limit`.
    fn recent_items(&self, limit: usize) -> Vec<LibraryItem> {
        match &self.library.library {
            Loadable::Loaded(library) => engine::home(library, &self.recent, limit).recent,
            _ => Vec::new(),
        }
    }

    pub(super) fn render_now_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_track = self.state.track.is_some() || self.state.pending.is_some();
        let recent = if has_track {
            Vec::new()
        } else {
            self.recent_items(RECENT_LIMIT)
        };
        // Kısa pencerede kapak küçülür: ad ve sanatçı kaydırmadan görünsün.
        let viewport = window.viewport_size();
        let (_, panel_h) =
            self.side_panel_frame(f32::from(viewport.width), f32::from(viewport.height));
        let has_next = self
            .now
            .queue
            .upcoming
            .first()
            .is_some_and(|e| e.track.is_some());
        let cover = now_cover_side(self.now.dock.panel_box.0, panel_h, has_next);
        let body = match now_body_kind(has_track, recent.len()) {
            NowBody::Track => self.now_track(cover, panel_h, window, cx),
            NowBody::Recent => self.now_recent(recent, cx),
            NowBody::Empty => {
                return div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(S5))
                    .child(state_view("music", t!(NothingPlaying), t!(NowEmptyBody)))
                    .into_any_element();
            }
        };
        // Kenarlar sözler sekmesindeki gibi solar (kaydırılmış içerik varsa).
        let handle = &self.now.dock.scroll;
        let offset = f32::from(handle.offset().y);
        let max = f32::from(handle.max_offset().height);
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("now-body")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(handle)
                    .px(px(S4))
                    .pb(px(S4))
                    .flex()
                    .flex_col()
                    .child(body),
            )
            .children(super::lyrics_view::edge_veil(true, -offset / S6))
            .children(super::lyrics_view::edge_veil(false, (max + offset) / S6))
            .into_any_element()
    }

    /// Çalan şarkı: kapak, ad / sanatçı, sıradaki, söz kesiti, sanatçı.
    fn now_track(
        &mut self,
        side: f32,
        panel_h: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (uri, name, artists, art) = match (&self.state.track, &self.state.pending) {
            (_, Some(p)) => (
                String::new(),
                p.name.clone(),
                String::new(),
                cover_image(&self.images, &p.image, &p.name, side, R_LG, "music"),
            ),
            (Some(t), None) => {
                // Motorun 256 px'lik kapağı; büyük kapak (640 px, tam ekranla
                // ortak) gelince üstünde kapak geçişi süresince belirir. Küçük
                // altta kalır: büyük çözülene dek boş kare olmaz. Köşe
                // resimlerin kendisinde de (kabın `overflow_hidden`'ı resmi
                // kırpmıyor).
                let art = match self.state.cover.clone() {
                    Some(image) => {
                        let sharp = self.hires_cover().map(|(sharp, shown)| {
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .opacity(shown)
                                .child(img(sharp).size_full().rounded(px(R_LG)))
                        });
                        framed(
                            div()
                                .relative()
                                .size_full()
                                .child(img(image).size_full().rounded(px(R_LG)))
                                .children(sharp),
                            side,
                            R_LG,
                        )
                    }
                    None => super::placeholder(&t.name, side, R_LG, "music"),
                };
                (t.uri.clone(), t.name.clone(), t.artists.clone(), art)
            }
            (None, None) => return div().into_any_element(),
        };
        // Kapak tam ekranı açar (haptaki kapak gibi): üzerine gelince ortada
        // simge.
        let cover = div()
            .id("now-cover")
            .relative()
            .flex_none()
            .rounded(px(R_LG))
            .cursor_pointer()
            .group("now-cover")
            .child(art)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.)
                    .group_hover("now-cover", |s| s.opacity(1.))
                    .child(
                        div()
                            .size(px(S7))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(shade(SCRIM))
                            .child(icon("maximize-2", 20., text())),
                    ),
            )
            .tooltip(Tooltip::text(t!(FullScreen), Some("F11")))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx)));

        // Ad ve sanatçı panelin asıl bilgisi (7 Kasım gecesi, kullanıcı geri
        // bildirimi: "çok arka planda"): ad Headline rolünde (Figtree 700),
        // altında 4 px arayla sanatçı Title rolünde (18 px) ön plana yakın
        // tonda; ikisi kapakla aynı sol kenardan başlar, adlar tıklanabilir.
        // Kalp ad satırının hizasında (satırın dikey ortası).
        let second = if uri.is_empty() {
            div()
                .type_role(T_TITLE)
                .text_color(label())
                .child(super::text::line(t!(Loading)))
                .into_any_element()
        } else {
            div()
                .flex()
                .type_role(T_TITLE)
                .text_color(soft())
                .child(self.pill_artists(&uri, &name, &artists, cx))
                .into_any_element()
        };
        let titles = div()
            .flex()
            .items_start()
            .gap(px(S3))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(S1))
                    .child(
                        div()
                            .font_family(font())
                            .type_role(T_HEADLINE)
                            .font_weight(FontWeight::BOLD)
                            .text_color(text())
                            .child(line_with_tip("now-name", name.clone()).flex().min_w_0()),
                    )
                    .child(second),
            )
            .children(
                (!uri.is_empty())
                    .then(|| {
                        self.pill_heart(
                            &uri,
                            &name,
                            &artists,
                            Some(("panel-heart", super::tab::PANEL, window)),
                            cx,
                        )
                    })
                    .flatten()
                    .map(|heart| {
                        div()
                            .flex_none()
                            .h(px(T_HEADLINE.line))
                            .flex()
                            .items_center()
                            .child(heart)
                    }),
            )
            // Künye (bkz. `credits`): kalbin sağında, aynı hizada.
            .children(
                (!uri.is_empty())
                    .then(|| self.credits_button(window, cx))
                    .flatten()
                    .map(|button| {
                        div()
                            .flex_none()
                            .h(px(T_HEADLINE.line))
                            .flex()
                            .items_center()
                            .child(button)
                    }),
            );

        // Sütun panelin boyunu doldurur: söz kesiti kalan yüksekliği alır
        // (bkz. `now_lyrics`); söz yoksa hiçbir şey esnemez, sütun içeriği
        // kadar kalır.
        let lyrics = self.now_lyrics(panel_h, side, cx);
        let mut column = div()
            .flex_grow()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .pt(px(S1))
            .child(div().flex().justify_center().child(cover))
            .child(div().pt(px(S4)).child(titles));
        for block in [self.now_next(cx), lyrics, self.now_artist(&uri, cx)]
            .into_iter()
            .flatten()
        {
            column = column.child(block);
        }
        column.into_any_element()
    }

    /// Sıradaki şarkı; tıklayınca "Sıradaki" sekmesi.
    fn now_next(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.now.queue.upcoming.first()?.track.clone()?;
        let row = list_row("now-next", Look::Live)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Queue, cx)))
            .child(cover_image(
                &self.images,
                &t.image,
                &t.album,
                THUMB,
                R_SM,
                "music",
            ))
            .child(two_lines(
                "now-next-name",
                t.name.clone(),
                "now-next-artists",
                t.artists.clone(),
            ))
            .child(
                div()
                    .flex_none()
                    .type_role(T_LABEL)
                    .tabular()
                    .text_color(label())
                    .child(fmt_time(t.duration_ms)),
            );
        Some(section(t!(NowUpNext)).child(row).into_any_element())
    }

    /// Sözlerden okunan satır ve çevresi; tıklayınca "Sözler" sekmesi. Söz
    /// yoksa (ya da henüz gelmediyse) hiç yer kaplamaz. Panelin kalan
    /// yüksekliğini doldurur: okunan satır kesitin üçte birinde (sözler
    /// sekmesi gibi; şarkının başında en üstte), altındaki satırlar alt kenara
    /// doğru, önceki satırlar varsa üst kenara doğru solar. Kaç satır
    /// gösterileceği panelin boyundan hesaplanır (ölçüm ya da ek kare yok);
    /// kesit yalnız satır değişince çizilir (zamanlayıcı, `schedule_lyrics`).
    fn now_lyrics(&self, panel_h: f32, cover: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let LyricsStatus::Ready(lyrics) = &self.now.lyrics else {
            return None;
        };
        if lyrics.lines.is_empty() {
            return None;
        }
        let synced = lyrics.sync == SyncKind::Line;
        let active = if synced {
            let (clock, _) = self.lyric_clock(Instant::now());
            lyrics.line_at((clock + LYRIC_LEAD_MS).max(0) as u32)
        } else {
            None
        };
        let accent = self.accent().text;
        let has_next = self
            .now
            .queue
            .upcoming
            .first()
            .is_some_and(|e| e.track.is_some());
        let room = excerpt_room(panel_h, cover, has_next);
        let (above, below) = excerpt_split(
            active,
            lyrics.lines.len(),
            excerpt_above(room),
            excerpt_below(panel_h),
        );
        let line = |i: usize| {
            let content = match lyrics.lines[i].text.as_str() {
                "" => "• • •".to_string(),
                t => t.to_string(),
            };
            lyric_line(content, active == Some(i), !synced, accent)
        };
        // Önceki satırlar: içerikleri kadar yer (kesit kısaysa üstten
        // kırpılır), okunan satır hemen altlarında.
        let earlier = above.start > 0;
        let top = div()
            .flex_shrink()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_end()
            .gap(px(S2))
            .children(above.clone().map(line));
        let rest = div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(S2))
            .children(below.clone().map(line));
        let block = div()
            .id("now-lyrics")
            .group("now-lyrics")
            .relative()
            .flex_1()
            .min_h(px(EXCERPT_MIN))
            // Başlıktan ilk satıra "Sıradaki"deki gibi (satırın kendi 8 px'i
            // ve başlığın 8 px'i) 16 px.
            .pt(px(S2))
            .flex()
            .flex_col()
            .gap(px(S2))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Lyrics, cx)))
            .when(!above.is_empty(), |el| el.child(top))
            .children(active.map(line))
            .child(rest)
            .when(earlier, |el| el.child(excerpt_fade(true)))
            .when(!below.is_empty(), |el| el.child(excerpt_fade(false)));
        Some(
            section(t!(Lyrics))
                .flex_grow()
                .flex_shrink_0()
                .child(block)
                .into_any_element(),
        )
    }

    /// Sanatçı hakkında: yalnız önbellekte sayfası varsa (portre, ad,
    /// tanıtım yazısı). Tıklayınca sanatçı sayfası.
    fn now_artist(&mut self, track_uri: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (artist_uri, _) = self.pages.links.get(track_uri)?.artists.first()?.clone();
        let page = self.now_artist_page(&artist_uri, cx)?;
        let bio = short_bio(&page.bio, BIO_CHARS);
        let target = artist_uri.clone();
        let card = div()
            .id("now-artist")
            .mx(px(-S2))
            .p(px(S2))
            .rounded(px(R_LG))
            .flex()
            .flex_col()
            .gap(px(S3))
            .cursor_pointer()
            .child(super::tween::hover_fill(
                veil(VEIL_HOVER),
                veil(VEIL_PRESSED),
                0.,
                R_LG,
            ))
            .on_click(
                cx.listener(move |this, _, _, cx| this.navigate(Route::Artist(target.clone()), cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .child(cover_image(
                        &self.images,
                        &page.image,
                        &page.name,
                        S7,
                        S7 / 2.,
                        "user",
                    ))
                    .child(two_lines(
                        "now-artist-name",
                        page.name.clone(),
                        "now-artist-meta",
                        t!(NowArtistPage).into(),
                    )),
            )
            .when(!bio.is_empty(), |el| {
                el.child(div().type_role(T_SECONDARY).text_color(muted()).child(bio))
            });
        Some(section(t!(Artist)).child(card).into_any_element())
    }

    /// Sanatçı sayfası önbellekten: bellekte yoksa diske sanatçı başına bir
    /// kez arka plan yürütücüsünde bakılır (çizimi bekletmez; sayfa gelince
    /// panel yenilenir). Ağa istek gitmez.
    fn now_artist_page(&mut self, uri: &str, cx: &mut Context<Self>) -> Option<Arc<ArtistPage>> {
        if let Some(PageLoad::Ready(page)) = self.pages.artists.get(uri) {
            return Some(page.clone());
        }
        if !self.now.dock.artist_checked.insert(uri.to_string()) {
            return None;
        }
        let uri = uri.to_string();
        cx.spawn(async move |this, cx| {
            let hit = cx
                .background_executor()
                .spawn({
                    let uri = uri.clone();
                    async move { engine::cached_artist(&uri) }
                })
                .await;
            if let Some((page, _stale)) = hit {
                let _ = this.update(cx, |shell, cx| {
                    shell.store_artist(uri, page);
                    cx.notify();
                });
            }
        })
        .detach();
        None
    }

    /// Hiçbir şey yüklü değil: son çalınan listeler; tıklayınca çalar.
    fn now_recent(&self, recent: Vec<LibraryItem>, cx: &mut Context<Self>) -> AnyElement {
        let intro = div()
            .pt(px(S2))
            .flex()
            .flex_col()
            .gap(px(S1))
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(t!(NothingPlaying)),
            )
            .child(
                div()
                    .type_role(T_SECONDARY)
                    .text_color(muted())
                    .child(t!(NowRecentHint)),
            );
        let mut list = section(t!(NowRecentTitle));
        for (i, item) in recent.into_iter().enumerate() {
            let id = item.id.clone();
            let name = if item.id == CollectionId::Liked {
                t!(LikedSongs).to_string()
            } else {
                item.name.clone()
            };
            list = list.child(
                list_row(("now-recent", i), Look::Live)
                    .group("now-recent")
                    .on_click(cx.listener(move |this, _, _, cx| this.play_recent(id.clone(), cx)))
                    .child(self.collection_art(&item.id, &item.name, &item.image, THUMB, R_SM))
                    .child(two_lines(
                        ("now-recent-name", i),
                        name,
                        ("now-recent-sub", i),
                        crate::i18n::canon::localize(&item.subtitle).into_owned(),
                    ))
                    .child(
                        div()
                            .flex_none()
                            .opacity(0.)
                            .group_hover("now-recent", |s| s.opacity(1.))
                            .child(icon("play-fill", 16., soft())),
                    ),
            );
        }
        // Bölüm kendi üst boşluğuyla (24 px) gelir.
        div()
            .flex()
            .flex_col()
            .child(intro)
            .child(list)
            .into_any_element()
    }

    /// Son çalınan listeyi baştan çal (kart menüsündeki "Çal" gibi).
    fn play_recent(&mut self, id: CollectionId, cx: &mut Context<Self>) {
        self.select_collection(id.clone(), cx);
        if self.library.tracks(&id).is_some_and(|t| !t.is_empty()) {
            self.play_from_start(cx);
        } else {
            self.pages.play_when_loaded = Some(id);
        }
    }
}

/// Bölüm: başlık ve altında 8 px ile içerik; bölümler arası 24 px (7 Kasım,
/// kullanıcı geri bildirimi: eski 12 px soluk etiket küçük ve kopuk
/// duruyordu). Başlık Body (14 px), kalın, ön plan renginde: sayfalardaki
/// bölüm başlığının (Title) panel boyu. Title olsaydı altındaki söz
/// satırlarıyla (Title, Figtree 700) aynı boyda kalır, başlık okunmazdı.
fn section(title: &'static str) -> Div {
    div().pt(px(S5)).flex().flex_col().child(
        div()
            .flex_none()
            .pb(px(S2))
            .type_role(T_BODY)
            .font_weight(FontWeight::BOLD)
            .text_color(text())
            .child(title),
    )
}

/// Söz kesitinin bir satırı (Title, Figtree 700). Okunan satırın kapak ışığı
/// çizgisi solda, sütunun kenar boşluğunda (yazının 8 px solunda): yazının
/// kendisi bölüm başlığı ve şarkı adıyla aynı kenardan başlar.
fn lyric_line(content: String, active: bool, plain: bool, accent: u32) -> Div {
    div()
        .flex_none()
        .relative()
        .font_family(font())
        .type_role(T_TITLE)
        .font_weight(FontWeight::BOLD)
        .text_color(if active || plain { text() } else { muted() })
        .when(!active && !plain, |el| {
            el.group_hover("now-lyrics", |s| s.text_color(soft()))
        })
        .child(content)
        .when(active, |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(-S2))
                    .top(px(S1))
                    .bottom(px(S1))
                    .w(px(S0))
                    .rounded(px(R_XS))
                    .bg(rgb(accent)),
            )
        })
}

/// Söz kesitinin üst ya da alt kenarındaki solma: panel yüzeyinden (`BAR`)
/// saydama (sözler sekmesindeki gibi). Olay almaz: tıklama kesite geçer.
fn excerpt_fade(top: bool) -> Div {
    let solid = linear_color_stop(with_alpha(BAR, 1.), if top { 0. } else { 1. });
    let clear = linear_color_stop(with_alpha(BAR, 0.), if top { 1. } else { 0. });
    let (from, to) = if top { (solid, clear) } else { (clear, solid) };
    let fade = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(EXCERPT_FADE))
        .bg(linear_gradient(180., from, to));
    if top { fade.top_0() } else { fade.bottom_0() }
}

/// Tıklanabilen 56 px satır (sıra satırıyla aynı ölçüler); içeriği sütunun
/// kenarında, zemini iki yanda 8 px taşar. `look`: vitrinde sabit durum.
fn list_row(id: impl Into<gpui::ElementId>, look: Look) -> gpui::Stateful<Div> {
    let row = div()
        .id(id)
        .mx(px(-S2))
        .px(px(S2))
        .h(px(ROW_HEIGHT))
        .rounded(px(R_MD))
        .flex()
        .items_center()
        .gap(px(S3));
    match look {
        Look::Hover => row.bg(veil(VEIL_HOVER)),
        Look::Pressed => row.bg(veil(VEIL_PRESSED)),
        Look::Disabled => row,
        Look::Live | Look::Focus => row.cursor_pointer().child(super::tween::hover_fill(
            veil(VEIL_HOVER),
            veil(VEIL_PRESSED),
            0.,
            R_MD,
        )),
    }
}

impl Shell {
    // i18n-skip-start: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez
    /// Bileşen vitrini (`OPENIFY_GALLERY`): "Şimdi çalıyor" sekmesinin
    /// satırları (sıradaki, son çalınan) durumlarıyla ve söz kesiti.
    pub(super) fn now_panel_gallery(&self, _cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let caption = |name: &'static str| {
            div()
                .w(px(160.))
                .type_role(T_SECONDARY)
                .text_color(muted())
                .child(name)
        };
        let width = PANEL_WIDTH - 2. * S4;
        let mut rows = div().flex().flex_col().gap(px(S1));
        for (i, (name, look)) in [
            ("Normal", Look::Disabled),
            ("Üzerine gelme", Look::Hover),
            ("Basma", Look::Pressed),
        ]
        .into_iter()
        .enumerate()
        {
            let next = list_row(("g-now-next", i), look)
                .child(cover_image(&self.images, "", "Dal", THUMB, R_SM, "music"))
                .child(two_lines(
                    ("g-now-n", i),
                    "Dal".into(),
                    ("g-now-a", i),
                    "Adamlar".into(),
                ))
                .child(
                    div()
                        .flex_none()
                        .type_role(T_LABEL)
                        .tabular()
                        .text_color(label())
                        .child("3:41"),
                );
            let recent = list_row(("g-now-recent", i), look)
                .child(self.collection_art(&CollectionId::Liked, t!(LikedSongs), "", THUMB, R_SM))
                .child(two_lines(
                    ("g-now-rn", i),
                    t!(LikedSongs).into(),
                    ("g-now-rs", i),
                    "326 şarkı".into(),
                ))
                .child(
                    div()
                        .flex_none()
                        .when(look != Look::Hover, |el| el.opacity(0.))
                        .child(icon("play-fill", 16., soft())),
                );
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S5))
                    .child(caption(name))
                    .child(div().w(px(width)).child(next))
                    .child(div().w(px(width)).child(recent)),
            );
        }
        let excerpt = div().w(px(width)).flex().flex_col().gap(px(S2)).children(
            [
                ("Gece uzun, şehir sessiz", false),
                ("Bir şarkı çalıyor eski bir radyoda", true),
                ("Sözler akıyor satır satır", false),
            ]
            .map(|(t, active)| lyric_line(t.to_string(), active, false, accent)),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(S3))
            .child(
                div()
                    .type_role(T_LABEL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(label())
                    .child("Şimdi çalıyor sekmesi: sıradaki ve son çalınan satırları, söz kesiti"),
            )
            .child(rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(S5))
                    .child(caption("Söz kesiti"))
                    .child(excerpt),
            )
            .into_any_element()
    }
    // i18n-skip-end: geliştirme vitrini (OPENIFY_GALLERY), kullanıcıya görünmez
}

/// Ad (Body) ve altında ikincil satır; ikisi de tek satırda kısalır.
fn two_lines(
    first_id: impl Into<gpui::ElementId>,
    first: String,
    second_id: impl Into<gpui::ElementId>,
    second: String,
) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .type_role(T_BODY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(text())
                .child(line_with_tip(first_id, first).flex().min_w_0()),
        )
        .child(
            div()
                .type_role(T_SECONDARY)
                .text_color(muted())
                .child(line_with_tip(second_id, second).flex().min_w_0()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_breakpoint_keeps_a_wide_list_beside_the_panel() {
        assert_eq!(DOCK_MIN_WIDTH, 1720.);
        // Eşikte liste (satır içeriği) 1240 px: sol kenar + liste + sağ kenar.
        let at = dock_layout(DOCK_MIN_WIDTH, true);
        assert!(at.docked);
        assert_eq!(at.main_width - 2. * PAGE_PAD, DOCK_LIST_MIN);
        assert_eq!((at.panel_width, at.panel_right), (DOCK_PANEL_WIDTH, S4));
        // Bir piksel darda panel yüzer, içerik pencerenin tamamı.
        let below = dock_layout(DOCK_MIN_WIDTH - 1., true);
        assert!(!below.docked);
        assert_eq!(below.main_width, DOCK_MIN_WIDTH - 1.);
        // Yüzen panel hep 360, sağda 16.
        assert_eq!((below.panel_width, below.panel_right), (PANEL_WIDTH, S4));
        assert!(dock_layout(1890., true).docked);
        assert!(!dock_layout(1600., true).docked);
    }

    #[test]
    fn list_fills_the_window_and_the_panel_sits_at_the_right_edge() {
        // Kullanıcı geri bildirimi (7 Kasım): sağda ölü boşluk yok. Liste
        // panelin solundaki bütün yeri alır, panel sağ kenardan 16 px
        // içeride, aralarında listenin sağ sayfa kenarı (32) kadar; panel
        // eni her pencerede 400.
        for w in [
            1720., 1721.5, 1760., 1800., 1890., 1920., 2200., 2560., 3840.,
        ] {
            let l = dock_layout(w, true);
            assert!(l.docked, "{w}");
            assert_eq!(
                (l.panel_width, l.panel_right),
                (DOCK_PANEL_WIDTH, S4),
                "{w}"
            );
            assert!(
                (l.main_width + l.panel_width + l.panel_right - w).abs() < 1e-3,
                "{w}"
            );
            let list_right = l.main_width - PAGE_PAD;
            let panel_left = w - l.panel_right - l.panel_width;
            assert_eq!(panel_left - list_right, PAGE_PAD, "{w}");
        }
        // Panel kapalıyken (ya da gösterecek bir şey yokken) liste
        // pencerenin tamamında: içerik ortalanmaz, sağda boşluk kalmaz.
        for w in [1180., 1720., 1920., 2560.] {
            let l = dock_layout(w, false);
            assert!(!l.docked);
            assert_eq!(l.main_width, w);
        }
    }

    #[test]
    fn pill_stays_centered_on_the_window_and_clears_the_panel() {
        // Panel açılıp kapanınca hap yerinden oynamaz: hep pencerenin ortası.
        for w in [1180., 1720., 1890., 1920., 2560.] {
            for open in [false, true] {
                let (left, width) = dock_layout(w, open).pill_frame(w);
                assert!((left + width / 2. - w / 2.).abs() <= 0.5, "{w} {open}");
            }
        }
        // Eşikten geniş her pencerede hap tam enli (800) ve yerleşik panele
        // 24 px'ten fazla yaklaşmaz (eşikte arada 44 px).
        let mut w = DOCK_MIN_WIDTH;
        while w <= 3840. {
            let l = dock_layout(w, true);
            let (left, width) = l.pill_frame(w);
            assert_eq!(width, pill_width(w), "{w}");
            let panel_left = w - l.panel_right - l.panel_width;
            assert!(left + width + S5 <= panel_left, "{w}");
            w += 7.5;
        }
        let l = dock_layout(DOCK_MIN_WIDTH, true);
        let (left, width) = l.pill_frame(DOCK_MIN_WIDTH);
        assert_eq!(
            DOCK_MIN_WIDTH - l.panel_right - l.panel_width - (left + width),
            44.
        );
        // Panel daha geniş olsaydı hap daralırdı (dar hap kuralları o enle):
        // ortası yine pencerenin ortası, panele 24 px.
        let wide_panel = DockLayout {
            docked: true,
            main_width: 1100.,
            panel_width: 604.,
            panel_right: S4,
            beside_pill: true,
        };
        let (left, width) = wide_panel.pill_frame(1720.);
        assert!(width < pill_width(1720.));
        assert_eq!(left + width + S5, 1720. - S4 - 604.);
        assert_eq!(left + width / 2., 860.);
    }

    #[test]
    fn panel_opens_and_hides_with_the_window_width() {
        use DockChange::*;
        // Açılış: geniş pencere, gösterilecek şey var, tercih açık.
        assert_eq!(dock_change(None, true, false, true, true, false), Open);
        // Kullanıcı kapattı (tercih kapalı): açılmaz.
        assert_eq!(dock_change(None, true, false, true, false, false), Keep);
        // Hiçbir şey yok: panel kapalı kalır (içerik ortalanır).
        assert_eq!(dock_change(None, true, false, false, true, false), Keep);
        // İlk şarkı / ilk son çalınan gelince açılır.
        assert_eq!(
            dock_change(Some(true), true, false, true, true, false),
            Open
        );
        // Pencere daralınca yerleşik panel kalkar, genişleyince geri gelir.
        assert_eq!(dock_change(Some(true), false, true, true, true, true), Hide);
        assert_eq!(
            dock_change(Some(false), true, true, true, true, false),
            Open
        );
        // Dar pencerede kullanıcının açtığı yüzen panele dokunulmaz.
        assert_eq!(
            dock_change(Some(false), false, true, true, true, true),
            Keep
        );
        // Geniş pencerede aynı durum sürerken hiçbir şey değişmez.
        assert_eq!(dock_change(Some(true), true, true, true, true, true), Keep);
        assert_eq!(dock_change(Some(true), true, true, true, true, false), Keep);
    }

    #[test]
    fn cover_shrinks_in_short_panels() {
        let narrow = now_cover_max(PANEL_WIDTH);
        assert_eq!(narrow, 326.);
        // 1180×760 ve üstü: panelin iç eni.
        assert_eq!(now_cover_side(PANEL_WIDTH, 588., false), narrow);
        assert_eq!(now_cover_side(PANEL_WIDTH, 1004., false), narrow);
        // 820×600 (panel 428 px): ad ve sanatçı kaydırmadan görünür.
        let side = now_cover_side(PANEL_WIDTH, 428., false);
        assert!(side < narrow);
        assert!(ROW_HEIGHT + 2. + S1 + side + S4 + TITLE_BLOCK <= 428.);
        // Çok kısa pencerede en az 160.
        assert_eq!(now_cover_side(PANEL_WIDTH, 200., false), NOW_COVER_MIN);
        // Sıradaki şarkı varsa: 900×700'de (panel 527) kapak küçülür ki
        // "Sıradaki" başlığı ve bir satır altta 12 px payla sığsın; 1180×760'ta
        // (588) kapak zaten en büyük, değişmez.
        assert_eq!(now_cover_side(PANEL_WIDTH, 527., true), 267.);
        assert_eq!(now_cover_side(PANEL_WIDTH, 588., true), 326.);
        // Sıradaki varken kapak 224'ün altına inecekse "Sıradaki" kaydırmaya
        // kalır, kapak sıradaki yokkenki boyunda durur.
        assert_eq!(
            now_cover_side(PANEL_WIDTH, 428., true),
            now_cover_side(PANEL_WIDTH, 428., false)
        );
    }

    #[test]
    fn docked_cover_never_upscales_640() {
        // 1920×1080 yerleşik panel (400, ~1004 px yüksek): kapak 366.
        let wide = now_cover_side(DOCK_PANEL_WIDTH, 1004., false);
        assert_eq!(wide, 366.);
        // %150 ölçekte 549, %175'te ~640 fiziksel px: kaynak büyütülmez.
        assert!(now_cover_max(DOCK_PANEL_WIDTH) * 1.5 <= 640.);
        assert!(now_cover_max(DOCK_PANEL_WIDTH) * 1.75 <= 641.);
        // Kısa ama geniş pencerede yine küçülür.
        assert!(now_cover_side(DOCK_PANEL_WIDTH, 428., false) < wide);
    }

    #[test]
    fn empty_state_prefers_track_then_recent() {
        assert_eq!(now_body_kind(true, 0), NowBody::Track);
        assert_eq!(now_body_kind(true, 5), NowBody::Track);
        assert_eq!(now_body_kind(false, 3), NowBody::Recent);
        assert_eq!(now_body_kind(false, 0), NowBody::Empty);
    }

    #[test]
    fn lyric_excerpt_holds_the_active_line_a_third_down() {
        // Okunan satırın üstünde en çok `above`, altında en çok `below`.
        assert_eq!(excerpt_split(Some(10), 40, 4, 20), (6..10, 11..31));
        // Şarkının başında okunan satır en üstte; sonunda alt boş kalır.
        assert_eq!(excerpt_split(Some(0), 40, 4, 20), (0..0, 1..21));
        assert_eq!(excerpt_split(Some(2), 40, 4, 20), (0..2, 3..23));
        assert_eq!(excerpt_split(Some(39), 40, 4, 20), (35..39, 40..40));
        // Okunan satır yok (ilk satırdan önce, eşzamansız): baştan.
        assert_eq!(excerpt_split(None, 40, 4, 20), (0..0, 0..20));
        assert_eq!(excerpt_split(None, 3, 4, 20), (0..0, 0..3));
        assert_eq!(excerpt_split(None, 0, 4, 20), (0..0, 0..0));
    }

    #[test]
    fn lyric_excerpt_fills_the_remaining_height() {
        // 1920×1080 yerleşik panel (~1004 px, kapak 366): kesite ~420 px
        // (ad 28 px, sanatçı 18 px oldu: eskisinden 18 px kısa), üstte 4
        // önceki satır (üçte bir), altta panel boyu kadar satır.
        let room = excerpt_room(1004., 366., false);
        assert!((380. ..460.).contains(&room), "{room}");
        assert_eq!(excerpt_above(room), 4);
        assert!(excerpt_below(1004.) as f32 * EXCERPT_STEP >= 1004.);
        // Sıradaki şarkı varsa kesit o kadar kısa.
        assert_eq!(
            excerpt_room(1004., 366., true),
            room - (S5 + T_BODY.line + S2 + ROW_HEIGHT)
        );
        // Kısa yüzen panelde (1180×760) en az: okunan satır ve iki yanında
        // solmanın dışında birer tam satır; üstte bir önceki satır.
        let short = excerpt_room(588., now_cover_side(PANEL_WIDTH, 588., false), true);
        assert_eq!(short, EXCERPT_MIN);
        assert_eq!(EXCERPT_MIN, 160.);
        assert!(excerpt_above(short) >= 1);
    }

    #[test]
    fn bio_is_cut_on_a_word_with_an_ellipsis() {
        assert_eq!(short_bio("Kısa  yazı\n", 50), "Kısa yazı");
        let long = "Şarkıcı ve söz yazarı, İstanbul'da doğdu. ".repeat(10);
        let cut = short_bio(&long, 60);
        assert!(cut.ends_with('…'));
        assert!(cut.chars().count() <= 61);
        assert!(!cut.contains("  "));
        // Çok baytlı harfin ortasından kesmez.
        assert_eq!(short_bio(&"ğ".repeat(10), 4), "ğğğğ…");
    }

    #[test]
    fn dock_slide_pushes_the_content_and_ends_at_the_static_layouts() {
        let w = 1920.;
        assert_eq!(dock_layout_at(w, true, 1.), dock_layout(w, true));
        assert_eq!(dock_layout_at(w, false, 0.), dock_layout(w, false));
        // Yarıda: panelin sol kenarı içeriğin sağ kenarında (itiyor), içerik
        // iki ucun arasında, panel pencerenin içinde.
        let half = dock_layout_at(w, false, 0.5);
        assert!(half.docked);
        assert!(half.main_width < w && half.main_width > w - DOCK_PANEL_WIDTH - S4);
        let panel_left = w - half.panel_right - half.panel_width;
        assert!(
            (panel_left - half.main_width).abs() <= 0.5,
            "{panel_left} {}",
            half.main_width
        );
        // Dar pencerede içerik hiç oynamaz (panel yüzer).
        let narrow = dock_layout_at(DOCK_MIN_WIDTH - 1., true, 0.5);
        assert!(!narrow.docked);
        assert_eq!(narrow.main_width, DOCK_MIN_WIDTH - 1.);
    }

    #[test]
    fn slide_runs_once_then_stops_asking_for_frames() {
        use std::time::Duration;
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut s = Slide::new();
        // İlk kare: hareketsiz, doğrudan hedefte.
        assert!(!s.advance(t0, true, Some(Panel::Queue), true));
        assert_eq!(s.value, 1.);
        // Kapanış başlar: hâlâ tam açık, kapanan panel çizilmeye devam eder.
        assert!(s.advance(t0, true, None, true));
        assert_eq!((s.value, s.exiting), (1., Some(Panel::Queue)));
        // Yarıda: ara değer, kare istenir.
        assert!(s.advance(t0 + motion_exit() / 2, true, None, true));
        assert!(s.value > 0. && s.value < 1.);
        // Bitti: 0, kare istenmez, kapanan panel bırakıldı.
        assert!(!s.advance(t0 + motion_exit() + ms(5), true, None, true));
        assert_eq!((s.value, s.exiting), (0., None));
        // Daha sonraki kareler de istemez.
        assert!(!s.advance(t0 + ms(2000), true, None, true));
        // Açılış: yavaşlayarak gelir; yarıda ara değer, sonunda 1.
        assert!(s.advance(t0 + ms(3000), true, Some(Panel::Now), true));
        assert!(s.advance(t0 + ms(3000) + motion() / 2, true, Some(Panel::Now), true));
        assert!(
            s.value > 0.5 && s.value < 1.,
            "ease-out: yarı sürede yarıdan fazla yol {}",
            s.value
        );
        assert!(!s.advance(
            t0 + ms(3000) + motion() + ms(5),
            true,
            Some(Panel::Now),
            true
        ));
        assert_eq!(s.value, 1.);
        // Pencere genişliği durumu değişince ve hareket kapalıyken atlar.
        assert!(!s.advance(t0 + ms(4000), false, None, true));
        assert_eq!((s.value, s.exiting), (0., None));
        assert!(!s.advance(t0 + ms(5000), false, Some(Panel::Now), false));
        assert_eq!(s.value, 1.);
    }
}
