//! Şarkı sözleri (Adım 14): sağdaki panelde ve tam ekranda.
//!
//! "Karanlık oda, tek ışık": okunan satır tam ön plan renginde ve solunda
//! kapak ışığı renginde ince bir çizgi; diğer satırlar sessiz (%64). Etkin
//! satır görünür alanın üçte biri yüksekliğinde tutulur, değişince liste
//! 300 ms'de yumuşakça kayar (en çok 60 kare/sn). Kullanıcı tekerlekle
//! kaydırırsa 4 sn kendiliğinden izleme durur. Satıra tıklamak o yere sarar.
//! Eşzamansız sözler düz metin, söz yoksa "yok" durumu.
//!
//! Kenarlar: kaydırma alanının üst ve alt kenarında satırlar sert kesilmez,
//! solar. Panelde panel yüzeyinden saydama giden bir geçiş (yalnız o kenarın
//! ardında kaydırılmış içerik varsa; kaydırılmamışken ve içerik sığıyorsa
//! hiç yok, tıklamayı engellemez). Tam ekranda arka plan bulanık kapak:
//! düz renkli geçiş yerine her satır, kenara yaklaştıkça saydamlaşır.
//!
//! Çeviri: Spotify'ın kendi çevirisi VARSA (arayüz dilinde; bkz.
//! `Lyrics::translation_index`) sözler panelinin üstünde (tam ekranda
//! denetimlerde) "Çeviri" düğmesi çıkar, açılınca her satırın altında soluk
//! bir çeviri satırı belirir (satır eşzamanı ve kaydırma aynı kalır: çeviri
//! satırın içinde). Çeviri yoksa düğme hiç görünmez. Tercih kalıcıdır
//! (`Settings::lyrics_translation`). Sınama kancası `OPENIFY_TEST_LYRICS_JSON`
//! (bkz. `fixture_lyrics`).
//!
//! Zamanlama DUYULAN konumdan (`Shell::lyric_clock`); satır değişimi bir
//! sonraki satırın başlangıcına kurulan zamanlayıcıyla (`schedule_lyrics`),
//! aradaki karelerde hiç çizim yok.

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Context, FontWeight, ScrollHandle, ScrollWheelEvent, Window, canvas, div,
    linear_color_stop, linear_gradient, prelude::*, px, rgb,
};

use super::{
    Shell,
    components::{Look, Tooltip, chip, state_view},
    now_playing::{LYRIC_LEAD_MS, LyricScroll, LyricsStatus, trace},
    theme::*,
};
use crate::engine::{self, Lyrics, SyncKind};
use crate::{t, tf};

/// Sözlerin çizildiği yer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    Panel,
    Full,
}

/// Etkin satırın görünür alandaki yeri (üstten oran).
const FOCUS_AT: f32 = 0.33;
/// Etkin satır değişince kayma süresi.
const SCROLL_MS: f32 = 300.;
/// Kullanıcı kaydırınca kendiliğinden izlemenin duraklama süresi.
const USER_PAUSE: Duration = Duration::from_secs(4);
/// Panelde kenar geçişinin yüksekliği.
const PANEL_FADE: f32 = S6;
/// Panelin iç köşe yarıçapı (12 px köşe, 1 px kenarlık): alt geçiş köşede
/// panelin yuvarlağını aşmasın.
const PANEL_INNER_RADIUS: f32 = R_XL - 1.;
/// Tam ekranda satırların kenara yaklaşırken saydamlaştığı bölge.
const FULL_FADE: f32 = S8;
/// Bu saydamlığın altındaki (neredeyse görünmeyen) satır tıklanmaz.
const MIN_CLICK_ALPHA: f32 = 0.25;

/// Arayüz dilinin etiketi (Spotify çevirisinin dili ile eşleştirilir).
fn ui_language() -> &'static str {
    match crate::i18n::lang() {
        crate::i18n::Lang::Tr => "tr",
        crate::i18n::Lang::En => "en",
    }
}

