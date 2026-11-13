//! Tam ekran "şimdi çalıyor" (Adım 14): duvar ekranı gibi sakin.
//!
//! Kapak odadaki tek aydınlık nesne; arkasında aynı kapağın bulanık, kısık
//! hâli bütün pencereyi kaplar. gpui'de bulanıklaştırma yok: kapak ayrı bir
//! iş parçacığında 32 px'e küçültülür, bulanıklaştırılır, 128 px'e
//! büyütülüp bir kez daha yumuşatılır, karartılır (en parlak yeri göreli
//! parlaklık 0,025'i, ışımanın tavanını, geçmez: üstündeki sessiz yazı her kapakta ≥ 4,5:1) ve
//! PNG olarak bir kez hazırlanır. Açık temada aynı kapak sıcak açık zeminle
//! düşük yoğunlukta karışır; koyu yazı için parlaklık tabanı korunur.
//! Ekranda gpui onu çift doğrusal büyütür. Her
//! şarkıda bir kez, kare başına iş yok; eskisi gpui'nin önbelleğinden atılır.
//!
//! Kapak: motor kapağı 256 px'e küçültüp saklıyor (kenar çubuğu ve hap için
//! yeter, büyük ekranda yumuşak). Tam ekran ya da paneldeki "Şimdi çalıyor"
//! sekmesi ekrandayken yalnız çalan şarkı için kapağın en büyük hâli
//! (Spotify'ın 640 px'lik görseli) ayrıca indirilir ve küçük kapağın üstüne
//! belirir; ikisi de kapanınca, pencere tepsiye gizlenince (ya da şarkının
//! kapağı değişince) bellekten bırakılır. Diske yazılmaz, motorun önbelleğine
//! dokunmaz. Resim arka planda çözülüp doğrudan `RenderImage` olarak verilir:
//! bırakırken gpui'nin doku atlasından da elle atılır (`App::drop_image`;
//! `Image::remove_asset` atlası boşaltmıyor, her açılışta ~4 MB birikiyordu).
//!
//! Solda kapak ve şarkı, sağda sözler; altta denetimler. Fare ya da klavye
//! 3 sn dokunulmayınca denetimler söner, dokununca belirir (220 ms). Dururken
//! hiç çizim yok; çalarken yalnız söz satırı değişince ve denetimler
//! görünürken ilerleme değişince.
//!
//! F11 ya da kapak / paneldeki düğme açar; Esc ya da F11 kapatır. Pencere
//! de Windows'un tam ekranına geçer (sınama koşusunda değil).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, AnyView, Context, FontWeight, ImageFormat, ObjectFit, RenderImage, StyleRefinement,
    Task, Window, div, img, linear_color_stop, linear_gradient, prelude::*, px, rgb,
};
use image::{ImageEncoder, imageops::FilterType};

use super::{
    Drag, Shell,
    components::{Look, Tooltip, icon_button, play_button},
    lyrics_view::Place,
    now_playing::{LyricsStatus, Panel},
    pill::{fraction, slider, toggle_button},
    player::fmt_time,
    text::line_with_tip,
    theme::*,
};
use crate::engine::RepeatMode;
use crate::t;

/// Denetimler bu kadar dokunulmayınca söner.
const IDLE: Duration = Duration::from_secs(3);
/// Denetimlerin kapladığı yükseklik (alttan): ilerleme, düğmeler, boşluk.
const CONTROLS_HEIGHT: f32 = 136.;
/// Kapağın ve sözlerin en büyük hâli.
const COVER_MAX: f32 = 480.;
const LYRICS_MAX: f32 = 640.;
/// Bu genişlikten (mantıksal px) geniş pencerede (1920 ve üstü) kompozisyon
/// büyür: kapak 640, söz sütunu 860 px, sözler Display (40 px). 2560'ta 480 px
/// kapak ve 28 px yazı ekranın ortasında küçük bir ada kalıyordu (ekran sınaması).
pub(super) const LARGE_FROM: f32 = 1800.;
const COVER_MAX_LARGE: f32 = 640.;
const LYRICS_MAX_LARGE: f32 = 860.;
/// Bu genişliğin altında sözler kapağın yanına sığmaz: yalnız kapak.
const SIDE_BY_SIDE_MIN: f32 = 900.;
/// Arka planın en parlak yeri (göreli parlaklık).
pub(super) const BACKDROP_MAX_LUMINANCE: f32 = 0.025;

/// Her iş tek bir tema ve kapak kimliğine aittir. Sıra sayısı aynı temaya
/// dönülen hızlı açık → koyu → açık geçişinde de eski işi ayırt eder.
struct SurfaceRequest {
    seq: u64,
    light: bool,
    uri: String,
}

impl SurfaceRequest {
    fn accepts(&self, seq: u64, light: bool, uri: Option<&str>) -> bool {
        self.seq == seq && self.light == light && uri == Some(self.uri.as_str())
    }
}

