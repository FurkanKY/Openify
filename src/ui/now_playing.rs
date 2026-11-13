//! Şimdi çalıyor (Adım 11 ve 14): karıştırma / tekrar, sıra paneli, şarkı
//! sözleri, tam ekran ve açılışta kaldığı yerden geri gelen durum.
//!
//! Bu dosya ortak durumu (`NowState`), motordan gelen sıra / söz olaylarını,
//! kısayolları, söz saatini ve iki önbellekli görünümü (`SideView` sağdaki
//! yüzen panel, `FullView` tam ekran) tutar. Çizim ayrı dosyalarda:
//! `queue_panel.rs` (panel ve sıra), `lyrics_view.rs` (sözler),
//! `fullscreen.rs` (tam ekran ve bulanık arka plan).
//!
//! Çizim yükü: panel ve tam ekran kendi görünümlerinde; çalarken yalnız söz
//! satırı değiştiğinde (bir sonraki satırın başlangıcına kurulan
//! zamanlayıcıyla) ya da kayma / belirme hareketi sürerken (en çok 60
//! kare/sn) yeniden çizilirler. Dururken hiç çizim yok.
//!
//! Söz saati DUYULAN konumdur (`AudioLevels::audible_track_ms`, ses kartının
//! saatinden): kuyruktaki ses, aygıt gecikmesi ve boşluksuz geçişte önceki
//! şarkının kuyruktaki sonu hesaba katılmıştır. Bilinmiyorsa (sarma sürüyor,
//! sahte çalma) arayüzün kendi saati.

use std::{
    cell::Cell,
    fs::File,
    io::{BufWriter, Write as _},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, KeyDownEvent, Render, ScrollHandle,
    Subscription, Task, UniformListScrollHandle, WeakEntity, Window, div, prelude::*,
};

use super::{Shell, fullscreen::FullState, shortcuts::Shortcut};
use crate::engine::{
    self, AudioLevels, CollectionId, Command, LyricLine, Lyrics, PlaybackEvent, QueueEntry,
    QueueSource, QueueView, RepeatMode, SyncKind,
};

/// Hareketlerin (panelin kayması, sözün kayması, denetimlerin belirmesi)
/// kare hızı sınırı: ekran daha hızlı olsa da en çok bu kadar.
pub(super) const ANIM_FPS: f32 = 60.;
/// Söz satırının ekrana geleceği ana ileri bakış: zamanlayıcının uyanması,
/// çizim ve bir sonraki kare. Ölçülen (`OPENIFY_LYRICS_TRACE`, 8 ms ileri
/// bakışla) satır başına göre +7…+22 ms geç boyanıyordu (ortanca 12–16):
/// 16 ms öne alınınca değişim satırın başına ortalanır.
pub(super) const LYRIC_LEAD_MS: i64 = 16;
/// Sarma ya da şarkı değişiminden sonra duyulan saat arayüzün saatine bu
/// kadar yaklaşana dek (ya da bu süre dolana dek) arayüzün saati kullanılır:
/// çıkış eski sesi atarken eski konumu bildirmesin.
const CLOCK_AGREE_MS: i64 = 250;
const CLOCK_GUARD: Duration = Duration::from_millis(2000);

/// Sınama kancası `OPENIFY_TEST_CONNECT=remote|spirc` (yalnız sınama koşusunda):
/// Spotify'a hiç dokunmadan Connect durumunu sabit gösterir (ekran sınaması:
/// "başka aygıtta çalıyor" şeridi ve "Buraya aktar" düğmesi). Motorun gerçek
/// durumu bu durumda yok sayılır.
pub(super) fn connect_fixture() -> Option<engine::ConnectStatus> {
    static FIXTURE: OnceLock<Option<engine::ConnectStatus>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let mode = std::env::var("OPENIFY_TEST_CONNECT")
                .ok()
                .filter(|_| crate::crash::is_test_run())?;
            let device_name = "Openify (TEST-PC)".to_string();
            Some(match mode.as_str() {
                "remote" => engine::ConnectStatus {
                    driver: engine::ConnectDriver::Remote,
                    device_name,
                    remote: Some(engine::RemoteNow {
                        device_name: "Pixel 8".to_string(),
                    }),
                },
                "spirc" => engine::ConnectStatus {
                    driver: engine::ConnectDriver::Spirc,
                    device_name,
                    remote: None,
                },
                _ => return None,
            })
        })
        .clone()
}

/// Sağdaki panelin hangi hâli (sekmesi).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Panel {
    /// Şimdi çalıyor: büyük kapak, sıradaki, söz kesiti, sanatçı; hiçbir şey
    /// yüklü değilken son çalınanlar (bkz. `now_panel`).
    Now,
    Queue,
    Lyrics,
    /// Arkadaş etkinliği (bkz. `friends`): öbür üçüyle aynı sağ yuvayı paylaşır,
    /// kendi başlığı var (sekme hapı değil); başlık çubuğundaki düğmeyle açılır.
    Friends,
}

/// Çalan şarkının sözü.
pub(super) enum LyricsStatus {
    /// İstenmedi (panel ve tam ekran kapalı).
    None,
    Loading,
    Ready(Arc<Lyrics>),
    /// Hiçbir kaynakta yok.
    Missing,
}

/// Söz listesinin kaydırma durumu (panelde ve tam ekranda ayrı).
pub(super) struct LyricScroll {
    pub handle: ScrollHandle,
    /// Son çizimde etkin satır.
    pub line: Option<usize>,
    /// Sürmekte olan kayma: başlangıç, hedef (kaydırma konumu, px), an.
    pub anim: Option<(f32, f32, Instant)>,
    /// Kullanıcı tekerlekle kaydırdı: bu ana dek kendiliğinden izleme yok.
    pub user_until: Option<Instant>,
    /// İz kaydı için: son boyanan etkin satır.
    pub painted: Rc<Cell<Option<usize>>>,
    /// Kenar solması yerleşimden okunur; içerik yeni gelince ya da açılışta
    /// yerleşim ölçülene dek bu kadar kare daha istenir (sonra dururken çizim yok).
    pub settle: u8,
    /// Rengi parlayan satır, bir önceki ve değişim anı: etkin satır rengi
    /// anında atlamaz, `motion` içinde aydınlanır (öncekisi söner).
    pub lit: Option<usize>,
    pub lit_prev: Option<usize>,
    pub lit_at: Option<Instant>,
}

