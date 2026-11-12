//! Karşılama ve "veri yok" durumları (11 Kasım).
//!
//! Önbellekte kütüphane yokken (ilk açılış, önbellek silindi) uygulama bağlanamıyor
//! ya da giriş gerekiyorsa Ana sayfa / Keşfet / Kütüphane sonsuza dek gri iskelet
//! gösteriyordu; tek ipucu hapın küçük durum satırıydı. Artık o sayfalarda
//! iskeletin yerine, ne olduğunu ve ne yapılabileceğini söyleyen tam sayfa durum
//! çizilir (`components::full_state`): ortada bir işaret, Headline başlık, tek
//! açıklama, asıl düğme.
//!
//! Durumlar (bağlantı durumundan, bkz. `engine::login`):
//! - giriş gerekli → **karşılama**: uygulama işareti, tek satır tanıtım, "Spotify ile
//!   giriş yap" (`Command::StartLogin`), Premium notu;
//! - tarayıcıda giriş bekleniyor → sakin ilerleme yazısı + "Girişi iptal et";
//! - giriş yapılamadı → neden (`msg::login_failed_text`) + "Yeniden dene";
//! - çevrimdışı → kendiliğinden yeniden denendiği söylenir + "Şimdi dene";
//! - hesap ücretsiz → Premium açıklaması, "Premium'a geç" (spotify.com/premium),
//!   "Yeniden kontrol et", "Çıkış yap";
//! - ses motoru durdu → "Yeniden başlat";
//! - uzun sürüyor (bağlanıyor / yükleniyor, `SLOW_AFTER` doldu) → "Şimdi dene";
//! - kütüphane başka bir nedenle gelmedi → hata açıklaması + "Yeniden dene".
//!
//! Önbellekte veri varsa bugünkü davranış sürer: içerik gösterilir, durum hapta.
//!
//! İskelet yalnız gerçekten yüklenirken: ilk `SKELETON_DELAY` boş sayfa (bağlantı
//! sonucu genelde bundan önce gelir; iskelet çakıp hemen durumla değişmesin),
//! sonra iskelet, `SLOW_AFTER`'dan sonra "uzun sürüyor" durumu. Bir durum
//! belirirken ya da iskelet / durumdan içeriğe geçilirken yeni görünen parça
//! %30 saydamlıktan 220 ms'de (`motion`, quint) gelir: arada boş kare ve ani
//! sıçrama yok ("animasyonları azalt"ta anında).
//!
//! Klavye: durumun asıl düğmesi (`gate-primary`) Tab durağıdır ve odak onda
//! ilk Tab'da buraya iner (`focus_gate_primary`); durum değişince (Giriş yap →
//! Girişi iptal et) aynı tutamaç kaldığı için odak yerinde kalır.
//!
//! Sınama kancaları: `OPENIFY_TEST_GATE_MS="iskelet,uzun"` (ms) iki süreyi
//! kısaltır; `OPENIFY_TEST_LINK=stall` bağlanmayı sonsuza dek bekletir
//! (`engine::first_connect`).

use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
    time::{Duration, Instant},
};

use gpui::{
    Animation, AnimationExt, AnyElement, Context, Div, ElementId, Stateful, Task, Window, div,
    prelude::*, px,
};

use super::{
    Shell,
    components::{
        Look, explain_error, full_state, ghost_button, primary_button, secondary_button, state_mark,
    },
    history::Route,
    library::Loadable,
    player::Connection,
    theme::*,
};
use crate::engine::{Command, Event, LoginFailReason};
use crate::{msg, t};

/// Premium sayfası (kullanıcının eylemiyle, kabuk üzerinden açılır).
const PREMIUM_URL: &str = "https://www.spotify.com/premium/";