pub(super) struct FullState {
    pub open: bool,
    /// Pencereyi Windows'un tam ekranına biz aldık (çıkınca geri).
    os: bool,
    pub show_lyrics: bool,
    /// Denetimler görünür mü; son değişimin anı (belirme / sönme hareketi).
    pub controls: bool,
    pub controls_at: Instant,
    /// Açılış anı (belirme hareketi) ve kapanış anı (sönme hareketi, bitince
    /// `None`): tam ekran açılırken `motion` ile belirir, kapanırken
    /// `motion_exit` ile söner (7 Kasım akşam).
    pub opened_at: Instant,
    pub closing: Option<Instant>,
    idle_deadline: Instant,
    pub hovering_controls: bool,
    hide_task: Option<Task<()>>,
    /// Çalan şarkının bulanık arka planı (şarkının adresiyle).
    pub backdrop: Option<(String, Arc<gpui::Image>)>,
    /// Küçük kapağın baytları: tema değişiminde ağa çıkmadan yeniden üretim.
    surface_source: Option<(String, Arc<[u8]>)>,
    surface_seq: u64,
    /// Hapın kapak ışığı: aynı kapaktan, aynı iş parçacığında (`pill_glass`).
    pub pill_light: super::pill_glass::Lights,
    /// Çalan şarkının kapağının büyük hâli; yalnız tam ekran ya da panelin
    /// "Şimdi çalıyor" sekmesi ekrandayken bellekte (bkz. `hires_wanted`).
    hires: Option<Hires>,
    /// İndirilmekte olan büyük kapağın kaynağı ve görevi (düşürülürse iptal).
    hires_pending: Option<(String, Task<()>)>,
    /// Pencere `LARGE_FROM`'dan geniş mi (son çizimde): sözlerin yazı boyu buna göre.
    pub large: bool,
}

/// Tam ekranın büyük kapağı.
struct Hires {
    /// Kaynak (kapak adresi); çalan şarkının kapağıyla aynıysa geçerli.
    source: String,
    image: Arc<RenderImage>,
    /// Geldiği an: küçük kapaktan açılma geçişi için.
    at: Instant,
}

impl FullState {
    pub(super) fn new() -> Self {
        let now = Instant::now();
        Self {
            open: false,
            os: false,
            show_lyrics: true,
            controls: true,
            controls_at: now,
            opened_at: now,
            closing: None,
            idle_deadline: now,
            hovering_controls: false,
            hide_task: None,
            backdrop: None,
            surface_source: None,
            surface_seq: 0,
            pill_light: Default::default(),
            hires: None,
            hires_pending: None,
            large: false,
        }
    }
}

