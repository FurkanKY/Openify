//! Sistemle bütünleşmenin arayüz tarafı (Adım 10).
//!
//! `crate::system` işletim sisteminden gelenleri bir kanala bırakır; burada
//! o kanal okunur ve `Shell`'in kendi işlemleriyle karşılanır (medya tuşu →
//! çal / duraklat, tepsi tıklaması → pencereyi göster, kısayol → her yerden
//! palet). Ters yönde, çalanın her değişimi sisteme bildirilir
//! (`Shell::sync_system`): Windows medya kontrolü, görev çubuğu düğmeleri,
//! tepsi ipucu.
//!
//! Pencere işlemleri (gizle, göster, açılır pencere aç, öne al) gpui'nin
//! güncellemesi bittikten sonra, ileti döngüsünde yapılır (`run`): Windows
//! bu işlemlerde pencereye eşzamanlı ileti gönderiyor, gpui de o iletileri
//! işlerken kendi durumunu ödünç alıyor; güncelleme içinden çağrılsaydı
//! çakışırdı.
//!
//! Kapat (X): tepsi simgesi varsa pencere gizlenir, müzik sürer; gerçek
//! çıkış tepsi menüsündeki "Çık". Gizliyken kendi zamanlayıcılarımız durur
//! (hap zamanlayıcısı, ekolayzer), gizli pencereye gpui kare çizmez.

pub use imp::*;

#[cfg(windows)]
mod imp {
    use std::{sync::Arc, time::Instant};

    use futures::StreamExt;
    use gpui::{AsyncApp, Context, Subscription, Task, WeakEntity, Window, WindowHandle};
    use windows::Win32::Foundation::HWND;

    use super::super::{Shell, player::Status, popups, windows_titlebar};
    use crate::{
        engine::{self, Command},
        system::{
            self, Cover, Features, Integration, MediaButton, NowPlaying, PlayStatus, Stamped,
            SystemEvent,
        },
    };

    /// Güncelleme bittikten sonra yapılacak pencere işleri (bkz. modül notu).
    pub enum Op {
        /// Ana pencereyi tepsiye gizle.
        Hide,
        /// Ana pencereyi göster; `front` ise öne al.
        Show {
            front: bool,
            at: Instant,
            why: &'static str,
        },
        /// Tepsi menüsünü bu ekran noktasında aç.
        OpenMenu { x: i32, y: i32, at: Instant },
        /// Her yerden paleti aç.
        OpenPalette { at: Instant },
        /// Odağı bu pencereye geri ver (palet kapandı).
        Focus(HWND),
        /// Kapanan açılır pencereyi gizle.
        HidePopup(HWND),
        /// Kullanıcıya bildirim.
        Notice(String),
        /// Bilgi bildirimi (ör. güncelleme hazır).
        Info(String),
        /// İlk kareden sonra: medya kontrolü, görev çubuğu, tepsi (açılışı
        /// ~35–45 ms geciktirmesin).
        InstallRest,
        /// `--tray` açılışı: tepsi kurulduysa pencere gizli kalır, yoksa gösterilir.
        StartHidden,
    }

    enum Msg {
        System(Stamped),
        Op(Op),
    }

    /// `Shell`'in sistemle ilgili durumu.
    pub struct Desk {
        discord: crate::discord::Presence,
        integration: Option<Integration>,
        pub(in crate::ui) hwnd: Option<HWND>,
        _ipc: Option<HWND>,
        /// Ana pencere tepsiye gizli.
        pub hidden: bool,
        /// Ayarlar: kapat düğmesi pencereyi tepsiye gizler (`false`: çıkar).
        pub close_to_tray: bool,
        /// "Çık" seçildi: kapatma isteği pencereyi gizlemesin.
        quitting: bool,
        pub(in crate::ui) ops: async_channel::Sender<Op>,
        /// Açılır pencereler: bir kez açılır, sonra gizlenip yeniden gösterilir.
        pub(in crate::ui) menu_window: Option<WindowHandle<popups::TrayMenu>>,
        pub(in crate::ui) palette_window: Option<WindowHandle<popups::QuickPalette>>,
        /// Her yerden palet açılmadan önce öndeki pencere (kapanınca odak ona döner).
        pub(in crate::ui) return_focus: Option<HWND>,
        /// Kapak önbelleği: aynı kapak her olayda yeniden kopyalanmasın.
        cover: Option<Cover>,
        _task: Task<()>,
        /// Gizliyken bellek bırakma zamanlayıcısı (bkz. `schedule_trim`).
        pub(in crate::ui) trim_task: Option<Task<()>>,
        _quit: Subscription,
    }

