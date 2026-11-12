//! Arkadaş etkinliği paneli (Spotify masaüstündeki "Friend Activity"): arkadaşların
//! son dinlediği şarkı. Veri `engine::friends`'ten gelir (gayriresmî uç, canlı
//! yanıtla doğrulanmadı); burada panel, yenileme kuralları ve sağ tık menüsü var.
//!
//! Yerleşim (karar, "kaplı liste" v2): panel, "Şimdi çalıyor / Sıradaki /
//! Sözler" panelinin AYNI sağ yuvasında dördüncü bir panel türüdür
//! (`Panel::Friends`). İçerik alanı en dar pencere eninin (760) altına
//! inmedikçe sayfanın YANINA yerleşir (360 px; ≥ 1136 px pencere: 1180×760
//! dahil), içeriği daraltır, örtmez; daha darda (900×700) içeriğin üstünde
//! yüzer (aynı yüzey ve gölge). Kısa bir kart: yüksekliği içerik kadar
//! (uzunsa kendi kaydırması), hapın üstünde biter ve hapı daraltmaz
//! (`now_panel::DockKind::FRIENDS`). Kayma, Esc (yüzende), gölge ve Tab düzeni
//! öbür panelle ortak; iki panel aynı yuvayı paylaşır, biri açılınca öbürü
//! kapanır. Üç sekmenin başlık şeridine dördüncü bir hap sığmıyor, bu yüzden
//! arkadaş paneli kendi başlığıyla açılır ve başlık çubuğundaki "Arkadaş
//! etkinliği" düğmesiyle (ayar açıkken görünür) açılıp kapanır. Kapanınca,
//! yerleşik "Şimdi çalıyor" paneli açık tercihteyse yerine o döner. Panelin
//! açılıp kapanması "panel açık başlasın" tercihini değiştirmez.
//!
//! İçerik: "Şimdi dinliyor" (canlı, `is_live`) ve "Daha önce" grupları (boş
//! grup yok); her arkadaş iki satırlık bir blok: A) profil resmi, ad, zaman;
//! B) şarkı kapağı, şarkı, sanatçı, varsa bağlam (liste / albüm). Her yer
//! tıklanır: blok ve kapak çalar, ad menüyü açar, sanatçı ve bağlam sayfasına
//! gider.
//!
//! Varsayılan KAPALI (`Settings::friend_activity`): kapalıyken ne düğme var ne
//! de Spotify'a tek istek gider.
//!
//! Yenileme (`Schedule`, saf, sınanır): yalnız ayar açık, panel ekranda ve
//! pencere tepsiye gizli değilken, 60 sn'de bir; gizlenince ya da panel
//! kapanınca durur (kaldığı yerden: yakın zamanda alındıysa yeniden
//! açılışta hemen sorulmaz). Hata ya da 429'da aralık katlanarak (en çok 10 dk)
//! uzar; 429'da en az 5 dk, 401 / 403 / 404'te 10 dk. Sonuçlar yalnız bellekte.
//!
//! Sınama kancası: `OPENIFY_TEST_FRIENDS_JSON=<dosya>` (ya da `!error`,
//! `!busy`, `!denied`, `!loading`) listeyi fikstürden doldurur, ağa hiç
//! çıkmaz, ayarı bellekte açar (yalnız sınama koşusunda: `OPENIFY_TEST_DISPLAY`
//! ya da `OPENIFY_SETTINGS_PATH` varken). Fikstürdeki `ageSeconds` ("şu kadar
//! sn önce") kancaya özgüdür; bkz. `engine/fixtures/friend_activity_demo.json`.
//! `OPENIFY_TEST_VIEW=friends` paneli açık getirir.

use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AnyElement, ClickEvent, Context, FocusHandle, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    Point, ScrollHandle, SharedString, Window, canvas, div, point, prelude::*, px, rgb,
};

use super::{
    Shell,
    added_date::{date_label, local_offset_minutes},
    components::{Look, Tooltip, icon_button, state_view},
    cover_image,
    history::Route,
    library::ROW_HEIGHT,
    link_text::{SpanHandler, link_line},
    manage::{Act, Entry},
    now_playing::Panel,
    pill::toggle_button,
    shortcuts::Shortcut,
    text::line_with_tip,
    theme::*,
};
use crate::engine::{self, Command, Friend, FriendsFailure, LibraryCommand, PlayPlan};
use crate::{t, tf};

/// Yenileme aralığı: panel ve pencere görünürken.
pub(super) const REFRESH: Duration = Duration::from_secs(60);
/// Hata sonrası bekleme en çok bu kadar.
const BACKOFF_MAX: Duration = Duration::from_secs(600);
/// 429'dan sonra en az bu kadar.
const RATE_LIMIT_MIN: Duration = Duration::from_secs(300);
/// Cevabı bu kadar sürede gelmeyen istek kayıp sayılır (yenisi sorulabilir).
const IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(90);
/// Bu kadar yeni dinleme "şu an dinliyor" sayılır. Yanıt şarkının süresini
/// taşımıyor: ortalama bir şarkı gibi 3 dk, kaba bir tahmin.
pub(super) const LIVE_WINDOW_MS: i64 = 3 * 60_000;
/// Profil resmi (A satırı).
const AVATAR: f32 = S6;
/// Şarkı kapağı (B satırı): üç satırlık yazı bloğu (54 px) 40 px'lik kapağı
/// aşıyordu, kapak bir kademe büyük.
const COVER: f32 = S7;
/// Bloğun iç dikey boşluğu (6 px boşluk + 2 px odak kenarlığı).
const ENTRY_PAD: f32 = S2;
/// A satırı ile B satırı arası.
const LINE_GAP: f32 = S1;
/// Şarkı + sanatçı + bağlam yazılarının toplam yüksekliği.
const TEXT_LINES_FULL: f32 = T_BODY.line + T_SECONDARY.line + T_LABEL.line;
/// Grup başlığının yüksekliği; ikinci ve sonraki gruplarda üstte ek boşluk.
const HEAD_HEIGHT: f32 = S6;
const HEAD_GAP: f32 = S3;
/// Gövdenin üst ve alt iç boşluğu.
const BODY_TOP: f32 = S1;
const BODY_BOTTOM: f32 = S2;
/// Panel çerçevesi (iki yanda 1 px kenarlık).
const BORDER: f32 = 2.;
/// Durum ekranlarının (boş, yükleniyor, hata) gövde yüksekliği.
const STATE_BODY: f32 = 208.;
/// Baş harf dairesinin ton çemberindeki adım sayısı (15°'lik adımlar).
const HUE_STEPS: u32 = 24;

// ── Yenileme kuralları (saf) ────────────────────────────────────────────

/// Bir sonraki adım.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// Şimdi sor.
    Fetch,
    /// Bu kadar sonra yeniden bak.
    Wait(Duration),
    /// Yenileme istenmiyor (kapalı, panel ya da pencere görünür değil).
    Idle,
}

/// Yenileme zamanlayıcısının durumu. Zaman dışarıdan verilir.
#[derive(Debug, Clone, Default)]
pub(super) struct Schedule {
    /// Sıradaki isteğin en erken anı; `None`: hiç sorulmadı (hemen).
    next_due: Option<Instant>,
    /// Yoldaki isteğin başladığı an.
    started: Option<Instant>,
    /// Üst üste hata sayısı.
    failures: u32,
}

impl Schedule {
    /// `wanted`: ayar açık, panel ekranda ve pencere görünür. Görünür değilken
    /// hiçbir şey sorulmaz; `next_due` korunur (yeniden görününce, süre
    /// dolduysa hemen, dolmadıysa kalanı kadar sonra).
    pub(super) fn step(&self, wanted: bool, now: Instant) -> Step {
        if !wanted {
            return Step::Idle;
        }
        if let Some(started) = self.started {
            let lost_at = started + IN_FLIGHT_TIMEOUT;
            if now < lost_at {
                return Step::Wait(lost_at - now);
            }
        }
        match self.next_due {
            Some(due) if due > now => Step::Wait(due - now),
            _ => Step::Fetch,
        }
    }