/// Yerleşimin ölçülmesi için gereken kare sayısı (bkz. `LyricScroll::settle`).
const SETTLE_FRAMES: u8 = 3;

impl LyricScroll {
    fn new() -> Self {
        Self {
            handle: ScrollHandle::new(),
            line: None,
            anim: None,
            user_until: None,
            painted: Rc::default(),
            settle: SETTLE_FRAMES,
            lit: None,
            lit_prev: None,
            lit_at: None,
        }
    }

    pub(super) fn reset(&mut self) {
        self.handle
            .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        self.line = None;
        self.anim = None;
        self.user_until = None;
        self.painted.set(None);
        self.lit = None;
        self.lit_prev = None;
        self.lit_at = None;
        self.settle = SETTLE_FRAMES;
    }
}

/// Tab ile gezilen yeni düğmeler (hapta).
pub(super) struct Stops {
    pub shuffle: FocusHandle,
    pub repeat: FocusHandle,
    pub mini: FocusHandle,
    pub queue: FocusHandle,
    pub lyrics: FocusHandle,
}

pub(super) struct NowState {
    /// Motorun son gönderdiği sıra (sürükle bırakta önce burada uygulanır,
    /// motorun cevabı üstüne yazar).
    pub queue: QueueView,
    pub queue_known: bool,
    pub panel: Option<Panel>,
    /// Panel her açılışta yeni kimlikle kayarak gelir.
    pub panel_gen: u64,
    /// Panelin açıldığı an (kayarak gelme hareketi için).
    pub panel_at: Instant,
    /// Kenarda kendiliğinden kaydırma sürüyor.
    pub autoscrolling: bool,
    pub list: UniformListScrollHandle,
    /// Klavyeyle seçili sıra satırı (görünür listedeki yeri).
    pub cursor: Option<usize>,
    /// Sürüklerken bırakılacak yer (görünür listede bu satırın önüne).
    pub drop_at: Option<usize>,
    /// Sürüklerken son fare yüksekliği (listenin üstüne göre) ve kenarda
    /// kendiliğinden kaydırma hızı (px / kare).
    pub drag_y: Option<f32>,
    pub autoscroll: f32,
    /// Panelin klavye odağı (sıra satırları ok tuşlarıyla).
    pub focus: FocusHandle,
    pub stops: Stops,
    pub lyrics: LyricsStatus,
    pub lyrics_uri: Option<String>,
    prefetched: Option<String>,
    pub side_scroll: LyricScroll,
    pub full_scroll: LyricScroll,
    lyric_timer: Option<Task<()>>,
    pub full: FullState,
    /// Sarma / şarkı değişiminden beri: duyulan saat güvenilir olana dek.
    clock_guard: Option<Instant>,
    /// İz kaydı için son sarma anı ve son bırakma (sürükle bırak) anı.
    pub seek_at: Option<Instant>,
    pub dropped_at: Option<Instant>,
    pub fake: bool,
    pub levels: AudioLevels,
    /// Spotify Connect durumu (motordan; kapalıyken hep varsayılan: yerel).
    pub connect: engine::ConnectStatus,
    /// Yerleşik panel (geniş pencere) durumu, bkz. `now_panel`.
    pub dock: super::now_panel::DockState,
    _quit: Subscription,
}

impl NowState {
    pub(super) fn new(levels: AudioLevels, cx: &mut Context<Shell>) -> Self {
        let stop =
            |cx: &mut Context<Shell>, index| cx.focus_handle().tab_stop(true).tab_index(index);
        // Kapanırken kaldığı yer diske yazılsın (hangi yoldan çıkılırsa
        // çıkılsın: pencere, tepsi, palet). En çok 1,5 sn bekler.
        let quit = cx.on_app_quit(|shell: &mut Shell, _| {
            shell.engine.flush_state();
            async {}
        });
        Self {
            queue: QueueView::default(),
            queue_known: false,
            panel: None,
            panel_gen: 0,
            panel_at: Instant::now(),
            autoscrolling: false,
            list: UniformListScrollHandle::new(),
            cursor: None,
            drop_at: None,
            drag_y: None,
            autoscroll: 0.,
            focus: cx.focus_handle(),
            // Hap sırası: karıştır önceki'den, tekrar sonraki'den önce /
            // sonra; sıra ve sözler sessize al'dan önce (bkz. `Nav`).
            stops: Stops {
                shuffle: stop(cx, super::tab::SHUFFLE),
                repeat: stop(cx, super::tab::REPEAT),
                mini: stop(cx, super::tab::MINI),
                queue: stop(cx, super::tab::QUEUE),
                lyrics: stop(cx, super::tab::LYRICS),
            },
            lyrics: LyricsStatus::None,
            lyrics_uri: None,
            prefetched: None,
            side_scroll: LyricScroll::new(),
            full_scroll: LyricScroll::new(),
            lyric_timer: None,
            full: FullState::new(),
            clock_guard: None,
            seek_at: None,
            dropped_at: None,
            fake: std::env::var_os("OPENIFY_TEST_FAKE_PLAY").is_some(),
            levels,
            connect: connect_fixture().unwrap_or_default(),
            dock: super::now_panel::DockState::default(),
            _quit: quit,
        }
    }