/// Bağlantı sonucunu bekleyen boş sayfa: bu süreden sonra iskelet çizilir.
const SKELETON_DELAY: Duration = Duration::from_millis(400);
/// Yükleme bu kadar sürerse iskelet yerine "uzun sürüyor" durumu.
const SLOW_AFTER: Duration = Duration::from_secs(15);
/// "Şimdi dene"ye basılınca durum bu süre "Deneniyor…" olarak kalır (düğme
/// pasif): basış karşılıksız kalmaz, durum iskeletle yer değiştirip geri gelmez.
const TRY_HOLD: Duration = Duration::from_secs(6);
/// Sayfa açılır açılmaz bu kadar iskelet çizilmez (önbellekteki veri gelsin).
const ROUTE_DELAY: Duration = Duration::from_millis(120);
/// İçeriğe geçişte belirme sarmalı bu süre sonra bırakılır (animasyon 220 ms).
const SETTLE_WINDOW: Duration = Duration::from_millis(500);
/// Yeni görünen parçanın başlangıç saydamlığı (panel sekmeleriyle aynı taban).
const FADE_FLOOR: f32 = 0.3;

fn timings() -> (Duration, Duration) {
    static T: OnceLock<(Duration, Duration)> = OnceLock::new();
    *T.get_or_init(|| {
        let ms = |v: &str| v.trim().parse::<u64>().ok().map(Duration::from_millis);
        let over = std::env::var("OPENIFY_TEST_GATE_MS").ok().and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((ms(a)?, ms(b)?))
        });
        over.unwrap_or((SKELETON_DELAY, SLOW_AFTER))
    })
}

/// Sayfa ne yüzünden içerik gösteremiyor?
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GateKind {
    /// Kayıtlı giriş yok ya da reddedildi: karşılama.
    SignIn,
    /// Tarayıcıda giriş bekleniyor.
    SigningIn,
    SignInFailed(LoginFailReason),
    /// Hesap ücretsiz.
    Premium,
    Offline,
    /// Ses motoru çöktü.
    Stopped,
    /// Bağlanma / yükleme uzun sürdü.
    Slow,
    /// Kütüphane başka bir nedenle gelmedi (hata metni).
    Failed(String),
}

impl GateKind {
    fn id(&self) -> &'static str {
        match self {
            Self::SignIn => "sign-in",
            Self::SigningIn => "signing-in",
            Self::SignInFailed(_) => "sign-in-failed",
            Self::Premium => "premium",
            Self::Offline => "offline",
            Self::Stopped => "stopped",
            Self::Slow => "slow",
            Self::Failed(_) => "failed",
        }
    }
}

/// Gösterilecek durum; `busy`: "Şimdi dene"ye yeni basıldı, sonuç bekleniyor.
#[derive(Clone, Debug)]
pub(super) struct Gate {
    pub kind: GateKind,
    pub busy: bool,
}

/// Karşılama ve durumların çalışma anı durumu (`Shell::welcome`).
pub(super) struct WelcomeUi {
    /// Yüklemenin başladığı an (açılış, her `Connecting` / `Connected`, "Şimdi dene").
    since: Instant,
    /// İskelet / "uzun sürüyor" sınırlarında sayfayı yenileyen zamanlayıcı.
    timer: Option<Task<()>>,
    /// "Şimdi dene"ye basılan durum ve an.
    trying: Option<(GateKind, Instant)>,
    /// Bir iskelet ya da durum çizildi: sıradaki içerik belirerek gelir.
    shown: Cell<bool>,
    /// İçerik belirmeye başladığı an (sarmal `SETTLE_WINDOW` sonra bırakılır).
    settled_at: Cell<Option<Instant>>,
    /// Son çizilen iskeletli sayfa ve ilk göründüğü an (`page_skeleton_due`).
    page_seen: RefCell<(&'static str, Instant)>,
}

impl WelcomeUi {
    pub(super) fn new() -> Self {
        Self { since: Instant::now(), timer: None, trying: None, shown: Cell::new(false), settled_at: Cell::new(None), page_seen: RefCell::new(("", Instant::now())) }
    }