    pub(super) fn started(&mut self, now: Instant) {
        self.started = Some(now);
    }

    pub(super) fn succeeded(&mut self, now: Instant) {
        self.started = None;
        self.failures = 0;
        self.next_due = Some(now + REFRESH);
    }

    pub(super) fn failed(&mut self, now: Instant, why: FriendsFailure) {
        self.started = None;
        self.failures = self.failures.saturating_add(1);
        self.next_due = Some(now + retry_delay(self.failures, why));
    }

    /// Ayar kapandı: bir sonraki açılışta temiz başla.
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// `n`. üst üste hatadan sonra bekleme: aralık her hatada katlanır (2, 4, 8 dk
/// …), en çok 10 dk; 429'da en az 5 dk; uç bu hesaba kapalıysa (401 / 403 / 404)
/// hemen en uzun bekleme.
pub(super) fn retry_delay(failures: u32, why: FriendsFailure) -> Duration {
    let doubled = REFRESH.saturating_mul(1u32 << failures.clamp(1, 8).min(8));
    match why {
        FriendsFailure::Denied | FriendsFailure::Unavailable => BACKOFF_MAX,
        FriendsFailure::RateLimited => doubled.clamp(RATE_LIMIT_MIN, BACKOFF_MAX),
        FriendsFailure::Offline => doubled.min(BACKOFF_MAX),
    }
}

// ── Zaman yazısı (saf) ──────────────────────────────────────────────────

/// Dinlemenin ne kadar önce olduğu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Age {
    Now,
    Minutes(u32),
    Hours(u32),
    Days(u32),
    Weeks(u32),
}

/// `age_ms`: şimdi eksi dinleme anı (saat farkından eksi çıkarsa "şimdi").
pub(super) fn age_of(age_ms: i64) -> Age {
    let secs = (age_ms / 1000).max(0);
    match secs {
        0..=59 => Age::Now,
        60..=3599 => Age::Minutes((secs / 60) as u32),
        3600..=86_399 => Age::Hours((secs / 3600) as u32),
        86_400..=604_799 => Age::Days((secs / 86_400) as u32),
        _ => Age::Weeks((secs / 604_800).min(99) as u32),
    }
}

pub(super) fn age_label(age: Age) -> String {
    match age {
        Age::Now => t!(FriendsAgoNow).to_string(),
        Age::Minutes(n) => tf!(FriendsAgoMinutes, n),
        Age::Hours(n) => tf!(FriendsAgoHours, n),
        Age::Days(n) => tf!(FriendsAgoDays, n),
        Age::Weeks(n) => tf!(FriendsAgoWeeks, n),
    }
}

/// Şu an dinliyor mu (kaba tahmin, bkz. `LIVE_WINDOW_MS`)? Zamanı bilinmeyen
/// (0) arkadaş canlı sayılmaz.
pub(super) fn is_live(timestamp_ms: i64, now_ms: i64) -> bool {
    timestamp_ms > 0 && (0..=LIVE_WINDOW_MS).contains(&(now_ms - timestamp_ms))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ── Gövdenin hâli (saf) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Body {
    /// Arkadaş satırları.
    Rows,
    /// Gösterilecek kimse yok (tek satır sessiz yazı).
    Empty,
    /// İlk cevap bekleniyor.
    Loading,
    /// Alınamadı ve gösterilecek eski liste de yok.
    Failed(FriendsFailure),
}

/// `known`: elde bir liste var mı ve kaç arkadaşlı; `failure`: son deneme
/// başarısız mı. Eski bir liste varsa hata sırasında o gösterilmeye devam eder.
pub(super) fn body_of(known: Option<usize>, failure: Option<FriendsFailure>) -> Body {
    match (known, failure) {
        (Some(n), _) if n > 0 => Body::Rows,
        (_, Some(why)) => Body::Failed(why),
        (Some(_), None) => Body::Empty,
        (None, None) => Body::Loading,
    }
}

/// Başlık yazısı: kimliği olmayan satır adı yerine boş bırakmaz.
fn initial_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

// ── Gruplar ve yükseklik (saf) ──────────────────────────────────────────

/// Listenin iki grubu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    /// Şu an dinleyenler (`is_live`).
    Live,
    /// Daha önce dinleyenler.
    Earlier,
}

/// Panelin gövdesindeki sıra: grup başlığı ya da bir arkadaş (listedeki sırası).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Item {
    Head(Group),
    Entry(usize),
}

/// Listeyi gruplara böler: önce "Şimdi dinliyor", sonra "Daha önce"; her grupta
/// listedeki (en yeni önde) sıra korunur; boş grubun başlığı yoktur.
pub(super) fn plan_items(list: &[Friend], now_ms: i64) -> Vec<Item> {
    let (live, earlier): (Vec<usize>, Vec<usize>) =
        (0..list.len()).partition(|&ix| is_live(list[ix].timestamp_ms, now_ms));
    let mut items = Vec::new();
    for (group, members) in [(Group::Live, live), (Group::Earlier, earlier)] {
        if members.is_empty() {
            continue;
        }
        items.push(Item::Head(group));
        items.extend(members.into_iter().map(Item::Entry));
    }
    items
}

/// Bağlamın adı (varsa); adı olmayan bağlam hiç gösterilmez.
fn context_name(f: &Friend) -> Option<&str> {
    f.context_name.as_deref().filter(|n| !n.is_empty())
}

/// Bağlamın türü: simgesi ve tıklanınca nereye gittiği.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContextKind {
    Playlist,
    Album,
    Artist,
    /// Beğenilenler, radyo, kullanıcı koleksiyonu …: sayfası uygulamada yok.
    Other,
}

pub(super) fn context_kind(uri: &str) -> ContextKind {
    if uri.starts_with("spotify:playlist:") {
        ContextKind::Playlist
    } else if uri.starts_with("spotify:album:") {
        ContextKind::Album
    } else if uri.starts_with("spotify:artist:") {
        ContextKind::Artist
    } else {
        ContextKind::Other
    }
}

fn context_icon(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::Playlist => "list-music",
        ContextKind::Album => "disc-3",
        ContextKind::Artist => "user",
        ContextKind::Other => "music",
    }
}

/// Bir arkadaş bloğunun yüksekliği: dikey boşluk, A satırı, B satırı (bağlam
/// satırı varsa yazılar kapaktan yüksek).
pub(super) fn entry_height(has_context: bool) -> f32 {
    2. * ENTRY_PAD + AVATAR + LINE_GAP + if has_context { TEXT_LINES_FULL } else { COVER }
}

fn item_height(item: Item, first: bool, list: &[Friend]) -> f32 {
    match item {
        Item::Head(_) => HEAD_HEIGHT + if first { 0. } else { HEAD_GAP },
        Item::Entry(ix) => entry_height(list.get(ix).is_some_and(|f| context_name(f).is_some())),
    }
}

/// Sıradaki öğelerin toplam yüksekliği.
pub(super) fn items_height(items: &[Item], list: &[Friend]) -> f32 {
    items.iter().enumerate().map(|(i, &item)| item_height(item, i == 0, list)).sum()
}

/// `ix`. arkadaşın bloğunun gövdenin üstünden uzaklığı (menü konumu için).
fn entry_top(items: &[Item], list: &[Friend], ix: usize) -> f32 {
    let at = items.iter().position(|&i| i == Item::Entry(ix)).unwrap_or(0);
    BODY_TOP + items_height(&items[..at], list)
}

/// Panelin yüksekliği: çerçeve + başlık şeridi + gövde, en çok `room` (uzunsa
/// gövde kendi içinde kayar). Durum ekranlarında sabit, kısa bir kart.
pub(super) fn panel_height(body: Body, items_h: f32, room: f32) -> f32 {
    let inner = match body {
        Body::Rows => BODY_TOP + items_h + BODY_BOTTOM,
        _ => STATE_BODY,
    };
    (BORDER + ROW_HEIGHT + inner).min(room)
}