    /// Görünür sıranın (taşınabilen, çıkarılabilen) uzunluğu: kullanıcı
    /// sırası ve bağlamın kalanı. Tekrar "hepsi"deki sonraki tur dahil değil.
    pub(super) fn visible_len(&self) -> usize {
        (self.queue.user_len + self.queue.context_remaining).min(self.queue.upcoming.len())
    }

    /// Ekranda çizilen panel: açık olan ya da kapanma hareketi sürenin
    /// (bkz. `now_panel::Slide`).
    pub(super) fn shown_panel(&self) -> Option<Panel> {
        self.panel.or(self.dock.slide.exiting)
    }

    /// Panel geniş pencerede listenin yanına yerleşmiş mi (yüzmüyor).
    pub(super) fn docked(&self) -> bool {
        self.dock.visual_docked && self.panel.is_some()
    }

    /// Sözler bir yerde görünüyor mu (sözler sekmesi, "Şimdi çalıyor"daki
    /// kesit ya da tam ekran): istenir, satır değişince çizilir.
    pub(super) fn lyrics_shown(&self) -> bool {
        matches!(self.panel, Some(Panel::Lyrics | Panel::Now))
            || (self.full.open && self.full.show_lyrics)
    }
}

/// Sıra görünümünde taşıma (motorun `Queue::move_item` kuralı, bkz.
/// `queue.rs`): ekranda bir karede görünsün diye motorun cevabı beklenmez.
pub(super) fn apply_move(view: &mut QueueView, from: usize, to: usize) {
    let total = (view.user_len + view.context_remaining).min(view.upcoming.len());
    if from >= total {
        return;
    }
    let to = to.min(total - 1);
    if from == to {
        return;
    }
    let ulen = view.user_len;
    let mut item = view.upcoming.remove(from);
    if from < ulen {
        let pos = to.min(ulen - 1);
        view.upcoming.insert(pos, item);
    } else if to < ulen {
        item.source = QueueSource::User;
        view.upcoming.insert(to, item);
        view.user_len += 1;
        view.context_remaining = view.context_remaining.saturating_sub(1);
    } else {
        view.upcoming.insert(to, item);
    }
}

/// Sıradan çıkarma (motorun `Queue::remove` kuralı).
pub(super) fn apply_remove(view: &mut QueueView, i: usize) {
    let total = (view.user_len + view.context_remaining).min(view.upcoming.len());
    if i >= total {
        return;
    }
    view.upcoming.remove(i);
    if i < view.user_len {
        view.user_len -= 1;
    } else {
        view.context_remaining = view.context_remaining.saturating_sub(1);
    }
}

/// Bırakılan yerden motorun "taşındıktan sonraki konum"una: satırın önüne
/// bırakılıyor, kendisi aradan çıkınca yer bir kayar.
pub(super) fn drop_target(from: usize, before: usize) -> usize {
    if before > from { before - 1 } else { before }
}

impl Shell {
    // ── Motor olayları ──────────────────────────────────────────────────

    /// Sıra ve kaldığı yer kanalı (`EngineHandle::playback`).
    pub(super) fn on_playback_event(&mut self, event: PlaybackEvent, cx: &mut Context<Self>) {
        match event {
            PlaybackEvent::Queue(view) => {
                // Sahte çalmada sıra arayüzde kurulur; motorun (boş) sırası
                // onu ezmesin.
                if self.now.fake {
                    return;
                }
                self.set_queue(view);
                self.maybe_prefetch_next();
                cx.notify();
            }
            PlaybackEvent::Restored(restored) => {
                // Geliştirme kancası `OPENIFY_TEST_NO_RESUME=1`: kaldığı yer
                // gösterilmez (hiçbir şey yüklü değilken "Son çalınanlar"
                // hâlini sınamak için; motorun kaydına dokunulmaz).
                if self.now.fake
                    || self.state.track.is_some()
                    || std::env::var_os("OPENIFY_TEST_NO_RESUME").is_some()
                {
                    return;
                }
                log::info!(
                    "kaldığı yer: {} @ {} ms",
                    restored.track.uri,
                    restored.position_ms
                );
                let restored = *restored;
                let uri = restored.track.uri.clone();
                // Motor notlarına göre: şarkı değişti + sıra + duraklatıldı +
                // kapak. Çal'a basınca motor kaldığı konumdan yükler.
                self.on_engine_event(engine::Event::TrackChanged(restored.track), cx);
                self.on_engine_event(engine::Event::QueueChanged(restored.queue), cx);
                self.on_engine_event(
                    engine::Event::Paused {
                        position_ms: restored.position_ms,
                    },
                    cx,
                );
                if let Some((bytes, accent)) = restored.cover {
                    self.on_engine_event(
                        engine::Event::Cover {
                            track_uri: uri,
                            bytes,
                            accent,
                        },
                        cx,
                    );
                }
                self.set_queue(restored.view);
                cx.notify();
            }
            PlaybackEvent::Connect(status) => {
                // Geliştirme kancası `OPENIFY_TEST_CONNECT`: durum sabit kalır.
                if connect_fixture().is_some() {
                    return;
                }
                self.now.connect = status;
                cx.notify();
            }
        }
    }

    fn set_queue(&mut self, view: QueueView) {
        self.now.queue = view;
        self.now.queue_known = true;
        let len = self.now.visible_len();
        if let Some(c) = self.now.cursor
            && c >= len
        {
            self.now.cursor = len.checked_sub(1);
        }
    }

