//! Openify penceresi (gpui).
//!
//! Yerleşim "Raf + yüzen hap" (`DESIGN.md`): üstte kendi çizdiğimiz başlık
//! çubuğu, altında son çalınana göre sıralı kapak rafı (`shelf.rs`), açık
//! listenin tek satırlık başlığı ve şarkıları; çalar listenin üstünde yüzen
//! bir hap (`pill.rs`). Pencerenin üst kısmında çalan şarkının kapağından
//! türetilen bir ışıma var.
//!
//! Kısayollar:
//!   Ctrl+K        komut paleti (arama)
//!   Ctrl+V        panodaki şarkı bağlantısını çal
//!   Boşluk        oynat / duraklat
//!   ↑ / ↓         listede satır seç, Enter çal
//!   ← / →         rafta gez (raf odaktayken), Enter aç
//!   Ctrl+← / →    önceki / sonraki şarkı
//!   Ctrl+↑ / ↓    ses düzeyi
//!   Alt+S / Alt+R karıştır / tekrar (kapalı → hepsi → tek)
//!   Alt+Shift+Q   sıra paneli, Alt+Shift+L sözler
//!   F11           tam ekran (Esc çıkar)

mod added_date;
mod components;
mod credits;
mod design_rules;
mod equalizer;
mod explore;
mod frames;
mod friends;
mod fullscreen;
mod fuzzy;
mod gallery;
mod history;
mod home;
mod images;
mod integration;
mod karaoke;
mod library;
mod library_layout;
mod library_tree;
mod link_text;
mod listening;
mod lyrics_view;
mod manage;
mod now_panel;
mod now_playing;
mod pages;
mod palette;
mod pill;
mod pill_glass;
mod player;
#[cfg(windows)]
mod popups;
mod queue_panel;
mod recent_page;
mod search_page;
mod settings;
mod shelf;
mod shortcuts;
mod smooth_scroll;
mod text;
mod theme;
mod tools;
mod tools_view;
mod tween;
mod views;
mod welcome;
#[cfg(windows)]
mod windows_titlebar;

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, AnyView, Application, Bounds, Context, DispatchPhase, FocusHandle, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString,
    StyleRefinement, Subscription, Task, TitlebarOptions, UniformListScrollHandle, Window,
    WindowBounds, WindowControlArea, WindowOptions, canvas, div, img, linear_color_stop,
    linear_gradient, prelude::*, px, rgb, size,
};
use librespot::core::SpotifyUri;

use crate::t;
use crate::{
    engine::{self, CollectionId, Command, EngineHandle, palette::Accent},
    link,
    settings::Settings,
};
use components::Look;
use images::ImageStore;
use library::{LibraryState, Loadable};
use palette::PaletteState;
use pill::{PILL_BOTTOM, PILL_HEIGHT, pill_width};
use player::{Connection, PlayerState, Status};
use shelf::AllState;
use shortcuts::Shortcut;
use smooth_scroll::SmoothScroll;
use text::line;
use theme::*;

const SEEK_STEP_MS: u32 = 5_000;
const VOLUME_STEP: f32 = 0.05;
/// Bu süreden sonra "önceki" düğmesi şarkıyı başa sarar, öncesinde önceki şarkıya geçer.
const RESTART_THRESHOLD_MS: u32 = 3_000;
const TITLEBAR_HEIGHT: f32 = 44.;
/// Sağ üstteki küçült / büyüt / kapat düğmelerinin her birinin eni
/// (Windows 11'in kendi başlık düğmeleri gibi).
const CAPTION_BUTTON_WIDTH: f32 = 46.;
/// Kütüphane arama alanının boyu.
const SEARCH_HEIGHT: f32 = 32.;
/// Başlık çubuğundaki arama hapının boyu (çubuk 44 px: dört px pay).
const TITLE_SEARCH_HEIGHT: f32 = 36.;
/// Hap zamanlayıcısının en kısa ve en uzun beklemesi (ms). Hap yalnız ekranda
/// bir şey değişince (dolu kısım bir piksel uzayınca ya da süre yazısı bir
/// saniye ilerleyince) yeniden çizilir; en kısa bekleme, çok kısa şarkıda ya
/// da hesap biraz erken düşünce boşa dönmesin diye.
const BAR_WAIT_MIN_MS: f32 = 4.;
const BAR_WAIT_MAX_MS: f32 = 1000.;
/// İlerleme çubuğu henüz ölçülmediyse varsayılan genişliği (px).
const PROGRESS_WIDTH_GUESS: f32 = 240.;
/// Pencerenin en dar eni (`window_min_size`); yerleşik bir panelin yanında
/// içerik alanı da bunun altına inmez (bkz. `now_panel::FRIENDS_DOCK_MIN_WIDTH`).
pub(super) const MIN_WINDOW_WIDTH: f32 = 760.;
/// Bu genişliğin altında dar düzen (arama kısalır).
const COMPACT_BELOW: f32 = 1000.;
/// Bu yüksekliğin altında raf şerit hâlinde, başlık küçük açılır: en küçük
/// pencerede (500 px) açık raf ve başlık listenin yarısını yiyordu.
const SHORT_BELOW: f32 = 640.;
/// Resim indikten sonra bütün pencerenin yenilenmesi için bekleme: gpui
/// resmi bu sürede çözmüş olur (bkz. `Shell::refresh_after_decode`).
const IMAGE_REFRESH_DELAY: Duration = Duration::from_millis(120);

/// Pencereyi açar ve uygulama kapanana kadar bekler.
pub fn run(start_hidden: bool) {
    // Motor gpui'den ÖNCE başlar: gpui / GPU hazırlığı (~0,4 sn) sürerken motor
    // önbelleği açar, kütüphaneyi ve açılış listesini okur; pencere kurulurken
    // olaylar hazır bekler (bkz. `Shell::new`).
    let settings = Settings::load();
    #[cfg(windows)]
    theme::set_light(crate::system::app_theme::light(settings.theme));
    #[cfg(not(windows))]
    theme::set_light(settings.theme == crate::settings::ThemePreference::Light);
    // Arayüz dili: Windows'un dili ya da Ayarlar'daki seçim (bkz. `i18n`).
    crate::i18n::apply_preference(&settings.language);
    // Ses ayarları motor kurulmadan önce (ilk şarkıdan geçerli olsun).
    engine::apply_audio(settings.kbps(), settings.normalise);
    engine::apply_normalisation_level(settings.normalise_pregain_db());
    engine::set_output_device(settings.output_device.clone());
    engine::set_resume(settings.resume);
    // Spotify Connect açıksa oturum kalıcı aygıt kimliğiyle açılır (kapalıyken hiçbir şey).
    engine::apply_connect_identity(settings.connect_enabled, &settings.device_id);
    theme::set_reduce_motion(&settings.reduce_motion);
    let engine = engine::start();
    engine.send(Command::SetVolume(settings.effective_volume()));
    engine.send(Command::SetQueueRules(settings.queue_rules()));
    log::debug!("aşama {} ms: motor başlatıldı", crate::since_start());
    let app = Application::new().with_assets(Assets);
    log::debug!("aşama {} ms: gpui hazır", crate::since_start());
    app.run(move |cx: &mut gpui::App| {
        log::debug!("aşama {} ms: gpui döngüsü başladı", crate::since_start());
        load_fonts(cx);
        log::debug!("aşama {} ms: yazı tipleri yüklendi", crate::since_start());
        // Geliştirme denemesi: OPENIFY_TEST_DISPLAY="genişlik,yükseklik"
        // verilirse pencere ikinci ekranın ortasında açılır ve odak istemez
        // (ana ekrandaki işi bölmemek için). Normal açılışı etkilemez.
        let test_size: Option<Vec<f32>> = std::env::var("OPENIFY_TEST_DISPLAY")
            .ok()
            .map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect());
        let mut display_id = None;
        let mut window_size = size(px(1180.), px(760.));
        if let Some(test_size) = &test_size {
            let primary = cx.primary_display().map(|d| d.id());
            display_id = cx
                .displays()
                .into_iter()
                .map(|d| d.id())
                .find(|id| Some(*id) != primary);
            if let [w, h] = test_size.as_slice() {
                window_size = size(px(*w), px(*h));
            }
        }
        let bounds = Bounds::centered(display_id, window_size, cx);
        let focus = test_size.is_none() && !start_hidden;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id,
                // Başlık çubuğunu kendimiz çiziyoruz.
                titlebar: Some(TitlebarOptions {
                    title: Some(crate::brand::APP_NAME.into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(500.))),
                app_id: Some("Openify".into()),
                focus,
                // `--tray` (Windows açılışı): pencere hiç gösterilmez; tepsi
                // kurulunca gizli kalır (bkz. `integration::Op::StartHidden`).
                show: !start_hidden,
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Shell::new(window, cx, engine, settings, start_hidden)),
        )
        .expect("Openify penceresi açılamadı");

        // Pencere kapanınca uygulama da kapansın (ses motoru da onunla gider).
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if focus {
            cx.activate(true);
        }
    });
}