/// Sınama kancası `OPENIFY_TEST_LYRICS_JSON=<dosya>`: sahte çalmada söz, bir
/// Spotify söz yanıtından (`engine/fixtures/lyrics_translated.json`,
/// `lyrics_plain.json`) okunur; ağa hiç çıkılmaz. `!none`: söz yok durumu.
/// Yalnız sınama koşusunda (`OPENIFY_TEST_DISPLAY` ya da `OPENIFY_SETTINGS_PATH`
/// varken); yoksa `None` (sahte deneme sözü).
pub(super) fn fixture_lyrics() -> Option<LyricsStatus> {
    static FIXTURE: OnceLock<Option<Option<Arc<Lyrics>>>> = OnceLock::new();
    let test_run = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
        || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some();
    let fixture = FIXTURE.get_or_init(|| {
        let spec = std::env::var("OPENIFY_TEST_LYRICS_JSON")
            .ok()
            .filter(|_| test_run)?;
        if spec == "!none" {
            return Some(None);
        }
        match std::fs::read_to_string(&spec) {
            Ok(raw) => Some(engine::parse_spotify_lyrics(&raw).map(Arc::new)),
            Err(e) => {
                log::warn!("OPENIFY_TEST_LYRICS_JSON okunamadı: {spec}: {e}");
                None
            }
        }
    });
    fixture.as_ref().map(|l| match l {
        Some(l) => LyricsStatus::Ready(l.clone()),
        None => LyricsStatus::Missing,
    })
}

impl Shell {
    /// Çalan şarkının sözünde arayüz dilinde Spotify çevirisi var mı? Varsa
    /// çevirinin sırası (`LyricLine::alts` içindeki yer). Yoksa düğme de yok.
    pub(super) fn lyrics_translation_ix(&self) -> Option<usize> {
        match &self.now.lyrics {
            LyricsStatus::Ready(l) => l.translation_index(ui_language()),
            _ => None,
        }
    }