    /// `Shell::on_engine_event`'in başında (durum güncellenmeden önce).
    pub(super) fn on_now_event(&mut self, event: &engine::Event, cx: &mut Context<Self>) {
        match event {
            engine::Event::TrackChanged(info) => {
                self.now.clock_guard = Some(Instant::now());
                if self.now.lyrics_uri.as_deref() != Some(info.uri.as_str()) {
                    self.now.lyrics = LyricsStatus::None;
                    self.now.lyrics_uri = None;
                    self.now.side_scroll.reset();
                    self.now.full_scroll.reset();
                }
                let uri = info.uri.clone();
                // Sahte çalmada sıra arayüzde: çalan değişince kaydır.
                if self.now.fake && self.now.queue_known {
                    self.now.queue.current = Some(QueueEntry {
                        uri: uri.clone(),
                        track: None,
                        source: QueueSource::Context,
                    });
                }
                if self.now.lyrics_shown() {
                    self.request_lyrics(&uri);
                }
                // Tam ekran ya da "Şimdi çalıyor" sekmesi açıksa yeni şarkının
                // büyük kapağı (aynı albümse eskisi kalır, farklıysa eskisi
                // hemen bırakılır).
                self.request_hires(info.cover_url.clone(), cx);
            }
            engine::Event::Cover {
                track_uri, bytes, ..
            } => {
                self.compute_backdrop(track_uri.clone(), bytes.clone(), cx);
            }
            engine::Event::Lyrics { uri, lyrics }
                if self.state.track.as_ref().is_some_and(|t| &t.uri == uri) =>
            {
                self.now.lyrics = match lyrics {
                    Some(l) => LyricsStatus::Ready(l.clone()),
                    None => LyricsStatus::Missing,
                };
                self.now.lyrics_uri = Some(uri.clone());
                self.now.side_scroll.reset();
                self.now.full_scroll.reset();
                self.maybe_prefetch_next();
            }
            _ => {}
        }
    }

    /// `Shell::on_engine_event`'in sonunda (durum güncellendikten sonra).
    pub(super) fn after_now_event(&mut self, cx: &mut Context<Self>) {
        self.schedule_lyrics(cx);
    }

    /// Kullanıcı sardı (`Shell::seek_to`): söz satırı bir karede yerine
    /// gelsin.
    pub(super) fn on_now_seek(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        self.now.clock_guard = Some(now);
        self.now.seek_at = Some(now);
        self.now.side_scroll.user_until = None;
        self.now.full_scroll.user_until = None;
        self.notify_lyric_views(cx);
        self.schedule_lyrics(cx);
    }

    /// Çalan şarkının sözünü iste (sahte çalmada deneme sözü: Spotify'a
    /// hiçbir şey gitmez).
    pub(super) fn request_lyrics(&mut self, uri: &str) {
        if self.now.lyrics_uri.as_deref() == Some(uri) {
            return;
        }
        if self.now.fake {
            let duration = self.state.duration_ms().max(60_000);
            // Sınama kancası `OPENIFY_TEST_LYRICS_JSON` (bkz. `lyrics_view`): söz bir Spotify yanıtı dosyasından.
            self.now.lyrics = super::lyrics_view::fixture_lyrics()
                .unwrap_or_else(|| LyricsStatus::Ready(Arc::new(fake_lyrics(duration))));
            self.now.lyrics_uri = Some(uri.to_string());
            return;
        }
        if matches!(self.now.lyrics, LyricsStatus::Loading) {
            return;
        }
        self.now.lyrics = LyricsStatus::Loading;
        self.engine.send(Command::FetchLyrics(uri.to_string()));
    }

    /// Sözler görünürken sıradaki şarkının sözünü önceden önbelleğe al.
    fn maybe_prefetch_next(&mut self) {
        if self.now.fake || !self.now.lyrics_shown() {
            return;
        }
        let Some(next) = self.now.queue.upcoming.first().map(|e| e.uri.clone()) else {
            return;
        };
        if self.now.prefetched.as_deref() == Some(next.as_str()) {
            return;
        }
        if !matches!(
            self.now.lyrics,
            LyricsStatus::Ready(_) | LyricsStatus::Missing
        ) {
            return;
        }
        self.now.prefetched = Some(next.clone());
        self.engine.send(Command::PrefetchLyrics(next));
    }

    // ── Söz saati ───────────────────────────────────────────────────────

    /// Söz saati (ms, eksi olabilir) ve kaynağı. Çalarken DUYULAN konum;
    /// duraklatılmışken ya da bilinmiyorsa arayüzün konumu.
    pub(super) fn lyric_clock(&self, now: Instant) -> (i64, ClockSource) {
        let ui = self.state.position_ms(now) as i64;
        if !self.state.is_playing() || self.now.fake {
            return (ui, ClockSource::Ui);
        }
        match self.now.levels.audible_track_ms() {
            Some(heard) => {
                let guarded = self
                    .now
                    .clock_guard
                    .is_some_and(|t| t.elapsed() < CLOCK_GUARD);
                if guarded && (heard - ui).abs() > CLOCK_AGREE_MS {
                    (ui, ClockSource::Ui)
                } else {
                    (
                        heard.min(self.state.duration_ms() as i64),
                        ClockSource::Audible,
                    )
                }
            }
            None => (ui, ClockSource::Ui),
        }
    }