    impl Desk {
        pub fn new(
            window: &mut Window,
            cx: &mut Context<Shell>,
            prefs: &crate::settings::Settings,
            start_hidden: bool,
        ) -> Self {
            let hwnd = windows_titlebar::hwnd(window);
            let features = Features::from_env();
            let events = system::listen();
            let integration = hwnd.map(|h| Integration::install(h, features, prefs.global_hotkey));
            let ipc = system::instance::create_ipc_window();
            let (ops, ops_rx) = async_channel::unbounded();
            if let Some(problem) = integration.as_ref().and_then(|i| i.hotkey_problem()) {
                let _ = ops.try_send(Op::Notice(problem));
            }
            // Güncelleme denetimi arka planda (bkz. `crate::update`).
            let (update_tx, update_rx) = async_channel::unbounded::<String>();
            crate::update::start(move |e| {
                let _ = update_tx.try_send(e.notice());
            });
            let info = ops.clone();
            cx.spawn(async move |_, _| {
                while let Ok(text) = update_rx.recv().await {
                    let _ = info.try_send(Op::Info(text));
                }
            })
            .detach();
            // Sınama koşusu: gpui pencereyi gösterirken öne aldı, odak hemen
            // kullanıcıya döner (bkz. `system::take_test_foreground`).
            if let Some(user) = system::take_test_foreground() {
                let _ = ops.try_send(Op::Focus(user));
            }
            // Ağır parçalar ilk kare çizildikten sonra kurulur. Gizli açılışta
            // (`--tray`) pencere hiç çizilmez, kare gelmez: kısa bir
            // beklemeden sonra kurulur, tepsi hazırsa pencere gizli kalır
            // (tepsi yoksa pencere gösterilir: ulaşılamaz kalmasın).
            let later = ops.clone();
            if start_hidden {
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                    let _ = later.try_send(Op::InstallRest);
                    let _ = later.try_send(Op::StartHidden);
                })
                .detach();
            } else {
                window.on_next_frame(move |_, _| {
                    let _ = later.try_send(Op::InstallRest);
                });
            }
            let task = cx.spawn(async move |this, cx| {
                let mut stream = std::pin::pin!(futures::stream::select(
                    events.map(Msg::System),
                    ops_rx.map(Msg::Op)
                ));
                while let Some(msg) = stream.next().await {
                    let ops = match msg {
                        Msg::System(s) => {
                            match this.update(cx, |shell, cx| shell.on_system_event(s, cx)) {
                                Ok(ops) => ops,
                                Err(_) => break,
                            }
                        }
                        Msg::Op(op) => vec![op],
                    };
                    for op in ops {
                        run(op, &this, cx);
                    }
                }
            });
            // Hangi yoldan çıkılırsa çıkılsın (tepsideki "Çık", paletteki
            // "Çıkış yap"): tepsi simgesi kalmasın, kısayol bırakılsın.
            let quit = cx.on_app_quit(|shell, _| {
                // Düzgün kapanış çökme değildir (bkz. `update::clean_exit`).
                crate::update::clean_exit();
                shell.desk.shutdown();
                shell.desk.discord.shutdown()
            });
            Desk {
                discord: crate::discord::Presence::new(),
                integration,
                hwnd,
                _ipc: ipc,
                hidden: false,
                close_to_tray: prefs.close_to_tray,
                quitting: false,
                ops,
                menu_window: None,
                palette_window: None,
                return_focus: None,
                cover: None,
                _task: task,
                trim_task: None,
                _quit: quit,
            }
        }

        fn shutdown(&mut self) {
            if let Some(integration) = &mut self.integration {
                integration.shutdown();
            }
        }

        /// Kapat (X) pencereyi gizlesin mi? Tepsi simgesi yoksa gizlenen
        /// pencereye dönülemezdi: o zaman uygulama kapanır.
        fn hide_on_close(&self) -> bool {
            !self.quitting
                && self.close_to_tray
                && self.integration.as_ref().is_some_and(|i| i.tray_active())
        }

        pub fn set_close_to_tray(&mut self, on: bool) {
            self.close_to_tray = on;
        }

        /// Ayarlar: her yerden arama kısayolunu aç / kapat. Alınamadıysa
        /// kullanıcıya söylenecek metin döner.
        pub fn set_global_hotkey(&mut self, on: bool) -> Option<String> {
            self.integration.as_mut().and_then(|i| i.set_hotkey(on))
        }

        pub fn send(&self, op: Op) {
            let _ = self.ops.try_send(op);
        }
    }

    impl Shell {
        /// Ana pencere tepsiye gizli mi? (Gizliyken zamanlayıcılar durur.)
        pub(in crate::ui) fn window_hidden(&self) -> bool {
            self.desk.hidden
        }

        /// Pencere kapatılmak istendi (X, Alt+F4). `true`: kapansın.
        pub(in crate::ui) fn on_close_requested(&mut self, cx: &mut Context<Self>) -> bool {
            self.save_settings();
            if self.desk.hide_on_close() {
                self.desk.send(Op::Hide);
                false
            } else {
                // Gizli açılır pencereler (menü, palet) gpui'nin pencere
                // listesinde kalıyor: "son pencere kapandı" beklenmez, çıkılır.
                self.desk.shutdown();
                cx.quit();
                true
            }
        }

        /// Gerçekten çık (tepsi menüsündeki "Çık").
        pub(in crate::ui) fn quit_app(&mut self, cx: &mut Context<Self>) {
            self.desk.quitting = true;
            self.save_settings();
            self.desk.shutdown();
            cx.quit();
        }

        /// Çalanı sisteme bildir. `timeline`: konum da yeniden verilsin
        /// (şarkı değişti, çaldı, durdu, sarıldı).
        pub(in crate::ui) fn sync_system(&mut self, timeline: bool) {
            self.sync_discord();
            if self.desk.integration.is_none() {
                return;
            }
            let now = self.now_playing();
            if let Some(integration) = &mut self.desk.integration {
                integration.update(now, timeline);
            }
        }

        pub(in crate::ui) fn sync_discord(&self) {
            let value = crate::discord::desired(
                self.prefs.discord_presence,
                self.private_session(),
                self.state.is_playing()
                    && !matches!(
                        self.state.connection,
                        super::super::player::Connection::EngineStopped
                            | super::super::player::Connection::NoAudio
                            | super::super::player::Connection::NeedsLogin
                            | super::super::player::Connection::NeedsPremium
                    ),
                self.state.track.as_ref(),
                self.state.position_ms(Instant::now()),
            );
            self.desk.discord.update(value);
        }

        fn now_playing(&mut self) -> NowPlaying {
            let can_play = self.startable_collection().is_some();
            let Some(track) = self.state.track.clone() else {
                return NowPlaying {
                    can_play,
                    ..Default::default()
                };
            };
            let status = match self.state.status {
                Status::Idle => PlayStatus::Stopped,
                Status::Loading => PlayStatus::Changing,
                Status::Playing => PlayStatus::Playing,
                Status::Paused => PlayStatus::Paused,
                Status::Ended | Status::Unavailable => PlayStatus::Stopped,
            };
            let row = self.playing_row(&track.uri);
            let album = row.map(|r| r.album.clone()).unwrap_or_default();
            let small = row.map(|r| r.image.clone()).unwrap_or_default();
            NowPlaying {
                title: track.name.clone(),
                artist: track.artists.clone(),
                album,
                status,
                duration_ms: track.duration_ms,
                position_ms: self.state.position_ms(Instant::now()),
                can_play: true,
                can_next: self.state.queue.has_next,
                can_previous: true,
                cover: self.system_cover(&track.uri, &small),
            }
        }

        /// Çalan şarkının kütüphanedeki satırı (albüm adı ve küçük kapak için).
        fn playing_row(&self, uri: &str) -> Option<&engine::TrackRow> {
            let queue = &self.state.queue;
            if let Some(row) = queue
                .context
                .as_ref()
                .and_then(|id| self.library.tracks(id))
                .and_then(|t| t.get(queue.index))
                .filter(|r| r.uri == uri)
            {
                return Some(row);
            }
            self.library.collections.values().find_map(|c| match c {
                super::super::library::Loadable::Loaded(tracks) => {
                    tracks.iter().find(|r| r.uri == uri)
                }
                _ => None,
            })
        }

        /// Medya kontrolündeki kapak: motorun getirdiği büyük kapak; o henüz
        /// gelmediyse kütüphanedeki küçük kapak (diskteki önbellekten).
        fn system_cover(&mut self, uri: &str, small_url: &str) -> Option<Cover> {
            // Anahtar: büyük kapakta resmin kendisi (aynı şarkının kapağı
            // yeniden gelebilir), küçükte adresi.
            let key = match &self.state.cover {
                Some(image) => format!("{uri}#{:x}", Arc::as_ptr(image) as usize),
                None if !small_url.is_empty() => small_url.to_string(),
                None => return None,
            };
            if let Some(cover) = &self.desk.cover
                && cover.key == key
            {
                return Some(cover.clone());
            }
            let bytes: Arc<[u8]> = match &self.state.cover {
                Some(image) => image.bytes.clone().into(),
                None => std::fs::read(self.images.get(small_url)?).ok()?.into(),
            };
            let cover = Cover { key, bytes };
            self.desk.cover = Some(cover.clone());
            Some(cover)
        }

        fn on_system_event(&mut self, s: Stamped, cx: &mut Context<Self>) -> Vec<Op> {
            match s.event {
                SystemEvent::Media(button) => {
                    self.media_button(button, cx);
                    log::info!(
                        "sistem: medya düğmesi {button:?} → komut {:.2} ms",
                        s.at.elapsed().as_secs_f64() * 1000.
                    );
                    vec![]
                }
                SystemEvent::SeekTo(ms) => {
                    self.seek_to(ms, cx);
                    log::info!("sistem: medya kontrolünden sarma {ms} ms");
                    vec![]
                }
                SystemEvent::Show => vec![Op::Show {
                    front: true,
                    at: s.at,
                    why: "tepsi / ikinci açılış",
                }],
                SystemEvent::TrayMenu { x, y } => vec![Op::OpenMenu { x, y, at: s.at }],
                SystemEvent::Hotkey => {
                    log::info!("sistem: her yerden palet kısayolu");
                    // Palet zaten kendi penceresinde açıksa kısayol kapatır.
                    if self.desk.palette_window.is_some()
                        && self.palette.open
                        && self.palette.detached
                    {
                        self.close_palette(cx);
                        return vec![];
                    }
                    // Openify zaten öndeyse palet pencerenin içinde açılır.
                    let front =
                        !self.desk.hidden && self.desk.hwnd.is_some_and(system::is_foreground);
                    if front {
                        self.open_palette(cx);
                        vec![]
                    } else {
                        vec![Op::OpenPalette { at: s.at }]
                    }
                }
                other => {
                    if let Some(integration) = &mut self.desk.integration {
                        integration.on_event(other);
                    }
                    vec![]
                }
            }
        }

        fn media_button(&mut self, button: MediaButton, cx: &mut Context<Self>) {
            let playing = matches!(self.state.status, Status::Playing | Status::Loading);
            match button {
                MediaButton::Play if !playing => self.toggle_play(cx),
                MediaButton::Pause | MediaButton::Stop if playing => {
                    self.engine.send(Command::Pause)
                }
                MediaButton::Toggle => self.toggle_play(cx),
                MediaButton::Next => self.next(cx),
                MediaButton::Previous => self.previous(cx),
                _ => {}
            }
        }
    }

    /// Pencere gizlendikten (ya da gizliyken açılan palet / menü kapandıktan)
    /// bu kadar sonra bellek bırakılır: sürücünün yüzeyleri serbest bırakması
    /// ~2 sn sürüyor, art arda açılıp kapanan palet her seferinde pencere
    /// kurmasın.
    const TRIM_DELAY: std::time::Duration = std::time::Duration::from_millis(1500);
    /// İlk bırakmadan ikincisine kadar (bkz. `schedule_trim`).
    const TRIM_AGAIN: std::time::Duration = std::time::Duration::from_millis(2000);

    /// Gizliyken belleği geri vermek için zamanlayıcıyı (yeniden) kur. Pencere
    /// görünürse ya da bir açılır pencere açıksa vazgeçilir (açılır pencere
    /// kapanınca yeniden kurulur).
    fn schedule_trim(this: &WeakEntity<Shell>, cx: &mut AsyncApp) {
        let _ = this.update(cx, |shell, cx| {
            if !shell.desk.hidden {
                return;
            }
            shell.desk.trim_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TRIM_DELAY).await;
                trim_memory(&this, cx);
                // Kapanan pencerelerin nesne yok etmelerini sürücü ilk Flush
                // anında bitirmiyor (GPU hâlâ kullanıyor sayıyor): ikinci bir
                // Flush ~4 MB daha serbest bırakıyor (en ağır durumda 75 → 71).
                cx.background_executor().timer(TRIM_AGAIN).await;
                trim_memory(&this, cx);
            }));
        });
    }

    fn popup_hwnd<V: 'static>(handle: Option<WindowHandle<V>>, cx: &mut AsyncApp) -> Option<HWND> {
        handle?
            .update(cx, |_, window, _| windows_titlebar::hwnd(window))
            .ok()
            .flatten()
    }

    /// Gizliyken bellek (çıta < 80 MB, en ağır durumda bile):
    /// görünür pencere yokken
    /// 1. kapalı açılır pencereler (palet, tepsi menüsü) yok edilir: her biri
    ///    kendi çizim kaynaklarını (atlas dokuları, boru hattı tamponları)
    ///    tutuyor, palette arama yazılınca 8–9 MB'a çıkıyor; bir sonraki açılış
    ///    pencereyi yeniden kurar (ölçüm: ilk açılış 11–18 ms, çıta 100);
    /// 2. kalan pencerelerin kareler arasında tutulan boş kapasitesi ve
    ///    elemanlar arenası bırakılır (`Window::release_memory`, gpui yaması);
    /// 3. NT yığınları sıkıştırılır.
    ///    Gösterilince hiçbirine ihtiyaç yok: çizim kaynakları gpui yamasıyla
    ///    zaten geri gelir, pencere yeniden kurulur.
    ///    Geliştirme kancası: `OPENIFY_TRIM_SKIP="popups,gpui,heap"` parçaları
    ///    atlar (hangisinin ne kazandırdığını ölçmek için).
    fn trim_memory(this: &WeakEntity<Shell>, cx: &mut AsyncApp) {
        let Ok((hidden, menu, palette)) = this.update(cx, |s, _| {
            (s.desk.hidden, s.desk.menu_window, s.desk.palette_window)
        }) else {
            return;
        };
        if !hidden {
            return;
        }
        // Açılır pencere ekrandaysa dokunma (kapanınca yeniden denenir).
        let shown = [popup_hwnd(menu, cx), popup_hwnd(palette, cx)]
            .into_iter()
            .flatten()
            .any(system::is_visible);
        if shown {
            return;
        }
        // Kulaklık profili kataloğu (ekolayzer araması, birkaç MB) gizliyken tutulmaz.
        let _ = this.update(cx, |s, _| s.eq_leave());
        let skip = std::env::var("OPENIFY_TRIM_SKIP").unwrap_or_default();
        let skips = |part: &str| skip.split(',').any(|s| s.trim() == part);
        let t = Instant::now();
        if !skips("popups") {
            if let Some(h) = menu {
                let _ = h.update(cx, |_, window, _| window.remove_window());
            }
            if let Some(h) = palette {
                let _ = h.update(cx, |_, window, _| window.remove_window());
            }
            let _ = this.update(cx, |s, _| {
                s.desk.menu_window = None;
                s.desk.palette_window = None;
            });
        }
        if !skips("gpui") {
            let _ = cx.update(|app| {
                for handle in app.windows() {
                    let _ = handle.update(app, |_, window, _| window.release_memory());
                }
            });
        }
        let gpui_ms = t.elapsed().as_secs_f64() * 1000.;
        if !skips("heap") {
            system::compact_heaps();
        }
        log::info!(
            "sistem: gizlenince bellek bırakıldı, pencereler + gpui {gpui_ms:.1} ms, toplam {:.1} ms",
            t.elapsed().as_secs_f64() * 1000.
        );
    }

    /// Pencere işleri: gpui güncellemesinin dışında (bkz. modül notu).
    fn run(op: Op, this: &WeakEntity<Shell>, cx: &mut AsyncApp) {
        match op {
            Op::Hide => {
                let Ok(Some(hwnd)) = this.update(cx, |shell, cx| {
                    shell.desk.hidden = true;
                    // Hap zamanlayıcısı durur (gizli pencere çizilmez).
                    shell.ticker = None;
                    // Panelin / tam ekranın büyük kapağı (640 px) bırakılır;
                    // görününce yeniden istenir.
                    shell.sync_hires(cx);
                    // Arkadaş etkinliği yenilemesi durur.
                    shell.sync_friends(cx);
                    shell.desk.hwnd
                }) else {
                    return;
                };
                system::hide_window(hwnd);
                log::info!("sistem: pencere tepsiye gizlendi");
                schedule_trim(this, cx);
            }
            Op::Show { front, at, why } => {
                let Ok(Some(hwnd)) = this.update(cx, |shell, cx| {
                    shell.desk.hidden = false;
                    shell.desk.trim_task = None;
                    shell.restart_ticker(cx);
                    // Gizlenirken bırakılan büyük kapak (gerekiyorsa).
                    shell.sync_hires(cx);
                    // Arkadaş paneli açıksa yenileme sürer (süre dolduysa hemen).
                    shell.sync_friends(cx);
                    cx.notify();
                    shell.desk.hwnd
                }) else {
                    return;
                };
                // Sınama koşusunda öne alınmaz (kullanıcının odağı).
                let fronted = system::show_window(hwnd, front && !system::test_mode());
                log::info!(
                    "sistem: pencere gösterildi ({why}), {:.1} ms, öne alındı: {fronted}",
                    at.elapsed().as_secs_f64() * 1000.
                );
            }
            Op::OpenMenu { x, y, at } => {
                let _ = this.update(cx, |s, _| s.desk.trim_task = None);
                popups::open_tray_menu(this, x, y, at, cx)
            }
            Op::OpenPalette { at } => {
                let _ = this.update(cx, |s, _| s.desk.trim_task = None);
                popups::open_global_palette(this, at, cx)
            }
            Op::Focus(hwnd) => {
                let ok = system::set_foreground(hwnd);
                log::debug!("sistem: odak geri verildi: {ok}");
            }
            Op::HidePopup(hwnd) => {
                system::hide_window(hwnd);
                schedule_trim(this, cx);
            }
            Op::InstallRest => {
                let _ = this.update(cx, |shell, _| {
                    if let Some(integration) = &mut shell.desk.integration {
                        integration.install_rest();
                    }
                });
            }
            Op::StartHidden => {
                let tray = this
                    .update(cx, |shell, _| {
                        shell
                            .desk
                            .integration
                            .as_ref()
                            .is_some_and(|i| i.tray_active())
                    })
                    .unwrap_or(false);
                if tray {
                    run(Op::Hide, this, cx);
                } else {
                    run(
                        Op::Show {
                            front: true,
                            at: Instant::now(),
                            why: "gizli açılış, tepsi yok",
                        },
                        this,
                        cx,
                    );
                }
            }
            Op::Info(text) => {
                let _ = this.update(cx, |shell, cx| {
                    shell.state.notify(engine::NoticeKind::Info, text);
                    cx.notify();
                });
            }
            Op::Notice(text) => {
                let _ = this.update(cx, |shell, cx| {
                    shell.state.notify(engine::NoticeKind::Problem, text);
                    cx.notify();
                });
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui::{Context, Window};

    use super::super::Shell;

    /// Windows dışında sistemle bütünleşme yok.
    pub struct Desk;

    impl Desk {
        pub fn new(
            _: &mut Window,
            _: &mut Context<Shell>,
            _: &crate::settings::Settings,
            _: bool,
        ) -> Self {
            Desk
        }

        pub fn set_global_hotkey(&mut self, _: bool) -> Option<String> {
            None
        }

        pub fn set_close_to_tray(&mut self, _: bool) {}
    }

    impl Shell {
        pub(in crate::ui) fn window_hidden(&self) -> bool {
            false
        }
        pub(in crate::ui) fn on_close_requested(&mut self, _: &mut Context<Self>) -> bool {
            self.save_settings();
            true
        }
        pub(in crate::ui) fn sync_system(&mut self, _timeline: bool) {}
        pub(in crate::ui) fn sync_discord(&self) {}
    }
}