impl Shell {
    pub(super) fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.now.full.open {
            self.exit_fullscreen(window, cx);
            return;
        }
        self.now.full.open = true;
        self.now.full.opened_at = Instant::now();
        self.now.full.closing = None;
        self.now.full_scroll.reset();
        // Sınama koşusunda pencere ölçülen boyunda kalır (2. ekranı kaplamasın).
        let test = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
            && std::env::var_os("OPENIFY_TEST_OS_FULLSCREEN").is_none();
        if !test && !window.is_fullscreen() {
            window.toggle_fullscreen();
            self.now.full.os = true;
        }
        window.focus(&self.focus);
        self.keyboard_focus = false;
        self.now.full.controls = false;
        self.wake_full_controls(cx);
        // Büyük kapak `after_view_change` → `sync_hires` ile istenir.
        self.after_view_change(cx);
    }

    pub(super) fn exit_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.now.full.open = false;
        self.now.full.closing = animations_enabled().then(Instant::now);
        self.now.full.hide_task = None;
        // Panelin "Şimdi çalıyor" sekmesi de göstermiyorsa büyük kapak hemen
        // bırakılır (gösteriyorsa kalır, yeniden indirilmez).
        if !self.hires_wanted() {
            self.release_hires(Some(window), cx);
        }
        if std::mem::take(&mut self.now.full.os) && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.after_view_change(cx);
    }

    /// Fare / klavye: denetimler görünsün, 3 sn sonra sönsün. Yalnız
    /// görünürlük değişince çizilir.
    pub(super) fn wake_full_controls(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        self.now.full.idle_deadline = now + IDLE;
        if !self.now.full.controls {
            self.now.full.controls = true;
            self.now.full.controls_at = now;
            self.views.full.update(cx, |_, cx| cx.notify());
        }
        if self.now.full.hide_task.is_none() {
            self.now.full.hide_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    let wait = this
                        .update(cx, |shell, _| {
                            shell
                                .now
                                .full
                                .idle_deadline
                                .saturating_duration_since(Instant::now())
                        })
                        .unwrap_or_default();
                    cx.background_executor()
                        .timer(wait + Duration::from_millis(5))
                        .await;
                    let done = this
                        .update(cx, |shell, cx| {
                            let full = &mut shell.now.full;
                            if Instant::now() < full.idle_deadline {
                                return false;
                            }
                            if full.hovering_controls {
                                full.idle_deadline = Instant::now() + IDLE;
                                return false;
                            }
                            full.controls = false;
                            full.controls_at = Instant::now();
                            full.hide_task = None;
                            shell.views.full.update(cx, |_, cx| cx.notify());
                            true
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                }
            }));
        }
    }

    /// Büyük kapağın kaynağı: gerçek çalmada şarkının (en büyük) kapak adresi;
    /// sahte çalmada (adres yok) `OPENIFY_TEST_HIRES_FILE` ile verilen dosya.
    fn hires_source(&self, cover_url: Option<String>) -> Option<String> {
        // Geliştirme kancası: büyük kapak kapalı (A/B karşılaştırma, bellek).
        if std::env::var_os("OPENIFY_TEST_NO_HIRES").is_some() {
            return None;
        }
        if self.now.fake {
            return std::env::var("OPENIFY_TEST_HIRES_FILE")
                .ok()
                .filter(|p| !p.is_empty());
        }
        cover_url.filter(|u| !u.is_empty())
    }

    /// Büyük kapak gerekiyor mu: tam ekran ya da panelin "Şimdi çalıyor"
    /// sekmesi ekranda (yüzen ya da yerleşik) ve pencere tepsiye gizli değil.
    /// Gizli pencerenin bellek çıtası (< 75 MB) için gizlenince bırakılır.
    pub(super) fn hires_wanted(&self) -> bool {
        !self.window_hidden() && (self.now.full.open || self.now.panel == Some(Panel::Now))
    }

    /// Büyük kapağı gerekene göre iste ya da bırak: tam ekran / panel açılıp
    /// kapanınca, sekme değişince, pencere gizlenip görününce.
    pub(super) fn sync_hires(&mut self, cx: &mut Context<Self>) {
        let cover_url = self.state.track.as_ref().and_then(|t| t.cover_url.clone());
        self.request_hires(cover_url, cx);
    }

    /// Büyük kapak gerekiyorsa çalan şarkının kapağının büyük hâlini iste
    /// (şarkı değişince de çağrılır), gerekmiyorsa bırak. Aynı kapak (aynı
    /// albüm) zaten varsa ya da yoldaysa bir şey yapmaz; başka kapaksa eskisini
    /// hemen bırakır.
    pub(super) fn request_hires(&mut self, cover_url: Option<String>, cx: &mut Context<Self>) {
        if !self.hires_wanted() {
            self.release_hires(None, cx);
            return;
        }
        let Some(source) = self.hires_source(cover_url) else {
            self.release_hires(None, cx);
            return;
        };
        if self
            .now
            .full
            .hires
            .as_ref()
            .is_some_and(|h| h.source == source)
        {
            return;
        }
        if self
            .now
            .full
            .hires_pending
            .as_ref()
            .is_some_and(|(s, _)| *s == source)
        {
            return;
        }
        self.release_hires(None, cx);
        let fake = self.now.fake;
        let fetch = {
            let source = source.clone();
            cx.background_spawn(async move { fetch_hires(&source, fake) })
        };
        let wanted = source.clone();
        let task = cx.spawn(async move |this, cx| {
            let decoded = fetch.await;
            let _ = this.update(cx, |shell, cx| {
                shell.now.full.hires_pending = None;
                let Some(image) = decoded else { return };
                // Bu arada şarkı (kapak) değiştiyse ya da tam ekran / panel
                // kapandıysa at (hiç çizilmedi: doku atlasında yok).
                let cover_url = shell.state.track.as_ref().and_then(|t| t.cover_url.clone());
                if !shell.hires_wanted()
                    || shell.hires_source(cover_url).as_deref() != Some(wanted.as_str())
                {
                    return;
                }
                shell.now.full.hires = Some(Hires {
                    source: wanted,
                    image,
                    at: Instant::now(),
                });
                shell.views.full.update(cx, |_, cx| cx.notify());
                shell.views.side.update(cx, |_, cx| cx.notify());
            });
        });
        self.now.full.hires_pending = Some((source, task));
    }

    /// Büyük kapağı bırak: yoldaki indirme iptal, çözülmüş resim gpui'nin
    /// varlık önbelleğinden ve doku atlasından atılır (bellek geri döner).
    /// `window`: çağıran bir pencere güncellemesinin içindeyse o pencere
    /// (gpui onu ayrıca ister). Verilmezse atma işi o güncelleme bitince
    /// yapılır (`App::defer`): pencere güncellemesinin içinde `None` ile
    /// atılsaydı o pencerenin atlası dokuyu tutmaya devam ederdi.
    pub(super) fn release_hires(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.now.full.hires_pending = None;
        let Some(old) = self.now.full.hires.take() else {
            return;
        };
        match window {
            Some(window) => cx.drop_image(old.image, Some(window)),
            None => cx.defer(move |cx| cx.drop_image(old.image, None)),
        }
        // Önbellekli görünümler eski kareyi (atılan dokuyla) tekrar oynatmasın.
        self.views.full.update(cx, |_, cx| cx.notify());
        self.views.side.update(cx, |_, cx| cx.notify());
    }

    /// Çalan şarkının büyük kapağı (geldiyse ve hâlâ bu şarkınınsa) ve küçük
    /// kapağın üstünde ne kadar belirdiği (0–1, kapak geçişi süresince;
    /// "animasyonları azalt"ta hemen 1). Tam ekran ve panel aynısını çizer.
    pub(super) fn hires_cover(&self) -> Option<(Arc<RenderImage>, f32)> {
        let cover_url = self.state.track.as_ref().and_then(|t| t.cover_url.clone());
        let wanted = self.hires_source(cover_url)?;
        let h = self
            .now
            .full
            .hires
            .as_ref()
            .filter(|h| h.source == wanted)?;
        let t = if animations_enabled() {
            (h.at.elapsed().as_secs_f32() / cover_fade().as_secs_f32()).min(1.)
        } else {
            1.
        };
        Some((h.image.clone(), ease_out(t)))
    }

    /// Büyük kapağın belirmesi sürüyor mu (görünüm bir sonraki kareyi istesin).
    pub(super) fn hires_fading(&self) -> bool {
        self.hires_cover().is_some_and(|(_, shown)| shown < 1.)
    }

    /// Kök için: tam ekranın önbellekli görünümü.
    pub(super) fn fullscreen_layer(&self) -> AnyElement {
        AnyView::from(self.views.full.clone())
            .cached(
                StyleRefinement::default()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .into_any_element()
    }

    /// Kapak geldi: bulanık arka planı ve hapın ışığını arka planda hazırla
    /// (her şarkıda bir kez).
    pub(super) fn compute_backdrop(&mut self, uri: String, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if self.state.track.as_ref().is_none_or(|t| t.uri != uri) {
            return;
        }
        if self
            .now
            .full
            .backdrop
            .as_ref()
            .is_some_and(|(u, _)| *u == uri)
        {
            return;
        }
        self.start_theme_surfaces(uri, bytes.into(), cx);
    }

    /// Eski tema dokularını bırak; sahte çalma dahil mevcut kapaktan yeniden üret.
    pub(super) fn refresh_theme_surfaces(&mut self, cx: &mut Context<Self>) {
        self.now.full.surface_seq = self.now.full.surface_seq.wrapping_add(1);
        if let Some((_, old)) = self.now.full.backdrop.take() {
            old.remove_asset(cx);
        }
        self.clear_pill_lights(cx);
        if let Some((uri, bytes)) = self.now.full.surface_source.clone()
            && self.state.track.as_ref().is_some_and(|t| t.uri == uri)
        {
            self.start_theme_surfaces(uri, bytes, cx);
        }
        self.views.full.update(cx, |_, cx| cx.notify());
    }

    fn start_theme_surfaces(&mut self, uri: String, bytes: Arc<[u8]>, cx: &mut Context<Self>) {
        let colors = palette();
        self.now.full.surface_seq = self.now.full.surface_seq.wrapping_add(1);
        let request = SurfaceRequest {
            seq: self.now.full.surface_seq,
            light: colors.light,
            uri: uri.clone(),
        };
        self.now.full.surface_source = Some((uri.clone(), bytes.clone()));
        let task = cx.background_spawn(async move {
            (
                backdrop_png(&bytes, colors),
                super::pill_glass::light_image(&bytes, colors),
            )
        });
        cx.spawn(async move |this, cx| {
            let (png, light) = task.await;
            let _ = this.update(cx, |shell, cx| {
                if !request.accepts(
                    shell.now.full.surface_seq,
                    palette().light,
                    shell.state.track.as_ref().map(|t| t.uri.as_str()),
                ) {
                    // A theme change without a newer request must still refresh.
                    if request.seq == shell.now.full.surface_seq && request.light != palette().light
                    {
                        shell.refresh_theme_surfaces(cx);
                    }
                    return;
                }
                if let Some(light) = light {
                    shell.set_pill_light(light, cx);
                }
                let Some(png) = png else { return };
                let image = Arc::new(gpui::Image::from_bytes(ImageFormat::Png, png));
                if let Some((_, old)) = shell.now.full.backdrop.replace((uri, image)) {
                    old.remove_asset(cx);
                }
                if shell.now.full.open {
                    shell.views.full.update(cx, |_, cx| cx.notify());
                }
            });
        })
        .detach();
    }

    /// Tam ekran (`FullView`). Hareket sürüyorsa `true`.
    pub(super) fn render_fullscreen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        // Kapanırken sönme hareketi (açıkken belirme): saydamlık tek sayı.
        let fade = if self.now.full.open {
            let t = self.now.full.opened_at.elapsed().as_secs_f32() / motion().as_secs_f32();
            (t < 1.).then(|| quint_out(t))
        } else if let Some(at) = self.now.full.closing {
            let t = at.elapsed().as_secs_f32() / motion_exit().as_secs_f32();
            if t >= 1. {
                // Bitti: kök katmanı kaldırsın.
                self.now.full.closing = None;
                cx.notify();
                return (div().into_any_element(), false);
            }
            Some(1. - cubic_in(t))
        } else {
            return (div().into_any_element(), false);
        };
        let viewport = window.viewport_size();
        let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
        let large = w >= LARGE_FROM;
        self.now.full.large = large;
        let (cover_max, lyrics_max) = if large {
            (COVER_MAX_LARGE, LYRICS_MAX_LARGE)
        } else {
            (COVER_MAX, LYRICS_MAX)
        };
        let accent = self.accent();
        let mut animating = fade.is_some();

        // Arka plan: kapağın bulanık hâli, yoksa kapak ışımasının büyüğü.
        let uri = self.state.track.as_ref().map(|t| t.uri.clone());
        let backdrop = match &self.now.full.backdrop {
            Some((u, image)) if Some(u) == uri.as_ref() => img(image.clone())
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            _ => div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(rgb(glow_top(accent.glow)), 0.),
                    linear_color_stop(rgb(palette().bg), 1.),
                ))
                .into_any_element(),
        };

        // Sözler kapağın yanında mı?
        let lyrics_known =
            !matches!(self.now.lyrics, LyricsStatus::Missing) && self.state.track.is_some();
        let side = self.now.full.show_lyrics && lyrics_known && w >= SIDE_BY_SIDE_MIN;
        let title_h = T_HEADLINE.line + S1 + T_TITLE.line;
        let free_h = h - S7 - CONTROLS_HEIGHT - S5 - title_h;
        let cover = if side {
            free_h.min(w * 0.34).min(cover_max)
        } else {
            free_h.min(cover_max + S7)
        }
        .max(S8 * 2.);
        // Büyük kapak (geldiyse) küçüğün üstünde; küçük altta kalır, büyük
        // çözülene dek boş kare olmaz. Gelince kapak geçişi süresince belirir.
        let hires = self.hires_cover();
        animating |= hires.as_ref().is_some_and(|(_, shown)| *shown < 1.);
        let art = match self.state.cover.clone() {
            Some(image) => div()
                .size(px(cover))
                .flex_none()
                .relative()
                .rounded(px(R_XL))
                .overflow_hidden()
                .bg(rgb(palette().placeholder))
                // Köşe resmin kendisinde de: kabın `overflow_hidden`'ı resmi
                // kırpmıyor (tam ekran kapağı kare çiziliyordu).
                .child(img(image).size_full().rounded(px(R_XL)))
                .children(hires.map(|(sharp, shown)| {
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(shown)
                        .child(img(sharp).size_full().rounded(px(R_XL)))
                }))
                .into_any_element(),
            None => div()
                .size(px(cover))
                .flex_none()
                .rounded(px(R_XL))
                .bg(rgb(palette().placeholder))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("music", (cover * 0.3).max(24.), label()))
                .into_any_element(),
        };
        let (name, artists) = match &self.state.track {
            Some(t) => (t.name.clone(), t.artists.clone()),
            None => (t!(NothingPlaying).into(), String::new()),
        };
        let titles = div()
            .w(px(cover))
            .flex()
            .flex_col()
            .gap(px(S1))
            .when(!side, |el| el.items_center())
            .child(
                div()
                    .max_w_full()
                    .font_family(font())
                    .type_role(T_HEADLINE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(line_with_tip("full-name", name).flex().min_w_0()),
            )
            .child(
                div()
                    .max_w_full()
                    .type_role(T_TITLE)
                    .text_color(muted())
                    .child(line_with_tip("full-artists", artists).flex().min_w_0()),
            );
        // Dar pencerede sözler kapağın yerine geçer (sözler düğmesi ikisi
        // arasında gidip gelir).
        let lyrics_only = self.now.full.show_lyrics && lyrics_known && !side;
        let left = (!lyrics_only).then(|| {
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(S5))
                .when(!side, |el| el.items_center())
                .child(art)
                .child(titles)
        });

        // Denetimler: belirme / sönme.
        let t =
            (self.now.full.controls_at.elapsed().as_secs_f32() / motion().as_secs_f32()).min(1.);
        animating |= t < 1.;
        let shown = if self.now.full.controls {
            ease_out(t)
        } else {
            1. - ease_out(t)
        };

        let lyrics_col = (side || lyrics_only).then(|| {
            let (lyrics, moving) = self.render_lyrics(Place::Full, window, cx);
            animating |= moving;
            let lyrics_w = if side {
                (w - 2. * S8 - cover - S8).min(lyrics_max)
            } else {
                (w - 2. * S8).min(lyrics_max)
            };
            // "Karaoke": sözlerin sağ üstünde, denetimlerle birlikte belirir / söner.
            let karaoke = (shown > 0. && self.state.track.is_some()).then(|| {
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .opacity(shown)
                    .child(self.karaoke_group(Place::Full, window, cx))
            });
            div()
                .w(px(lyrics_w))
                .h_full()
                .relative()
                .child(lyrics)
                .children(karaoke)
        });

        let content = div()
            .absolute()
            .top(px(S7))
            .left(px(S8))
            .right(px(S8))
            .bottom(px(CONTROLS_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(S8))
            .children(left)
            .children(lyrics_col);

        let controls = (shown > 0.).then(|| self.full_controls(w, shown, cx));

        let element = div()
            .id("fullscreen")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(palette().bg))
            .font_family(UI_FONT)
            .when_some(fade, |el, f| el.opacity(f))
            .on_mouse_move(cx.listener(|this, _, _, cx| this.wake_full_controls(cx)))
            .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                window.focus(&this.focus);
                this.keyboard_focus = false;
                this.wake_full_controls(cx);
            }))
            .child(backdrop)
            .child(content)
            .children(controls)
            .into_any_element();
        (element, animating)
    }

    /// Alttaki denetimler: ilerleme; sözler, karıştır, önceki, çal, sonraki,
    /// tekrar, tam ekrandan çık.
    fn full_controls(&self, w: f32, shown: f32, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let has_track = self.state.track.is_some();
        let has_next = self.state.queue.has_next;
        let playing = self.state.is_playing();
        let duration = self.state.duration_ms();
        let (position, progress) = match self.dragging {
            Some(Drag::Progress(f)) => ((duration as f32 * f) as u32, f),
            _ => {
                let now = Instant::now();
                (self.state.position_ms(now), self.state.progress(now))
            }
        };
        let bar = slider(
            "full-progress",
            progress,
            accent,
            has_track,
            None,
            self.progress_bounds.clone(),
            Some(Box::new(cx.listener(
                |this, ev: &gpui::MouseDownEvent, _, cx| {
                    if let Some(f) = fraction(&this.progress_bounds, ev.position) {
                        this.dragging = Some(Drag::Progress(f));
                        this.views.full.update(cx, |_, cx| cx.notify());
                    }
                },
            ))),
        );
        let progress_row = div()
            .flex()
            .items_center()
            .gap(px(S3))
            .type_role(T_LABEL)
            .tabular()
            .text_color(muted())
            .when(!has_track, |el| el.opacity(0.))
            .child(
                div()
                    .w(px(S7))
                    .flex_none()
                    .text_right()
                    .child(fmt_time(position)),
            )
            .child(bar)
            .child(div().w(px(S7)).flex_none().child(fmt_time(duration)));

        let shuffle = self.now.queue.shuffle;
        let repeat = self.now.queue.repeat;
        let lyrics_on = self.now.full.show_lyrics;
        let center = div()
            .flex()
            .items_center()
            .gap(px(S2))
            .child(
                toggle_button("full-shuffle", "shuffle", shuffle, Look::Live, accent)
                    .tooltip(Tooltip::text(
                        if shuffle {
                            t!(ShuffleOff)
                        } else {
                            t!(ShuffleOn)
                        },
                        Some("Alt S"),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))),
            )
            .child(
                icon_button(
                    "full-prev",
                    "skip-back-fill",
                    if has_track {
                        Look::Live
                    } else {
                        Look::Disabled
                    },
                    accent,
                )
                .tooltip(Tooltip::text(t!(PrevTrack), Some("Ctrl ←")))
                .when(has_track, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| this.previous(cx)))
                }),
            )
            .child(
                play_button(
                    "full-play",
                    playing,
                    if has_track {
                        Look::Live
                    } else {
                        Look::Disabled
                    },
                    accent,
                )
                .tooltip(Tooltip::text(
                    if playing { t!(Pause) } else { t!(Play) },
                    Some(t!(KeySpace)),
                ))
                .when(has_track, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)))
                }),
            )
            .child(
                icon_button(
                    "full-next",
                    "skip-forward-fill",
                    if has_next { Look::Live } else { Look::Disabled },
                    accent,
                )
                .tooltip(Tooltip::text(t!(NextTrack), Some("Ctrl →")))
                .when(has_next, |el| {
                    el.on_click(cx.listener(|this, _, _, cx| this.next(cx)))
                }),
            )
            .child(
                toggle_button(
                    "full-repeat",
                    if repeat == RepeatMode::One {
                        "repeat-1"
                    } else {
                        "repeat"
                    },
                    repeat != RepeatMode::Off,
                    Look::Live,
                    accent,
                )
                .tooltip(Tooltip::text(
                    super::pill::repeat_tip(repeat),
                    Some("Alt R"),
                ))
                .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
            );
        let buttons = div()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(S2))
                    .child(
                        toggle_button("full-lyrics", "mic-vocal", lyrics_on, Look::Live, accent)
                            .tooltip(Tooltip::text(
                                if lyrics_on {
                                    t!(HideLyrics)
                                } else {
                                    t!(ShowLyrics)
                                },
                                None,
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.now.full.show_lyrics = !this.now.full.show_lyrics;
                                this.after_view_change(cx);
                            })),
                    )
                    // "Çeviri": yalnız sözler açıkken ve Spotify'ın çevirisi varken (bkz. `lyrics_view`).
                    .children(
                        (lyrics_on && self.lyrics_translation_ix().is_some()).then(|| {
                            self.translation_chip("full-translation", None, Look::Live, cx)
                        }),
                    ),
            )
            .child(center)
            .child(
                div().flex_1().flex().justify_end().child(
                    icon_button("full-exit", "minimize-2", Look::Live, accent)
                        .tooltip(Tooltip::text(t!(ExitFullScreen), Some("Esc")))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.exit_fullscreen(window, cx)),
                        ),
                ),
            );

        let width = (w - 2. * S8).min(720.);
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(S6))
            .flex()
            .justify_center()
            .opacity(shown)
            .child(
                div()
                    .id("full-controls")
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap(px(S2))
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        this.now.full.hovering_controls = *hovered
                    }))
                    .child(progress_row)
                    .child(buttons),
            )
            .into_any_element()
    }
}