    /// Satır değişimine kadar bekleyen zamanlayıcıyı yeniden kur. Sözler
    /// görünmüyorsa, çalmıyorsa ya da pencere gizliyse zamanlayıcı yok.
    pub(super) fn schedule_lyrics(&mut self, cx: &mut Context<Self>) {
        self.now.lyric_timer = None;
        if !self.now.lyrics_shown() || !self.state.is_playing() || self.window_hidden() {
            return;
        }
        let LyricsStatus::Ready(lyrics) = &self.now.lyrics else {
            return;
        };
        if lyrics.sync != SyncKind::Line {
            return;
        }
        let (clock, _) = self.lyric_clock(Instant::now());
        let at = clock + LYRIC_LEAD_MS;
        let next = lyrics
            .lines
            .iter()
            .map(|l| l.start_ms as i64)
            .find(|&s| s > at);
        let Some(next) = next else { return };
        let wait = Duration::from_millis((next - at).clamp(1, 60_000) as u64);
        self.now.lyric_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |shell, cx| {
                shell.notify_lyric_views(cx);
                shell.schedule_lyrics(cx);
            });
        }));
    }

    /// Söz gösteren görünümleri yenile (yalnız onları).
    pub(super) fn notify_lyric_views(&self, cx: &mut Context<Self>) {
        if matches!(self.now.panel, Some(Panel::Lyrics | Panel::Now)) {
            self.views.side.update(cx, |_, cx| cx.notify());
        }
        if self.now.full.open {
            self.views.full.update(cx, |_, cx| cx.notify());
        }
    }

    // ── Kullanıcı eylemleri ─────────────────────────────────────────────

    /// Sıra komutu: sahte çalmada motora gitmez (motorun boş sırası
    /// arayüzdekini ezmesin).
    pub(super) fn queue_command(&self, command: Command) {
        if !self.now.fake {
            self.engine.send(command);
        }
    }

    pub(super) fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        let on = !self.now.queue.shuffle;
        self.now.queue.shuffle = on;
        self.queue_command(Command::SetShuffle(on));
        cx.notify();
    }

    /// Kapalı → hepsi → tek → kapalı (Spotify'daki sıra).
    pub(super) fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let next = match self.now.queue.repeat {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        self.now.queue.repeat = next;
        self.queue_command(Command::SetRepeat(next));
        cx.notify();
    }

    /// Paneli bu hâlde aç; zaten açıksa kapat. Geniş pencerede (yerleşik
    /// panel) kullanıcının açması / kapatması ayarlarda hatırlanır.
    pub(super) fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        if self.now.panel == Some(panel) {
            self.close_panel(cx);
            return;
        }
        self.show_panel(panel);
        self.remember_dock(true);
        self.after_view_change(cx);
    }

    /// Paneli bu sekmeyle göster (açılıyorsa kayarak gelir).
    pub(super) fn show_panel(&mut self, panel: Panel) {
        if self.now.panel.is_none() {
            self.now.panel_gen += 1;
            self.now.panel_at = Instant::now();
        }
        self.now.panel = Some(panel);
        // Arkadaş paneli geçici bir konuk: "kendiliğinden açılınca hangi sekme"
        // hâlâ sekmelerden biri kalır.
        if panel != Panel::Friends {
            self.now.dock.tab = panel;
        }
        if matches!(panel, Panel::Queue | Panel::Now) {
            self.ensure_fake_queue();
        }
    }

    /// Kullanıcı paneli kapattı (X, hap, kısayol, Esc).
    pub(super) fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.hide_panel();
        self.remember_dock(false);
        self.after_view_change(cx);
    }

    /// Paneli gizle (kullanıcının tercihi değişmeden: pencere daraldı).
    pub(super) fn hide_panel(&mut self) {
        self.now.panel = None;
        self.now.cursor = None;
        self.now.drop_at = None;
    }

    /// Geniş pencerede panelin açık / kapalı tercihi ayarlara yazılır (dar
    /// pencerenin yüzen paneli geçicidir, tercihi değiştirmez).
    fn remember_dock(&mut self, open: bool) {
        if self.now.dock.wide && self.now.dock.open != open {
            self.now.dock.open = open;
            self.save_settings();
        }
    }

    /// Panel ya da tam ekran açılıp kapandı: sözler gerekiyorsa iste,
    /// zamanlayıcıları yeniden kur.
    pub(super) fn after_view_change(&mut self, cx: &mut Context<Self>) {
        if self.now.lyrics_shown()
            && let Some(uri) = self.state.track.as_ref().map(|t| t.uri.clone())
        {
            self.request_lyrics(&uri);
            self.maybe_prefetch_next();
        }
        self.now.side_scroll.line = None;
        self.now.full_scroll.line = None;
        // Büyük kapak (640 px) yalnız tam ekran ya da "Şimdi çalıyor"
        // sekmesi ekrandayken bellekte.
        self.sync_hires(cx);
        self.schedule_lyrics(cx);
        // Gizlenen bir bölüm (liste, hap) yeniden görününce yeniden kurulsun.
        self.restart_ticker(cx);
        // Arkadaş paneli görünür oldu / olmadı: yenileme başlar ya da durur.
        self.sync_friends(cx);
        cx.notify();
    }

    /// Sahte çalmada (ölçüm, ekran sınaması) sıra beğenilenlerden kurulur.
    pub(super) fn ensure_fake_queue(&mut self) {
        if !self.now.fake || self.now.queue_known {
            return;
        }
        let Some(tracks) = self.library.tracks(&CollectionId::Liked).cloned() else {
            return;
        };
        let entry = |t: &engine::TrackRow, source| QueueEntry {
            uri: t.uri.clone(),
            track: Some(t.clone()),
            source,
        };
        let mut upcoming: Vec<QueueEntry> = tracks
            .iter()
            .skip(1)
            .take(50)
            .map(|t| entry(t, QueueSource::Context))
            .collect();
        // İki "sıraya eklenen" şarkı: kullanıcı sırası bölümü de görünsün.
        let user: Vec<QueueEntry> = tracks
            .iter()
            .rev()
            .take(2)
            .map(|t| entry(t, QueueSource::User))
            .collect();
        let user_len = user.len();
        upcoming.splice(0..0, user);
        self.now.queue = QueueView {
            current: tracks.first().map(|t| entry(t, QueueSource::Context)),
            context_remaining: upcoming.len() - user_len,
            upcoming,
            user_len,
            context: Some(CollectionId::Liked),
            shuffle: false,
            repeat: RepeatMode::Off,
            radio: None,
        };
        self.now.queue_known = true;
    }

    // ── Klavye ──────────────────────────────────────────────────────────

    /// Şimdi çalıyor kısayolları. İşlendiyse `true`.
    pub(super) fn now_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        if self.now.full.open {
            self.wake_full_controls(cx);
        }
        match (Shortcut::from_keystroke(ks), key) {
            (Some(Shortcut::FullScreen), _) => {
                self.toggle_fullscreen(window, cx);
                return true;
            }
            (_, "escape") if self.now.full.open => {
                self.exit_fullscreen(window, cx);
                return true;
            }
            // Yerleşik panel (geniş pencere) Esc ile kapanmaz: içeriğin
            // üstünde yüzmüyor, Esc başka işler (palet, menü, bul) içindir.
            // (Paneldeki satırın sağ tık menüsü açıksa Esc önce onu kapatır.)
            (_, "escape")
                if self.now.panel.is_some() && !self.now.docked() && !self.manage.modal() =>
            {
                self.close_panel(cx);
                window.focus(&self.focus);
                return true;
            }
            (Some(Shortcut::Shuffle), _) => {
                self.toggle_shuffle(cx);
                return true;
            }
            (Some(Shortcut::Repeat), _) => {
                self.cycle_repeat(cx);
                return true;
            }
            (Some(Shortcut::QueuePanel), _) => {
                self.toggle_panel(Panel::Queue, cx);
                return true;
            }
            (Some(Shortcut::LyricsPanel), _) => {
                self.toggle_panel(Panel::Lyrics, cx);
                return true;
            }
            _ => {}
        }
        // Arkadaş etkinliği: satır odaktayken ↑ / ↓ ve Shift+F10 (bkz. `friends`).
        if self.now.panel == Some(Panel::Friends) && self.friends_key(event, window, cx) {
            return true;
        }
        // Odaktaki yeni düğmeyi gpui kendisi tıklar (Enter / Boşluk): genel
        // kısayol aynı tuşta ikinci kez çalışmasın.
        let s = &self.now.stops;
        let on_stop = [&s.shuffle, &s.repeat, &s.mini, &s.queue, &s.lyrics]
            .iter()
            .any(|h| h.is_focused(window));
        if on_stop && (key == "enter" || key == "space") {
            return true;
        }
        // Sıra paneli odaktayken: ↑ / ↓ seç, Alt+↑ / ↓ taşı, Del çıkar,
        // Enter çal.
        if self.now.panel == Some(Panel::Queue) && self.now.focus.is_focused(window) {
            return self.queue_key(key, m.alt, cx);
        }
        false
    }

    fn queue_key(&mut self, key: &str, alt: bool, cx: &mut Context<Self>) -> bool {
        let len = self.now.visible_len();
        if len == 0 {
            return false;
        }
        let cursor = self.now.cursor;
        match key {
            "up" | "down" if alt => {
                let Some(from) = cursor else { return true };
                let to = if key == "up" {
                    from.checked_sub(1)
                } else {
                    (from + 1 < len).then_some(from + 1)
                };
                if let Some(to) = to {
                    self.move_queue_item(from, to, cx);
                    self.now.cursor = Some(to);
                    self.reveal_queue_row(to);
                }
                true
            }
            "up" | "down" => {
                let next = match (cursor, key) {
                    (None, _) => 0,
                    (Some(c), "up") => c.saturating_sub(1),
                    (Some(c), _) => (c + 1).min(len - 1),
                };
                self.now.cursor = Some(next);
                self.reveal_queue_row(next);
                self.views.side.update(cx, |_, cx| cx.notify());
                true
            }
            "delete" | "backspace" => {
                if let Some(c) = cursor {
                    self.remove_queue_item(c, cx);
                    let len = self.now.visible_len();
                    self.now.cursor = if len == 0 { None } else { Some(c.min(len - 1)) };
                }
                true
            }
            "enter" => {
                if let Some(c) = cursor {
                    self.play_queue_item(c, cx);
                }
                true
            }
            _ => false,
        }
    }

    pub(super) fn move_queue_item(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to {
            return;
        }
        apply_move(&mut self.now.queue, from, to);
        self.queue_command(Command::MoveInQueue(from, to));
        cx.notify();
    }

    pub(super) fn remove_queue_item(&mut self, i: usize, cx: &mut Context<Self>) {
        apply_remove(&mut self.now.queue, i);
        self.queue_command(Command::RemoveFromQueue(i));
        cx.notify();
    }

    pub(super) fn clear_user_queue(&mut self, cx: &mut Context<Self>) {
        let n = self.now.queue.user_len.min(self.now.queue.upcoming.len());
        self.now.queue.upcoming.drain(..n);
        self.now.queue.user_len = 0;
        self.now.cursor = None;
        self.queue_command(Command::ClearQueue);
        cx.notify();
    }

    pub(super) fn play_queue_item(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.now.fake {
            return;
        }
        if !self.ready_to_play(cx) {
            return;
        }
        if let Some(e) = self.now.queue.upcoming.get(i)
            && let Some(t) = &e.track
        {
            self.state.pending = Some(super::player::PendingTrack {
                name: t.name.clone(),
                image: t.image.clone(),
            });
        }
        self.engine.send(Command::PlayQueueItem(i));
        cx.notify();
    }
}