    /// "Çeviri" düğmesi: yalnız çeviri varken (`lyrics_translation_ix`).
    /// `stop`: klavye durağı (panelde Tab ile gezilir; tam ekranda yok).
    pub(super) fn translation_chip(
        &self,
        id: &'static str,
        stop: Option<gpui::FocusHandle>,
        look: Look,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.prefs.lyrics_translation;
        chip(id, t!(LyricsTranslation), on, look, self.accent().text)
            .when_some(stop, |el, h| el.track_focus(&h))
            .tooltip(Tooltip::text(
                if on {
                    t!(HideLyricsTranslation)
                } else {
                    t!(ShowLyricsTranslation)
                },
                None,
            ))
            .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                this.leave_button(ev, window);
                this.toggle_lyrics_translation(cx);
            }))
            .into_any_element()
    }

    fn lyric_scroll(&mut self, place: Place) -> &mut LyricScroll {
        match place {
            Place::Panel => &mut self.now.side_scroll,
            Place::Full => &mut self.now.full_scroll,
        }
    }

    /// Sözler; kayma sürüyorsa `true`. Panelde sözlerin üstünde bir şerit:
    /// solda "Karaoke" (hap + açıkken sürgü), sağda (yalnız çeviri varken)
    /// "Çeviri". Çalan şarkı yokken şerit de yok. Tam ekranda iki düğme de
    /// denetimlerde (`fullscreen`).
    pub(super) fn render_lyrics(
        &mut self,
        place: Place,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        let (body, animating) = self.render_lyrics_body(place, cx);
        if place == Place::Full || self.state.track.is_none() {
            return (body, animating);
        }
        let translation = self.lyrics_translation_ix().is_some().then(|| {
            let (stop, look) =
                self.stop_look("lyrics-translation", super::tab::PANEL, true, window, cx);
            self.translation_chip("lyrics-translation", stop, look, cx)
        });
        let karaoke = self.karaoke_group(Place::Panel, window, cx);
        let view = div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(S4))
                    .pb(px(S2))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(S2))
                    .child(karaoke)
                    .children(translation),
            )
            .child(div().flex_1().min_h_0().relative().child(body))
            .into_any_element();
        (view, animating)
    }

    fn render_lyrics_body(&mut self, place: Place, cx: &mut Context<Self>) -> (AnyElement, bool) {
        let full = place == Place::Full;
        let accent = self.accent().text;
        let message = |icon_name: &'static str, title: &'static str, detail: &'static str| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px(px(S5))
                .child(state_view(icon_name, title, detail))
                .into_any_element()
        };
        if self.state.track.is_none() {
            return (
                message("mic-vocal", t!(NothingPlaying), t!(LyricsIdleBody)),
                false,
            );
        }
        let lyrics = match &self.now.lyrics {
            LyricsStatus::Ready(l) => l.clone(),
            LyricsStatus::Missing => {
                return (
                    message("mic-vocal", t!(LyricsNoneTitle), t!(LyricsNoneBody)),
                    false,
                );
            }
            LyricsStatus::None | LyricsStatus::Loading => {
                // Yeri ayrılı, sessiz bir satır: geldiğinde hiçbir şey zıplamaz.
                return (
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .type_role(T_SECONDARY)
                        .text_color(label())
                        .child(t!(LyricsLoading))
                        .into_any_element(),
                    false,
                );
            }
        };
        let synced = lyrics.sync == SyncKind::Line;
        // Çeviri: var mı (düğme) ve açık mı (satır altları).
        let translation = lyrics.translation_index(ui_language());
        let show_translation = translation.filter(|_| self.prefs.lyrics_translation);
        let now = Instant::now();
        let (clock, _) = self.lyric_clock(now);
        let active = if synced {
            lyrics.line_at((clock + LYRIC_LEAD_MS).max(0) as u32)
        } else {
            None
        };

        // Kayma: etkin satır değiştiyse hedefe yumuşakça.
        let mut animating = false;
        {
            let scroll = self.lyric_scroll(place);
            let paused = scroll.user_until.is_some_and(|t| now < t);
            if synced && !paused {
                if scroll.line != active {
                    if let Some(target) = active.and_then(|line| focus_offset(scroll, line)) {
                        let from = f32::from(scroll.handle.offset().y);
                        scroll.anim = ((target - from).abs() > 0.5).then_some((from, target, now));
                        scroll.line = active;
                    } else if active.is_some() {
                        // Satırların yeri henüz ölçülmedi (ilk çizim): bir kare sonra.
                        animating = true;
                    } else {
                        scroll.line = None;
                    }
                }
                if let Some((from, to, start)) = scroll.anim {
                    let t = (start.elapsed().as_secs_f32() * 1000. / SCROLL_MS).min(1.);
                    let y = if animations_enabled() {
                        from + (to - from) * ease_out(t)
                    } else {
                        to
                    };
                    let x = scroll.handle.offset().x;
                    scroll.handle.set_offset(gpui::point(x, px(y)));
                    if t >= 1. {
                        scroll.anim = None;
                    } else {
                        animating = true;
                    }
                }
            }
        }

        let (role, weight, gap) = if full {
            // Geniş pencerede (bkz. `fullscreen::LARGE_FROM`) bir basamak büyük.
            (
                if self.now.full.large {
                    T_DISPLAY
                } else {
                    T_HEADLINE
                },
                FontWeight::EXTRA_BOLD,
                S4,
            )
        } else {
            (T_TITLE, FontWeight::BOLD, S3)
        };
        let scroll = self.lyric_scroll(place);
        let handle = scroll.handle.clone();
        let painted = scroll.painted.clone();
        let prefix = if full { "lf" } else { "lp" };
        // Kenar solması yerleşimden okur: içerik değişince ya da ilk açılışta
        // yerleşim bir kare geriden gelir, ölçülene dek birkaç kare daha iste.
        if scroll.settle > 0 {
            scroll.settle -= 1;
            animating = true;
        }
        // Tam ekran: satır saydamlıkları (ilk çocuk baştaki boşluk, eşzamansız
        // sözde ikincisi uyarı yazısı).
        let fades = full.then(|| EdgeFade::read(&handle, if synced { 1 } else { 2 }));

        let mut column = div()
            .id(if full { "lyrics-full" } else { "lyrics-panel" })
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&handle)
            .flex()
            .flex_col()
            .gap(px(gap))
            .px(px(if full { 0. } else { S4 }))
            .on_scroll_wheel(cx.listener(move |this, _: &ScrollWheelEvent, _, _| {
                let s = this.lyric_scroll(place);
                s.user_until = Some(Instant::now() + USER_PAUSE);
                s.anim = None;
            }))
            // Baştaki boşluk: ilk satır da odak yüksekliğine gelebilsin.
            .child(div().flex_none().h(px(if full { S8 } else { S5 })));
        if !synced {
            column = column.child(
                div()
                    .type_role(T_LABEL)
                    .text_color(label())
                    .child(t!(LyricsUnsynced)),
            );
        }
        for (i, l) in lyrics.lines.iter().enumerate() {
            let is_active = active == Some(i);
            let alpha = fades.as_ref().map_or(1., |f| f.alpha(i));
            let color = if is_active || !synced {
                text()
            } else {
                lyric_idle()
            };
            let start = l.start_ms;
            let content: String = if l.text.is_empty() {
                "• • •".into()
            } else {
                l.text.clone()
            };
            // Soluk çeviri satırı (boş çeviri: hiçbir şey; ara satırı "• • •" da çevrilmez).
            let translated = show_translation
                .and_then(|ix| l.alts.get(ix))
                .filter(|a| !a.is_empty() && !l.text.is_empty())
                .cloned();
            // Panelde yazı panelin kenar boşluğundan (16 px) başlar, sekme
            // haplarıyla aynı kenardan; okunan satırın çizgisi yazının 8 px
            // solunda, kenar boşluğunun içinde (7 Kasım). Tam ekranda kenar
            // boşluğu yok: çizgi için yazı 12 px içeride.
            let line = div()
                .id((prefix, i))
                .relative()
                .when(full, |el| el.pl(px(S3)))
                .font_family(font())
                .type_role(role)
                .font_weight(weight)
                .text_color(color)
                .child(content)
                .when_some(translated, |el, tr| {
                    // Panelde Body, tam ekranda Title; asıl satırdan hafif, rengi bir ton geride.
                    el.child(
                        div()
                            .pt(px(S1))
                            .type_role(if full { T_TITLE } else { T_BODY })
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if is_active { muted() } else { label() })
                            .child(tr),
                    )
                })
                .when(alpha < 1., |el| el.opacity(alpha))
                .when(is_active, |el| {
                    el.child(
                        div()
                            .absolute()
                            .left(px(if full { 0. } else { -S2 }))
                            .top(px(S1))
                            .bottom(px(S1))
                            .w(px(S0))
                            .rounded(px(R_XS))
                            .bg(rgb(accent)),
                    )
                })
                .when(synced && alpha >= MIN_CLICK_ALPHA, |el| {
                    el.cursor_pointer()
                        .when(!is_active, |el| el.hover(|s| s.text_color(soft())))
                        .on_click(cx.listener(move |this, _, _, cx| this.seek_to(start, cx)))
                });
            column = column.child(line);
        }
        if let Some(provider) = &lyrics.provider {
            let alpha = fades.as_ref().map_or(1., |f| f.alpha(lyrics.lines.len()));
            column = column.child(
                div()
                    .pt(px(S5))
                    .when(full, |el| el.pl(px(S3)))
                    .type_role(T_LABEL)
                    .text_color(label())
                    .when(alpha < 1., |el| el.opacity(alpha))
                    .child(tf!(LyricsProvider, provider)),
            );
        }
        // Sondaki boşluk: son satır da odak yüksekliğine gelebilsin.
        column = column.child(div().flex_none().h(px(if full { 360. } else { 240. })));

        // İz: etkin satır değişimi boyandığı an (bkz. `now_playing::trace`).
        let probe = (trace::enabled() && synced).then(|| self.clock_probe(now));
        let seek_at = if probe.is_some() {
            self.now.seek_at.take()
        } else {
            None
        };
        let view = if full { "full" } else { "panel" };
        let starts: Vec<u32> = if probe.is_some() {
            lyrics.lines.iter().map(|l| l.start_ms).collect()
        } else {
            Vec::new()
        };
        let tracer = probe.map(|probe| {
            canvas(
                |_, _, _| {},
                move |_, _, _, _| {
                    if painted.get() == active && seek_at.is_none() {
                        return;
                    }
                    painted.set(active);
                    let Some(line) = active else { return };
                    let (clock, source) = probe.read();
                    trace::write(trace::Row {
                        view,
                        line,
                        start_ms: starts.get(line).copied().unwrap_or_default(),
                        clock_ms: clock,
                        source,
                        since_seek_ms: seek_at.map(|t| t.elapsed().as_secs_f32() * 1000.),
                    });
                },
            )
            .absolute()
            .size_0()
        });

        // Panel: üst ve alt kenarda panel yüzeyinden saydama geçiş. Gücü, o
        // kenarın ardında ne kadar kaydırılmış içerik kaldığına bağlı (32 px'e
        // kadar doğrusal): en üstteyken ve içerik sığarken geçiş yok.
        let (top_fade, bottom_fade) = if full {
            (None, None)
        } else {
            let offset = f32::from(handle.offset().y);
            let max = f32::from(handle.max_offset().height);
            (
                edge_veil(true, -offset / PANEL_FADE),
                edge_veil(false, (max + offset) / PANEL_FADE),
            )
        };

        let view = div()
            .size_full()
            .relative()
            .child(column)
            .children(top_fade)
            .children(bottom_fade)
            .children(tracer);
        let view = view.into_any_element();
        (view, animating)
    }
}