    /// Yükleme yeniden başlıyor: saati sıfırla, sınırlarda sayfayı yenile.
    fn arm(&mut self, cx: &mut Context<Shell>) {
        self.since = Instant::now();
        let (skeleton, slow) = timings();
        let mut marks = vec![skeleton, TRY_HOLD, slow];
        marks.sort();
        marks.dedup();
        self.timer = Some(cx.spawn(async move |this, cx| {
            let mut last = Duration::ZERO;
            for at in marks {
                cx.background_executor().timer(at - last).await;
                last = at;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }
}

/// Bağlantı durumundan (veri yokken) sayfanın göstereceği durum; yoksa
/// (gerçekten yükleniyor) `None`. Saf karar (sınanır). `trying`: "Şimdi dene"ye
/// yeni basılan durum (sonuç gelene dek aynı durum, düğme pasif);
/// `failure`: kütüphane hatası; `failed_all`: sayfanın bütün istekleri başarısız
/// (sayfanın kendi hata görünümü var, "uzun sürüyor" denmez); `slow`: yükleme
/// `SLOW_AFTER`'ı aştı.
fn decide(connection: &Connection, trying: Option<&GateKind>, failure: Option<&str>, failed_all: bool, slow: bool) -> Option<Gate> {
    let kind = match connection {
        Connection::NeedsLogin => GateKind::SignIn,
        Connection::LoggingIn => GateKind::SigningIn,
        Connection::LoginFailed(reason) => GateKind::SignInFailed(*reason),
        Connection::NeedsPremium => GateKind::Premium,
        Connection::EngineStopped => GateKind::Stopped,
        Connection::Failed(_) => GateKind::Offline,
        Connection::Connecting | Connection::Connected(_) | Connection::NoAudio => {
            if let Some(kind) = trying {
                return Some(Gate { kind: kind.clone(), busy: true });
            }
            if let Some(error) = failure {
                GateKind::Failed(error.to_string())
            } else if !failed_all && slow {
                GateKind::Slow
            } else {
                return None;
            }
        }
    };
    Some(Gate { kind, busy: false })
}

/// Yeni görünen parça: %30 saydamlıktan ve 8 px aşağıdan 220 ms'de gelir
/// (`page_enter`'ın tabanlı hâli: iskelet / durum silinip yerine yenisi
/// gelirken arada boş kare olmasın). "Animasyonları azalt"ta sarmalanmaz.
fn fade_in(id: impl Into<ElementId>, content: Div) -> AnyElement {
    if !animations_enabled() {
        return content.into_any_element();
    }
    content
        .with_animation(id, Animation::new(motion()).with_easing(gpui::ease_out_quint()), |el, d| {
            el.opacity(FADE_FLOOR + (1. - FADE_FLOOR) * d).mt(px(S2 * (1. - d)))
        })
        .into_any_element()
}

impl Shell {
    // ── Karar ───────────────────────────────────────────────────────────

    /// Yükleme `SKELETON_DELAY`'dir sürüyor mu (iskelet çizilebilir mi)?
    pub(super) fn skeleton_due(&self) -> bool {
        self.welcome.since.elapsed() >= timings().0
    }

    fn load_is_slow(&self) -> bool {
        self.welcome.since.elapsed() >= timings().1
    }

    /// Veri gelebilecek bir durumda mıyız (bağlanıyor / bağlı, yavaş değil)?
    /// Hayırsa bekleyen bölüm iskeleti hiç çizilmez: beklenen veri gelmeyecek.
    pub(super) fn load_may_arrive(&self) -> bool {
        matches!(self.state.connection, Connection::Connecting | Connection::Connected(_) | Connection::NoAudio)
            && !self.load_is_slow()
    }

    /// Bağlantı durumundan (veri yokken) sayfanın göstereceği durum; yoksa
    /// (gerçekten yükleniyor) `None`. `failure`: kütüphane hatası;
    /// `failed_all`: sayfanın bütün istekleri başarısız (kendi hata görünümü var).
    pub(super) fn gate_for(&self, failure: Option<&str>, failed_all: bool) -> Option<Gate> {
        let trying = self.welcome.trying.as_ref().filter(|(_, at)| at.elapsed() < TRY_HOLD).map(|(kind, _)| kind);
        decide(&self.state.connection, trying, failure, failed_all, self.load_is_slow())
    }

    /// Kütüphane (Ana sayfa, Kütüphane sekmesi) için: kütüphane yoksa nedeni.
    pub(super) fn library_gate(&self) -> Option<Gate> {
        let failure = match &self.library.library {
            Loadable::Loaded(_) => return None,
            Loadable::Failed(e) => Some(e.as_str()),
            Loadable::Loading => None,
        };
        self.gate_for(failure, false)
    }

    /// Şu an ekranda bir durum görünüyor mu (hangi sayfa olursa)?
    fn visible_gate(&self) -> Option<Gate> {
        if self.all.open {
            return self.library_gate();
        }
        match self.pages.history.current() {
            Route::Home => self.library_gate(),
            Route::Explore => self.explore_gate(),
            _ => None,
        }
    }

    /// Kütüphane sekmesinde raf, kütüphane gelene (ya da durum çıkana) dek
    /// çizilmez: durumun altında gri iskelet kartlar durmasın.
    pub(super) fn hide_shelf(&self) -> bool {
        !matches!(self.library.library, Loadable::Loaded(_)) && (self.library_gate().is_some() || !self.skeleton_due())
    }

    // ── Olaylar ─────────────────────────────────────────────────────────

    /// Motor olayları: yükleme saati, "deniyor" bayrağı, yeniden istek.
    pub(super) fn welcome_event(&mut self, event: &Event, cx: &mut Context<Self>) {
        match event {
            Event::Connecting | Event::Connected { .. } => {
                // Yeniden bağlanılıyor: eski hata (çevrimdışı / giriş) bayat,
                // kütüphane yeniden gelecek.
                if matches!(self.library.library, Loadable::Failed(_)) {
                    self.library.library = Loadable::Loading;
                }
                self.welcome.arm(cx);
                if matches!(event, Event::Connected { .. }) {
                    // Bağlantı yokken sorulan sayfa verileri düşmüştü (motor
                    // bekleyen komutları çevrimdışı olunca atar): şimdi yeniden.
                    self.reask_explore();
                }
            }
            Event::ConnectFailed(_)
            | Event::NeedsLogin
            | Event::LoggingIn
            | Event::LoginFailed(_)
            | Event::NeedsPremium
            | Event::EngineStopped(_)
            | Event::LibraryLoaded(_) => self.welcome.trying = None,
            _ => {}
        }
    }

    /// İlk Tab: odak görünmez çapadaysa ve sayfada bir durum varsa asıl
    /// düğmesine iner (başlık çubuğunun on durağından geçmek gerekmez).
    pub(super) fn focus_gate_primary(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.visible_gate().is_none() {
            return false;
        }
        let handle = self.nav.extra(cx, "gate-primary".into(), super::tab::PAGE);
        window.focus(&handle);
        true
    }

    // ── Eylemler ────────────────────────────────────────────────────────

    /// "Şimdi dene" / "Yeniden kontrol et" / "Yeniden dene": bağlanmayı ve
    /// kütüphaneyi yeniden iste; sonuç gelene dek durum "Deneniyor…".
    fn gate_retry(&mut self, kind: GateKind, cx: &mut Context<Self>) {
        if matches!(self.library.library, Loadable::Failed(_)) {
            self.library.library = Loadable::Loading;
        }
        self.welcome.trying = Some((kind, Instant::now()));
        self.welcome.arm(cx);
        self.engine.send(Command::RefreshLibrary);
        cx.notify();
    }

    fn gate_restart(&mut self, cx: &mut Context<Self>) {
        self.welcome.trying = Some((GateKind::Stopped, Instant::now()));
        self.welcome.arm(cx);
        self.restart_engine(cx);
    }

    /// Premium sayfasını kabuğun varsayılan tarayıcısında aç (kullanıcı eylemi).
    /// Sınama kopyalarında hiç açılmaz.
    fn open_premium_page(&self, cx: &mut Context<Self>) {
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
            log::info!("sınama kopyası: Premium sayfası açılmadı");
            return;
        }
        cx.open_url(PREMIUM_URL);
    }

    // ── Görünüm ─────────────────────────────────────────────────────────

    /// Sayfa içeriği (Ana sayfa, Keşfet) yerine: kütüphane ya da sayfa verisi
    /// henüz yokken. Durum varsa o, yoksa (yükleniyor) boş sayfa, sonra iskelet.
    pub(super) fn pending_page(&self, gate: Option<Gate>, skeleton: impl FnOnce() -> Div, window: &Window, cx: &mut Context<Self>) -> Div {
        if let Some(gate) = gate {
            return self.gate_page(&gate, window, cx);
        }
        if !self.page_skeleton_due("home", window) {
            return div();
        }
        self.mark_shown();
        div().child(fade_in("pending-skeleton", skeleton()))
    }

    /// `page` sayfasının iskeleti çizilebilir mi? Açılıştan bu yana
    /// `SKELETON_DELAY` ve sayfa ilk göründüğünden bu yana `ROUTE_DELAY`
    /// geçmiş olmalı: önbellekteki veri birkaç karede gelir, iskelet çakıp
    /// silinmesin. Değilse bir kare daha istenir (bu görünüm).
    pub(super) fn page_skeleton_due(&self, page: &'static str, window: &Window) -> bool {
        let now = Instant::now();
        let mut seen = self.welcome.page_seen.borrow_mut();
        if seen.0 != page {
            *seen = (page, now);
        }
        let due = now.duration_since(seen.1) >= ROUTE_DELAY && self.skeleton_due();
        if !due {
            window.request_animation_frame();
        }
        due
    }

    /// Veri gelince içerik: iskelet ya da durumdan sonraysa belirerek gelir.
    pub(super) fn settle(&self, key: &'static str, content: Div) -> Div {
        if !self.welcome.shown.get() {
            return content;
        }
        let now = Instant::now();
        let since = self.welcome.settled_at.get().unwrap_or(now);
        self.welcome.settled_at.set(Some(since));
        if now.duration_since(since) > SETTLE_WINDOW {
            self.welcome.shown.set(false);
            self.welcome.settled_at.set(None);
            return content;
        }
        div().child(fade_in(ElementId::Name(format!("settle-{key}").into()), content))
    }

    pub(super) fn mark_shown(&self) {
        self.welcome.shown.set(true);
        self.welcome.settled_at.set(None);
    }

    /// Kaydırmalı sayfada (Ana sayfa, Keşfet) durum: sayfa kutusunun boyunda,
    /// hapın üstünde kalan alanda ortalı (`page_error` ile aynı kural).
    pub(super) fn gate_page(&self, gate: &Gate, window: &Window, cx: &mut Context<Self>) -> Div {
        let height = f32::from(window.viewport_size().height) - super::TITLEBAR_HEIGHT;
        div().h(px(height)).flex().items_center().justify_center().child(self.gate_card(gate, true, window, cx))
    }

    /// Kütüphane sekmesinde (kaymayan alan) durum.
    pub(super) fn gate_fill(&self, gate: &Gate, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .pt(px(S2))
            .pb(px(super::pill::PILL_CLEARANCE + S5))
            .flex()
            .items_center()
            .justify_center()
            .child(self.gate_card(gate, false, window, cx))
            .into_any_element()
    }

    /// Asıl düğmenin Tab durağı (hep aynı tutamaç: durum değişince odak kalır).
    fn gate_stop(&self, probe: bool, window: &Window, cx: &mut Context<Self>) -> (Option<gpui::FocusHandle>, Look, bool) {
        let (stop, look) = self.stop_look("gate-primary", super::tab::PAGE, true, window, cx);
        (stop, look, probe)
    }

    fn gate_card(&self, gate: &Gate, probe: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        self.mark_shown();
        let accent = self.accent().text;
        let busy = gate.busy;
        let (stop, look, probe) = self.gate_stop(probe, window, cx);
        // Asıl düğmeye Tab durağı ve (kaydırmalı sayfada) görünür yere kaydırma.
        let track = |button: Stateful<Div>| -> AnyElement {
            let button = match &stop {
                Some(h) if probe => button.child(self.focus_probe(h)).track_focus(h),
                Some(h) => button.track_focus(h),
                None => button,
            };
            button.into_any_element()
        };
        let kind = gate.kind.clone();
        let again = |label: &'static str| if busy { t!(GateTrying) } else { label };
        let retry_look = if busy { Look::Disabled } else { look };

        // (işaret, başlık, açıklama, eylemler, not)
        let (mark, title, body, actions, note): (AnyElement, &str, String, Vec<AnyElement>, Option<&'static str>) = match &kind {
            GateKind::SignIn => (
                super::theme::icon("openify-mark", S8, text()).into_any_element(),
                t!(WelcomeTitle),
                t!(WelcomeLine).into(),
                vec![track(
                    primary_button("gate-primary", "user", t!(WelcomeSignIn), look, accent)
                        .on_click(cx.listener(|this, _, _, cx| this.start_login(cx))),
                )],
                Some(t!(WelcomePremiumNote)),
            ),
            GateKind::SigningIn => (
                super::theme::icon("openify-mark", S8, text()).into_any_element(),
                t!(WelcomeWaitingTitle),
                t!(WelcomeWaitingBody).into(),
                vec![track(
                    secondary_button("gate-primary", "x", t!(BtnCancelLogin), look, accent)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx))),
                )],
                None,
            ),
            GateKind::SignInFailed(reason) => (
                state_mark("circle-alert"),
                t!(PillLoginFailedTitle),
                msg::login_failed_text(*reason).into(),
                vec![track(
                    primary_button("gate-primary", "refresh-cw", t!(Retry), look, accent)
                        .on_click(cx.listener(|this, _, _, cx| this.start_login(cx))),
                )],
                Some(t!(WelcomePremiumNote)),
            ),
            GateKind::Premium => {
                // İkincil düğmeler de kendi Tab duraklarıyla (odak halkası yalnız odaktakinde).
                let (check_stop, check_look) = self.stop_look("gate-check", super::tab::PAGE, true, window, cx);
                let (out_stop, out_look) = self.stop_look("gate-sign-out", super::tab::PAGE, true, window, cx);
                let check_look = if busy { Look::Disabled } else { check_look };
                let check = secondary_button("gate-check", "refresh-cw", again(t!(GateCheckAgain)), check_look, accent)
                    .when(!busy, |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.gate_retry(GateKind::Premium, cx)))
                    });
                let out = ghost_button("gate-sign-out", "log-out", t!(SignOut), false, out_look, accent)
                    .on_click(cx.listener(|this, _, _, cx| this.ask_sign_out(cx)));
                let stop_of = |button: Stateful<Div>, h: &Option<gpui::FocusHandle>| -> AnyElement {
                    match h {
                        Some(h) if probe => button.child(self.focus_probe(h)).track_focus(h).into_any_element(),
                        Some(h) => button.track_focus(h).into_any_element(),
                        None => button.into_any_element(),
                    }
                };
                (
                    state_mark("circle-alert"),
                    t!(PillNeedsPremiumTitle),
                    t!(GatePremiumBody).into(),
                    vec![
                        track(
                            primary_button("gate-primary", "arrow-right", t!(GateGetPremium), look, accent)
                                .on_click(cx.listener(|this, _, _, cx| this.open_premium_page(cx))),
                        ),
                        stop_of(check, &check_stop),
                        stop_of(out, &out_stop),
                    ],
                    None,
                )
            }
            GateKind::Offline => (
                state_mark("wifi-off"),
                t!(GateOfflineTitle),
                t!(GateOfflineBody).into(),
                vec![track(
                    primary_button("gate-primary", "refresh-cw", again(t!(GateTryNow)), retry_look, accent)
                        .when(!busy, |el| el.on_click(cx.listener(|this, _, _, cx| this.gate_retry(GateKind::Offline, cx)))),
                )],
                None,
            ),
            GateKind::Stopped => (
                state_mark("circle-alert"),
                t!(PillEngineStoppedTitle),
                t!(GateStoppedBody).into(),
                vec![track(
                    primary_button("gate-primary", "refresh-cw", again(t!(BtnRestart)), retry_look, accent)
                        .when(!busy, |el| el.on_click(cx.listener(|this, _, _, cx| this.gate_restart(cx)))),
                )],
                None,
            ),
            GateKind::Slow => (
                state_mark("refresh-cw"),
                t!(GateSlowTitle),
                t!(GateSlowBody).into(),
                vec![track(
                    primary_button("gate-primary", "refresh-cw", again(t!(GateTryNow)), retry_look, accent)
                        .when(!busy, |el| el.on_click(cx.listener(|this, _, _, cx| this.gate_retry(GateKind::Slow, cx)))),
                )],
                None,
            ),
            GateKind::Failed(error) => {
                let (icon_name, title, message) = explain_error(error);
                let error = error.clone();
                (
                    state_mark(icon_name),
                    title,
                    message.into(),
                    vec![track(
                        primary_button("gate-primary", "refresh-cw", again(t!(Retry)), retry_look, accent).when(!busy, |el| {
                            el.on_click(cx.listener(move |this, _, _, cx| this.gate_retry(GateKind::Failed(error.clone()), cx)))
                        }),
                    )],
                    None,
                )
            }
        };
        let card = div().child(full_state(mark, title, body, actions, note.map(Into::into)));
        fade_in(ElementId::Name(format!("gate-{}", kind.id()).into()), card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(conn: Connection, trying: Option<&GateKind>, failure: Option<&str>, failed_all: bool, slow: bool) -> Option<GateKind> {
        decide(&conn, trying, failure, failed_all, slow).map(|g| g.kind)
    }

    #[test]
    fn every_blocked_connection_has_its_own_state() {
        let any = |c| kind(c, None, None, false, false);
        assert_eq!(any(Connection::NeedsLogin), Some(GateKind::SignIn));
        assert_eq!(any(Connection::LoggingIn), Some(GateKind::SigningIn));
        assert_eq!(
            any(Connection::LoginFailed(LoginFailReason::TimedOut)),
            Some(GateKind::SignInFailed(LoginFailReason::TimedOut))
        );
        assert_eq!(any(Connection::NeedsPremium), Some(GateKind::Premium));
        assert_eq!(any(Connection::EngineStopped), Some(GateKind::Stopped));
        assert_eq!(any(Connection::Failed("network: x".into())), Some(GateKind::Offline));
    }

    #[test]
    fn loading_shows_no_state_until_it_is_slow() {
        for conn in [Connection::Connecting, Connection::Connected("a".into()), Connection::NoAudio] {
            assert_eq!(kind(conn.clone(), None, None, false, false), None);
            assert_eq!(kind(conn.clone(), None, None, false, true), Some(GateKind::Slow));
            // Sayfanın kendi hata görünümü varsa "uzun sürüyor" denmez.
            assert_eq!(kind(conn, None, None, true, true), None);
        }
    }

    #[test]
    fn a_library_failure_while_connected_is_explained() {
        let k = kind(Connection::Connected("a".into()), None, Some("boom"), false, false);
        assert_eq!(k, Some(GateKind::Failed("boom".into())));
        // Çevrimdışıyken hata metni değil çevrimdışı durumu.
        assert_eq!(kind(Connection::Failed("e".into()), None, Some("network: x"), false, false), Some(GateKind::Offline));
    }

    #[test]
    fn pressing_try_now_keeps_the_state_busy_while_connecting() {
        let d = decide(&Connection::Connecting, Some(&GateKind::Offline), None, false, false).unwrap();
        assert_eq!(d.kind, GateKind::Offline);
        assert!(d.busy);
        // Sonuç gelince (çevrimdışı yine) normal düğme.
        let d = decide(&Connection::Failed("e".into()), Some(&GateKind::Offline), None, false, false).unwrap();
        assert!(!d.busy);
    }

    #[test]
    fn every_state_has_text_in_both_languages() {
        use crate::i18n::{Lang, with_lang};
        for lang in [Lang::Tr, Lang::En] {
            with_lang(lang, || {
                for text in [
                    t!(WelcomeTitle),
                    t!(WelcomeLine),
                    t!(WelcomeSignIn),
                    t!(WelcomePremiumNote),
                    t!(WelcomeWaitingTitle),
                    t!(WelcomeWaitingBody),
                    t!(GateOfflineTitle),
                    t!(GateOfflineBody),
                    t!(GateTryNow),
                    t!(GateTrying),
                    t!(GatePremiumBody),
                    t!(GateGetPremium),
                    t!(GateCheckAgain),
                    t!(GateStoppedBody),
                    t!(GateSlowTitle),
                    t!(GateSlowBody),
                ] {
                    assert!(!text.is_empty());
                }
            });
        }
    }

    #[test]
    fn timings_are_calm() {
        // İskelet gecikmesi hissedilmez ama bağlantı sonucunu bekler; "uzun
        // sürüyor" bir iki saniyede değil, gerçekten uzun sürünce.
        assert!(SKELETON_DELAY >= Duration::from_millis(200) && SKELETON_DELAY <= Duration::from_millis(600));
        assert!(SLOW_AFTER >= Duration::from_secs(10));
        assert!(TRY_HOLD < SLOW_AFTER);
        assert!(SETTLE_WINDOW > motion());
    }
}