/// Söz saatinin kaynağı (iz kaydı için).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ClockSource {
    Audible,
    Ui,
}

/// Sahte çalmada gösterilen deneme sözü: 2,5–4,5 sn arayla satırlar, bir
/// enstrümantal ara. Gerçek sözle aynı çizim yolundan geçer.
fn fake_lyrics(duration_ms: u32) -> Lyrics {
    const WORDS: [&str; 8] = [
        "Gece uzun, şehir sessiz, ışıklar uzakta",
        "Bir şarkı çalıyor eski bir radyoda",
        "Sözler akıyor satır satır, yavaşça",
        "Kapak tek ışık, oda karanlıkta",
        "",
        "Sürükle bırak, sıra değişsin bir karede",
        "Duyduğun an ile yazı aynı yerde",
        "Ve yeniden başa, dinlerken derinden",
    ];
    let mut lines = Vec::new();
    let mut t = 4_000u32;
    let mut i = 0usize;
    while t + 3_000 < duration_ms {
        lines.push(LyricLine {
            start_ms: t,
            end_ms: None,
            text: WORDS[i % WORDS.len()].to_string(),
            alts: Vec::new(),
        });
        t += 2_500 + ((i * 7919) % 5) as u32 * 500;
        i += 1;
    }
    Lyrics {
        source: engine::LyricsSource::Lrclib,
        sync: SyncKind::Line,
        lines,
        provider: Some("Openify deneme".into()),
        translations: Vec::new(),
    }
}