/// Uygulamanın kök görünümü.
struct Shell {
    /// Pencere dar mı (1000 px altı)? Arama kısalır.
    compact: bool,
    /// Pencere kısa mı (`SHORT_BELOW`): raf şerit hâlinde açılır.
    pub(crate) short: bool,
    engine: EngineHandle,
    state: PlayerState,
    library: LibraryState,
    palette: PaletteState,
    /// "Tümü": bütün kütüphane, kapak ızgarasında.
    all: AllState,
    /// Sayfalar ve geri / ileri (Adım 12, bkz. `pages`).
    pages: pages::PageState,
    /// Beğenme, menü, seçim, bul, geri al, eşitleme (Adım 13, bkz. `manage`).
    manage: manage::ManageState,
    images: ImageStore,
    /// Sessize alınmadan önceki ses düzeyi.
    volume_before_mute: Option<f32>,
    /// Son çalınan listelerin anahtarları, en yenisi başta (raf sırası;
    /// ayarlarda saklanır).
    recent: Vec<String>,
    focus: FocusHandle,
    /// Tab ile gezilen düğmeler.
    nav: Nav,
    /// Rafta klavyeyle gezilen kart (son yer "Tümü").
    shelf_cursor: usize,
    /// Son hareket klavyeden mi (Tab)? Odak halkası yalnızca o zaman görünür;
    /// fareyle tıklayınca gizlenir.
    keyboard_focus: bool,
    /// İlerleme ve ses çubuklarının ekrandaki yeri (tıklamayı orana çevirmek için).
    progress_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Sürüklenen çubuk. İlerleme çubuğu sürüklenirken konum yalnızca
    /// ekranda değişir, bırakınca atlanır (motora art arda sarma gitmez).
    dragging: Option<Drag>,
    volume_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Karaoke miktar sürgüsünün ekrandaki yeri: sözler paneli ve tam ekran (bkz. `karaoke`).
    karaoke_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    karaoke_full_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Çalarken süre yazısını ve çubuğu güncel tutan görev.
    ticker: Option<Task<()>>,
    /// Hapın son çizildiği hâlin özeti (zamanlayıcı yalnız bu değişince
    /// hapı yeniler).
    bar_signature: Option<BarSignature>,
    /// Ekranın piksel ölçeği (ilerlemenin kaç piksel olduğunu bulmak için).
    scale_factor: f32,
    /// Önbelleğe alınan parçalar (bkz. `views`).
    views: views::Views,
    /// Listede çalan satırın numara hücresi ve satırların göründüğü alan
    /// (ekranda). Liste çizilirken ölçülür, ekolayzer katmanı okur; çalan
    /// satır görünmüyorsa `None`.
    playing_cell: Rc<Cell<Option<Bounds<Pixels>>>>,
    list_viewport: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Bilgi bildiriminin süresi dolunca yeniden çizim.
    notice_expiry: Option<Task<()>>,
    /// İnen resimler çözülünce bütün pencerenin bir kez yenilenmesi.
    image_refresh: Option<Task<()>>,
    /// Pencerenin içerik genişliği (px; yerleşik panel açıkken onun solu),
    /// son çizimden. Rafa kaç kart sığacağı ve ızgaranın sütunları bundan.
    window_width: f32,
    /// Pencerenin eni ve boyu (px), son çizimden: arama hapının eni ve arama
    /// yüzeyinin sığacağı yer bundan.
    viewport_w: f32,
    viewport_h: f32,
    /// Hapın eni (px), son çizimden: pencereden (`pill_width`), yerleşik
    /// panele yetişirse daha dar. Dar hapın kuralları buna göre.
    pill_width: f32,
    _events: Task<()>,
    _appearance: Subscription,
    /// Sistemle bütünleşme (Adım 10, bkz. `integration`).
    desk: integration::Desk,
    /// İlk karenin çizildiği an kayda düşsün diye.
    first_frame: bool,
    /// Bir sonraki çizimde odağı görünmez öğeye geri ver.
    refocus_anchor: bool,
    /// Şimdi çalıyor: sıra, sözler, tam ekran (bkz. `now_playing`).
    now: now_playing::NowState,
    /// Kullanıcının Ayarlar sayfasından seçtikleri (diske yazılırken ses,
    /// son çalınanlar gibi çalışma anı durumu üstüne eklenir).
    prefs: Settings,
    /// Ayarlar sayfası, profil menüsü, onay kutusu (bkz. `settings`).
    sui: settings::SettingsUi,
    /// Karşılama ve "veri yok" durumları (bkz. `welcome`).
    welcome: welcome::WelcomeUi,
    /// Arkadaş etkinliği paneli ve yenileme kuralları (bkz. `friends`).
    friends: friends::FriendsUi,
    /// Kütüphane araçları sayfası ve dinleme sayacı (bkz. `tools`).
    tools: tools::ToolsUi,
    /// Gizlenen şarkılar ve gizli oturum (bkz. `listening`).
    listen: listening::ListeningUi,
}