// ── Profil resmi yokken baş harf dairesi (saf) ──────────────────────────

/// Addan türeyen ton: 24 eşit adımlı ton çemberinden biri (0–23). Aynı ad hep
/// aynı tonu verir (FNV-1a, büyük / küçük harf fark etmez; sürümler arası
/// değişmez).
pub(super) fn avatar_hue_step(name: &str) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let mut buf = [0u8; 4];
        for b in c.encode_utf8(&mut buf).bytes() {
            hash ^= u32::from(b);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    hash % HUE_STEPS
}

/// (daire zemini, baş harf rengi): zemin yüzey tonuna doygun olmayan bir ton
/// karışımı, harf aynı tonun açığı. Gökkuşağı değil: doygunluk ve karışım
/// düşük.
pub(super) fn avatar_colors(name: &str) -> (u32, u32) {
    let hue = avatar_hue_step(name) as f32 * (360. / HUE_STEPS as f32);
    (mix(PLACEHOLDER, hsl_color(hue, 0.55, 0.52), 0.30), hsl_color(hue, 0.55, 0.82))
}

// ── Zaman yazısı (mutlak) ───────────────────────────────────────────────

/// "14:32" (yerel saat; `offset_minutes` UTC'den dakika).
fn clock_label(at_ms: i64, offset_minutes: i64) -> String {
    let minutes = (at_ms.div_euclid(1000) / 60 + offset_minutes).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// "Nov 12, 2026, 14:32" / "12 Kas 2026 14:32": zamanın üzerine gelince.
fn absolute_label_in(at_ms: i64, offset_minutes: i64) -> String {
    tf!(FriendsTimeAt, date_label(at_ms, offset_minutes), clock_label(at_ms, offset_minutes))
}

// ── Durum ───────────────────────────────────────────────────────────────

pub(super) struct FriendsUi {
    /// Son başarılı liste (bellekte; diske yazılmaz).
    pub list: Option<Arc<Vec<Friend>>>,
    /// Son deneme başarısız (eski liste varsa o gösterilir).
    pub failure: Option<FriendsFailure>,
    pub schedule: Schedule,
    timer: Option<gpui::Task<()>>,
    pub scroll: ScrollHandle,
    /// Sınama kancası: fikstürden dolduruldu, ağa çıkılmaz.
    pub fixture: bool,
}

impl FriendsUi {
    pub(super) fn new() -> Self {
        Self { list: None, failure: None, schedule: Schedule::default(), timer: None, scroll: ScrollHandle::new(), fixture: false }
    }

    fn friend(&self, ix: usize) -> Option<Friend> {
        self.list.as_ref().and_then(|l| l.get(ix)).cloned()
    }

    fn body(&self) -> Body {
        body_of(self.list.as_ref().map(|l| l.len()), self.failure)
    }
}

// ── Sınama kancası ──────────────────────────────────────────────────────

/// Sınama koşusu mu? (Kanca gerçek ayar dosyasını etkileyemesin.)
fn test_run() -> bool {
    std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some()
}

/// Kancanın doldurduğu durum.
#[derive(Debug, PartialEq)]
pub(super) enum Fixture {
    Feed(Vec<Friend>),
    Failed(FriendsFailure),
    /// Cevap hiç gelmez (yükleniyor hâli).
    Loading,
}

/// `OPENIFY_TEST_FRIENDS_JSON` değeri: dosya yolu ya da `!error|!busy|!denied|!loading`.
pub(super) fn load_fixture(spec: &str, now_ms: i64) -> Option<Fixture> {
    match spec {
        "!error" => return Some(Fixture::Failed(FriendsFailure::Offline)),
        "!busy" => return Some(Fixture::Failed(FriendsFailure::RateLimited)),
        "!denied" => return Some(Fixture::Failed(FriendsFailure::Denied)),
        "!loading" => return Some(Fixture::Loading),
        _ => {}
    }
    let text = std::fs::read_to_string(spec).ok()?;
    let mut value: serde_json::Value = serde_json::from_str(&text).ok()?;
    // `ageSeconds` → `timestamp` (göreli zaman sınamada hep aynı görünsün).
    if let Some(friends) = value.get_mut("friends").and_then(|f| f.as_array_mut()) {
        for friend in friends {
            if let Some(age) = friend.get("ageSeconds").and_then(|a| a.as_i64()) {
                friend["timestamp"] = serde_json::json!(now_ms - age * 1000);
            }
        }
    }
    Some(Fixture::Feed(engine::parse_friends(&value)))
}

impl Shell {
    /// Açılışta: sınama kancası varsa listeyi doldur ve ayarı bellekte aç.
    pub(super) fn init_friends_fixture(&mut self) {
        let Some(spec) = std::env::var("OPENIFY_TEST_FRIENDS_JSON").ok().filter(|_| test_run()) else { return };
        let Some(fixture) = load_fixture(&spec, now_ms()) else {
            log::warn!("OPENIFY_TEST_FRIENDS_JSON okunamadı: {spec}");
            return;
        };
        self.prefs.friend_activity = true;
        self.friends.fixture = true;
        match fixture {
            Fixture::Feed(list) => self.friends.list = Some(Arc::new(list)),
            Fixture::Failed(why) => self.friends.failure = Some(why),
            Fixture::Loading => {}
        }
    }

    // ── Yenileme ────────────────────────────────────────────────────────

    /// Yenileme istenir mi: ayar açık, arkadaş paneli ekranda, pencere görünür,
    /// tam ekran açık değil.
    pub(super) fn friends_wanted(&self) -> bool {
        self.prefs.friend_activity
            && self.now.panel == Some(Panel::Friends)
            && !self.window_hidden()
            && !self.now.full.open
    }

    /// Panel açıldı / kapandı, pencere gizlendi / göründü, ayar değişti ya da
    /// zamanlayıcı çaldı: bir sonraki adımı belirler (sor, bekle, dur).
    pub(super) fn sync_friends(&mut self, cx: &mut Context<Self>) {
        if self.friends.fixture {
            return;
        }
        let now = Instant::now();
        let wake = match self.schedule_step(now) {
            Step::Idle => {
                if self.friends.timer.take().is_some() {
                    log::debug!("arkadaş etkinliği: yenileme durdu (panel ya da pencere görünmüyor)");
                }
                return;
            }
            Step::Fetch => {
                log::debug!("arkadaş etkinliği: istek gönderildi");
                self.friends.schedule.started(now);
                self.engine.send(Command::Library(LibraryCommand::OpenFriends));
                IN_FLIGHT_TIMEOUT
            }
            Step::Wait(d) => {
                log::debug!("arkadaş etkinliği: {} sn sonra yeniden bakılacak", d.as_secs());
                d
            }
        };
        self.friends.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wake).await;
            let _ = this.update(cx, |shell, cx| shell.sync_friends(cx));
        }));
    }

    fn schedule_step(&self, now: Instant) -> Step {
        self.friends.schedule.step(self.friends_wanted(), now)
    }

    pub(super) fn on_friends(&mut self, list: Arc<Vec<Friend>>, cx: &mut Context<Self>) {
        if self.friends.fixture {
            return;
        }
        self.friends.schedule.succeeded(Instant::now());
        self.friends.failure = None;
        self.friends.list = Some(list);
        self.views.side.update(cx, |_, cx| cx.notify());
        self.sync_friends(cx);
    }

    pub(super) fn on_friends_failed(&mut self, why: FriendsFailure, cx: &mut Context<Self>) {
        if self.friends.fixture {
            return;
        }
        self.friends.schedule.failed(Instant::now(), why);
        self.friends.failure = Some(why);
        self.views.side.update(cx, |_, cx| cx.notify());
        self.sync_friends(cx);
    }

    /// Ayar değişti: kapanınca panel kapanır, bellekteki liste ve zamanlayıcı
    /// atılır; açılınca (panel açıksa) yenileme başlar.
    pub(super) fn after_friend_pref(&mut self, cx: &mut Context<Self>) {
        if self.prefs.friend_activity {
            self.sync_friends(cx);
            return;
        }
        self.friends.timer = None;
        self.friends.list = None;
        self.friends.failure = None;
        self.friends.schedule.reset();
        if self.now.panel == Some(Panel::Friends) {
            self.hide_panel();
            self.after_view_change(cx);
        }
    }

    // ── Açma / kapama ───────────────────────────────────────────────────

    /// Başlık çubuğundaki düğme: paneli aç; açıksa kapat.
    pub(super) fn toggle_friends(&mut self, cx: &mut Context<Self>) {
        if !self.prefs.friend_activity {
            return;
        }
        if self.now.panel == Some(Panel::Friends) {
            self.leave_friends(cx);
        } else {
            self.show_panel(Panel::Friends);
            self.after_view_change(cx);
        }
    }

    /// Paneli kapat: yerleşik "Şimdi çalıyor" paneli açık tercihteyse yerine o
    /// döner (arkadaş paneli onun yuvasını geçici olarak aldı).
    pub(super) fn leave_friends(&mut self, cx: &mut Context<Self>) {
        let d = &self.now.dock;
        if d.wide && d.open && d.has_content {
            let tab = d.tab;
            self.show_panel(tab);
        } else {
            self.hide_panel();
        }
        self.after_view_change(cx);
    }

    /// Menüden bir sayfaya gidilince yüzen panel (içeriği örtüyor) kapanır;
    /// yerleşik panel kalır.
    fn dismiss_floating_friends(&mut self, cx: &mut Context<Self>) {
        if self.now.panel == Some(Panel::Friends) && !self.now.docked() {
            self.hide_panel();
            self.after_view_change(cx);
        }
    }

    // ── Eylemler ────────────────────────────────────────────────────────

    /// Satıra tıklama: şarkıyı bağlamıyla çal.
    pub(super) fn play_friend(&mut self, friend: &Friend) {
        match engine::play_plan(friend) {
            PlayPlan::Context { uri, track, hint } => {
                self.engine.send(Command::PlayContextAt { uri, track, hint });
            }
            PlayPlan::Track(uri) => {
                if let Ok(uri) = librespot::core::SpotifyUri::from_uri(&uri) {
                    self.engine.send(Command::Load(uri));
                }
            }
        }
    }

    pub(super) fn go_friend_album(&mut self, uri: String, cx: &mut Context<Self>) {
        self.open_album(uri, cx);
        self.dismiss_floating_friends(cx);
    }

    pub(super) fn go_friend_artist(&mut self, uri: String, cx: &mut Context<Self>) {
        self.navigate(Route::Artist(uri), cx);
        self.dismiss_floating_friends(cx);
    }

    /// Bağlam satırına tıklama: liste, albüm ya da sanatçı sayfası.
    pub(super) fn go_friend_context(&mut self, uri: String, name: String, cx: &mut Context<Self>) {
        match context_kind(&uri) {
            ContextKind::Playlist => {
                let id = engine::CollectionId::Playlist(uri);
                self.library.titles.entry(id.clone()).or_insert(name);
                self.select_collection(id, cx);
            }
            ContextKind::Album => self.open_album(uri, cx),
            ContextKind::Artist => self.navigate(Route::Artist(uri), cx),
            ContextKind::Other => return,
        }
        self.dismiss_floating_friends(cx);
    }

    /// Satırın sağ tık menüsü: çal, albüme git, sanatçıya git, profili Spotify'da aç.
    pub(super) fn open_friend_menu(&mut self, ix: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(f) = self.friends.friend(ix) else { return };
        let item = |icon: &'static str, label: String, act: Act| Entry::Item {
            icon,
            label,
            shortcut: None,
            danger: false,
            act: Some(act),
            sub: Vec::new(),
        };
        let mut entries = vec![item("play", t!(Play).into(), Act::PlayFriend(Box::new(f.clone())))];
        if !f.album_uri.is_empty() {
            entries.push(item("disc-3", t!(MenuGoAlbum).into(), Act::GoAlbumUri(f.album_uri.clone())));
        }
        if !f.artist_uri.is_empty() {
            entries.push(item("user", t!(MenuGoArtist).into(), Act::GoArtistUri(f.artist_uri.clone())));
        }
        if crate::link::app_target(&f.user_uri).is_some() {
            entries.push(Entry::Separator);
            entries.push(item("external-link", t!(FriendsMenuProfile).into(), Act::OpenInSpotify(f.user_uri.clone())));
        }
        self.show_menu(at, entries, cx);
    }

    /// Klavye (panel odaktayken, satır odaklıysa): ↑ / ↓ komşu satıra, Shift+F10
    /// menü. İşlendiyse `true`. Enter / Boşluk'u gpui satırın tıklamasına çevirir.
    pub(super) fn friends_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.manage.modal() {
            return false;
        }
        let Some((key, _)) = self.nav.extra_focused(window) else { return false };
        let Some(ix) = key.strip_prefix("fr-row-").and_then(|n| n.parse::<usize>().ok()) else { return false };
        let ks = &event.keystroke;
        log::debug!("arkadaş satırı {ix}: tuş {:?}", ks.key);
        match (Shortcut::from_keystroke(ks), ks.key.as_str()) {
            (Some(Shortcut::RowMenu), _) => {
                let viewport = window.viewport_size();
                let (top, _) = self.side_panel_frame(f32::from(viewport.width), f32::from(viewport.height));
                let (width, right) = self.now.dock.panel_box;
                let offset = f32::from(self.friends.scroll.offset().y);
                let list = self.friends.list.clone().unwrap_or_default();
                let at = entry_top(&plan_items(&list, now_ms()), &list, ix);
                let x = f32::from(viewport.width) - right - width - S2 - 240.;
                let y = top + BORDER / 2. + ROW_HEIGHT + at + offset + S5;
                self.open_friend_menu(ix, point(px(x.max(S2)), px(y)), cx);
                true
            }
            (None, "up" | "down") if !ks.modifiers.shift && !ks.modifiers.control && !ks.modifiers.alt => {
                self.keyboard_focus = true;
                self.nav.reveal.set(true);
                if ks.key == "up" { window.focus_prev() } else { window.focus_next() }
                cx.notify();
                true
            }
            _ => false,
        }
    }

    // ── Görünüm ─────────────────────────────────────────────────────────

    /// Başlık çubuğundaki düğme (ayar açıkken).
    pub(super) fn render_friends_button(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.prefs.friend_activity {
            return None;
        }
        let (stop, look) = self.stop_look("friends-button", super::tab::FRIENDS, true, window, cx);
        let on = self.now.panel == Some(Panel::Friends);
        Some(
            div()
                .mr(px(S2))
                .child(
                    toggle_button("friends-button", "users", on, look, self.accent().text)
                        .when_some(stop, |el, h| el.track_focus(&h))
                        .tooltip(Tooltip::text(t!(FriendsTitle), None))
                        .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                            this.leave_button(ev, window);
                            this.toggle_friends(cx);
                        })),
                )
                .into_any_element(),
        )
    }

    /// Panelin başlık şeridi: ad ve kapat.
    pub(super) fn friends_header(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (stop, look) = self.stop_look("friends-close", super::tab::PANEL, true, window, cx);
        div()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .pl(px(S4))
            .pr(px(S2))
            .flex()
            .items_center()
            .gap(px(S2))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .type_role(T_BODY)
                    .font_weight(FontWeight::BOLD)
                    .text_color(text())
                    .child(line_with_tip("friends-title", t!(FriendsTitle))),
            )
            .child(
                icon_button("friends-close", "x", look, accent)
                    .when_some(stop, |el, h| el.track_focus(&h))
                    .tooltip(Tooltip::text(t!(Close), (!self.now.docked()).then_some("Esc")))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.leave_friends(cx);
                        window.focus(&this.focus);
                    })),
            )
            .into_any_element()
    }

    /// Panelin yüksekliği (`room`: sığabilecek en çok): içerik kadar.
    pub(super) fn friends_panel_height(&self, room: f32) -> f32 {
        let body = self.friends.body();
        if body != Body::Rows {
            return panel_height(body, 0., room);
        }
        let list = self.friends.list.clone().unwrap_or_default();
        let items = plan_items(&list, now_ms());
        panel_height(body, items_height(&items, &list), room)
    }

    /// Panelin gövdesi: gruplu arkadaş blokları ya da ortada sakin bir durum.
    pub(super) fn render_friends_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = self.friends.body();
        if body != Body::Rows {
            return friends_state(body);
        }
        let list = self.friends.list.clone().unwrap_or_default();
        let now = now_ms();
        let items = plan_items(&list, now);
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(i, item)| match *item {
                Item::Head(group) => friends_head(i, group),
                Item::Entry(ix) => self.friend_entry(ix, &list[ix], now, window, cx),
            })
            .collect();
        self.flush_image_requests(cx);
        // Uzun listede kenarlar solar (kaydırılmış içerik varsa), "Şimdi
        // çalıyor" paneliyle aynı.
        let handle = &self.friends.scroll;
        let offset = f32::from(handle.offset().y);
        let max = f32::from(handle.max_offset().height);
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("friends-body")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(handle)
                    .px(px(S2))
                    .pt(px(BODY_TOP))
                    .pb(px(BODY_BOTTOM))
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .children(super::lyrics_view::edge_veil(true, -offset / S6))
            .children(super::lyrics_view::edge_veil(false, (max + offset) / S6))
            .into_any_element()
    }

    /// Bir arkadaşın bloğu: A) profil resmi, ad, zaman; B) şarkı kapağı, şarkı,
    /// sanatçı, bağlam. Blok ve kapak çalar; ad menüyü açar; sanatçı ve bağlam
    /// kendi sayfalarına gider.
    fn friend_entry(&self, ix: usize, f: &Friend, now_ms: i64, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (stop, look) = self.stop_look(format!("fr-row-{ix}"), super::tab::PANEL, true, window, cx);
        let focused = look == Look::Focus;
        let live = is_live(f.timestamp_ms, now_ms);
        let age = (f.timestamp_ms > 0).then(|| age_label(age_of(now_ms - f.timestamp_ms)));
        let at = (f.timestamp_ms > 0).then(|| absolute_label_in(f.timestamp_ms, local_offset_minutes()));

        // A satırı: profil resmi, ad (menü), zaman.
        let time = div()
            .id(("fr-time", ix))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(S1))
            .type_role(T_LABEL)
            .tabular()
            .text_color(if live { muted() } else { label() })
            .when(live, |el| el.child(div().size(px(S1 + S0)).rounded_full().bg(rgb(accent))))
            .children(age)
            .when_some(at, |el, at| el.tooltip(Tooltip::text(at, None)));
        let name = div()
            .id(("fr-name-btn", ix))
            .flex_1()
            .min_w_0()
            .type_role(T_BODY)
            .font_weight(FontWeight::MEDIUM)
            .text_color(text())
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.open_friend_menu(ix, ev.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(line_with_tip(("fr-name", ix), f.name.clone()).flex().min_w_0());
        let line_a = div().flex().items_center().gap(px(S2)).child(self.avatar(f)).child(name).child(time);

        // B satırı: kapak (üzerine gelince çal), şarkı, sanatçı, bağlam.
        let group: SharedString = format!("fr-cover-{ix}").into();
        let cover = div()
            .id(("fr-cover", ix))
            .relative()
            .flex_none()
            .size(px(COVER))
            .rounded(px(R_SM))
            .group(group.clone())
            .child(cover_image(&self.images, &f.track_image, &f.album_uri, COVER, R_SM, "music"))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .child(
                        div()
                            .size(px(S6))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(shade(SCRIM))
                            .child(icon("play-fill", 16., text())),
                    ),
            )
            .tooltip(Tooltip::text(t!(FriendsPlayTip), None));
        let title = div()
            .type_role(T_BODY)
            .text_color(text())
            .child(line_with_tip(("fr-song", ix), f.track.clone()).flex().min_w_0());
        let artist = (!f.artist.is_empty()).then(|| {
            let names = [f.artist.as_str()];
            let on_click: SpanHandler = if f.artist_uri.is_empty() {
                Rc::new(|_, _, _| {})
            } else {
                let (uri, this) = (f.artist_uri.clone(), cx.entity().downgrade());
                Rc::new(move |_, _, cx| {
                    let _ = this.update(cx, |shell, cx| shell.go_friend_artist(uri.clone(), cx));
                })
            };
            div()
                .type_role(T_SECONDARY)
                .text_color(muted())
                .child(link_line(("fr-artist", ix), &names, on_click, None).flex().min_w_0())
        });
        let context = context_name(f).map(|name| {
            let kind = f.context_uri.as_deref().map_or(ContextKind::Other, context_kind);
            let line: AnyElement = match (&f.context_uri, kind) {
                (Some(uri), kind) if kind != ContextKind::Other => {
                    let (uri, title, this) = (uri.clone(), name.to_string(), cx.entity().downgrade());
                    let on_click: SpanHandler = Rc::new(move |_, _, cx| {
                        let _ = this.update(cx, |shell, cx| shell.go_friend_context(uri.clone(), title.clone(), cx));
                    });
                    link_line(("fr-context", ix), &[name], on_click, None).flex().min_w_0().into_any_element()
                }
                _ => line_with_tip(("fr-context", ix), name.to_string()).flex().min_w_0().into_any_element(),
            };
            div()
                .flex()
                .items_center()
                .gap(px(S1))
                .type_role(T_LABEL)
                .text_color(label())
                .child(icon(context_icon(kind), 12., label()))
                .child(div().flex_1().min_w_0().child(line))
        });
        let texts = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .justify_center()
            .child(title)
            .children(artist)
            .children(context);
        let line_b = div().pl(px(AVATAR + S2)).flex().items_center().gap(px(S3)).child(cover).child(texts);

        let friend = f.clone();
        div()
            .id(("fr-row", ix))
            .relative()
            .w_full()
            .px(px(S2 - S0))
            .py(px(S2 - S0))
            .rounded(px(R_MD))
            .flex()
            .flex_col()
            .gap(px(LINE_GAP))
            .border_2()
            .border_color(if focused { rgb(accent).into() } else { gpui::transparent_black() })
            .when(focused, |el| el.bg(veil(VEIL_SELECTED)))
            .cursor_pointer()
            .child(super::tween::hover_fill(veil(VEIL_HOVER), veil(VEIL_PRESSED), 0., R_MD))
            .when_some(stop.as_ref(), |el, h| el.child(self.friends_probe(h)))
            .when_some(stop, |el, h| el.track_focus(&h))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.play_friend(&friend);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.open_friend_menu(ix, ev.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(line_a)
            .child(line_b)
            .into_any_element()
    }

    /// Yuvarlak profil resmi; resim yoksa (ya da henüz inmediyse) adın tonunda
    /// sakin bir daire içinde baş harf.
    fn avatar(&self, f: &Friend) -> AnyElement {
        if self.images.get(&f.image).is_some() {
            return cover_image(&self.images, &f.image, &f.user_uri, AVATAR, AVATAR / 2., "user");
        }
        let (fill, ink) = avatar_colors(&f.name);
        div()
            .size(px(AVATAR))
            .flex_none()
            .rounded_full()
            .bg(rgb(fill))
            .flex()
            .items_center()
            .justify_center()
            .type_role(T_BODY)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(ink))
            .child(initial_of(&f.name))
            .into_any_element()
    }

    /// Tab ile odaklanan satırı görünür alana kaydırır (kaydırma alanı panelin
    /// gövdesi; bkz. `Shell::focus_probe`).
    fn friends_probe(&self, handle: &FocusHandle) -> AnyElement {
        let handle = handle.clone();
        let reveal = self.nav.reveal.clone();
        let scroll = self.friends.scroll.clone();
        canvas(
            move |b, window, _| {
                if !reveal.get() || !handle.is_focused(window) {
                    return;
                }
                reveal.set(false);
                let view = scroll.bounds();
                let top = view.origin.y;
                let bottom = view.origin.y + view.size.height;
                let bottom_edge = b.origin.y + b.size.height;
                let dy = if b.origin.y < top {
                    top - b.origin.y
                } else if bottom_edge > bottom {
                    bottom - bottom_edge
                } else {
                    return;
                };
                let offset = scroll.offset();
                scroll.set_offset(gpui::point(offset.x, offset.y + dy));
                window.refresh();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }
}

/// Grup adı: sıra panelinin bölüm başlıklarıyla aynı yazı (Label, yarı kalın,
/// etiket tonu); kutu listenin boyuna göre (bkz. `item_height`).
fn friends_head(i: usize, group: Group) -> AnyElement {
    let title = match group {
        Group::Live => t!(FriendsGroupLive),
        Group::Earlier => t!(FriendsGroupEarlier),
    };
    div()
        .id(("fr-head", i))
        .w_full()
        .flex_none()
        .px(px(S2))
        .when(i > 0, |el| el.mt(px(HEAD_GAP)))
        .h(px(HEAD_HEIGHT))
        .flex()
        .items_center()
        .type_role(T_LABEL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(label())
        .child(line_with_tip(("fr-head-t", i), title).flex().min_w_0())
        .into_any_element()
}

/// Boş, yükleniyor ve hata durumları: panelin ortasında sakin bir simge, başlık
/// ve tek cümle (öbür panellerin `state_view`'ı).
fn friends_state(body: Body) -> AnyElement {
    let (title, message) = match body {
        Body::Empty => (t!(FriendsEmpty), t!(FriendsEmptyBody)),
        Body::Loading => (t!(FriendsLoading), t!(FriendsLoadingBody)),
        Body::Failed(FriendsFailure::RateLimited) => (t!(FriendsBusyTitle), t!(FriendsBusyBody)),
        Body::Failed(FriendsFailure::Denied | FriendsFailure::Unavailable) => {
            (t!(FriendsFailedDenied), t!(FriendsDeniedBody))
        }
        Body::Failed(FriendsFailure::Offline) => (t!(FriendsFailedOffline), t!(FriendsOfflineBody)),
        Body::Rows => ("", ""),
    };
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .px(px(S5))
        .child(state_view("users", title, message).w_full())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn at(base: Instant, secs: u64) -> Instant {
        base + S * secs as u32
    }

    // ── Yenileme zamanlayıcısı ─────────────────────────────────────────

    #[test]
    fn nothing_is_asked_unless_wanted() {
        let t0 = Instant::now();
        let s = Schedule::default();
        assert_eq!(s.step(false, t0), Step::Idle, "kapalı / görünmez: hiçbir şey sorulmaz");
        assert_eq!(s.step(true, t0), Step::Fetch, "ilk görünüşte hemen sorulur");
    }

    #[test]
    fn visible_polls_every_sixty_seconds() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.started(t0);
        // Cevap beklenirken ikinci istek yok; bekleme kayıp eşiğine kadar.
        assert_eq!(s.step(true, at(t0, 5)), Step::Wait(IN_FLIGHT_TIMEOUT - 5 * S));
        s.succeeded(at(t0, 2));
        assert_eq!(s.step(true, at(t0, 2)), Step::Wait(REFRESH));
        assert_eq!(s.step(true, at(t0, 61)), Step::Wait(S));
        assert_eq!(s.step(true, at(t0, 62)), Step::Fetch);
        assert_eq!(s.step(true, at(t0, 600)), Step::Fetch);
    }

    #[test]
    fn hiding_stops_polling_and_keeps_the_clock() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.started(t0);
        s.succeeded(t0);
        // Gizliyken (panel kapalı ya da pencere tepside) süre dolsa da sorulmaz.
        assert_eq!(s.step(false, at(t0, 30)), Step::Idle);
        assert_eq!(s.step(false, at(t0, 600)), Step::Idle);
        // Kısa sürede yeniden görününce hemen sorulmaz (kalan süre), uzun
        // sürede görününce hemen sorulur.
        assert_eq!(s.step(true, at(t0, 10)), Step::Wait(50 * S));
        assert_eq!(s.step(true, at(t0, 601)), Step::Fetch);
    }

    #[test]
    fn a_lost_request_does_not_block_forever() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.started(t0);
        assert!(matches!(s.step(true, at(t0, 89)), Step::Wait(_)));
        assert_eq!(s.step(true, at(t0, 90)), Step::Fetch, "cevap gelmedi: yeniden sorulabilir");
        // Görünmezken de kayıp istek yeni bir şey başlatmaz.
        assert_eq!(s.step(false, at(t0, 200)), Step::Idle);
    }

    #[test]
    fn failures_back_off_and_success_resets() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        let delays: Vec<u64> = (0..6)
            .map(|_| {
                s.started(t0);
                s.failed(t0, FriendsFailure::Offline);
                match s.step(true, t0) {
                    Step::Wait(d) => d.as_secs(),
                    other => panic!("{other:?}"),
                }
            })
            .collect();
        assert_eq!(delays, [120, 240, 480, 600, 600, 600], "katlanır, 10 dk'da durur");
        s.started(t0);
        s.succeeded(t0);
        assert_eq!(s.step(true, t0), Step::Wait(REFRESH), "başarı aralığı sıfırlar");
        s.started(t0);
        s.failed(t0, FriendsFailure::Offline);
        assert_eq!(s.step(true, t0), Step::Wait(2 * REFRESH), "hata sayacı da sıfırlandı");
    }

    #[test]
    fn rate_limits_and_denials_wait_longer() {
        assert_eq!(retry_delay(1, FriendsFailure::RateLimited), RATE_LIMIT_MIN, "429: en az 5 dk");
        assert_eq!(retry_delay(3, FriendsFailure::RateLimited), Duration::from_secs(480));
        assert_eq!(retry_delay(1, FriendsFailure::Denied), BACKOFF_MAX);
        assert_eq!(retry_delay(1, FriendsFailure::Unavailable), BACKOFF_MAX);
        assert_eq!(retry_delay(1, FriendsFailure::Offline), 2 * REFRESH);
        // Çok büyük hata sayısı taşmaz.
        assert_eq!(retry_delay(u32::MAX, FriendsFailure::Offline), BACKOFF_MAX);
    }

    #[test]
    fn reset_forgets_everything_when_the_setting_is_turned_off() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.started(t0);
        s.failed(t0, FriendsFailure::Denied);
        s.reset();
        assert_eq!(s.step(true, t0), Step::Fetch);
    }

    // ── Göreli zaman ───────────────────────────────────────────────────

    #[test]
    fn age_buckets() {
        let secs = |s: i64| age_of(s * 1000);
        assert_eq!(secs(-30), Age::Now, "saat farkı: eksi süre şimdi");
        assert_eq!(secs(0), Age::Now);
        assert_eq!(secs(59), Age::Now);
        assert_eq!(secs(60), Age::Minutes(1));
        assert_eq!(secs(3 * 60 + 20), Age::Minutes(3));
        assert_eq!(secs(59 * 60 + 59), Age::Minutes(59));
        assert_eq!(secs(3600), Age::Hours(1));
        assert_eq!(secs(23 * 3600 + 59 * 60), Age::Hours(23));
        assert_eq!(secs(86_400), Age::Days(1));
        assert_eq!(secs(6 * 86_400), Age::Days(6));
        assert_eq!(secs(7 * 86_400), Age::Weeks(1));
        assert_eq!(secs(10_000 * 86_400), Age::Weeks(99), "uç değer taşmaz");
    }

    #[test]
    fn age_labels_follow_the_language() {
        use crate::i18n::{Lang, with_lang};
        with_lang(Lang::En, || {
            assert_eq!(age_label(Age::Now), "now");
            assert_eq!(age_label(Age::Minutes(3)), "3 min");
            assert_eq!(age_label(Age::Hours(2)), "2 h");
            assert_eq!(age_label(Age::Days(4)), "4 d");
            assert_eq!(age_label(Age::Weeks(2)), "2 w");
        });
        with_lang(Lang::Tr, || {
            assert_eq!(age_label(Age::Minutes(3)), "3 dk");
            assert_eq!(age_label(Age::Hours(2)), "2 sa");
        });
    }

    #[test]
    fn live_window() {
        let now = 10_000_000;
        assert!(is_live(now - 20_000, now));
        assert!(is_live(now - LIVE_WINDOW_MS, now));
        assert!(!is_live(now - LIVE_WINDOW_MS - 1, now));
        assert!(!is_live(0, now), "zamanı bilinmeyen canlı sayılmaz");
        assert!(!is_live(now + 5_000, now), "gelecekteki damga canlı sayılmaz");
    }

    // ── Gövde hâlleri ──────────────────────────────────────────────────

    #[test]
    fn body_states() {
        assert_eq!(body_of(None, None), Body::Loading);
        assert_eq!(body_of(Some(0), None), Body::Empty);
        assert_eq!(body_of(Some(3), None), Body::Rows);
        // Eski liste varsa hata sırasında o gösterilir; yoksa tek satır hata.
        assert_eq!(body_of(Some(3), Some(FriendsFailure::Offline)), Body::Rows);
        assert_eq!(body_of(None, Some(FriendsFailure::Denied)), Body::Failed(FriendsFailure::Denied));
        assert_eq!(body_of(Some(0), Some(FriendsFailure::RateLimited)), Body::Failed(FriendsFailure::RateLimited));
    }

    #[test]
    fn initials_skip_symbols_and_uppercase() {
        assert_eq!(initial_of("elif"), "E");
        assert_eq!(initial_of("  _ ayse"), "A");
        assert_eq!(initial_of(""), "");
    }

    // ── Sınama kancası ─────────────────────────────────────────────────

    #[test]
    fn fixture_specs() {
        assert_eq!(load_fixture("!error", 0), Some(Fixture::Failed(FriendsFailure::Offline)));
        assert_eq!(load_fixture("!busy", 0), Some(Fixture::Failed(FriendsFailure::RateLimited)));
        assert_eq!(load_fixture("!denied", 0), Some(Fixture::Failed(FriendsFailure::Denied)));
        assert_eq!(load_fixture("!loading", 0), Some(Fixture::Loading));
        assert_eq!(load_fixture("yok-boyle-bir-dosya.json", 0), None);
    }

    #[test]
    fn fixture_file_turns_age_seconds_into_timestamps() {
        let dir = std::env::temp_dir().join(format!("openify-friends-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.json");
        let body = r#"{"friends":[
            {"ageSeconds":600,"user":{"uri":"spotify:user:a","name":"A"},"track":{"uri":"spotify:track:x","name":"N","artist":{"name":"R"}}},
            {"ageSeconds":30,"user":{"uri":"spotify:user:b","name":"B"},"track":{"uri":"spotify:track:y","name":"M"}}]}"#;
        std::fs::write(&path, body).unwrap();
        let now = 1_762_900_000_000;
        let Some(Fixture::Feed(list)) = load_fixture(path.to_str().unwrap(), now) else { panic!("liste bekleniyordu") };
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(list.len(), 2);
        // En yeni önde: B 30 sn, A 600 sn önce.
        assert_eq!((list[0].name.as_str(), list[0].timestamp_ms), ("B", now - 30_000));
        assert_eq!((list[1].name.as_str(), list[1].timestamp_ms), ("A", now - 600_000));
    }

    /// Ekran sınaması fikstürü gerçek bir kullanım gibi çözülür ve hem şu an
    /// dinleyenleri hem eski kayıtları içerir (görsel durumların hepsi görünür).
    #[test]
    fn demo_fixture_covers_the_visual_states() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/engine/fixtures/friend_activity_demo.json");
        let now = 1_762_900_000_000;
        let Some(Fixture::Feed(list)) = load_fixture(path, now) else { panic!("liste bekleniyordu") };
        assert_eq!(list.len(), 9);
        let live = list.iter().filter(|f| is_live(f.timestamp_ms, now)).count();
        assert_eq!(live, 2, "iki arkadaş şu an dinliyor");
        let ages: Vec<Age> = list.iter().map(|f| age_of(now - f.timestamp_ms)).collect();
        assert!(ages.contains(&Age::Now) && ages.iter().any(|a| matches!(a, Age::Minutes(_)))
            && ages.iter().any(|a| matches!(a, Age::Hours(_))) && ages.iter().any(|a| matches!(a, Age::Days(_))));
    }

    // ── Kaplı liste (v2): gruplar, yükseklik, baş harf tonu, yerleşim ─────

    const NOW: i64 = 1_762_900_000_000;

    fn friend(name: &str, age_secs: i64, context: Option<&str>) -> Friend {
        Friend {
            user_uri: format!("spotify:user:{}", name.to_lowercase()),
            name: name.into(),
            image: String::new(),
            track_image: String::new(),
            timestamp_ms: NOW - age_secs * 1000,
            track_uri: "spotify:track:t".into(),
            track: "Şarkı".into(),
            artist: "Sanatçı".into(),
            artist_uri: "spotify:artist:a".into(),
            album: "Albüm".into(),
            album_uri: "spotify:album:b".into(),
            context_uri: context.map(|c| format!("spotify:playlist:{c}")),
            context_name: context.map(str::to_string),
            context_index: None,
        }
    }

    #[test]
    fn friends_are_grouped_live_first_then_earlier() {
        // En yeni önde (motorun sırası): iki canlı, üç eski.
        let list = [
            friend("A", 20, None),
            friend("B", 150, None),
            friend("C", 400, None),
            friend("D", 5_000, None),
            friend("E", 90_000, None),
        ];
        assert_eq!(
            plan_items(&list, NOW),
            [
                Item::Head(Group::Live),
                Item::Entry(0),
                Item::Entry(1),
                Item::Head(Group::Earlier),
                Item::Entry(2),
                Item::Entry(3),
                Item::Entry(4),
            ]
        );
        // Sınır: tam 3 dk hâlâ canlı, 1 ms sonrası eski; zamanı bilinmeyen eski.
        let edge = [friend("A", 180, None), Friend { timestamp_ms: NOW - 180_001, ..friend("B", 0, None) }, Friend { timestamp_ms: 0, ..friend("C", 0, None) }];
        assert_eq!(
            plan_items(&edge, NOW),
            [Item::Head(Group::Live), Item::Entry(0), Item::Head(Group::Earlier), Item::Entry(1), Item::Entry(2)]
        );
    }

    #[test]
    fn an_empty_group_is_omitted() {
        let only_live = [friend("A", 10, None), friend("B", 60, None)];
        assert_eq!(plan_items(&only_live, NOW), [Item::Head(Group::Live), Item::Entry(0), Item::Entry(1)]);
        let only_earlier = [friend("A", 600, None), friend("B", 7_200, None)];
        assert_eq!(plan_items(&only_earlier, NOW), [Item::Head(Group::Earlier), Item::Entry(0), Item::Entry(1)]);
        assert!(plan_items(&[], NOW).is_empty());
    }

    #[test]
    fn heights_follow_the_items_and_the_panel_hugs_its_content() {
        // Bağlam satırı varsa yazılar (54) kapaktan (48) yüksek: blok 6 px uzar.
        assert_eq!(entry_height(false), 2. * ENTRY_PAD + AVATAR + LINE_GAP + COVER);
        assert_eq!(entry_height(true) - entry_height(false), TEXT_LINES_FULL - COVER);
        let list = [friend("A", 10, Some("p")), friend("B", 600, None)];
        let items = plan_items(&list, NOW);
        // İlk başlıkta üst boşluk yok, ikincisinde var.
        assert_eq!(items_height(&items, &list), 2. * HEAD_HEIGHT + HEAD_GAP + entry_height(true) + entry_height(false));
        assert_eq!(entry_top(&items, &list, 0), BODY_TOP + HEAD_HEIGHT);
        assert_eq!(entry_top(&items, &list, 1), BODY_TOP + HEAD_HEIGHT + entry_height(true) + HEAD_GAP + HEAD_HEIGHT);
        // Panel içerik kadar: sığıyorsa o, sığmıyorsa alanın tamamı (kayar).
        let h = items_height(&items, &list);
        let want = BORDER + ROW_HEIGHT + BODY_TOP + h + BODY_BOTTOM;
        assert_eq!(panel_height(Body::Rows, h, 2000.), want);
        assert_eq!(panel_height(Body::Rows, h, 300.), 300.);
        // Durum ekranları sabit, kısa bir kart.
        for body in [Body::Empty, Body::Loading, Body::Failed(FriendsFailure::Offline)] {
            assert_eq!(panel_height(body, 0., 2000.), BORDER + ROW_HEIGHT + STATE_BODY);
        }
    }

    #[test]
    fn avatar_fallback_tone_is_deterministic_and_calm() {
        // Aynı ad hep aynı ton (büyük / küçük harf ve kenar boşluğu fark etmez).
        assert_eq!(avatar_colors("Maya"), avatar_colors("Maya"));
        assert_eq!(avatar_hue_step("Maya"), avatar_hue_step("  maya "));
        // Bilinen değerler: hash sürümler arası değişmemeli (renk titremesin).
        let steps: Vec<u32> = ["Maya", "Leo", "Ivy Okafor", "Noah", "Sofia", "Arlo", "June", ""].iter().map(|n| avatar_hue_step(n)).collect();
        assert_eq!(steps, [5, 15, 15, 19, 21, 7, 5, 13]);
        // Ton 24 adımdan biri; birkaç ad farklı tonlara dağılır.
        assert!(steps.iter().all(|s| *s < HUE_STEPS));
        assert!(steps.iter().collect::<std::collections::HashSet<_>>().len() >= 5);
        // Sakin: zemin yüzey tonuna yakın (her kanal yüzeyden en çok 55 uzak),
        // harf zeminden belirgin açık (okunur).
        for name in ["Maya", "Leo", "Ivy Okafor", "Noah", "Sofia", "Arlo", "June", "Zeynep", "Can"] {
            let (fill, ink) = avatar_colors(name);
            for shift in [16, 8, 0] {
                let (a, b) = (((fill >> shift) & 0xFF) as i32, ((PLACEHOLDER >> shift) & 0xFF) as i32);
                assert!((a - b).abs() <= 55, "{name}: kanal {shift} yüzeyden uzak ({a} / {b})");
            }
            assert!(contrast(ink, fill) >= 4.5, "{name}: baş harf okunmuyor ({:.2})", contrast(ink, fill));
        }
    }

    #[test]
    fn context_kinds_and_icons() {
        assert_eq!(context_kind("spotify:playlist:x"), ContextKind::Playlist);
        assert_eq!(context_kind("spotify:album:x"), ContextKind::Album);
        assert_eq!(context_kind("spotify:artist:x"), ContextKind::Artist);
        assert_eq!(context_kind("spotify:user:x:collection"), ContextKind::Other);
        assert_eq!(context_icon(ContextKind::Playlist), "list-music");
        assert_eq!(context_icon(ContextKind::Other), "music");
        // Adı olmayan bağlam gösterilmez.
        assert_eq!(context_name(&friend("A", 1, Some("Liste"))), Some("Liste"));
        assert_eq!(context_name(&Friend { context_name: Some(String::new()), ..friend("A", 1, None) }), None);
    }

    #[test]
    fn absolute_time_labels() {
        use crate::i18n::{Lang, with_lang};
        // 2025-11-11 23:32 UTC; Türkiye'de (UTC+3) ertesi gün 02:32.
        let at = 1_762_903_920_000;
        assert_eq!(clock_label(at, 0), "23:32");
        assert_eq!(clock_label(at, 180), "02:32");
        assert_eq!(clock_label(at, -300), "18:32");
        with_lang(Lang::En, || assert_eq!(absolute_label_in(at, 180), "Nov 12, 2025, 02:32"));
        with_lang(Lang::Tr, || assert_eq!(absolute_label_in(at, 180), "12 Kas 2025 02:32"));
    }

    /// Yerleşim kararı (900 / 1180 / 1920): içerik alanı en dar pencere eninin
    /// altına inmedikçe panel sayfanın yanına yerleşir; yoksa yüzer. Şimdi
    /// çalıyor paneli kendi eşiğinde (1720) kalır.
    #[test]
    fn friends_dock_decision_at_common_window_widths() {
        use super::super::now_panel::{DockKind, FRIENDS_DOCK_MIN_WIDTH, FRIENDS_PANEL_WIDTH, dock_layout_at, dock_layout_for};
        let friends = |w: f32| dock_layout_for(DockKind::FRIENDS, w, true, 1.);
        // 900×700: yerleşemez (içerik 524 px kalırdı), yüzer; içerik daralmaz.
        let l = friends(900.);
        assert!(!l.docked);
        assert_eq!(l.main_width, 900.);
        // 1180×760: yerleşir, içerik en dar pencereden (760) geniş.
        let l = friends(1180.);
        assert!(l.docked);
        assert_eq!((l.panel_width, l.panel_right), (FRIENDS_PANEL_WIDTH, S4));
        assert_eq!(l.main_width, 1180. - FRIENDS_PANEL_WIDTH - S4);
        assert!(l.main_width >= super::super::MIN_WINDOW_WIDTH);
        // 1920×1080: yerleşir.
        let l = friends(1920.);
        assert!(l.docked);
        assert_eq!(l.main_width, 1920. - FRIENDS_PANEL_WIDTH - S4);
        // Eşik: içerik tam 760 olan pencere yerleşir, 1 px darı yüzer.
        assert_eq!(FRIENDS_DOCK_MIN_WIDTH, 760. + 360. + 16.);
        assert!(friends(FRIENDS_DOCK_MIN_WIDTH).docked);
        assert!(!friends(FRIENDS_DOCK_MIN_WIDTH - 1.).docked);
        // Kapalıyken hiçbir pencerede içerik daralmaz.
        for w in [900., 1180., 1920.] {
            let closed = dock_layout_for(DockKind::FRIENDS, w, false, 0.);
            assert!(!closed.docked && closed.main_width == w, "{w}");
        }
        // Hap daralmaz (panel hapın üstünde biter) ve hâlâ pencerenin ortasında.
        for w in [1180., 1720., 1920.] {
            let (left, width) = friends(w).pill_frame(w);
            assert_eq!(width, super::super::pill::pill_width(w), "{w}");
            assert!((left + width / 2. - w / 2.).abs() <= 0.5, "{w}");
        }
        // Şimdi çalıyor paneli değişmedi: 1720'nin altında (1180, 1600) yüzer…
        assert!(!dock_layout_at(1180., true, 1.).docked);
        assert!(!dock_layout_at(1600., true, 1.).docked);
        // …1920'de yerleşir ve hapı daraltır.
        assert!(dock_layout_at(1920., true, 1.).docked && dock_layout_at(1920., true, 1.).beside_pill);
    }
}