// ── Görünümler ──────────────────────────────────────────────────────────

/// Kare hızı sınırlı hareket: bir sonraki çizim en erken `1 / ANIM_FPS`
/// sonra (ekranın yenilenmesine hizalı; bkz. `views::wait_for_tick`).
fn pace<V: Paced>(window: &Window, view: WeakEntity<V>, since: Instant) {
    let due = Duration::from_secs_f32(0.85 / ANIM_FPS);
    window.on_next_frame(move |window, cx| {
        if since.elapsed() >= due {
            let _ = view.update(cx, |v, cx| {
                v.set_waiting(false);
                cx.notify();
            });
        } else {
            pace(window, view, since);
        }
    });
}

pub(super) trait Paced: Render + 'static {
    fn set_waiting(&mut self, waiting: bool);
}

/// Parçayı `Shell` adına çizer; parça hareket sürüyor derse (`true`) bir
/// sonraki kare istenir (en çok `ANIM_FPS`).
fn render_paced<V: Paced>(
    shell: &WeakEntity<Shell>,
    waiting: bool,
    window: &mut Window,
    cx: &mut Context<V>,
    part: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) -> (AnyElement, bool),
) -> (AnyElement, bool) {
    let result = shell.update(cx, |shell, cx| {
        let (element, animating) = part(shell, window, cx);
        shell.flush_image_requests(cx);
        let fading = shell.images.take_fading();
        (element, animating || fading)
    });
    match result {
        Ok((element, animating)) => {
            let element = div()
                .size_full()
                .flex()
                .flex_col()
                .child(element)
                .into_any_element();
            if animating && !waiting {
                pace(window, cx.entity().downgrade(), Instant::now());
                return (element, true);
            }
            (element, waiting)
        }
        Err(_) => (div().into_any_element(), waiting),
    }
}

/// Sağdaki yüzen panel: sıra ya da sözler.
pub struct SideView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
    waiting: bool,
}

impl SideView {
    pub fn new(shell: &Entity<Shell>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| SideView {
            shell: shell.downgrade(),
            _follow: cx.observe(shell, |_, _, cx| cx.notify()),
            waiting: false,
        })
    }
}

impl Paced for SideView {
    fn set_waiting(&mut self, waiting: bool) {
        self.waiting = waiting;
    }
}

impl Render for SideView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (element, waiting) = render_paced(
            &self.shell,
            self.waiting,
            window,
            cx,
            |shell, window, cx| shell.render_side_panel(window, cx),
        );
        self.waiting = waiting;
        element
    }
}

/// Tam ekran.
pub struct FullView {
    shell: WeakEntity<Shell>,
    _follow: Subscription,
    waiting: bool,
}

impl FullView {
    pub fn new(shell: &Entity<Shell>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| FullView {
            shell: shell.downgrade(),
            _follow: cx.observe(shell, |_, _, cx| cx.notify()),
            waiting: false,
        })
    }
}

impl Paced for FullView {
    fn set_waiting(&mut self, waiting: bool) {
        self.waiting = waiting;
    }
}

impl Render for FullView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (element, waiting) = render_paced(
            &self.shell,
            self.waiting,
            window,
            cx,
            |shell, window, cx| shell.render_fullscreen(window, cx),
        );
        self.waiting = waiting;
        element
    }
}

// ── Söz izi (OPENIFY_LYRICS_TRACE) ──────────────────────────────────────

/// Geliştirme kancası `OPENIFY_LYRICS_TRACE=<csv>`: her etkin satır değişimi
/// BOYANDIĞI anda bir satır: `t_ms,view,line,start_ms,clock_ms,late_ms,source,since_seek_ms`.
/// `late_ms` = boyama anındaki söz saati - satırın başlangıcı (artı: geç).
/// Sarmadan sonraki ilk boyamada `since_seek_ms` sarmadan boyamaya geçen süre.
pub(super) mod trace {
    use super::*;

    fn file() -> Option<&'static Mutex<BufWriter<File>>> {
        static FILE: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();
        FILE.get_or_init(|| {
            let path = std::env::var_os("OPENIFY_LYRICS_TRACE")?;
            let mut f = BufWriter::new(File::create(path).ok()?);
            let _ = writeln!(
                f,
                "t_ms,view,line,start_ms,clock_ms,late_ms,source,since_seek_ms"
            );
            Some(Mutex::new(f))
        })
        .as_ref()
    }

    pub fn enabled() -> bool {
        file().is_some()
    }

    pub struct Row {
        pub view: &'static str,
        pub line: usize,
        pub start_ms: u32,
        pub clock_ms: i64,
        pub source: ClockSource,
        pub since_seek_ms: Option<f32>,
    }

    /// Bir olayın boyanması (`view`: olayın adı; son sütun olaydan boyamaya ms).
    pub fn event(view: &'static str, since_ms: f32) {
        write(Row {
            view,
            line: 0,
            start_ms: 0,
            clock_ms: 0,
            source: ClockSource::Ui,
            since_seek_ms: Some(since_ms),
        });
    }

    pub fn write(row: Row) {
        let Some(f) = file() else { return };
        static START: OnceLock<Instant> = OnceLock::new();
        let t = START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.;
        if let Ok(mut f) = f.lock() {
            let _ = writeln!(
                f,
                "{t:.1},{},{},{},{},{},{:?},{}",
                row.view,
                row.line,
                row.start_ms,
                row.clock_ms,
                row.clock_ms - row.start_ms as i64,
                row.source,
                row.since_seek_ms
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_default()
            );
            let _ = f.flush();
        }
    }
}