impl Shell {
    /// Tercih ya da Windows uygulama modu değişti: renkleri, kapak yüzeylerini
    /// ve önbellekli çocukları aynı güncellemede yenile. Sınama zorlaması
    /// yalnız çözümleme katmanında kalır, `prefs.theme` değiştirilmez.
    fn apply_theme(&mut self, cx: &mut Context<Self>) {
        #[cfg(windows)]
        let light = crate::system::app_theme::light(self.prefs.theme);
        #[cfg(not(windows))]
        let light = self.prefs.theme == crate::settings::ThemePreference::Light;
        if theme::set_light(light) {
            self.refresh_theme_surfaces(cx);
            cx.notify();
        }
        #[cfg(windows)]
        if let Some(hwnd) = self.desk.hwnd {
            windows_titlebar::apply_hwnd(hwnd);
        }
    }

    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        engine: EngineHandle,
        settings: Settings,
        start_hidden: bool,
    ) -> Self {
        let prefs = settings.clone();
        log::debug!("aşama {} ms: pencere oluşturuldu", crate::since_start());
        #[cfg(windows)]
        windows_titlebar::apply(window);

        // gpui çerçeveyi Windows moduna döndürebilir: etkin uygulama temasını
        // yeniden uygula. Asıl uygulama modu ayrıca ImmersiveColorSet
        // kanalından okunur; görev çubuğunun modu uygulamadan farklı olabilir.
        let appearance = cx.observe_window_appearance(window, |this, _window, cx| {
            this.apply_theme(cx);
            #[cfg(windows)]
            windows_titlebar::apply(_window);
        });

        // Motorun gpui hazırlanırken gönderdiği olaylar (kütüphane, açılış
        // listesi) ilk çizimden önce işlenir: liste ilk karede gelir. Motor
        // gpui'den önce başladığı için bunlar pencere kurulurken çoktan hazırdır;
        // beklenmez (eskiden en fazla 150 ms uyunurdu). Gelmediyse (çok yavaş
        // disk) liste "yükleniyor" iskeletiyle açılır, olay gelince dolar.
        let mut early = Vec::new();
        while let Ok(event) = engine.events.try_recv() {
            early.push(event);
        }
        log::debug!(
            "aşama {} ms: erken olaylar {} (liste {})",
            crate::since_start(),
            early.len(),
            if early
                .iter()
                .any(|e| matches!(e, engine::Event::CollectionLoaded { .. }))
            {
                "hazır"
            } else {
                "yok"
            }
        );

        // Pencere kapanırken ses düzeyini ve son çalınanları kaydet. Tepsi
        // simgesi varsa pencere kapanmaz, tepsiye gizlenir (Adım 10; bkz.
        // `integration`).
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            this.upgrade()
                .is_none_or(|shell| shell.update(cx, |shell, cx| shell.on_close_requested(cx)))
        });

        // Motordan gelen olayları sırayla işle.
        let events = engine.events.clone();
        let events_task = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update(cx, |shell, cx| shell.on_engine_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
            // Olay kanalı kapandı: motor iş parçacığı ölmüş (tutamak kanalı
            // açık tutar, normalde `EngineStopped` zaten gelir). Zombi kalma.
            let _ = this.update(cx, |shell, cx| {
                shell.on_engine_event(
                    engine::Event::EngineStopped("event channel closed".into()),
                    cx,
                )
            });
        });

        // Sıra ve kaldığı yer olayları ayrı kanalda (bkz. `now_playing`).
        let playback = engine.playback.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = playback.recv().await {
                if this
                    .update(cx, |shell, cx| shell.on_playback_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus);
        log::info!("pencere açıldı");
        // Geliştirme denemesi: OPENIFY_TEST_PLAY_AFTER=saniye verilirse açık
        // listenin ilk şarkısı o kadar sonra çalınır. Açılıştaki eşitleme
        // sırasında çalmanın bozulmadığını sınamak için.
        if let Some(secs) = std::env::var("OPENIFY_TEST_PLAY_AFTER")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                let _ = this.update(cx, |shell, cx| shell.test_play(cx));
            })
            .detach();
        }

        // Parça görünümleri: kendi başlarına yenilenebilsinler diye ayrı.
        let views = views::Views::new(&cx.entity(), window, cx);
        // Medya tuşları, görev çubuğu, tepsi, tek kopya, her yerden palet.
        let desk = integration::Desk::new(window, cx, &prefs, start_hidden);

        let mut now = now_playing::NowState::new(engine.levels(), cx);
        // Geniş pencerede "Şimdi çalıyor" paneli açık mı (kullanıcı kapattıysa kapalı kalır).
        now.dock.open = settings.now_panel;
        let mut palette = PaletteState::new();
        palette.recent_searches = settings.recent_searches.clone();
        let mut shell = Self {
            now,
            engine,
            state: PlayerState::new(settings.effective_volume()),
            library: LibraryState::new(),
            palette,
            all: AllState::from_settings(&settings),
            pages: pages::PageState::new(),
            manage: manage::ManageState::new(),
            images: ImageStore::default(),
            compact: false,
            short: false,
            volume_before_mute: settings.muted.then_some(settings.volume),
            recent: settings.recent,
            focus,
            nav: Nav::new(cx),
            keyboard_focus: false,
            shelf_cursor: 0,
            progress_bounds: Rc::default(),
            dragging: None,
            volume_bounds: Rc::default(),
            karaoke_bounds: Rc::default(),
            karaoke_full_bounds: Rc::default(),
            ticker: None,
            bar_signature: None,
            scale_factor: window.scale_factor(),
            views,
            playing_cell: Rc::default(),
            list_viewport: Rc::default(),
            notice_expiry: None,
            image_refresh: None,
            window_width: f32::from(window.viewport_size().width),
            viewport_w: f32::from(window.viewport_size().width),
            viewport_h: f32::from(window.viewport_size().height),
            pill_width: pill_width(f32::from(window.viewport_size().width)),
            _events: events_task,
            _appearance: appearance,
            desk,
            first_frame: true,
            refocus_anchor: false,
            prefs,
            sui: settings::SettingsUi::new(),
            welcome: welcome::WelcomeUi::new(),
            friends: friends::FriendsUi::new(),
            tools: tools::ToolsUi::new(),
            listen: listening::ListeningUi::new(),
        };
        shell.apply_prefs(true);
        // Sınama kancası `OPENIFY_TEST_FRIENDS_JSON`: arkadaş listesi fikstürden.
        shell.init_friends_fixture();
        for event in early {
            shell.on_engine_event(event, cx);
        }
        // Açılışta kaldığı yer (motor diskten okuyup hemen gönderiyor): ilk
        // karede duraklatılmış hâliyle görünsün.
        while let Ok(event) = shell.engine.playback.try_recv() {
            shell.on_playback_event(event, cx);
        }
        // Geliştirme denemesi: OPENIFY_TEST_FAKE_PLAY verilirse beğenilenlerin
        // ilk şarkısı motora hiçbir şey gönderilmeden "çalıyor" gösterilir:
        // çalarken arayüzün çizim yükünü Spotify'a dokunmadan ölçmek için.
        // `OPENIFY_TEST_FAKE_ROW=n`: ilki yerine n. şarkı (farklı kapaklarla
        // hapın ışığını sınamak için).
        if std::env::var_os("OPENIFY_TEST_FAKE_PLAY").is_some()
            && let Some(t) = shell
                .library
                .tracks(&CollectionId::Liked)
                .and_then(|t| t.get(fake_row()))
                .cloned()
        {
            let now = Instant::now();
            shell.state.apply(
                engine::Event::TrackChanged(engine::TrackInfo {
                    uri: t.uri.clone(),
                    name: t.name.clone(),
                    artists: t.artists.clone(),
                    duration_ms: t.duration_ms,
                    cover_url: None,
                }),
                now,
            );
            shell.state.apply(
                engine::Event::QueueChanged(engine::QueueState {
                    context: Some(CollectionId::Liked),
                    index: 0,
                    has_next: true,
                    has_previous: false,
                }),
                now,
            );
            shell
                .state
                .apply(engine::Event::Playing { position_ms: 0 }, now);
            shell.update_ticker(cx);
            shell.sync_system(true);
        }
        // Sınama kancaları `OPENIFY_TEST_PRIVATE` / `OPENIFY_TEST_RADIO_JSON` (bkz. `listening`).
        shell.init_listening_fixtures();
        // Geliştirme denemesi: OPENIFY_TEST_NOTICE="tür:metin" (working,
        // info, problem) açılışta bir bildirim gösterir; çalmadan bildirim
        // görünümünü sınamak için.
        if let Some((kind, message)) = std::env::var("OPENIFY_TEST_NOTICE").ok().and_then(|v| {
            let (kind, message) = v.split_once(':')?;
            let kind = match kind {
                "working" => engine::NoticeKind::Working,
                "info" => engine::NoticeKind::Info,
                _ => engine::NoticeKind::Problem,
            };
            Some((kind, message.to_string()))
        }) {
            shell.on_engine_event(engine::Event::Notice { kind, message }, cx);
        }
        // Ana sayfanın Spotify akışı (bağlantı yoksa bağlanınca sorulur).
        shell.ask_explore(&[engine::ExploreKey::Home]);
        shell.apply_test_view(window, cx);
        shell
    }

    /// Kalıcı ayarlar: ses düzeyi (sessizse sessize almadan önceki düzey)
    /// ve son çalınan listeler.
    fn save_settings(&self) {
        // Ayarlar sayfasının değerleri `prefs`te; üstüne çalışma anı durumu.
        // (`now_panel` bir tercih: panel çalışırken açılıp kapanınca değişmez.)
        let settings = Settings {
            volume: self.volume_before_mute.unwrap_or(self.state.volume),
            muted: self.volume_before_mute.is_some(),
            recent: self.recent.clone(),
            library_view: if self.all.list { "list" } else { "grid" }.into(),
            library_sort: self.all.sort.key().into(),
            recent_searches: self.palette.recent_searches.clone(),
            ..self.prefs.clone()
        };
        if let Err(e) = settings.save() {
            log::warn!("Ayarlar kaydedilemedi: {e:#}");
        }
    }

    /// Kullanıcı bu listeyi çalmaya başladı: raf sırası için "son çalınan"
    /// olarak kaydedilir ve hemen diske yazılır (dosya küçük; çökmede
    /// kaybolmasın).
    fn note_played(&mut self, id: &CollectionId) {
        // Gizli oturumda Son çalınanlar'a yazılmaz (bkz. `listening`).
        if listening::note_recent(self.private_session(), &mut self.recent, id.key()) {
            self.save_settings();
        }
    }

    /// O anki vurgu rengi (şarkı değişince eski renkten yenisine geçer).
    ///
    /// Vurgu yazısı her kapakta okunur: ışımanın en parlak yerine (ve dolayısıyla
    /// zemine) göre en az 4,5:1 olacak kadar açılır, tonu korunur. Koyu bir
    /// kapaktan gelen lacivert, zeminde okunmuyordu.
    fn accent(&self) -> Accent {
        readable_accent(self.state.accent_at(Instant::now()))
    }

    fn on_engine_event(&mut self, event: engine::Event, cx: &mut Context<Self>) {
        log::debug!("motor olayı: {}", short_debug(&event));
        self.on_now_event(&event, cx);
        self.welcome_event(&event, cx);
        // Yerel dinleme istatistiği: çalma olaylarından sayılan çalmalar (bkz. `tools`).
        self.tools_note_playback(&event, cx);
        match &event {
            engine::Event::LibraryLoaded(library) => {
                log::info!("kütüphane gösterildi ({} liste)", library.playlists.len());
                self.library.library = Loadable::Loaded(library.clone());
                if self.all.open {
                    self.recompute_all();
                }
                // Açılış sayfası ana sayfa (geçmiş orada başlar). Beğenilenler
                // yine arkadan yüklenir: kaldığı yerden çalma ve "Beğenilenler"e
                // dönüş anında olsun. Listeye geçmiş eklenmez.
                if self.library.selected.is_none() {
                    self.show_collection(CollectionId::Liked, None, cx);
                }
            }
            engine::Event::LibraryFailed(e) => {
                self.library.library = Loadable::Failed(e.clone());
            }
            engine::Event::CollectionLoaded { id, tracks } => {
                log::info!("liste gösterildi: {} ({} şarkı)", id.key(), tracks.len());
                self.library
                    .collections
                    .insert(id.clone(), Loadable::Loaded(tracks.clone()));
                // "Eklenme tarihi" sütunu için tarihler (önbellekten, ağa gidilmez).
                self.request_added_dates(id);
                self.play_if_waiting(id, cx);
                self.tools_on_data(cx);
            }
            // Sayfalar, şarkı bağları, yazma kuyruğu (bkz. `pages`, `manage`).
            engine::Event::Library(e) => self.on_library_event(e.clone(), cx),
            engine::Event::CollectionFailed { id, error } => {
                self.library
                    .collections
                    .insert(id.clone(), Loadable::Failed(error.clone()));
            }
            engine::Event::SearchIndex(tracks) => {
                log::info!("arama dizini: {} şarkı", tracks.len());
                // Anahtarları arka planda hazırla: ana iş parçacığında
                // açılıştaki listeyi ~0,1 sn geciktiriyordu.
                let tracks = tracks.clone();
                let keys = cx.background_spawn({
                    let tracks = tracks.clone();
                    async move { palette::index_keys(&tracks) }
                });
                cx.spawn(async move |this, cx| {
                    let keys = keys.await;
                    let _ = this.update(cx, |shell, cx| {
                        shell.palette.set_index(tracks, keys);
                        if shell.palette.open {
                            shell.recompute_palette();
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            engine::Event::SearchResults { seq, results } => {
                log::info!(
                    "Spotify araması: {} şarkı, {} sanatçı, {} albüm, {} liste",
                    results.tracks.len(),
                    results.artists.len(),
                    results.albums.len(),
                    results.playlists.len()
                );
                // Tam arama sayfasınınki mi, paletinki mi?
                if !self.on_page_search(*seq, Ok(results.clone())) {
                    self.on_search_results(*seq, Ok(results.clone()));
                }
            }
            engine::Event::SearchFailed { seq, error } => {
                log::warn!("Spotify araması başarısız: {error}");
                if !self.on_page_search(*seq, Err(error.clone())) {
                    self.on_search_results(*seq, Err(error.clone()));
                }
            }
            engine::Event::ImageReady(url) => {
                self.images.mark_ready(url);
                self.refresh_after_decode(cx);
            }
            // Motor sıradaki şarkıya geçtiyse (atlama, sonraki): yüklenene
            // kadar hap o şarkıyı göstersin.
            engine::Event::QueueChanged(queue) => {
                let next = queue
                    .context
                    .as_ref()
                    .and_then(|id| self.library.tracks(id))
                    .and_then(|t| t.get(queue.index));
                if let Some(t) = next
                    && self
                        .state
                        .track
                        .as_ref()
                        .is_none_or(|current| current.uri != t.uri)
                {
                    self.state.pending = Some(player::PendingTrack {
                        name: t.name.clone(),
                        image: t.image.clone(),
                    });
                }
            }
            _ => {}
        }
        // Sarma, şarkı değişimi, çalmaya başlama ve konum düzeltmesi
        // zamanlayıcının takvimini eskitir: yeniden kurulsun (yoksa hap
        // eski takvimle bir saniyeye kadar geç kalabiliyordu).
        let reschedule = matches!(
            event,
            engine::Event::TrackChanged(_)
                | engine::Event::Playing { .. }
                | engine::Event::Position { .. }
        );
        // Windows medya kontrolünün konumu da bunlarda (ve durunca) yenilenir.
        let timeline =
            reschedule || matches!(event, engine::Event::Paused { .. } | engine::Event::Ended);
        self.state.apply(event, Instant::now());
        // Yerini yenisine bırakan kapakların çözülmüş pikselleri gpui'nin
        // önbelleğinden atılır (yoksa her şarkıda bir kapak birikiyordu).
        for image in self.state.take_retired() {
            image.remove_asset(cx);
        }
        self.arm_notice_expiry(cx);
        if reschedule {
            self.restart_ticker(cx);
        } else {
            self.update_ticker(cx);
        }
        self.sync_system(timeline);
        self.after_now_event(cx);
        cx.notify();
    }

    /// Resim indi: çizim onu çözmeye başlar, ama gpui çözülünce yalnız onu
    /// ilk isteyen görünümü yeniler (`Window::use_asset`). Aynı kapak iki
    /// görünümde (raf ve "Tümü", hap ve satır) olabiliyor: çözme bitince bir
    /// kez bütün pencere yenilensin. Art arda gelen resimler tek yenilemede
    /// toplanır (her yeni resim bekleyeni öteler).
    fn refresh_after_decode(&mut self, cx: &mut Context<Self>) {
        self.image_refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(IMAGE_REFRESH_DELAY).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }

    /// Çizim sırasında istenen resimleri motora gönder (çizimden hemen sonra).
    fn flush_image_requests(&self, cx: &mut Context<Self>) {
        // Görünen satırların sanatçı / albüm bağları da (bkz. `pages`).
        self.flush_link_requests();
        if !self.images.has_pending() {
            return;
        }
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |shell, _| {
                let urls = shell.images.take_pending();
                if !urls.is_empty() {
                    shell.engine.send(Command::FetchImages(urls));
                }
            });
        });
    }

    /// Çalarken hapı güncel tut; dururken hiç çizme.
    ///
    /// Yalnız hapın görünümü yenilenir, `Shell` değil: `Shell`'in
    /// yenilenmesi bütün pencereyi yeniden kurdururdu (bkz. `views`).
    fn update_ticker(&mut self, cx: &mut Context<Self>) {
        // Pencere tepsiye gizliyken de durur (gösterilince yeniden kurulur).
        if !self.state.is_playing() || self.window_hidden() {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.restart_ticker(cx);
    }

    /// Zamanlayıcıyı şimdiden yeniden kur (konum sıçradıysa eski takvim
    /// geçersiz). Çalmıyorsa durdurur.
    fn restart_ticker(&mut self, cx: &mut Context<Self>) {
        if !self.state.is_playing() || self.window_hidden() {
            self.ticker = None;
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(wait) = this.update(cx, |shell, cx| shell.tick_bar(cx)) else {
                    break;
                };
                cx.background_executor().timer(wait).await;
            }
        }));
    }

    /// Hap ekranda değişecekse yalnız onu yeniden çiz. Bir sonraki
    /// değişime kadar ne kadar beklenebileceğini döndürür.
    fn tick_bar(&mut self, cx: &mut Context<Self>) -> Duration {
        let now = Instant::now();
        let signature = self.bar_signature_at(now);
        if self.bar_signature != Some(signature) {
            self.bar_signature = Some(signature);
            if !self.now.full.open {
                self.notify_pill(cx);
            } else if self.now.full.controls {
                // Tam ekranda hap yok; ilerleme denetimler görünürken orada.
                self.views.full.update(cx, |_, cx| cx.notify());
            }
        }
        self.next_bar_change(now)
    }

    /// İlerleme çubuğunun ekrandaki genişliği (cihaz pikseli).
    fn progress_device_width(&self) -> f32 {
        let width = self
            .progress_bounds
            .get()
            .map_or(PROGRESS_WIDTH_GUESS, |b| f32::from(b.size.width));
        width * self.scale_factor
    }

    /// Hapta zamanla değişen her şeyin özeti: süre yazısının saniyesi ve
    /// dolu kısmın kaç piksel olduğu.
    fn bar_signature_at(&self, now: Instant) -> BarSignature {
        BarSignature {
            playing: self.state.is_playing(),
            seconds: self.state.position_ms(now) / 1000,
            fill_px: (self.state.progress(now) * self.progress_device_width()).floor() as i32,
        }
    }

    /// Hapta bir sonraki görünür değişime kalan süre: süre yazısının
    /// saniyesi ya da dolu kısmın bir piksel uzaması, hangisi önceyse.
    fn next_bar_change(&self, now: Instant) -> Duration {
        let position = self.state.position_ms(now) as f32;
        let to_second = 1000. - position % 1000.;
        let duration = self.state.duration_ms() as f32;
        let width = self.progress_device_width();
        let to_pixel = if duration > 0. && width > 0. {
            let ms_per_px = duration / width;
            ((position / ms_per_px).floor() + 1.) * ms_per_px - position
        } else {
            f32::INFINITY
        };
        // Hesap bir tık erken kalırsa değişim kaçmasın: bir milisaniye pay.
        let ms = to_second
            .min(to_pixel)
            .clamp(BAR_WAIT_MIN_MS, BAR_WAIT_MAX_MS)
            + 1.;
        Duration::from_secs_f32(ms / 1000.)
    }

    // ── Kullanıcı eylemleri ─────────────────────────────────────────────

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        match self.state.status {
            Status::Playing => self.engine.send(Command::Pause),
            Status::Paused | Status::Loading => self.engine.send(Command::Play),
            // Şarkı bittiyse baştan çal.
            Status::Ended => self.reload_current(),
            // Spotify vermediyse: aynı şarkıyı bir daha iste.
            Status::Unavailable if self.state.track.is_some() => self.reload_current(),
            // Hiçbir şey çalmıyor: açık listeyi başlat (seçili satırdan,
            // yoksa baştan). Yüklenen bir şarkı varken ikinci kez başlatma.
            Status::Idle | Status::Unavailable => {
                if self.startable_collection().is_some() {
                    let row = self.library.selected_row.unwrap_or(0);
                    self.play_row(row, cx);
                }
            }
        }
        cx.notify();
    }

    fn reload_current(&mut self) {
        if let Some(uri) = self
            .state
            .track
            .as_ref()
            .and_then(|t| SpotifyUri::from_uri(&t.uri).ok())
        {
            self.engine.send(Command::Load(uri));
        }
    }

    /// Şarkının başındaysak önceki şarkıya geç, değilse başa sar.
    fn previous(&mut self, cx: &mut Context<Self>) {
        let position = self.state.position_ms(Instant::now());
        if self.state.queue.has_previous && position < RESTART_THRESHOLD_MS {
            self.engine.send(Command::Previous);
        } else if self.state.status == Status::Ended {
            self.reload_current();
        } else if self.state.track.is_some() {
            self.seek_to(0, cx);
        }
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        if self.state.queue.has_next {
            self.engine.send(Command::Next);
            cx.notify();
        }
    }

    fn seek_to(&mut self, position_ms: u32, cx: &mut Context<Self>) {
        if self.state.track.is_none() || self.state.status == Status::Ended {
            return;
        }
        let position_ms = position_ms.min(self.state.duration_ms());
        self.state.set_position(position_ms, Instant::now());
        self.engine.send(Command::Seek(position_ms));
        self.restart_ticker(cx);
        self.sync_system(true);
        self.on_now_seek(cx);
        cx.notify();
    }

    fn seek_by(&mut self, delta_ms: i64, cx: &mut Context<Self>) {
        let now = self.state.position_ms(Instant::now()) as i64;
        self.seek_to((now + delta_ms).max(0) as u32, cx);
    }

    fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.apply_volume(volume);
        cx.notify();
    }

    /// Ses düzeyini değiştirir ama yeniden çizdirmez (sürüklerken yalnız
    /// hap yenilenir).
    fn apply_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.state.volume = volume;
        self.volume_before_mute = None;
        self.engine.send(Command::SetVolume(volume));
    }

    /// Yalnız hapı yeniden çiz (bkz. `views`).
    fn notify_pill(&self, cx: &mut Context<Self>) {
        self.views.pill.update(cx, |_, cx| cx.notify());
    }

    /// Kaydırma alanının sahibi olan görünümü yeniden çiz (liste de "Tümü"
    /// de liste görünümünde).
    fn notify_scroll_owner(&self, _bar: ScrollBar, cx: &mut Context<Self>) {
        self.views.list.update(cx, |_, cx| cx.notify());
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        match self.volume_before_mute.take() {
            // Dönülecek düzey 0 olamaz: "Sesi aç" hep ses açar.
            Some(previous) if previous > 0.0 => self.set_volume(previous, cx),
            Some(_) => self.set_volume(Settings::default().volume, cx),
            // Çubuk 0'a çekilmişken "Sesi aç": varsayılan düzey.
            None if self.state.volume == 0.0 => self.set_volume(Settings::default().volume, cx),
            None => {
                let previous = self.state.volume;
                self.set_volume(0.0, cx);
                self.volume_before_mute = Some(previous);
            }
        }
    }

    /// Çalma başlatılabilir mi? Çevrimdışıyken ya da ses çıkışı yokken
    /// komut sıraya konmaz (internet dakikalar sonra gelince kendiliğinden
    /// çalmaya başlamasın, hap sonsuza dek "Yükleniyor…" demesin): ne
    /// olduğu ve ne yapılabileceği söylenir.
    pub(super) fn ready_to_play(&mut self, cx: &mut Context<Self>) -> bool {
        let message = match self.state.connection {
            Connection::Failed(_) => t!(NoticeOffline),
            Connection::NoAudio => t!(NoticeNoAudio),
            // Adım 17: giriş / Premium / motor durumları (yazılar `crate::msg`).
            Connection::NeedsLogin => t!(PlayNeedsLogin),
            Connection::LoggingIn => t!(PlayLoggingIn),
            Connection::LoginFailed(_) => t!(PlayLoginFailed),
            Connection::NeedsPremium => t!(PlayNeedsPremium),
            Connection::EngineStopped => t!(PlayEngineStopped),
            Connection::Connecting | Connection::Connected(_) => return true,
        };
        self.state.notify(engine::NoticeKind::Info, message);
        self.arm_notice_expiry(cx);
        cx.notify();
        false
    }

    fn paste_link(&mut self, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        match text.as_deref().map(link::parse_track) {
            Some(Ok(uri)) => {
                if self.ready_to_play(cx) {
                    self.state.notice = None;
                    self.engine.send(Command::Load(uri));
                }
            }
            _ => {
                self.state
                    .notify(engine::NoticeKind::Info, t!(NoticeNotTrackLink));
                self.arm_notice_expiry(cx);
            }
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Künye penceresi açıkken bütün tuşlar ona gider (bkz. `credits`).
        if self.sui.credits.open {
            self.credits_key(event, window, cx);
            cx.stop_propagation();
            return;
        }
        // Kısayol yardımı açıkken bütün tuşlar ona gider (bkz. `shortcuts`).
        if self.sui.help.open {
            self.help_key(event, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.palette.open {
            self.palette_key(event, cx);
            cx.stop_propagation();
            return;
        }
        // Odak arama hapındayken yazmaya başlamak aramayı açar, ilk harf girişe
        // düşer (Enter / Boşluk gpui'nin klavye tıklamasıyla zaten açar).
        if self.keyboard_focus && self.nav.focused(window) == Some(NavTarget::Search) {
            let ks = &event.keystroke;
            let typed = ks
                .key_char
                .as_deref()
                .filter(|c| !c.trim().is_empty() && !c.chars().any(char::is_control));
            if let Some(ch) = typed
                && !ks.modifiers.control
                && !ks.modifiers.alt
            {
                self.open_palette(cx);
                self.palette.query.push_str(ch);
                self.on_query_changed(cx);
                cx.stop_propagation();
                return;
            }
        }
        // Profil menüsü, aygıt listesi, onay kutusu ve Ctrl+, (bkz. `settings`).
        if self.settings_key(event, cx) {
            cx.stop_propagation();
            return;
        }
        // Ekolayzer: odaktaki sürgü ve kulaklık arama kutusu (bkz. `equalizer`).
        if self.eq_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        // Kütüphane araçları: seçim kutuları, ad alanı, onay kartı (bkz. `tools`).
        if self.tools_name_paste(event, window, cx) || self.tools_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.now_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        // Gezinme ve kütüphane yönetimi (bkz. `manage::manage_key`).
        if self.manage_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        let keystroke = &event.keystroke;
        let ctrl = keystroke.modifiers.control;
        // "Tümü" açıkken yazılanlar süzgece gider (klavyeyle bir düğmeye
        // gelinmişse o düğmenin tuşları önce).
        let on_button = self.keyboard_focus && self.nav.focused(window).is_some();
        if self.all.open && !on_button && self.all_key(event, cx) {
            cx.stop_propagation();
            return;
        }
        if self.nav_key(
            keystroke.key.as_str(),
            keystroke.modifiers.shift,
            ctrl,
            window,
            cx,
        ) {
            cx.stop_propagation();
            return;
        }
        // Bağlı kısayollar tek tablodan çözülür (bkz. `shortcuts`); yardım
        // katmanındaki tuşlar da aynı tablodan çizilir.
        let shortcut = Shortcut::from_keystroke(keystroke);
        match (shortcut, keystroke.key.as_str()) {
            (Some(Shortcut::Search), _) => self.open_palette(cx),
            (Some(Shortcut::PasteLink), _) => self.paste_link(cx),
            (Some(Shortcut::PlayPause), _) if !event.is_held => self.toggle_play(cx),
            // Spotify'daki gibi (Adım 9 kararı): Ctrl+←/→ önceki/sonraki,
            // Shift+←/→ sarma, Ctrl+↑/↓ ses, Ctrl+Shift+↓ sessize al.
            (Some(Shortcut::Previous), _) => self.previous(cx),
            (Some(Shortcut::Next), _) => self.next(cx),
            (Some(Shortcut::SeekBack), _) => self.seek_by(-(SEEK_STEP_MS as i64), cx),
            (Some(Shortcut::SeekForward), _) => self.seek_by(SEEK_STEP_MS as i64, cx),
            (Some(Shortcut::Mute), _) => self.toggle_mute(cx),
            (Some(Shortcut::VolumeUp), _) => self.set_volume(self.state.volume + VOLUME_STEP, cx),
            (Some(Shortcut::VolumeDown), _) => self.set_volume(self.state.volume - VOLUME_STEP, cx),
            (Some(Shortcut::Help), _) => self.open_help(Some(window), cx),
            (_, "up") => self.move_row(-1, cx),
            (_, "down") => self.move_row(1, cx),
            (_, "enter") => self.play_selected_row(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Tab / Shift+Tab gezinme, odaktaki düğmeyi Enter ya da Boşluk ile
    /// çalıştırma, Esc ile odağı bırakma. İşlendiyse `true`.
    fn nav_key(
        &mut self,
        key: &str,
        shift: bool,
        ctrl: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let focused = self.nav.focused(window);
        match key {
            "tab" if !ctrl => {
                self.keyboard_focus = true;
                self.nav.reveal.set(true);
                // Sayfada bir durum (karşılama, çevrimdışı …) varsa ilk Tab asıl
                // düğmesine iner.
                if !shift && focused.is_none() && self.focus_gate_primary(window, cx) {
                    cx.notify();
                    return true;
                }
                if shift {
                    window.focus_prev()
                } else {
                    window.focus_next()
                }
                // Rafa gelince imleç açık olan listede başlar.
                if self.nav.focused(window) == Some(NavTarget::Shelf) {
                    self.shelf_cursor = self.selected_shelf_index();
                }
                cx.notify();
                true
            }
            "left" | "right" | "home" | "end" | "enter" | "space"
                if !ctrl && !shift && focused == Some(NavTarget::Shelf) =>
            {
                self.shelf_key(key, cx);
                true
            }
            // Seçenek düğmesinde ← / → gruptaki komşuya geçer; başka düğmede
            // ok tuşu bir şey yapmaz.
            "left" | "right"
                if !ctrl
                    && !shift
                    && focused == Some(NavTarget::Other)
                    && self.karaoke_slider_focused(window) =>
            {
                self.karaoke_step(key == "right", cx);
                true
            }
            "left" | "right" if !ctrl && !shift && focused == Some(NavTarget::Other) => {
                self.segment_step(key == "right", window, cx);
                true
            }
            "left" | "right" if !ctrl && !shift && focused == Some(NavTarget::Progress) => {
                let step = SEEK_STEP_MS as i64;
                self.seek_by(if key == "left" { -step } else { step }, cx);
                true
            }
            "left" | "right" | "up" | "down"
                if !ctrl && !shift && focused == Some(NavTarget::Volume) =>
            {
                let up = key == "right" || key == "up";
                let base = self.volume_before_mute.unwrap_or(self.state.volume);
                self.set_volume(base + if up { VOLUME_STEP } else { -VOLUME_STEP }, cx);
                true
            }
            "escape" if self.keyboard_focus => {
                self.keyboard_focus = false;
                window.focus(&self.focus);
                cx.notify();
                true
            }
            // Odaktaki düğmeyi gpui kendisi çalıştırır: tuş bırakılınca
            // öğeye bir "tıklama" gönderir. Burada yalnızca tuşu tutuyoruz ki
            // genel kısayol (Boşluk = çal/duraklat) aynı tuşta ikinci kez
            // çalışmasın.
            "enter" | "space" if focused.is_some() => true,
            _ => false,
        }
    }

    /// Düğmenin o anki görünüşü: klavyeyle odaklanmışsa halka.
    fn look(&self, handle: &FocusHandle, window: &Window, enabled: bool) -> Look {
        if !enabled {
            Look::Disabled
        } else if self.keyboard_focus && handle.is_focused(window) {
            Look::Focus
        } else {
            Look::Live
        }
    }

    /// Menü ya da onay açan düğme klavyeyle (Enter / Boşluk) tıklandıysa odak
    /// köke döner: menü tuşları kökte işlenir, Enter'ın bırakılması da açılan
    /// menüyü düğmenin tıklamasıyla geri açmasın.
    fn leave_button(&mut self, ev: &gpui::ClickEvent, window: &mut Window) {
        if ev.is_keyboard() {
            window.focus(&self.focus);
            self.keyboard_focus = false;
        }
    }

    /// Anahtarlı Tab durağı ve görünüşü. Kullanılamayan öğe (`enabled` yok)
    /// Tab sırasına girmez. Dönen tutamaç öğeye `track_focus` ile verilir;
    /// Enter / Boşluk'u gpui öğenin `on_click`ine çevirir.
    fn stop_look(
        &self,
        key: impl Into<SharedString>,
        index: isize,
        enabled: bool,
        window: &Window,
        cx: &mut gpui::App,
    ) -> (Option<FocusHandle>, Look) {
        if !enabled {
            return (None, Look::Disabled);
        }
        let handle = self.nav.extra(cx, key.into(), index);
        let look = self.look(&handle, window, true);
        (Some(handle), look)
    }

    /// Kayan sayfadaki (ayarlar) Tab durağının içine konan görünmez ölçüm:
    /// durak klavyeyle odaklanınca sayfayı, öğe görünür alana (hapın üstüne)
    /// girene kadar kaydırır. Kaydırma ölçümde yapılır (odaklanan öğenin
    /// yeri en son çizimden gelir) ve bir kare daha istenir.
    fn focus_probe(&self, handle: &FocusHandle) -> AnyElement {
        let handle = handle.clone();
        let reveal = self.nav.reveal.clone();
        let scroll = self.pages.scroll.0.borrow().base_handle.clone();
        canvas(
            move |b, window, _| {
                if !reveal.get() || !handle.is_focused(window) {
                    return;
                }
                reveal.set(false);
                let view = scroll.bounds();
                let top = view.origin.y + px(S4);
                let bottom = view.origin.y + view.size.height - px(pill::PILL_CLEARANCE);
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

    /// Seçenek düğmeleri arasında ok tuşuyla gezinme (anahtar
    /// `seg:grup:seçenek`): odak yalnız aynı grubun içinde kalır.
    fn segment_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, old)) = self.nav.extra_focused(window) else {
            return;
        };
        let Some(group) = key
            .strip_prefix("seg:")
            .and_then(|k| k.rsplit_once(':'))
            .map(|(g, _)| format!("seg:{g}:"))
        else {
            return;
        };
        if forward {
            window.focus_next()
        } else {
            window.focus_prev()
        }
        if !self
            .nav
            .extra_focused(window)
            .is_some_and(|(k, _)| k.starts_with(&group))
        {
            window.focus(&old);
        }
        cx.notify();
    }

    fn on_drag_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.dragging else { return };
        if ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        // Çubuklar sürüklenirken (fare her kıpırdadığında) yalnız hap
        // yeniden çizilir; bırakınca bütün pencere (`on_drag_end`).
        match drag {
            Drag::Progress(_) => {
                if let Some(f) = pill::fraction(&self.progress_bounds, ev.position) {
                    self.dragging = Some(Drag::Progress(f));
                    self.notify_pill(cx);
                }
            }
            Drag::Volume => {
                if let Some(f) = pill::fraction(&self.volume_bounds, ev.position) {
                    self.apply_volume(f);
                    self.notify_pill(cx);
                }
            }
            Drag::Eq(slot) => self.eq_drag_to(slot, ev.position, cx),
            Drag::Karaoke(place) => self.karaoke_drag_to(place, ev.position, cx),
            Drag::Scroll {
                bar,
                start_y,
                start_offset,
            } => {
                let Some(m) = self.scroll_metrics(bar) else {
                    return;
                };
                let (h, _) = m.thumb();
                let track = (m.viewport - h).max(1.);
                let moved = f32::from(ev.position.y) - start_y;
                self.set_scroll(bar, start_offset + moved * m.max() / track);
                self.notify_scroll_owner(bar, cx);
            }
        }
    }

    /// Kaydırma alanının gpui tutamacı ve tekerlek hareketi.
    fn scroll_parts(&self, bar: ScrollBar) -> (&UniformListScrollHandle, &SmoothScroll) {
        match bar {
            ScrollBar::List => (&self.library.scroll, &self.library.list_glide),
            ScrollBar::All => (&self.all.scroll, &self.all.glide),
            ScrollBar::Page => (&self.pages.scroll, &self.pages.glide),
        }
    }

    /// Kaydırma ölçüleri. Görünen yükseklik listenin son yerleşiminden,
    /// içerik şimdiki durumdan (bkz. `list_extent`). Konum gpui'nin çizimde
    /// yapacağı gibi sınıra kırpılır.
    fn scroll_metrics(&self, bar: ScrollBar) -> Option<ScrollMetrics> {
        let (viewport, content) = match bar {
            ScrollBar::List => self.list_extent()?,
            ScrollBar::All => self.all_extent()?,
            ScrollBar::Page => self.page_extent()?,
        };
        let offset = -f32::from(self.scroll_parts(bar).0.0.borrow().base_handle.offset().y);
        let max = (content - viewport).max(0.);
        Some(ScrollMetrics {
            viewport,
            content,
            offset: offset.clamp(0., max),
        })
    }

    /// Kaydırma çubuğuyla kaydır: süren tekerlek hareketi bırakılır.
    fn set_scroll(&self, bar: ScrollBar, offset: f32) {
        let Some(m) = self.scroll_metrics(bar) else {
            return;
        };
        let y = -offset.clamp(0., m.max());
        let (handle, glide) = self.scroll_parts(bar);
        glide.cancel();
        if bar == ScrollBar::List {
            // İzde konum değişmeden önceki hâl: yeni konum çizilmeden önce
            // onarılır (raf açıksa kapanmaya aktarılır, bkz.
            // `heal_list_collapse`), onarım satırı sonucu gösterir.
            self.trace_scroll("bar", || format!("to={:.1}", -y));
        }
        let state = handle.0.borrow();
        let x = state.base_handle.offset().x;
        state.base_handle.set_offset(gpui::point(x, px(y)));
    }

    /// İnce kaydırma çubuğu: dururken soluk, alanın üzerine gelince belirgin,
    /// sürüklenebilir; boş yere tıklamak bir sayfa kaydırır. İçerik sığıyorsa
    /// hiç görünmez. (gpui'de hazır kaydırma çubuğu yok.)
    pub(super) fn scrollbar(&self, bar: ScrollBar, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.scroll_metrics(bar)?;
        if m.content <= m.viewport + 1. {
            return None;
        }
        let (h, top) = m.thumb();
        let (group, thumb_id, track_id) = match bar {
            ScrollBar::List => ("list-scroll", "list-thumb", "list-track"),
            ScrollBar::All => ("all-scroll", "all-thumb", "all-track"),
            ScrollBar::Page => ("page-scroll", "page-thumb", "page-track"),
        };
        let dragging = matches!(self.dragging, Some(Drag::Scroll { bar: b, .. }) if b == bar);
        let thumb = div()
            .id(thumb_id)
            .absolute()
            .top(px(top))
            .right(px(S0))
            .w(px(SCROLL_THUMB_WIDTH))
            .h(px(h))
            .rounded_full()
            .bg(veil(if dragging { LINE_HOVER } else { LINE_INPUT }))
            .when(!dragging, |el| {
                el.group_hover(group, |s| s.bg(veil(LINE_HOVER)))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if let Some(m) = this.scroll_metrics(bar) {
                        this.scroll_parts(bar).1.cancel();
                        this.dragging = Some(Drag::Scroll {
                            bar,
                            start_y: f32::from(ev.position.y),
                            start_offset: m.offset,
                        });
                        cx.stop_propagation();
                        this.notify_scroll_owner(bar, cx);
                    }
                }),
            );
        Some(
            div()
                .id(track_id)
                .absolute()
                .top_0()
                .right_0()
                .h_full()
                .w(px(SCROLL_THUMB_WIDTH + S1))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        // Tutamacın üstüne / altına tıklamak bir sayfa kaydırır.
                        let Some(m) = this.scroll_metrics(bar) else {
                            return;
                        };
                        let (h, top) = m.thumb();
                        let origin = this
                            .scroll_parts(bar)
                            .0
                            .0
                            .borrow()
                            .base_handle
                            .bounds()
                            .origin
                            .y;
                        let y = f32::from(ev.position.y - origin);
                        let page = m.viewport * 0.9;
                        if y < top {
                            this.set_scroll(bar, m.offset - page);
                        } else if y > top + h {
                            this.set_scroll(bar, m.offset + page);
                        }
                        this.notify_scroll_owner(bar, cx);
                    }),
                )
                .child(thumb)
                .into_any_element(),
        )
    }

    fn on_drag_end(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.dragging.take() else {
            return;
        };
        if let Drag::Progress(f) = drag {
            let target = (self.state.duration_ms() as f32 * f) as u32;
            self.seek_to(target, cx);
        }
        if matches!(drag, Drag::Eq(_)) {
            self.eq_commit();
        }
        if matches!(drag, Drag::Karaoke(_)) {
            self.save_settings();
        }
        // Sürükleme görünüşü (tutamaç rengi, ses düzeyinin başka yerlerdeki
        // izi) kalmasın: her bırakışta bütün pencere bir kez yenilenir.
        cx.notify();
    }

    // ── Görünüm ─────────────────────────────────────────────────────────

    fn render_titlebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let maximized = window.is_maximized();
        let accent = self.accent().text;
        // Pencere düğmelerini fare Windows'un kendi isabet sınamasıyla
        // kullanır (tıklama gpui'ye gelmez); klavyeyle Tab'la gelinir,
        // Enter / Boşluk `on_click` ile aynı işi yapar.
        let control = |id: &'static str,
                       area: WindowControlArea,
                       icon_name: &'static str,
                       close: bool,
                       index: isize,
                       cx: &mut Context<Self>| {
            let (handle, look) = self.stop_look(id, index, true, window, cx);
            div()
                .id(id)
                .group("caption-control")
                .w(px(CAPTION_BUTTON_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .window_control_area(area)
                .text_color(muted())
                // Odak halkası: düğmenin içinde, hep duran 2 px'lik kenar.
                .border_2()
                .border_color(if look == Look::Focus {
                    rgb(accent).into()
                } else {
                    gpui::transparent_black()
                })
                .when(close, |el| {
                    el.hover(|s| s.bg(rgb(palette().close_hover)).text_color(cover_ink()))
                })
                .when(!close, |el| {
                    el.hover(|s| s.bg(veil(VEIL_HOVER)).text_color(text()))
                })
                .when_some(handle, |el, handle| {
                    el.track_focus(&handle)
                        .on_click(cx.listener(move |this, _, window, cx| match area {
                            WindowControlArea::Min => window.minimize_window(),
                            WindowControlArea::Max => window.zoom_window(),
                            _ => {
                                if this.on_close_requested(cx) {
                                    window.remove_window();
                                }
                            }
                        }))
                })
                .child(
                    icon(icon_name, 14., muted()).group_hover("caption-control", |s| {
                        s.text_color(if close { cover_ink() } else { text() })
                    }),
                )
        };

        // Ortada görünür arama: komut paletini sadece Ctrl+K bilen bulmasın.
        // Sürükleme alanı aramanın iki yanında; arama kendi tıklamasını alır.
        // Kendi önbellekli görünümünde (bkz. `views::SearchView`): ekolayzer
        // akarken başlık çubuğunun geri kalanı her karede yeniden kuruluyor,
        // arama önceki kareden tekrar oynatılıyor.
        let search = AnyView::from(self.views.search.clone()).cached(
            StyleRefinement::default()
                .w(px(self.search_width()))
                .h(px(TITLE_SEARCH_HEIGHT))
                .flex_shrink_0(),
        );

        div()
            .h(px(TITLEBAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(
                // Solda geri / ileri / ana sayfa (Adım 12; geri simgesi
                // sayfanın 32 px'lik sol kenarında), sonra sürükleme alanı.
                // Düğmeler sürükleme alanının içinde olamaz: Windows o noktayı
                // başlık sayar, tıklama düğmeye gelmez.
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    // En solda profil hapı (bkz. `settings`), sürükleme alanının dışında.
                    .child(self.render_profile_button(window, cx))
                    // Kendi önbellekli görünümünde (bkz. `views::NavView`).
                    .child(
                        AnyView::from(self.views.nav.clone()).cached(
                            StyleRefinement::default()
                                .w(px(pages::NAV_WIDTH))
                                .h_full()
                                .flex_shrink_0(),
                        ),
                    )
                    .child(
                        div()
                            .id("titlebar-drag")
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    // Sekmeler aramanın hemen solunda (Ana sayfa · Keşfet · Kütüphane).
                    .child(self.render_tabs(window, cx))
                    .child(div().w(px(S4)).flex_none()),
            )
            .child(search)
            // Sağ taraf da sol kadar esnek: arama pencerenin tam ortasında kalır.
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .id("titlebar-drag-2")
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    // Gizli oturumda küçük simge (bkz. `listening`).
                    .children(self.render_private_badge(window, cx))
                    // Ayar açıksa arkadaş etkinliği düğmesi (bkz. `friends`).
                    .children(self.render_friends_button(window, cx))
                    // Bekleyen ya da yazılamayan değişiklik varsa eşitleme.
                    .children(self.render_sync_badge(cx))
                    .child(control(
                        "win-min",
                        WindowControlArea::Min,
                        "minus",
                        false,
                        tab::MINIMIZE,
                        cx,
                    ))
                    .child(control(
                        "win-max",
                        WindowControlArea::Max,
                        if maximized { "copy" } else { "square" },
                        false,
                        tab::MAXIMIZE,
                        cx,
                    ))
                    .child(control(
                        "win-close",
                        WindowControlArea::Close,
                        "x",
                        true,
                        tab::CLOSE,
                        cx,
                    )),
            )
            .into_any_element()
    }

    /// Başlık çubuğundaki arama hapının eni: geniş pencerede geniş, pencere
    /// daraldıkça küçülür (bkz. `search_width_for`).
    fn search_width(&self) -> f32 {
        search_width_for(self.viewport_w)
    }

    /// Başlık çubuğundaki arama hapı (kendi görünümünde çizilir): tam yuvarlak,
    /// çalan kapağın ışığıyla hafifçe tonlu dolgu, solda büyüteç, sıcak yer
    /// tutucu. Üzerine gelince dolgu açılır (140 ms), klavye odağında vurgu
    /// renginde ince halka ve dışında soluk ikinci halka. Tuş ipucu yok:
    /// tıklamak, yazmaya başlamak ve Ctrl+K aynı arama yüzeyini açar.
    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent();
        let focused = self.look(&self.nav.search, window, true) == Look::Focus;
        div()
            .relative()
            .w(px(self.search_width()))
            .h(px(TITLE_SEARCH_HEIGHT))
            .flex_none()
            .when(focused, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(-S0))
                        .left(px(-S0))
                        .right(px(-S0))
                        .bottom(px(-S0))
                        .rounded(px(R_FULL))
                        .border_2()
                        .border_color(accent_veil(accent.text, SEARCH_HALO)),
                )
            })
            .child(
                div()
                    .id("titlebar-search")
                    .relative()
                    .size_full()
                    .px(px(S4))
                    .rounded(px(R_FULL))
                    .flex()
                    .items_center()
                    .gap(px(S3))
                    .bg(rgb(search_fill(accent.glow, false)))
                    .border_1()
                    .border_color(if focused {
                        rgb(accent.text)
                    } else {
                        veil(LINE_INPUT)
                    })
                    .cursor_pointer()
                    .track_focus(&self.nav.search)
                    .on_click(cx.listener(|this, _, _, cx| this.open_palette(cx)))
                    .child(tween::hover_fill(
                        veil(VEIL_HOVER),
                        veil(VEIL_SELECTED),
                        0.,
                        R_FULL,
                    ))
                    .child(icon("search", 16., soft()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .type_role(T_BODY)
                            .text_color(search_hint(accent.text))
                            .child(line(palette::search_placeholder())),
                    ),
            )
            .into_any_element()
    }

    /// Pencerenin üstünde, kapak renginde ışıma: rafın arkasına yayılır,
    /// raf şeride inince (ve "Tümü"de) yarıya söner.
    ///
    /// Saydam renkli degrade gpui'de sonunda sert bir çizgi bırakıyordu; renk
    /// zeminle önceden karıştırılıp opak, dikey bir degrade çiziliyor: üstte
    /// kapak rengi, aşağıda zemin renginin kendisine eriyor (dikiş yok).
    fn render_glow(&self) -> AnyElement {
        let collapse = self.collapse_progress();
        // Tür sayfasında ışıma türün rengi (başlık o renkte); diğer yerlerde
        // çalan şarkının kapak rengi.
        let top = match self.category_glow_color() {
            Some(color) => color,
            None => mix(
                palette().bg,
                glow_top(self.accent().glow),
                1. - GLOW_SCROLLED_DIM * collapse,
            ),
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(GLOW_HEIGHT))
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(top), 0.),
                linear_color_stop(rgb(palette().bg), 1.),
            ))
            .into_any_element()
    }

    /// "Giriş yap": tarayıcıda Spotify girişini başlatır (kullanıcı eylemi;
    /// tarayıcı başka hiçbir yolla açılmaz).
    pub(super) fn start_login(&mut self, cx: &mut Context<Self>) {
        self.engine.send(Command::StartLogin);
        cx.notify();
    }

    /// Tarayıcıda giriş bekleyişini bırakır.
    pub(super) fn cancel_login(&mut self, cx: &mut Context<Self>) {
        self.engine.send(Command::CancelLogin);
        cx.notify();
    }

    /// Ses motoru çöktü: yeni bir motor başlat (olay kanalları aynı kalır),
    /// ses düzeyini yeniden ver.
    pub(super) fn restart_engine(&mut self, cx: &mut Context<Self>) {
        self.engine.restart();
        self.engine.send(Command::SetVolume(self.state.volume));
        self.state.connection = Connection::Connecting;
        cx.notify();
    }

    /// Bilgi bildirimi süresi dolunca hapı yeniden çiz (başka bir sebeple
    /// çizilmeyebilir: duraklatılmışken).
    fn arm_notice_expiry(&mut self, cx: &mut Context<Self>) {
        let Some(n) = self.state.notice.as_ref() else {
            return;
        };
        if n.kind != engine::NoticeKind::Info {
            return;
        }
        let left = player::INFO_NOTICE.saturating_sub(n.at.elapsed()) + Duration::from_millis(50);
        self.notice_expiry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(left).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }

    /// Raf, başlık ve liste tek hareket: tekerleğin bu karelik adımı kökte,
    /// raf ve liste çizilmeden önce atılır (üçü aynı karede aynı konumu
    /// okusun; rafın boyu burada, kökte verilir). Sürüyorsa bir sonraki
    /// karede yalnız liste görünümü uyarılır: kök de onunla birlikte çizilir
    /// (bir parça uyarılınca gpui ataları da kirli sayar), ekolayzer listeyi
    /// izler. Böylece hareket sürerken her karede liste kirlidir; rafın
    /// boyu değişince önbellekli raf da kendiliğinden yeniden çizilir.
    /// (Çizim sırasında bir görünümü uyarmak o kareye yetişmiyor: gpui kirli
    /// görünümleri çizimin başında topluyor.)
    fn advance_list_glide(&mut self, window: &mut Window) {
        if self.all.open || !self.tick_list_glide() {
            return;
        }
        let list = self.views.list.downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = list.update(cx, |_, cx| cx.notify());
        });
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        frames::render_started();
        // Palet ya da "Tümü" açılınca odak düğmelerden alınır: Enter, odakta
        // kalmış bir düğmeye klavye tıklaması göndermesin.
        if std::mem::take(&mut self.refocus_anchor) {
            window.focus(&self.focus);
            self.keyboard_focus = false;
        }
        if std::mem::take(&mut self.first_frame) {
            log::debug!(
                "aşama {} ms: ilk kare (liste {})",
                crate::since_start(),
                if self.library.selected.as_ref().is_some_and(|id| {
                    matches!(self.library.collections.get(id), Some(Loadable::Loaded(_)))
                }) {
                    "hazır"
                } else {
                    "yok"
                }
            );
        }
        // Kök her karede çizilir (bir parça görünüm uyarılınca kök de kirli
        // sayılır), parça görünümlerinden önce: onlar bu karenin ölçülerini
        // ve kaydırma konumunu okur.
        let viewport = window.viewport_size();
        self.compact = viewport.width < px(COMPACT_BELOW);
        self.viewport_w = f32::from(viewport.width);
        self.viewport_h = f32::from(viewport.height);
        // "Şimdi çalıyor" paneli geniş pencerede listenin yanına yerleşir:
        // raf, liste ve sayfalar panelin solunda kalan bütün alana göre
        // ölçülür; hap hep pencerenin ortasında, panele yetişirse daralır
        // (bkz. `now_panel`).
        let dock = self.dock_frame(window, cx);
        self.window_width = dock.main_width;
        let (pill_left, pill_w) = dock.pill_frame(f32::from(viewport.width));
        self.pill_width = pill_w;
        let short = viewport.height < px(SHORT_BELOW);
        if short != self.short {
            self.short = short;
            // Liste kaymışsa raf kapalı kalır: açılsaydı liste konumu
            // korunurken raf da açık olurdu (değişmez bozulurdu).
            let scrolled = self.list_offset() > 0.;
            self.library.header_collapse = library::initial_collapse(short || scrolled);
            self.library.list_glide.cancel();
            self.trace_scroll("short", || {
                format!("short={short} h={:.1}", f32::from(viewport.height))
            });
        }
        self.scale_factor = window.scale_factor();
        // Raf ile liste arasındaki değişmez her karede, raf ve liste
        // çizilmeden önce onarılır (bkz. `heal_list_collapse`).
        self.heal_list_collapse();
        self.advance_list_glide(window);
        if library::scroll_trace::enabled() {
            let (collapse, offset) = (
                self.library.header_collapse.min(shelf::COLLAPSE_MAX),
                self.list_offset(),
            );
            let extent = self.list_extent();
            library::scroll_trace::state(collapse, offset, || {
                format!(
                    "root extent={extent:?} glide={}",
                    self.library.list_glide.is_active()
                )
            });
        }

        // Başlık çubuğu önbelleğe alınamaz: pencere düğmeleri ve sürükleme
        // alanı (`window_control_area`) önceki kareden tekrar oynatılmıyor,
        // önbellekli bir görünümde olsalar çalışmazlardı.
        let titlebar = self.render_titlebar(window, cx);
        // Her yerden açılan palet kendi penceresinde (bkz. `popups`).
        let palette_overlay =
            (self.palette.open && !self.palette.detached).then(|| self.render_palette(cx));
        // Raf, liste ve hap kendi görünümlerinde çizilir (bkz. `views`);
        // burada yalnız yerlerini tutan kutular var. Kutunun boyu içerikten
        // gelmez, burada verilir: rafın boyu kapanma oranından.
        let list = AnyView::from(self.views.list.clone())
            .cached(StyleRefinement::default().flex_1().min_h_0().w_full());
        let shelf = AnyView::from(self.views.shelf.clone()).cached(
            StyleRefinement::default()
                .w_full()
                .h(px(shelf::shelf_height(self.collapse_progress())))
                .flex_shrink_0(),
        );
        let pill = AnyView::from(self.views.pill.clone()).cached(
            StyleRefinement::default()
                .w(px(pill_w))
                .h(px(PILL_HEIGHT))
                .flex_shrink_0(),
        );
        // Ekolayzer bütün pencereyi kaplayan saydam bir katman: listeden
        // sonra gelir (aynı karede ölçülen yeri okusun), hapın ve paletin
        // altında kalır (hapın altına giren satırın ekolayzeri hapla örtülür).
        let equalizer = AnyView::from(self.views.equalizer.clone()).cached(
            StyleRefinement::default()
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
        // Ölçüm kancası (yalnız geliştirme): OPENIFY_PROF_HIDE="shelf,list,
        // pill,titlebar,glow,equalizer" bölgeleri çizmez; kare süresinin
        // nereden geldiğini bulmak için. (Eski adlar "sidebar" ve "bar" da
        // geçerli: ölçüm betikleri onları kullanıyor.)
        let hide = frames::hidden;
        let titlebar = if hide("titlebar") {
            div().h(px(TITLEBAR_HEIGHT)).into_any_element()
        } else {
            titlebar
        };
        let list = if hide("list") {
            div().flex_1().into_any_element()
        } else {
            list.into_any_element()
        };
        let shelf = if hide("shelf") || hide("sidebar") {
            div()
                .h(px(shelf::shelf_height(self.collapse_progress())))
                .into_any_element()
        } else {
            shelf.into_any_element()
        };
        let pill = (!(hide("pill") || hide("bar"))).then_some(pill);
        // Hapın arkasındaki alt perde (bkz. `theme::PILL_SCRIM_ALPHA`): hap
        // olmadan perde de yok; tıklamayı ve tekerleği etkilemez (dinleyicisi
        // yok), ekolayzerin üstünde, hapın altında.
        let scrim = pill.is_some().then(|| {
            div()
                .absolute()
                .left_0()
                .bottom_0()
                .w_full()
                .h(px(pill::PILL_CLEARANCE + PILL_SCRIM_FADE))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(bottom_scrim(0.), 0.),
                    linear_color_stop(bottom_scrim(PILL_SCRIM_ALPHA), 1.),
                ))
        });
        let equalizer = (!hide("equalizer")).then_some(equalizer);
        let glow = if hide("glow") {
            div().into_any_element()
        } else {
            self.render_glow()
        };
        self.flush_image_requests(cx);
        // Renk geçişi sürerken her karede yeniden çiz.
        if self.state.accent_animating(Instant::now()) {
            window.request_animation_frame();
        }
        // Panelin kayması sürerken: içerik alanı her karede yeni enine geçer
        // (kayma biter bitmez kare istenmez).
        if self.now.dock.sliding {
            window.request_animation_frame();
        }

        // Kütüphane sekmesinde raf var: altında açık liste ya da kütüphane
        // ızgarası (`all`). Sayfa (ana sayfa, keşfet, tür, sanatçı, albüm,
        // arama) açıkken raf yok; hap yerinde kalır.
        let main = div()
            .flex_1()
            .min_h_0()
            .w(px(dock.main_width))
            .flex()
            .flex_col()
            .when(!self.page_open() && !self.hide_shelf(), |el| {
                el.child(shelf)
            })
            .child(list);
        // Sağ tık menüsü, ad kutusu, geri al bildirimi, eşitleme paneli.
        let mut overlays = if self.now.full.open {
            Vec::new()
        } else {
            self.render_manage_overlays(window, cx)
        };
        if !self.now.full.open {
            overlays.extend(self.render_settings_overlays(window, cx));
        }

        // Sürüklenen çubuk: fare hapın (ya da başka bir öğenin) üstündeyken
        // de izlensin. Hap altını örttüğü için kökün kendi fare dinleyicisi
        // orada çalışmıyor: dinleyici pencerenin tamamına kurulur.
        let drag_watch = self.dragging.is_some().then(|| {
            let this = cx.weak_entity();
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = this.update(cx, |shell, cx| shell.on_drag_move(ev, window, cx));
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        });

        // Tam ekranda pencereyi tek görünüm kaplar: başlık çubuğu, liste, hap
        // ve ekolayzer hiç çizilmez (pencere düğmelerinin isabet alanı da yok).
        let full = (self.now.full.open || self.now.full.closing.is_some())
            .then(|| self.fullscreen_layer());
        let side = if full.is_none() {
            self.side_panel_layer(window)
        } else {
            None
        };
        let (glow, equalizer, pill, titlebar_main) = if full.is_some() {
            (None, None, None, None)
        } else {
            (
                Some(glow),
                equalizer,
                pill,
                Some(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(titlebar)
                        .child(main),
                ),
            )
        };

        let root = div()
            .relative()
            .size_full()
            .bg(rgb(palette().bg))
            .font_family(UI_FONT)
            .text_color(text())
            .on_key_down(cx.listener(Self::on_key_down))
            // Fareyle tıklanınca odak görünmez öğeye döner: yoksa tıklanan
            // düğme odakta kalıyor, sonraki Enter / Boşluk gpui'nin klavye
            // tıklamasıyla o düğmeyi de çalıştırıyordu (Boşluk çal/duraklat'ı
            // iki kez çevirip hiçbir şey yapmıyordu). Hap kendi tıklamasında
            // aynısını yapar (altını örttüğü için kök orada tıklama almaz).
            .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus);
                if std::mem::take(&mut this.keyboard_focus) {
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_drag_end))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_drag_end))
            // Farenin yan tuşları: geri / ileri.
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _, _, cx| this.go_back(cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _, _, cx| this.go_forward(cx)),
            )
            // Klavye odağı görünmez, sıfır boyutlu bir öğede. Tuşlar oradan
            // köke (yukarıdaki dinleyiciye) çıkar. Odak kökte olsaydı gpui
            // pencerenin her yerindeki fare basışını "işlendi" sayıyor ve
            // Windows'un kendi işleri (kenardan boyutlandırma, küçült /
            // büyüt / kapat) hiç başlamıyordu.
            .child(div().track_focus(&self.focus).absolute().size_0())
            .children(glow)
            .children(titlebar_main)
            .children(equalizer)
            .children(scrim)
            // Hap pencerenin ortasında (panel açılıp kapanınca kaymaz),
            // alttan 24 px yukarıda. Dış kutunun fare dinleyicisi yok: hapın
            // iki yanındaki satırlara tıklanabilir.
            .children(pill.map(|pill| {
                div()
                    .absolute()
                    .left(px(pill_left))
                    .bottom(px(PILL_BOTTOM))
                    .w(px(pill_w))
                    .flex()
                    .justify_center()
                    .child(pill)
            }))
            .children(side)
            .children(full)
            .children(overlays)
            .children(palette_overlay)
            .children(drag_watch)
            // Kare ölçümü (yalnız `OPENIFY_FRAME_STATS` ile): en son boyanan öğe.
            .when(frames::enabled(), |el| {
                el.child(
                    canvas(|_, _, _| {}, |_, _, _, _| frames::painted())
                        .absolute()
                        .size_0(),
                )
            });
        frames::render_built();
        root
    }
}