// ── Büyük kapak (iş parçacığında) ───────────────────────────────────────

/// En çok bu kadar bayt (Spotify'ın 640 px'lik JPEG'i ~100 KB).
const HIRES_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Küçük kapak bu kenara küçültülüyor: büyük kapak bundan büyük olmalı.
const HIRES_MIN_SIDE: u32 = 257;

/// Büyük kapak, çözülmüş ve gpui'nin bekleyeceği BGRA düzeninde; indirilemezse
/// ya da kullanışsızsa `None`. Engellemeli (ağ + çözme): arka planda çağır.
/// Kapak adresi herkese açık CDN'dedir (Spotify oturumu gerekmez); çalan şarkı
/// başına en çok bir GET. `fake`: sahte çalmada `source` yerel bir dosyadır
/// (ağa çıkılmaz).
fn fetch_hires(source: &str, fake: bool) -> Option<Arc<RenderImage>> {
    let bytes = if fake {
        std::fs::read(source).ok()?
    } else {
        download_hires(source)?
    };
    // Başlıktan boyut: bozuk dosya ya da zaten küçük bir resim işe yaramaz.
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w.max(h) < HIRES_MIN_SIDE {
        return None;
    }
    let mut pixels = image::load_from_memory(&bytes).ok()?.into_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

fn download_hires(url: &str) -> Option<Vec<u8>> {
    if !url.starts_with("https://") {
        return None;
    }
    // Kısa ömürlü, tek iş parçacıklı çalışma zamanı: bu bir kerelik GET için
    // sürekli bir iş parçacığı ya da havuz tutulmaz.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent("Openify")
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .ok()?;
        let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
        if response
            .content_length()
            .is_some_and(|n| n as usize > HIRES_MAX_BYTES)
        {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        (bytes.len() <= HIRES_MAX_BYTES).then(|| bytes.to_vec())
    })
}