/// Söz saatinin boyama anında yeniden okunabilmesi için anlık kopyası (iz).
#[derive(Clone)]
pub(super) struct ClockProbe {
    pub ui_anchor_ms: i64,
    pub ui_anchor_at: Instant,
    pub playing: bool,
    pub audible: bool,
    pub levels: AudioLevels,
    pub duration_ms: i64,
}

impl ClockProbe {
    pub fn read(&self) -> (i64, ClockSource) {
        let ui = if self.playing {
            self.ui_anchor_ms + self.ui_anchor_at.elapsed().as_millis() as i64
        } else {
            self.ui_anchor_ms
        };
        if self.audible
            && let Some(h) = self.levels.audible_track_ms()
        {
            return (h.min(self.duration_ms), ClockSource::Audible);
        }
        (ui.min(self.duration_ms.max(0)), ClockSource::Ui)
    }
}

impl Shell {
    pub(super) fn clock_probe(&self, now: Instant) -> ClockProbe {
        let (_, source) = self.lyric_clock(now);
        ClockProbe {
            ui_anchor_ms: self.state.position_ms(now) as i64,
            ui_anchor_at: now,
            playing: self.state.is_playing(),
            audible: source == ClockSource::Audible,
            levels: self.now.levels.clone(),
            duration_ms: self.state.duration_ms() as i64,
        }
    }
}

/// Geliştirme kancası `OPENIFY_TEST_VIEW=now|queue|lyrics|fullscreen|home|
/// search:<sorgu>|artist:<uri>|album:<uri>`: açılışta bu görünüm ya da sayfa
/// açık gelir (ekran sınaması ve kare ölçümü için).
pub(super) fn test_view() -> Option<&'static str> {
    static VIEW: OnceLock<Option<String>> = OnceLock::new();
    VIEW.get_or_init(|| std::env::var("OPENIFY_TEST_VIEW").ok())
        .as_deref()
}

impl Shell {
    /// `OPENIFY_TEST_VIEW` (bkz. `test_view`). Sahte çalmada çalan şarkıya
    /// diskteki küçük kapağı verir: tam ekranın arka planı ve ışık gerçek bir
    /// kapakla ölçülsün (Spotify'a hiçbir şey gitmez).
    pub(super) fn apply_test_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = test_view() else { return };
        if self.now.fake
            && let Some(t) = self
                .library
                .tracks(&CollectionId::Liked)
                .and_then(|t| t.get(super::fake_row()))
                .cloned()
            && self.state.track.as_ref().is_some_and(|c| c.uri == t.uri)
            && let Some(bytes) = engine::cover_path(&t.image).and_then(|p| std::fs::read(p).ok())
        {
            let accent = engine::palette::accent_from_image(&bytes);
            self.on_engine_event(
                engine::Event::Cover {
                    track_uri: t.uri.clone(),
                    bytes,
                    accent,
                },
                cx,
            );
        }
        // Sayfalar: "home", "explore", "library", "recent", "settings", "category:<sayfa kimliği>", "search:<sorgu>", "artist:<uri>", "album:<uri>"
        // (arama sayfası paletten Ctrl+Enter ister, sınamada iletiyle
        // gönderilemiyor; yanlış adres hata hâlini gösterir).
        // `palette` / `palette:<sorgu>`: arama yüzeyi açık gelir, sorgu yazılmış
        // (klavye iletileri yazıyı kimi zaman yutuyor; ekran görüntüsü için
        // kesin yol). `OPENIFY_TEST_VIEW_AFTER` saniye sonra, varsayılan 3.
        if view == "palette" || view.starts_with("palette:") {
            let query = view
                .strip_prefix("palette:")
                .unwrap_or_default()
                .to_string();
            let after = std::env::var("OPENIFY_TEST_VIEW_AFTER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(after))
                    .await;
                let _ = this.update(cx, |shell, cx| {
                    shell.open_palette(cx);
                    if !query.is_empty() {
                        shell.palette.query = query;
                        shell.on_query_changed(cx);
                    }
                });
            })
            .detach();
            return;
        }
        use super::history::Route;
        let route = match view.split_once(':') {
            Some(("search", q)) => Some(Route::Search(q.to_string())),
            Some(("artist", uri)) => Some(Route::Artist(uri.to_string())),
            Some(("album", uri)) => Some(Route::Album(uri.to_string())),
            Some(("category", id)) => Some(Route::Category(format!("spotify:page:{id}"))),
            _ if view == "home" => Some(Route::Home),
            _ if view == "explore" => Some(Route::Explore),
            _ if view == "library" => Some(Route::Library),
            _ if view == "settings" => Some(Route::Settings),
            _ if view == "recent" => Some(Route::Recent),
            _ if view == "tools" => Some(Route::Tools),
            _ => None,
        };
        if let Some(route) = route {
            // Bağlantı kurulduktan sonra (açılışta sayfa isteği bağlantıdan
            // önce gidip "ulaşılamadı" dönüyordu); `OPENIFY_TEST_VIEW_AFTER`
            // saniye, varsayılan 3. 0 hemen açar (hata hâlini görmek için).
            let after = std::env::var("OPENIFY_TEST_VIEW_AFTER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(after))
                    .await;
                let _ = this.update(cx, |shell, cx| shell.navigate(route, cx));
            })
            .detach();
            return;
        }
        match view {
            "queue" => self.toggle_panel(Panel::Queue, cx),
            "lyrics" => self.toggle_panel(Panel::Lyrics, cx),
            "now" => self.toggle_panel(Panel::Now, cx),
            "friends" => self.toggle_friends(cx),
            "fullscreen" => self.toggle_fullscreen(window, cx),
            "help" => self.open_help(Some(window), cx),
            "credits" => self.apply_credits_test_view(window, cx),
            _ => {}
        }
    }
}