// ── Küçük bileşenler ────────────────────────────────────────────────────

/// Raf şeride inince üstteki ışımanın ne kadar söneceği (yarıya).
const GLOW_SCROLLED_DIM: f32 = 0.5;

/// Kare, köşeleri yuvarlatılmış çerçeve içinde resim.
fn framed(child: impl IntoElement, side: f32, radius: f32) -> AnyElement {
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(radius))
        .overflow_hidden()
        .bg(rgb(palette().placeholder))
        .child(child)
        .into_any_element()
}

/// Kapağı olmayan öğe: sessiz ton karosu ve ortada soluk simge.
fn placeholder(_seed: &str, side: f32, radius: f32, icon_name: &'static str) -> AnyElement {
    // Renk yalnızca gerçek kapaklardan gelir (eskiden addan türetilen doygun
    // degradeler vardı). Sakin: üstten alta hafifçe koyulaşan ton, küçük ve
    // soluk simge (7 Kasım gecesi: "gri simge karosu her yerde" yorucuydu).
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(radius))
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(mix(palette().placeholder, 0xFFFFFF, 0.05)), 0.),
            linear_color_stop(rgb(palette().placeholder), 1.),
        ))
        .child(icon(icon_name, (side * 0.3).max(12.), label()))
        .into_any_element()
}