/// Panelin üst ya da alt kenarındaki geçiş: panel yüzeyinden (`palette().bar`) saydama.
/// `strength` 0..1; ~0 ise hiç öğe yok. Hiçbir olaya bağlanmaz: tıklama ve
/// tekerlek altındaki satıra geçer.
pub(super) fn edge_veil(top: bool, strength: f32) -> Option<AnyElement> {
    let strength = strength.clamp(0., 1.);
    if strength < 0.01 {
        return None;
    }
    let solid = linear_color_stop(
        with_alpha(palette().bar, strength),
        if top { 0. } else { 1. },
    );
    let clear = linear_color_stop(with_alpha(palette().bar, 0.), if top { 1. } else { 0. });
    let (from, to) = if top { (solid, clear) } else { (clear, solid) };
    let veil = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(PANEL_FADE))
        .bg(linear_gradient(180., from, to));
    Some(
        if top {
            veil.top_0()
        } else {
            veil.bottom_0()
                .rounded_bl(px(PANEL_INNER_RADIUS))
                .rounded_br(px(PANEL_INNER_RADIUS))
        }
        .into_any_element(),
    )
}

/// Tam ekranda satır saydamlıkları: son yerleşimdeki satır konumlarından
/// (kaydırma konumu canlı okunur) hesaplanır.
struct EdgeFade {
    /// Söz satırlarının (ve sondaki kaynak yazısının) ekranda (üst, alt) yeri.
    rows: Vec<(f32, f32)>,
    /// Görünür alanın üstü ve altı.
    view: (f32, f32),
}