// ── Bulanık arka plan (iş parçacığında) ─────────────────────────────────

/// Kapak baytlarından temaya uyarlanmış, bulanık 128 px'lik PNG.
pub(super) fn backdrop_png(bytes: &[u8], colors: Palette) -> Option<Vec<u8>> {
    const SMALL: u32 = 32;
    const SIZE: u32 = 128;
    let source = image::load_from_memory(bytes).ok()?.to_rgb8();
    let small = image::imageops::resize(&source, SMALL, SMALL, FilterType::Triangle);
    let mut buf: Vec<[f32; 3]> = small
        .pixels()
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
        .collect();
    for _ in 0..3 {
        box_blur(&mut buf, SMALL as usize, 3);
    }
    let mut big = upscale(&buf, SMALL as usize, SIZE as usize);
    for _ in 0..2 {
        box_blur(&mut big, SIZE as usize, 6);
    }
    let mut out = Vec::with_capacity((SIZE * SIZE * 3) as usize);
    for (i, c) in big.iter().enumerate() {
        let packed = backdrop_color(pack(*c), colors);
        // Karanlık, geniş degradede bantlaşma olmasın: yarım basamaklık titreşim.
        let n = dither(i);
        for shift in [16u32, 8, 0] {
            let v = ((packed >> shift) & 0xFF) as f32 + n;
            out.push(v.round().clamp(0., 255.) as u8);
        }
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(&out, SIZE, SIZE, image::ExtendedColorType::Rgb8)
    .ok()?;
    Some(png)
}

fn pack(c: [f32; 3]) -> u32 {
    let ch = |v: f32| v.round().clamp(0., 255.) as u32;
    (ch(c[0]) << 16) | (ch(c[1]) << 8) | ch(c[2])
}

/// Oda gece kalır: zeminle karışım, sonra parlaklık tavanı.
fn backdrop_color(color: u32, colors: Palette) -> u32 {
    if colors.light {
        let mut t = BACKDROP_LIGHT_TINT_MIX;
        loop {
            let c = mix(colors.bg, color, t);
            if luminance(c) >= COVER_TINT_MIN_LUMINANCE || t <= 0. {
                return c;
            }
            t -= 0.02;
        }
    }
    let mut t = 0.5;
    loop {
        let c = mix(colors.bg, color, t);
        if luminance(c) <= BACKDROP_MAX_LUMINANCE || t <= 0. {
            return c;
        }
        t -= 0.02;
    }
}

fn dither(i: usize) -> f32 {
    let mut x = i as u32 ^ 0x9E37_79B9;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x & 0xFF) as f32 / 255. - 0.5
}