/// Resim diskteyse onu, değilse yer tutucuyu gösterir (ve indirmeyi ister).
fn cover_image(
    images: &ImageStore,
    url: &str,
    seed: &str,
    side: f32,
    radius: f32,
    icon_name: &'static str,
) -> AnyElement {
    // Ölçüm kancası: OPENIFY_PROF_HIDE içinde "images" varsa resim yerine karo.
    match images.get(url).filter(|_| !frames::hidden("images")) {
        Some(path) => {
            // Yeni inen kapak yer tutucunun üstünde belirir (çerçeve değil,
            // yalnız resim solar).
            // Köşeler resmin kendisinde de: kabın köşesi resmi kırpmıyor
            // (sanatçı portresi yuvarlak yerine kare çiziliyordu).
            let picture = match images.fade(url) {
                Some(t) => img(path).size_full().rounded(px(radius)).opacity(t),
                None => img(path).size_full().rounded(px(radius)),
            };
            framed(picture, side, radius)
        }
        None => placeholder(seed, side, radius, icon_name),
    }
}

/// Sahte çalmada çalınan beğenilen şarkının sırası (`OPENIFY_TEST_FAKE_ROW`,
/// varsayılan 0).
fn fake_row() -> usize {
    std::env::var("OPENIFY_TEST_FAKE_ROW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Başlık çubuğundaki arama hapının eni (pencere eninden): ≥ 1500 px 520,
/// ≥ 1180 px 480, ≥ 1000 px 420; daha dar pencerede 280 (sol yarıda gezinme
/// düğmeleri ve sekmeler, sağda pencere düğmeleri var; hap ortada kalır).
fn search_width_for(window: f32) -> f32 {
    if window >= 1500. {
        520.
    } else if window >= 1180. {
        480.
    } else if window >= COMPACT_BELOW {
        420.
    } else {
        280.
    }
}

/// Vurgu rengini zeminde ve ışımada okunur hâle getirir (bkz. `Shell::accent`).
fn readable_accent(accent: Accent) -> Accent {
    Accent {
        text: ensure_contrast(accent.text, glow_top(accent.glow), 4.5),
        glow: accent.glow,
    }
}

/// Tab sırası: görsel sıra (başlık çubuğu, sayfa, panel, hap). gpui düğmeleri
/// bu sayıya göre (aynı sayıda çizim sırasıyla) gezer; sayı tutamacın
/// kendisinde verilir (bkz. `Nav`). Yeni bir durak bu ölçeğe eklenir
/// (`ORDER` görsel sırayı sınar).
pub(super) mod tab {
    pub const PROFILE: isize = 1;
    pub const BACK: isize = 2;
    pub const FORWARD: isize = 3;
    pub const HOME: isize = 4;
    pub const EXPLORE: isize = 5;
    pub const LIBRARY: isize = 6;
    pub const SEARCH: isize = 7;
    /// Arkadaş etkinliği düğmesi (aramanın sağında; ayar açıkken).
    pub const FRIENDS: isize = 7;
    pub const MINIMIZE: isize = 8;
    pub const MAXIMIZE: isize = 9;
    pub const CLOSE: isize = 10;
    /// Sayfa: raf, "Tümü"nün kapatma düğmesi (ikisi birlikte görünmez),
    /// listeyi çal, sonra sayfanın kendi düğmeleri (çizim sırasıyla).
    pub const SHELF: isize = 20;
    pub const ALL_CLOSE: isize = 20;
    pub const COLLECTION_PLAY: isize = 21;
    pub const PAGE: isize = 22;
    /// Sağdaki panel (sekmeler, tam ekran, kapat, kalp).
    pub const PANEL: isize = 30;
    /// Hap: kalp, karıştır … ses (soldan sağa).
    pub const HEART: isize = 40;
    pub const SHUFFLE: isize = 41;
    pub const PREV: isize = 42;
    pub const PLAY: isize = 43;
    pub const NEXT: isize = 44;
    pub const REPEAT: isize = 45;
    pub const PROGRESS: isize = 46;
    pub const QUEUE: isize = 47;
    pub const LYRICS: isize = 48;
    pub const MUTE: isize = 49;
    pub const VOLUME: isize = 50;

    /// Görsel sırayla (soldan sağa, yukarıdan aşağı) bütün duraklar.
    #[cfg(test)]
    pub const ORDER: &[(&str, isize)] = &[
        ("profil", PROFILE),
        ("geri", BACK),
        ("ileri", FORWARD),
        ("ana sayfa", HOME),
        ("keşfet", EXPLORE),
        ("kütüphane", LIBRARY),
        ("arama", SEARCH),
        ("arkadaşlar", FRIENDS),
        ("küçült", MINIMIZE),
        ("büyüt", MAXIMIZE),
        ("kapat", CLOSE),
        ("raf", SHELF),
        ("listeyi çal", COLLECTION_PLAY),
        ("sayfa", PAGE),
        ("panel", PANEL),
        ("kalp", HEART),
        ("karıştır", SHUFFLE),
        ("önceki", PREV),
        ("çal", PLAY),
        ("sonraki", NEXT),
        ("tekrar", REPEAT),
        ("ilerleme", PROGRESS),
        ("sıra", QUEUE),
        ("sözler", LYRICS),
        ("sessize al", MUTE),
        ("ses", VOLUME),
    ];
}

/// Tab ile gezilen düğmeler. Sabit olanlar alanlarda; sayfalara göre
/// değişenler (ayarlar, sekmeler, süzgeçler …) anahtarla tembel açılır
/// (`Nav::extra`, `Shell::stop_look`). "Tümü"nün kapatma düğmesi rafın
/// sırasında (ikisi birlikte görünmez).
struct Nav {
    search: FocusHandle,
    shelf: FocusHandle,
    all_close: FocusHandle,
    collection_play: FocusHandle,
    prev: FocusHandle,
    play: FocusHandle,
    next: FocusHandle,
    progress: FocusHandle,
    mute: FocusHandle,
    volume: FocusHandle,
    extra: std::cell::RefCell<std::collections::HashMap<SharedString, FocusHandle>>,
    /// Tab'a basıldı, odaktaki kayan sayfa öğesi görünür yere kaydırılacak
    /// (bkz. `Shell::focus_probe`).
    reveal: Rc<Cell<bool>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavTarget {
    Search,
    Shelf,
    AllClose,
    CollectionPlay,
    Previous,
    Play,
    Next,
    Progress,
    Mute,
    Volume,
    /// Anahtarla açılan düğmelerden biri (`Nav::extra`).
    Other,
}

impl Nav {
    fn new(cx: &mut Context<Shell>) -> Self {
        // Tab sırası tutamacın kendisinde tutuluyor: gpui `track_focus` ile
        // verilen tutamaçta öğenin `tab_index`'ini dikkate almıyor.
        let stop = |index| cx.focus_handle().tab_stop(true).tab_index(index);
        Nav {
            search: stop(tab::SEARCH),
            shelf: stop(tab::SHELF),
            all_close: stop(tab::ALL_CLOSE),
            collection_play: stop(tab::COLLECTION_PLAY),
            prev: stop(tab::PREV),
            play: stop(tab::PLAY),
            next: stop(tab::NEXT),
            progress: stop(tab::PROGRESS),
            mute: stop(tab::MUTE),
            volume: stop(tab::VOLUME),
            extra: Default::default(),
            reveal: Rc::new(Cell::new(false)),
        }
    }

    /// Anahtarla tembel açılan Tab durağı (aynı anahtar hep aynı tutamaç).
    fn extra(&self, cx: &mut gpui::App, key: SharedString, index: isize) -> FocusHandle {
        self.extra
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| cx.focus_handle().tab_stop(true).tab_index(index))
            .clone()
    }

    /// Odaktaki anahtarlı düğmenin anahtarı ve tutamacı.
    fn extra_focused(&self, window: &Window) -> Option<(SharedString, FocusHandle)> {
        self.extra
            .borrow()
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(k, h)| (k.clone(), h.clone()))
    }

    fn focused(&self, window: &Window) -> Option<NavTarget> {
        [
            (&self.search, NavTarget::Search),
            (&self.shelf, NavTarget::Shelf),
            (&self.all_close, NavTarget::AllClose),
            (&self.collection_play, NavTarget::CollectionPlay),
            (&self.prev, NavTarget::Previous),
            (&self.play, NavTarget::Play),
            (&self.next, NavTarget::Next),
            (&self.progress, NavTarget::Progress),
            (&self.mute, NavTarget::Mute),
            (&self.volume, NavTarget::Volume),
        ]
        .into_iter()
        .find(|(h, _)| h.is_focused(window))
        .map(|(_, t)| t)
        .or_else(|| self.extra_focused(window).map(|_| NavTarget::Other))
    }
}

#[derive(Clone, Copy)]
enum Drag {
    /// Sürüklenen konum (0.0–1.0); bırakınca oraya atlanır.
    Progress(f32),
    Volume,
    /// Ekolayzer sürgüsü: 0..10 bant, 10 ön kazanç (bkz. `equalizer`).
    Eq(usize),
    /// Karaoke miktar sürgüsü: panelde ya da tam ekranda (bkz. `karaoke`).
    Karaoke(lyrics_view::Place),
    /// Kaydırma çubuğu tutamacı: tutulduğu andaki fare ve kaydırma konumu.
    Scroll {
        bar: ScrollBar,
        start_y: f32,
        start_offset: f32,
    },
}

/// Hapın zamanla değişen kısmı (bkz. `Shell::bar_signature_at`).
#[derive(Clone, Copy, PartialEq, Eq)]
struct BarSignature {
    playing: bool,
    seconds: u32,
    fill_px: i32,
}

/// Kaydırma çubuğu olan alanlar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScrollBar {
    /// Açık listenin şarkıları.
    List,
    /// "Tümü" ızgarası.
    All,
    /// Sayfa (sanatçı, albüm, ana sayfa, arama).
    Page,
}