impl EdgeFade {
    /// `first`: ilk söz satırının çocuk sırası.
    fn read(handle: &ScrollHandle, first: usize) -> Self {
        let view = handle.bounds();
        let offset = f32::from(handle.offset().y);
        let mut rows = Vec::new();
        while let Some(b) = handle.bounds_for_item(first + rows.len()) {
            rows.push((f32::from(b.top()) + offset, f32::from(b.bottom()) + offset));
        }
        Self {
            rows,
            view: (f32::from(view.top()), f32::from(view.bottom())),
        }
    }

    /// `line`: satırın sırası (sondaki kaynak yazısı için satır sayısı).
    /// Ölçülmemiş satır (ilk kare) tam görünür.
    fn alpha(&self, line: usize) -> f32 {
        match self.rows.get(line) {
            Some(&(top, bottom)) if self.view.1 > self.view.0 => {
                edge_alpha(top, bottom, self.view, FULL_FADE)
            }
            _ => 1.,
        }
    }
}

/// Bir satırın saydamlığı (0..1): üst kenara yakın satır üstünden, alt kenara
/// yakın satır altından ölçülür; `fade` px'lik bölgede 0'dan 1'e (yumuşak
/// adım). Kenara değen satır tümden saydam: kesilen satır görünmez, kenara
/// gelen satır bir bütün olarak belirir.
fn edge_alpha(top: f32, bottom: f32, view: (f32, f32), fade: f32) -> f32 {
    let up = ((top - view.0) / fade).clamp(0., 1.);
    let down = ((view.1 - bottom) / fade).clamp(0., 1.);
    let t = up.min(down);
    t * t * (3. - 2. * t)
}