/// Kenarları uzatarak yatay ve dikey kutu bulanıklığı (kare resim).
pub(super) fn box_blur(buf: &mut [[f32; 3]], side: usize, radius: usize) {
    let mut tmp = vec![[0f32; 3]; buf.len()];
    let r = radius as isize;
    let n = (2 * radius + 1) as f32;
    let at = |v: isize| v.clamp(0, side as isize - 1) as usize;
    for y in 0..side {
        for x in 0..side {
            let mut sum = [0f32; 3];
            for d in -r..=r {
                let p = buf[y * side + at(x as isize + d)];
                for k in 0..3 {
                    sum[k] += p[k];
                }
            }
            tmp[y * side + x] = sum.map(|s| s / n);
        }
    }
    for y in 0..side {
        for x in 0..side {
            let mut sum = [0f32; 3];
            for d in -r..=r {
                let p = tmp[at(y as isize + d) * side + x];
                for k in 0..3 {
                    sum[k] += p[k];
                }
            }
            buf[y * side + x] = sum.map(|s| s / n);
        }
    }
}

/// Çift doğrusal büyütme.
fn upscale(src: &[[f32; 3]], from: usize, to: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(to * to);
    let scale = from as f32 / to as f32;
    for y in 0..to {
        let fy = ((y as f32 + 0.5) * scale - 0.5).clamp(0., (from - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(from - 1);
        for x in 0..to {
            let fx = ((x as f32 + 0.5) * scale - 0.5).clamp(0., (from - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(from - 1);
            let mut c = [0f32; 3];
            for k in 0..3 {
                let top = src[y0 * from + x0][k] * (1. - tx) + src[y0 * from + x1][k] * tx;
                let bottom = src[y1 * from + x0][k] * (1. - tx) + src[y1 * from + x1][k] * tx;
                c[k] = top * (1. - ty) + bottom * ty;
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_jpeg(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(64, 64, image::Rgb(rgb));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut out)
            .write_image(&img, 64, 64, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    /// Çıta: en parlak kapakta da (beyaz, sarı) arka plan karanlık ve
    /// üstündeki sessiz yazı (sözler, sanatçı, süre) ≥ 4,5:1.
    #[test]
    fn backdrop_keeps_both_themes_and_text_readable() {
        for colors in [Palette::DARK, Palette::LIGHT] {
            for color in [
                [255, 255, 255],
                [250, 220, 40],
                [240, 60, 160],
                [230, 20, 20],
                [30, 60, 255],
                [20, 230, 60],
                [10, 10, 10],
            ] {
                let png = backdrop_png(&solid_jpeg(color), colors).expect("arka plan");
                let img = image::load_from_memory(&png).unwrap().to_rgb8();
                assert_eq!(img.dimensions(), (128, 128));
                for p in img.pixels() {
                    let c = ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32;
                    // Titreşim yarım basamak: tavanın hemen üstü kabul.
                    if colors.light {
                        assert!(luminance(c) >= COVER_TINT_MIN_LUMINANCE - 0.002);
                    } else {
                        assert!(luminance(c) <= BACKDROP_MAX_LUMINANCE + 0.002);
                    }
                    for alpha in [colors.muted_alpha, colors.label_alpha] {
                        let quiet = mix(c, colors.text, alpha);
                        assert!(contrast(quiet, c) >= 4.5, "{color:?}: {quiet:06X}/{c:06X}");
                    }
                }
            }
        }
    }

    #[test]
    fn surface_request_rejects_old_theme_cover_and_repeated_theme() {
        let request = SurfaceRequest {
            seq: 1,
            light: true,
            uri: "track:a".into(),
        };
        assert!(request.accepts(1, true, Some("track:a")));
        assert!(!request.accepts(1, false, Some("track:a")));
        assert!(!request.accepts(3, true, Some("track:a")));
        assert!(!request.accepts(1, true, Some("track:b")));
        assert!(!request.accepts(1, true, None));
    }

    #[test]
    fn upscale_keeps_flat_color() {
        let src = vec![[100., 50., 25.]; 4 * 4];
        let out = upscale(&src, 4, 16);
        assert!(
            out.iter()
                .all(|c| (c[0] - 100.).abs() < 1e-3 && (c[2] - 25.).abs() < 1e-3)
        );
    }
}