/// Kaydırma ölçüleri: görünen yükseklik, içerik yüksekliği, kaydırma (px).
struct ScrollMetrics {
    viewport: f32,
    content: f32,
    offset: f32,
}

impl ScrollMetrics {
    fn max(&self) -> f32 {
        (self.content - self.viewport).max(0.)
    }

    /// Tutamacın yüksekliği ve üst kenarı.
    fn thumb(&self) -> (f32, f32) {
        let h = (self.viewport * self.viewport / self.content).max(SCROLL_THUMB_MIN);
        let top = if self.max() > 0. {
            self.offset / self.max() * (self.viewport - h)
        } else {
            0.
        };
        (h, top)
    }
}

/// Kaydırma çubuğu tutamacının en kısa boyu ve genişliği.
const SCROLL_THUMB_MIN: f32 = 32.;
const SCROLL_THUMB_WIDTH: f32 = 6.;

/// Kayıtlarda kapak baytları ve uzun listeler yer kaplamasın.
fn short_debug(event: &engine::Event) -> String {
    match event {
        engine::Event::Cover {
            track_uri, accent, ..
        } => format!("Cover {{ {track_uri}, {accent:?} }}"),
        engine::Event::CollectionLoaded { id, tracks } => {
            format!(
                "CollectionLoaded {{ {}, {} şarkı }}",
                id.key(),
                tracks.len()
            )
        }
        engine::Event::SearchIndex(t) => format!("SearchIndex({} şarkı)", t.len()),
        other => {
            let s = format!("{other:?}");
            s.chars().take(160).collect()
        }
    }
}