/// Etkin satırı odak yüksekliğine getiren kaydırma konumu (px, eksi).
/// Satırların yeri son yerleşimden; henüz yoksa `None`.
fn focus_offset(scroll: &LyricScroll, line: usize) -> Option<f32> {
    // İlk çocuk baştaki boşluk.
    let item = scroll.handle.bounds_for_item(line + 1)?;
    let view = scroll.handle.bounds();
    let height = f32::from(view.size.height);
    if height <= 0. {
        return None;
    }
    let max = f32::from(scroll.handle.max_offset().height);
    let target = f32::from(view.top()) + height * FOCUS_AT
        - f32::from(item.size.height) / 2.
        - f32::from(item.top());
    Some(target.clamp(-max, 0.))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: (f32, f32) = (100., 700.);

    #[test]
    fn rows_in_the_middle_are_opaque() {
        assert_eq!(edge_alpha(300., 340., VIEW, FULL_FADE), 1.);
    }

    #[test]
    fn rows_fade_towards_both_edges_and_vanish_at_them() {
        // Üst kenarda: satırın üstü kenara geldiğinde tümden saydam.
        assert_eq!(edge_alpha(100., 140., VIEW, FULL_FADE), 0.);
        assert_eq!(edge_alpha(60., 120., VIEW, FULL_FADE), 0.);
        // Alt kenarda: satırın altı kenara geldiğinde.
        assert_eq!(edge_alpha(660., 700., VIEW, FULL_FADE), 0.);
        assert_eq!(edge_alpha(680., 740., VIEW, FULL_FADE), 0.);
        // Bölgenin yarısında yarısı kadar (simetrik, yumuşak adım 0,5).
        let half = FULL_FADE / 2.;
        assert!((edge_alpha(100. + half, 140. + half, VIEW, FULL_FADE) - 0.5).abs() < 1e-6);
        assert!((edge_alpha(640. - half, 700. - half, VIEW, FULL_FADE) - 0.5).abs() < 1e-6);
        // Bölgenin sonunda tam görünür.
        assert_eq!(
            edge_alpha(100. + FULL_FADE, 140. + FULL_FADE, VIEW, FULL_FADE),
            1.
        );
    }

    #[test]
    fn fade_never_leaves_zero_to_one() {
        for i in 0..=600 {
            let top = 100. + i as f32;
            let a = edge_alpha(top, top + 34., VIEW, FULL_FADE);
            assert!((0. ..=1.).contains(&a));
        }
    }
}
