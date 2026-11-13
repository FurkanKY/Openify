//! Yüzen çalar hapı (taslak D).
//!
//! Pencerenin altında, ortada, alttan 24 px yukarıda yüzer: 72 px yüksek, en
//! fazla 720 px (dar pencerede pencere eksi iki yanda 32 px). Solda 48 px
//! yuvarlak kapak ve şarkı, ortada önceki / çal / sonraki ve ilerleme, sağda
//! ses. Işık kaynağı kapak: bir şey çalarken hap, kapağın durduğu soldan
//! kapak rengiyle aydınlanır. Listeler hapın altından geçer; altlarında hap
//! payı kadar boşluk vardır (`PILL_CLEARANCE`).
//!
//! Hap kendi önbellekli görünümünde (`views::PillView`): çalarken saniyede
//! bir (ya da dolu kısım bir piksel uzayınca) yalnız o yeniden çizilir.
//!
//! Adım 11 / 14: ortada karıştır / önceki / çal / sonraki / tekrar, sağda
//! sıra ve sözler paneli düğmeleri. Karıştırma ve tekrar açıkken simge kapak
//! ışığı renginde ve altında küçük bir nokta: durumu bir bakışta. Tam ekran
//! kapağa tıklayınca (üzerine gelince ortasında simge) ya da F11. Dar hapta
//! (760 px'ten dar) ses çubuğu çekilir (sessize al kalır, üzerinde tekerlek
//! sesi değiştirir).
//!
//! Orta sütun iki satır: üstte düğmeler, altında süre yazılarıyla ilerleme
//! çubuğu (sütunun bütün eni; 800 px'lik hapta ~280 px, en dar hapta
//! ~250 px). Eskiden çubuk düğmelerin yanındaki artan yere sığıyordu: ~60 px.

use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, Context, Div, ElementId, FocusHandle,
    FontWeight, Hsla, MouseButton, MouseDownEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    SharedString, Stateful, Transformation, Window, canvas, div, ease_out_quint, img,
    linear_color_stop, linear_gradient, percentage, prelude::*, px, rgb,
};

use super::{
    Drag, Shell,
    components::{Look, Tooltip, icon_button, play_button, secondary_button},
    cover_image,
    library::{Loadable, PAGE_PAD},
    now_playing::Panel,
    player::{self, Connection, fmt_time},
    text::{self, line, line_with_tip},
    theme::*,
};
use crate::engine::{self, RepeatMode};
use crate::{t, tf};

/// Hapın yüksekliği, alttan uzaklığı ve listelerin altında bıraktığı pay.
pub(super) const PILL_HEIGHT: f32 = 72.;
pub(super) const PILL_BOTTOM: f32 = S5;
pub(super) const PILL_CLEARANCE: f32 = PILL_HEIGHT + PILL_BOTTOM;
/// Kaydırılan listenin sonundaki pay: hap payı + nefes boşluğu. En sona
/// kaydırınca son satır hapın üst kenarına yapışmaz, 16 px üstünde durur.
pub(super) const PILL_LIST_END: f32 = PILL_CLEARANCE + S4;
/// Hapın en geniş hâli. Adım 11'de 720'den 800'e: karıştır, tekrar, sıra ve
/// sözler düğmeleri geldi; ilerleme satırı yine en az 136 px.
pub(super) const PILL_MAX_WIDTH: f32 = 800.;
/// Bu genişliğin altında ses çubuğu gizlenir (sessize al kalır).
const VOLUME_SLIDER_MIN_PILL: f32 = 760.;
/// Soldaki yuvarlak kapak.
pub(super) const PILL_COVER: f32 = 48.;
/// Şarkı bilgisinin genişliği (dar hapta en az `PILL_INFO_MIN`'e iner).
const PILL_INFO: f32 = 180.;
const PILL_INFO_MIN: f32 = 112.;
/// Süre yazılarının genişliği ("12:34").
const TIME_WIDTH: f32 = 36.;
/// Orta sütun (düğmeler üstte, ilerleme altta): düğme satırının eni
/// (4 × 36 px düğme + 44 px çal + 4 aralık). Sütun bunun altına inmez.
const CONTROLS_WIDTH: f32 = 36. * 4. + 44. + S1 * 4.;
/// İlerleme satırının yüksekliği (tıklama alanı); çizgi 4 px. Düğme satırı
/// (44 px çal düğmesi) ile birlikte 64 px: hapın 72 px'ine 4 px boşlukla sığar.
const PROGRESS_ROW: f32 = S4 + S1;
/// Ses çubuğu: genişliği ve en dar hâli.
const VOLUME_WIDTH: f32 = 72.;
const VOLUME_MIN: f32 = 40.;
/// Kaydırıcının tutamacı (üzerine gelince beliren yuvarlak).
const SLIDER_THUMB: f32 = 12.;
/// Kaydırıcının tıklama alanı: 32 px yüksek, saydam (çizgi 4 px).
const SLIDER_HIT: f32 = 32.;

/// Hapın genişliği: en fazla 720 px, dar pencerede iki yanda sayfa kenarı
/// kadar (32 px) boşluk.
pub(super) fn pill_width(window_width: f32) -> f32 {
    (window_width - 2. * PAGE_PAD).clamp(0., PILL_MAX_WIDTH)
}

/// Hapın solunda ne görünür.
pub(super) enum PillLeft {
    /// Çalan (ya da duraklatılmış) şarkı; bildirim varsa sanatçının yerinde.
    Track {
        uri: String,
        name: String,
        artists: String,
        cover: Option<Arc<gpui::Image>>,
        notice: Option<player::Notice>,
        /// Spotify'a ulaşılamıyor (kaldığı yerden açılan şarkı çevrimdışı):
        /// sanatçının yerinde soluk wifi-off + "Çevrimdışı · Kütüphane
        /// önbellekten".
        offline: bool,
    },
    /// Çal'a basıldı, şarkı yükleniyor.
    Pending {
        name: String,
        image: String,
        notice: Option<player::Notice>,
    },
    /// Çalan bir şey yokken bildirim.
    Notice(player::Notice),
    /// Çalan bir şey yok ve Spotify'a ulaşılamıyor / ses çıkışı yok.
    Offline {
        icon: &'static str,
        title: &'static str,
        detail: &'static str,
    },
    /// Hiçbir şey çalmıyor.
    Idle,
}

/// Hapın çizilecek hâli. Uygulamada `Shell::pill_model` durumdan kurar;
/// vitrin bütün durumları yan yana göstermek için kendisi kurar (`live`
/// kapalı: odak, dinleyici ve ölçü yok).
pub(super) struct PillModel {
    pub left: PillLeft,
    pub playing: bool,
    pub has_track: bool,
    pub has_next: bool,
    /// Hiçbir şey çalmıyorken çal düğmesinin başlatacağı liste ("Çal: …").
    pub start_title: Option<String>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub progress: f32,
    /// Ses düzeyi (simge için) ve çubukta gösterilen düzey (sessizken önceki).
    pub volume: f32,
    pub volume_shown: f32,
    pub muted: bool,
    /// Bir şey çalarken kapağın ışığı.
    pub glow: Option<u32>,
    pub live: bool,
}

/// Sınama kancası `OPENIFY_TEST_LOOK_ONLINE=1` (yalnız sınama koşusunda): çevrimdışı
/// sınama kopyasında çalan şarkı hapı "Çevrimdışı" yazısı yerine sanatçıyı ve
/// ilerlemeyi gösterir. README ekran görüntüleri için; bağlantı durumu ve
/// başka hiçbir şey değişmez.
fn look_online() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let test_run = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
            || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some();
        test_run && std::env::var_os("OPENIFY_TEST_LOOK_ONLINE").is_some()
    })
}

impl Shell {
    /// Hapın şu anki hâli.
    pub(super) fn pill_model(&self, now: Instant) -> PillModel {
        let notice = self.state.visible_notice(now).cloned();
        let left = if let Some(p) = &self.state.pending {
            PillLeft::Pending {
                name: p.name.clone(),
                image: p.image.clone(),
                notice,
            }
        } else if let Some(t) = &self.state.track {
            PillLeft::Track {
                uri: t.uri.clone(),
                name: t.name.clone(),
                artists: t.artists.clone(),
                cover: self.state.cover.clone(),
                notice,
                offline: matches!(self.state.connection, Connection::Failed(_)) && !look_online(),
            }
        } else if let Some(n) = notice {
            PillLeft::Notice(n)
        } else {
            match self.state.connection {
                Connection::NoAudio => PillLeft::Offline {
                    icon: "volume-x",
                    title: t!(PillNoAudioTitle),
                    detail: t!(PillNoAudioDetail),
                },
                Connection::Failed(_) if matches!(self.library.library, Loadable::Loaded(_)) => {
                    PillLeft::Offline {
                        icon: "wifi-off",
                        title: t!(Offline),
                        detail: t!(OfflineLibrary),
                    }
                }
                Connection::Failed(_) => PillLeft::Offline {
                    icon: "wifi-off",
                    title: t!(Offline),
                    detail: t!(OfflineWaiting),
                },
                // Adım 17: giriş / Premium / motor durumları (yazılar `crate::msg`).
                Connection::NeedsLogin => PillLeft::Offline {
                    icon: "user",
                    title: t!(PillNeedsLoginTitle),
                    detail: t!(PillNeedsLoginDetail),
                },
                Connection::LoggingIn => PillLeft::Offline {
                    icon: "user",
                    title: t!(PillLoggingInTitle),
                    detail: t!(PillLoggingInDetail),
                },
                Connection::LoginFailed(_) => PillLeft::Offline {
                    icon: "circle-alert",
                    title: t!(PillLoginFailedTitle),
                    detail: t!(Retry),
                },
                Connection::NeedsPremium => PillLeft::Offline {
                    icon: "circle-alert",
                    title: t!(PillNeedsPremiumTitle),
                    detail: t!(PillNeedsPremiumDetail),
                },
                Connection::EngineStopped => PillLeft::Offline {
                    icon: "circle-alert",
                    title: t!(PillEngineStoppedTitle),
                    detail: t!(BtnRestart),
                },
                Connection::Connecting | Connection::Connected(_) => PillLeft::Idle,
            }
        };
        let has_track = self.state.track.is_some();
        let duration_ms = self.state.duration_ms();
        let (position_ms, progress) = match self.dragging {
            Some(Drag::Progress(f)) => ((duration_ms as f32 * f) as u32, f),
            _ => (self.state.position_ms(now), self.state.progress(now)),
        };
        PillModel {
            left,
            playing: self.state.is_playing(),
            has_track,
            has_next: self.state.queue.has_next,
            start_title: self.startable_collection().map(|name| name.to_string()),
            position_ms,
            duration_ms,
            progress,
            volume: self.state.volume,
            volume_shown: self.volume_before_mute.unwrap_or(self.state.volume),
            muted: self.volume_before_mute.is_some(),
            glow: has_track.then(|| self.accent().glow),
            live: true,
        }
    }

    pub(super) fn render_pill(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let model = self.pill_model(Instant::now());
        self.pill(&model, window, cx)
    }

    /// Hap; `m.live` kapalıysa (vitrin) odak, dinleyici ve ölçü olmadan.
    pub(super) fn pill(
        &self,
        m: &PillModel,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Vurgu hapın kapak ışığının üstünde de okunur (bkz. `pill_accent`).
        let accent = pill_accent(self.accent().text);
        let nav = &self.nav;
        let live = m.live;
        let look = |handle: &FocusHandle, enabled: bool| {
            if live {
                self.look(handle, window, enabled)
            } else if enabled {
                Look::Live
            } else {
                Look::Disabled
            }
        };

        // Sol: kapak ve şarkı bilgisi. Çalan bir şey yokken bildirim ya da
        // çevrimdışı durumu, gizli ilerleme satırının yerini de alır: uzun
        // bildirim 120 px'lik yazı alanına sığmıyor, hapın dışına taşıyordu.
        // (Boşta ve yüklenirken ilerlemenin yeri korunur: çal'a basınca
        // düğmeler zıplamasın.)
        // Kaldığı yerden açılan şarkı çevrimdışıyken de: çalınamaz, durum
        // satırı ("Çevrimdışı · Kütüphane önbellekten") 76 px'e sığmıyordu.
        let wide_left = matches!(
            m.left,
            PillLeft::Notice(_) | PillLeft::Offline { .. } | PillLeft::Track { offline: true, .. }
        );
        let left = div()
            .when(wide_left, |el| el.flex_1())
            .when(!wide_left, |el| el.flex_basis(px(PILL_INFO)).flex_shrink())
            .min_w(px(PILL_INFO_MIN))
            .child(self.pill_left(m, window, cx));

        // Orta: önceki / çal / sonraki. Hiçbir şey çalmıyorken çal düğmesi
        // ölü durmaz: açık listeyi başlatır. Kullanılamayan düğmeler Tab
        // sırasına girmez.
        let can_play = m.has_track || m.start_title.is_some();
        let play_tip = match (&m.start_title, m.playing) {
            (Some(title), _) if !m.has_track => tf!(PlayNamed, title),
            (_, true) => t!(Pause).into(),
            _ => t!(Play).into(),
        };
        let play = play_button("play", m.playing, look(&nav.play, can_play), accent)
            .when(live && can_play, |el| {
                el.track_focus(&nav.play)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)))
            })
            .tooltip(Tooltip::text(play_tip, Some(t!(KeySpace))));
        let prev = icon_button(
            "prev",
            "skip-back-fill",
            look(&nav.prev, m.has_track),
            accent,
        )
        .when(live && m.has_track, |el| {
            el.track_focus(&nav.prev)
                .on_click(cx.listener(|this, _, _, cx| this.previous(cx)))
        })
        .tooltip(Tooltip::text(t!(PrevTrack), Some("Ctrl ←")));
        let next = icon_button(
            "next",
            "skip-forward-fill",
            look(&nav.next, m.has_next),
            accent,
        )
        .when(live && m.has_next, |el| {
            el.track_focus(&nav.next)
                .on_click(cx.listener(|this, _, _, cx| this.next(cx)))
        })
        .tooltip(Tooltip::text(t!(NextTrack), Some("Ctrl →")));
        // Karıştır ve tekrar: önceki / sonraki'nin iki yanında (Spotify'daki
        // yerleri), açıkken kapak ışığı renginde.
        let stops = &self.now.stops;
        let (shuffle_on, repeat) = if live {
            (self.now.queue.shuffle, self.now.queue.repeat)
        } else {
            (false, RepeatMode::Off)
        };
        let shuffle = toggle_button(
            "shuffle",
            "shuffle",
            shuffle_on,
            look(&stops.shuffle, true),
            accent,
        )
        .when(live, |el| {
            el.track_focus(&stops.shuffle)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx)))
        })
        .tooltip(Tooltip::text(
            if shuffle_on {
                t!(ShuffleOff)
            } else {
                t!(ShuffleOn)
            },
            Some("Alt S"),
        ));
        let repeat_icon = if repeat == RepeatMode::One {
            "repeat-1"
        } else {
            "repeat"
        };
        let repeat_button = toggle_button(
            "repeat",
            repeat_icon,
            repeat != RepeatMode::Off,
            look(&stops.repeat, true),
            accent,
        )
        .when(live, |el| {
            el.track_focus(&stops.repeat)
                .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx)))
        })
        .tooltip(Tooltip::text(repeat_tip(repeat), Some("Alt R")));
        let controls = div()
            .flex_shrink_0()
            // Düğmelerin arası hapın "boş yeri" sayılmaz (bkz. hapın on_click).
            .block_mouse_except_scroll()
            .flex()
            .items_center()
            .gap(px(S1))
            .child(shuffle)
            .child(prev)
            .child(play)
            .child(next)
            .child(repeat_button);

        // İlerleme: düğmelerin altında, orta sütunun bütün eninde (800 px'lik
        // hapta ~280 px; eskiden düğmelerin yanında 60 px'e sıkışıyordu).
        // Şarkı yokken "0:00 ─── 0:00" bozuk bir çalar gibi duruyordu:
        // gizlenir ama yerini korur (şarkı gelince öğeler zıplamasın).
        let progress_slider = if live {
            slider_hit(
                "progress",
                m.progress,
                accent,
                m.has_track,
                Some((
                    nav.progress.clone(),
                    look(&nav.progress, true) == Look::Focus,
                )),
                self.progress_bounds.clone(),
                Some(Box::new(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    if let Some(f) = fraction(&this.progress_bounds, ev.position) {
                        this.dragging = Some(Drag::Progress(f));
                        this.notify_pill(cx);
                    }
                }))),
                PROGRESS_ROW,
            )
        } else {
            slider_hit(
                "progress",
                m.progress,
                accent,
                m.has_track,
                None,
                Rc::default(),
                None,
                PROGRESS_ROW,
            )
        };
        // Dar hap (pencere 824 px'ten dar): ses çubuğu çekilir (sessize al
        // kalır); ilerleme satırı yine tam: 696 px'lik hapta bile sütun
        // ≥ 340 px, çubuk ≥ 250 px.
        let wide = !live || self.pill_width >= VOLUME_SLIDER_MIN_PILL;
        let progress = div()
            .w_full()
            .h(px(PROGRESS_ROW))
            // Sarınca hapın boş yer tıklaması (panel) çalışmasın.
            .block_mouse_except_scroll()
            .flex()
            .items_center()
            .gap(px(S2))
            .when(!m.has_track, |el| el.opacity(0.))
            .type_role(T_LABEL)
            .tabular()
            .text_color(muted())
            .child(
                div()
                    .w(px(TIME_WIDTH))
                    .flex_none()
                    .text_right()
                    .child(fmt_time(m.position_ms)),
            )
            .child(progress_slider)
            .child(
                div()
                    .w(px(TIME_WIDTH))
                    .flex_none()
                    .child(fmt_time(m.duration_ms)),
            );
        // Orta sütun: üstte karıştır / önceki / çal / sonraki / tekrar, altında
        // ilerleme. Solda bildirim ya da çevrimdışı durumu varken ilerleme
        // yok ama satırı yerinde (düğmeler iki durum arasında kaymasın).
        let center = div()
            .when(wide_left, |el| el.flex_none())
            .when(!wide_left, |el| el.flex_1())
            .min_w(px(CONTROLS_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .child(controls)
            .child(if wide_left {
                div().h(px(PROGRESS_ROW)).into_any_element()
            } else {
                progress.into_any_element()
            });

        // Sağ: ses. Sessizken çubuk boşalmaz: önceki düzey soluk görünür,
        // sesi açınca nereye döneceği belli.
        let volume_icon = match m.volume {
            0.0 => "volume-x",
            v if v < 0.5 => "volume-1",
            _ => "volume-2",
        };
        let volume_fill = if m.muted {
            mix(palette().text, palette().bg, 0.62)
        } else {
            palette().text
        };
        let volume_slider = if live {
            slider(
                "volume",
                m.volume_shown,
                volume_fill,
                true,
                Some((nav.volume.clone(), look(&nav.volume, true) == Look::Focus)),
                self.volume_bounds.clone(),
                Some(Box::new(cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    if let Some(f) = fraction(&this.volume_bounds, ev.position) {
                        this.dragging = Some(Drag::Volume);
                        this.apply_volume(f);
                        this.notify_pill(cx);
                    }
                }))),
            )
        } else {
            slider(
                "volume",
                m.volume_shown,
                volume_fill,
                true,
                None,
                Rc::default(),
                None,
            )
        };
        let mute = icon_button("mute", volume_icon, look(&nav.mute, true), accent)
            .tooltip(Tooltip::text(
                if m.volume == 0.0 {
                    t!(Unmute)
                } else {
                    t!(Mute)
                },
                Some("Ctrl Shift ↓"),
            ))
            .when(live, |el| {
                el.track_focus(&nav.mute)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx)))
            });
        // Dar hapta ses çubuğu yok: sessize al düğmesinin üstünde tekerlek
        // sesi değiştirir (Ctrl+↑ / ↓ de).
        let mute = mute.when(live && !wide, |el| {
            el.on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let notches = match ev.delta {
                    ScrollDelta::Lines(l) => l.y,
                    ScrollDelta::Pixels(p) => f32::from(p.y) / 40.,
                };
                if notches != 0. {
                    let base = this.volume_before_mute.unwrap_or(this.state.volume);
                    this.set_volume(base + notches.signum() * super::VOLUME_STEP, cx);
                }
            }))
        });
        // Sıra ve sözler paneli.
        let panel = if live { self.now.panel } else { None };
        // Mini oynatıcı (ayrı, hep üstte duran pencere): açıkken vurgu
        // renginde, kapalıyken sakin. Durum ana pencerede tutulmaz: pencere
        // varsa açıktır.
        let mini_open = live && self.mini.is_some();
        let mini = toggle_button(
            "mini",
            "picture-in-picture-2",
            mini_open,
            look(&stops.mini, true),
            accent,
        )
        .when(live, |el| {
            el.track_focus(&stops.mini)
                .on_click(cx.listener(|this, _, window, cx| {
                    let screen = window.display(cx).map(|d| d.bounds());
                    this.toggle_mini(screen, cx);
                }))
        })
        .tooltip(Tooltip::text(t!(MiniPlayer), Some("Alt M")));
        let queue = toggle_button(
            "queue",
            "list-music",
            panel == Some(Panel::Queue),
            look(&stops.queue, true),
            accent,
        )
        .when(live, |el| {
            el.track_focus(&stops.queue)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Queue, cx)))
        })
        .tooltip(Tooltip::text(t!(QueueTip), Some("Alt Shift Q")));
        let lyrics = toggle_button(
            "lyrics",
            "mic-vocal",
            panel == Some(Panel::Lyrics),
            look(&stops.lyrics, true),
            accent,
        )
        .when(live, |el| {
            el.track_focus(&stops.lyrics)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Lyrics, cx)))
        })
        .tooltip(Tooltip::text(t!(Lyrics), Some("Alt Shift L")));
        let volume = div()
            // Düğmeler ve ses çubuğu hapın boş yeri sayılmaz.
            .block_mouse_except_scroll()
            .flex()
            .items_center()
            .gap(px(S1))
            .child(mini)
            .child(queue)
            .child(lyrics)
            .child(mute)
            .when(wide, |el| {
                el.child(
                    div()
                        .w(px(VOLUME_WIDTH))
                        .flex_shrink()
                        .min_w(px(VOLUME_MIN))
                        .child(volume_slider),
                )
            });

        // Çalarken gelen bilgi / sorun bildirimi hapın içine sığmaz (şarkı
        // bilgisinin ~130 px'lik alanı): hapın üstünde şerit olarak çıkar.
        let toast = match &m.left {
            PillLeft::Track {
                notice: Some(n), ..
            }
            | PillLeft::Pending {
                notice: Some(n), ..
            } if n.kind != engine::NoticeKind::Working => Some(self.notice_toast(n, cx)),
            _ => None,
        };
        // Başka bir aygıtta çalıyorsa (Spotify Connect): hapın üstünde sakin
        // bir şerit, "Buraya aktar" ile. Bildirim şeridi varsa o önce görünür.
        let remote = match (&self.now.connect.remote, toast.is_some()) {
            (Some(remote), false) if live => {
                Some(self.remote_strip(&remote.device_name, window, cx))
            }
            _ => None,
        };

        let pill = div()
            .id("pill")
            .w_full()
            .h(px(PILL_HEIGHT))
            .pl(px(S3))
            .pr(px(S4))
            .flex()
            .items_center()
            .gap(px(S4))
            .relative()
            .rounded_full()
            .shadow(capsule_shadow())
            // Koyu cam (7 Kasım kararı, `pill_glass`): üstten alta hafifçe
            // koyulaşan zemin; çalarken kapağın durduğu soldan kapağın kendi
            // renkleriyle yumuşak ışık, üstte ince ışık çizgisi, soluk iç
            // halka. Çalmıyorken renksiz, sakin cam.
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(palette().pill_glass_top), 0.),
                linear_color_stop(rgb(palette().pill_glass_bottom), 1.),
            ))
            .children(self.pill_surface(m.glow.is_some(), window));
        pill.when(live, |el| {
            // Hap altındaki satırları örter: boş yerine tıklamak arkadaki
            // satırı seçmesin, üzerine gelmek onu boyamasın. (Tekerlek yine
            // listeyi kaydırır.) Kök örtülü kaldığı için odak burada
            // bırakılır (bkz. `Shell::render`).
            el.block_mouse_except_scroll()
                .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus);
                    if std::mem::take(&mut this.keyboard_focus) {
                        cx.notify();
                    }
                }))
                // Boş yere tıklamak "Şimdi çalıyor" panelini açar / kapatır
                // (bkz. `now_panel`). Düğmeler, kapak, ilerleme, ses, kalp ve
                // sanatçı adları kendi tıklamalarını alır: kapları
                // `block_mouse_except_scroll` ile hapın tıklamasını örter.
                .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Now, cx)))
        })
        .child(left)
        .child(center)
        .child(volume)
        .children(toast)
        .children(remote)
        .into_any_element()
    }

    /// Başka bir aygıtta çalıyor (Spotify Connect, bkz. `engine::connect`):
    /// bildirim şeridinin yerinde sakin bir satır ("<aygıt> cihazında çalıyor") ve
    /// çalmayı bu bilgisayara alan düğme. Uzak aygıta başka komut gitmez.
    fn remote_strip(&self, device: &str, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = pill_accent(self.accent().text);
        let (stop, look) =
            self.stop_look("remote-transfer", super::tab::HEART - 1, true, window, cx);
        let transfer = secondary_button(
            "remote-transfer",
            "arrow-left",
            t!(ConnectTransferHere),
            look,
            accent,
        )
        .when_some(stop, |el, handle| el.track_focus(&handle))
        .tooltip(Tooltip::text(t!(ConnectTransferHereTip), None))
        .on_click(cx.listener(|this, _, _, _| {
            this.engine.send(engine::Command::Connect(
                engine::ConnectCommand::TransferHere,
            ));
        }));
        div()
            .id("remote-strip")
            .absolute()
            .left(px(S6))
            .right(px(S6))
            .bottom(px(PILL_HEIGHT + S3))
            .min_h(px(S7))
            .pl(px(S4))
            .pr(px(S2))
            .py(px(S2))
            .flex()
            .items_center()
            .gap(px(S3))
            .rounded(px(R_XL))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().bar))
            // Altındaki satırlar tıklanmasın, üzerine gelmek onları boyamasın.
            .block_mouse_except_scroll()
            .child(icon("cast", 18., label()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .type_role(T_SECONDARY)
                    .text_color(soft())
                    .child(line_with_tip(
                        "remote-device",
                        tf!(ConnectPlayingOn, device),
                    )),
            )
            .child(transfer)
            .into_any_element()
    }

    /// Hapın solu: kapak ve iki satır (ya da bildirim, çevrimdışı, boş).
    fn pill_left(&self, m: &PillModel, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let two_lines = |first: AnyElement, second: AnyElement| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(S0))
                .child(first)
                .child(second)
        };
        let title = |content: AnyElement| {
            div()
                .type_role(T_BODY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(text())
                .child(content)
                .into_any_element()
        };
        let row = || div().flex().items_center().gap(px(S3));
        match &m.left {
            PillLeft::Track {
                uri,
                name,
                artists,
                cover,
                notice,
                offline,
            } => {
                let art = match cover.clone() {
                    // Köşe resmin kendisinde: kabın `overflow_hidden`'ı
                    // resmi kırpmıyor (kapak hapta kare çiziliyordu).
                    Some(image) => round(img(image).size_full().rounded_full().into_any_element()),
                    None => round_tile(icon("music", 18., label()).into_any_element()),
                };
                // Kapak tam ekranı açar: üzerine gelince ortasında simge.
                let art = div()
                    .id("np-cover")
                    .relative()
                    .flex_none()
                    .rounded_full()
                    .cursor_pointer()
                    .group("np-cover")
                    .child(art)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(cover_scrim(SCRIM))
                            .opacity(0.)
                            .group_hover("np-cover", |s| s.opacity(1.))
                            .child(icon("maximize-2", 18., cover_ink())),
                    )
                    .tooltip(Tooltip::text(t!(FullScreen), Some("F11")))
                    // Kapak tam ekranı açar, paneli değil (hapın on_click'i).
                    .block_mouse_except_scroll()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx)),
                    );
                let second = match notice {
                    Some(n) if n.kind == engine::NoticeKind::Working => self.notice_line(n, cx),
                    // Çevrimdışı: Çal'a basmadan da belli (7 Kasım kararı).
                    _ if *offline => offline_line(),
                    // Sanatçı adları tıklanabilir (Adım 12, `manage::pill_artists`).
                    _ => div()
                        .flex()
                        .type_role(T_SECONDARY)
                        .text_color(muted())
                        .child(self.pill_artists(uri, name, artists, cx))
                        .into_any_element(),
                };
                row()
                    .child(art)
                    .child(two_lines(
                        title(text::line_with_tip("np-name", name.clone()).into_any_element()),
                        second,
                    ))
                    // Beğen (Adım 13, `manage::pill_heart`).
                    .children(self.pill_heart(
                        uri,
                        name,
                        artists,
                        m.live.then_some(("pill-heart", super::tab::HEART, window)),
                        cx,
                    ))
                    // Şarkı değişince belirerek gelir.
                    .with_animation(
                        ElementId::Name(format!("np-{uri}").into()),
                        Animation::new(motion()).with_easing(ease_out_quint()),
                        |el, d| el.opacity(d).ml(px(-S2 * (1. - d))),
                    )
                    .into_any_element()
            }
            PillLeft::Pending {
                name,
                image,
                notice,
            } => {
                let second = match notice {
                    Some(n) if n.kind == engine::NoticeKind::Working => self.notice_line(n, cx),
                    _ => div()
                        .type_role(T_SECONDARY)
                        .text_color(label())
                        .child(line(t!(Loading)))
                        .into_any_element(),
                };
                row()
                    .child(cover_image(
                        &self.images,
                        image,
                        name,
                        PILL_COVER,
                        PILL_COVER / 2.,
                        "music",
                    ))
                    .child(two_lines(
                        title(text::line_with_tip("np-pending", name.clone()).into_any_element()),
                        second,
                    ))
                    .into_any_element()
            }
            PillLeft::Notice(notice) => {
                let (_, color) = Self::notice_style(notice.kind);
                row()
                    .child(round_tile(Self::notice_icon(notice.kind, 20.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .type_role(T_SECONDARY)
                            .text_color(color)
                            .child(notice.text.clone()),
                    )
                    .children(self.notice_dismiss(notice, cx))
                    .into_any_element()
            }
            PillLeft::Offline {
                icon: icon_name,
                title: heading,
                detail,
            } => {
                let look = offline_look(&self.state.connection);
                let tile_color: Hsla = if look.danger_icon {
                    rgb(palette().danger_text).into()
                } else {
                    label().into()
                };
                let content = row()
                    .child(round_tile(
                        icon(icon_name, 20., tile_color).into_any_element(),
                    ))
                    .child(two_lines(
                        // Dar hapta (bilgi 112 px'e iner) kısalabilir: tamamı ipucunda.
                        title(text::line_with_tip("np-off-title", *heading).into_any_element()),
                        // Tıklanınca bir şey yapan ikinci satır ("Yeniden başlat")
                        // bağlantı gibi: vurgu rengi, yarı kalın.
                        div()
                            .type_role(T_SECONDARY)
                            .when(look.accent_detail, |d| {
                                d.text_color(rgb(pill_accent(self.accent().text)))
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(!look.accent_detail, |d| d.text_color(muted()))
                            .child(line_with_tip("np-off-detail", *detail).flex().min_w_0())
                            .into_any_element(),
                    ));
                // Adım 17: giriş gerekirken / motor çökünce hapın bu bölümüne
                // tıklamak o adımı yapar (kütüphane önbellekten görünürken de
                // çıkmaz sokak olmasın). Hapın "paneli aç" tıklamasına gitmez.
                match self.state.connection {
                    Connection::NeedsLogin
                    | Connection::LoginFailed(_)
                    | Connection::EngineStopped => content
                        .id("np-offline-action")
                        .cursor_pointer()
                        .block_mouse_except_scroll()
                        .on_click(cx.listener(|this, _, _, cx| match this.state.connection {
                            Connection::EngineStopped => this.restart_engine(cx),
                            _ => this.start_login(cx),
                        }))
                        .into_any_element(),
                    _ => content.into_any_element(),
                }
            }
            PillLeft::Idle => row()
                .child(round_tile(icon("music", 18., label()).into_any_element()))
                .child(two_lines(
                    div()
                        .type_role(T_BODY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(soft())
                        .child(line(t!(NothingPlaying)))
                        .into_any_element(),
                    div()
                        .type_role(T_SECONDARY)
                        .text_color(label())
                        .child(
                            line_with_tip("np-start", start_name(m.start_title.as_deref()))
                                .flex()
                                .min_w_0(),
                        )
                        .into_any_element(),
                ))
                .into_any_element(),
        }
    }

    /// Bildirimin simgesi ve rengi: süren iş sakin (dönen ok), bilgi soluk,
    /// sorun kırmızı. Kırmızı yalnız gerçekten bir şey ters gittiğinde.
    fn notice_style(kind: engine::NoticeKind) -> (&'static str, Hsla) {
        match kind {
            engine::NoticeKind::Working => ("refresh-cw", label().into()),
            engine::NoticeKind::Info => ("info", muted().into()),
            engine::NoticeKind::Problem => ("circle-alert", rgb(palette().danger_text).into()),
        }
    }

    /// Bildirim simgesi; süren işte yavaşça döner (animasyonlar açıksa).
    fn notice_icon(kind: engine::NoticeKind, size: f32) -> AnyElement {
        let (name, color) = Self::notice_style(kind);
        let svg = icon(name, size, color);
        if kind == engine::NoticeKind::Working && animations_enabled() {
            svg.with_animation(
                "notice-spin",
                Animation::new(Duration::from_millis(1200)).repeat(),
                |svg, d| svg.with_transformation(Transformation::rotate(percentage(d))),
            )
            .into_any_element()
        } else {
            svg.into_any_element()
        }
    }

    /// Şarkı bilgisinin ikinci satırı yerine geçen bildirim satırı.
    fn notice_line(&self, notice: &player::Notice, _cx: &mut Context<Self>) -> AnyElement {
        let (_, color) = Self::notice_style(notice.kind);
        // Süren işin yazısı hapta etiket değil ikincil tonda: kapağın ışığı
        // bu satırın arkasında en parlak (bkz. `PILL_LIGHT_MAX_LUMINANCE`);
        // simge soluk kalır.
        let color: Hsla = if notice.kind == engine::NoticeKind::Working {
            muted().into()
        } else {
            color
        };
        div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(S1))
            .type_role(T_SECONDARY)
            .text_color(color)
            .child(Self::notice_icon(notice.kind, 14.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(text::line_with_tip("np-notice", notice.text.clone())),
            )
            .into_any_element()
    }

    /// Sorun bildirimi kendiliğinden kalkmaz: kapatma düğmesi (tıklama alanı
    /// 32 px). (Süren iş çözülünce, bilgi birkaç saniyede kalkar.)
    fn notice_dismiss(
        &self,
        notice: &player::Notice,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if notice.kind != engine::NoticeKind::Problem {
            return None;
        }
        Some(self.dismiss_button(cx))
    }

    /// Bildirimi kapatan 32 px'lik düğme.
    fn dismiss_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("notice-dismiss")
            // Kapatmak hapın paneli açan tıklamasına gitmesin.
            .block_mouse_except_scroll()
            .flex_none()
            .size(px(S6))
            .rounded(px(R_MD))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(super::tween::hover_fill(
                veil(VEIL_HOVER),
                veil(VEIL_PRESSED),
                0.,
                R_MD,
            ))
            .tooltip(Tooltip::text(t!(Close), None))
            .on_click(cx.listener(|this, _, _, cx| {
                this.state.notice = None;
                cx.notify();
            }))
            .child(icon("x", 14., label()))
            .into_any_element()
    }

    /// Çalarken gelen bilgi / sorun bildirimi: hapın üstünde şerit (hapın düz üst
    /// kenarının içinde: iki yanda `S6` içeride, alt kenarı hapın üst kenarından
    /// `S3` yukarıda). Yazı sarılır (satır sınırı yok); kapatma düğmesi her
    /// türde var. Sorunda yalnız simge kırmızı; yazı okunur kalır (`soft`).
    fn notice_toast(&self, notice: &player::Notice, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("notice-toast")
            .absolute()
            .left(px(S6))
            .right(px(S6))
            .bottom(px(PILL_HEIGHT + S3))
            .min_h(px(S7))
            .pl(px(S4))
            .pr(px(S2))
            .py(px(S2))
            .flex()
            .items_center()
            .gap(px(S3))
            .rounded(px(R_XL))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().bar))
            // Gölgesi yok: hapın gölgesiyle üst üste binen ikinci bir gölge
            // şeffaf piksel çizgileri bırakıyordu (bkz. `pill_shadow`).
            // Altındaki satırlar tıklanmasın, üzerine gelmek onları boyamasın.
            .block_mouse_except_scroll()
            .child(Self::notice_icon(notice.kind, 18.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .type_role(T_SECONDARY)
                    .text_color(soft())
                    .child(notice.text.clone()),
            )
            .child(self.dismiss_button(cx))
            .into_any_element()
    }
}

/// Çevrimdışı hap satırının görünümü, bağlantı durumundan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OfflineLook {
    /// İkinci satır bir eylem: vurgu rengi + yarı kalın (tıklanınca giriş /
    /// yeniden başlatma).
    accent_detail: bool,
    /// Karo simgesi kırmızı (bir şey gerçekten ters gitti).
    danger_icon: bool,
}

fn offline_look(connection: &Connection) -> OfflineLook {
    OfflineLook {
        accent_detail: matches!(
            connection,
            Connection::NeedsLogin | Connection::LoginFailed(_) | Connection::EngineStopped
        ),
        danger_icon: matches!(
            connection,
            Connection::LoginFailed(_) | Connection::EngineStopped
        ),
    }
}

/// Kaldığı yerden açılan şarkı çevrimdışıyken ikinci satır: bildirim
/// satırının düzeninde soluk wifi-off simgesi ve durum.
fn offline_line() -> AnyElement {
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(S1))
        .type_role(T_SECONDARY)
        .text_color(muted())
        .child(icon("wifi-off", 14., label()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(text::line_with_tip("np-offline", t!(OfflineFull))),
        )
        .into_any_element()
}

/// Boştaki hapın ikinci satırı: açık listenin adı ("Çal: " önekiyle gelen
/// başlıktan öneksiz; çal düğmesinin ipucu önekli kalır).
fn start_name(start_title: Option<&str>) -> String {
    let title = start_title.unwrap_or_default();
    title.to_string()
}

/// Yuvarlak kapak çerçevesi.
fn round(child: AnyElement) -> AnyElement {
    div()
        .size(px(PILL_COVER))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(palette().placeholder))
        .child(child)
        .into_any_element()
}

/// Kapak yerinde sessiz ton karosu ve ortada simge.
fn round_tile(content: AnyElement) -> AnyElement {
    div()
        .size(px(PILL_COVER))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(palette().placeholder))
        .flex()
        .items_center()
        .justify_center()
        .child(content)
        .into_any_element()
}

/// Kaydırıcı tutamacının odak halkası: dolu kısmın renginden ayrışsın.
fn fill_ring(fill: u32) -> u32 {
    if fill == palette().text {
        DEFAULT_ACCENT.text
    } else {
        mix(fill, 0xFFFFFF, 0.5)
    }
}

/// Açılıp kapanan simge düğmesi (karıştır, tekrar, sıra, sözler): 36 px
/// yuvarlak. Açıkken simge kapak ışığı renginde ve altında 4 px'lik nokta
/// (yalnız renge bakmadan da durum belli); kapalıyken simge düğmesi gibi.
pub(super) fn toggle_button(
    id: impl Into<ElementId>,
    icon_name: &'static str,
    on: bool,
    look: Look,
    accent: u32,
) -> Stateful<Div> {
    let color: Hsla = match (look.enabled(), on) {
        (false, _) => disabled().into(),
        (true, true) => rgb(accent).into(),
        (true, false) => soft().into(),
    };
    let el = div()
        .id(id)
        .size(px(36.))
        .flex_shrink_0()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_2()
        .border_color(if look == Look::Focus {
            rgb(accent).into()
        } else {
            gpui::transparent_black()
        })
        .children(super::components::veil_layer(look, super::components::FULL))
        .child(icon(icon_name, 20., color))
        .when(on, |el| {
            // 36 px'lik düğmede (2 px kenar) ortada, simgenin hemen altında.
            el.child(
                div()
                    .absolute()
                    .bottom(px(S0 / 2.))
                    .left(px((36. - 4. - S1) / 2.))
                    .size(px(S1))
                    .rounded_full()
                    .bg(color),
            )
        });
    match look {
        Look::Live | Look::Focus => el.cursor_pointer(),
        Look::Hover => el.bg(veil(VEIL_HOVER)),
        Look::Pressed => el.bg(veil(VEIL_PRESSED)),
        Look::Disabled => el,
    }
}

/// Tekrar düğmesinin ipucu: basınca ne olacağı.
pub(super) fn repeat_tip(repeat: RepeatMode) -> &'static str {
    match repeat {
        RepeatMode::Off => t!(RepeatTipOff),
        RepeatMode::All => t!(RepeatTipAll),
        RepeatMode::One => t!(RepeatTipOne),
    }
}

pub(super) type OnDown = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

/// İnce, tıklanabilir ve sürüklenebilir yatay çubuk (ilerleme, ses).
/// Tıklama alanı 32 px yüksek, çizgi 4 px. Üzerine gelince ucunda bir tutamaç
/// belirir ve dolu kısım tam beyaza döner. `focus` yoksa (vitrin) Tab
/// sırasına girmez; `on_down` yoksa tıklanmaz.
pub(super) fn slider(
    id: &'static str,
    value: f32,
    fill: u32,
    enabled: bool,
    focus: Option<(FocusHandle, bool)>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    on_down: Option<OnDown>,
) -> AnyElement {
    slider_hit(id, value, fill, enabled, focus, bounds, on_down, SLIDER_HIT)
}

/// `slider`, tıklama alanı `hit` px yüksek (ilerleme satırında 20 px: altındaki
/// ve üstündeki düğmelerin alanına taşmasın).
#[allow(clippy::too_many_arguments)]
pub(super) fn slider_hit(
    id: &'static str,
    value: f32,
    fill: u32,
    enabled: bool,
    focus: Option<(FocusHandle, bool)>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    on_down: Option<OnDown>,
    hit: f32,
) -> AnyElement {
    let value = value.clamp(0.0, 1.0);
    let group: SharedString = format!("slider-{id}").into();
    let ring = focus.as_ref().is_some_and(|(_, ring)| *ring);
    div()
        .id(id)
        // Kullanılamayan çubuk Tab sırasına girmez.
        .when_some(focus.filter(|_| enabled), |el, (handle, _)| {
            el.track_focus(&handle)
        })
        .group(group.clone())
        .flex_1()
        .h(px(hit))
        .relative()
        .flex()
        .items_center()
        .when_some(on_down.filter(|_| enabled), |el, on_down| {
            el.cursor_pointer()
                .on_mouse_down(MouseButton::Left, on_down)
        })
        // Çubuğun ekrandaki yeri (tıklamayı orana çevirmek için).
        .child(
            canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
        .child(
            div()
                .w_full()
                .h(px(4.))
                .rounded(px(R_XS))
                .bg(rgb(palette().track))
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(value))
                        .rounded(px(R_XS))
                        .bg(rgb(fill))
                        .when(enabled, |el| {
                            el.group_hover(group.clone(), |s| s.bg(rgb(0xFFFFFF)))
                        }),
                ),
        )
        .when(enabled, |el| {
            el.child(
                div()
                    .absolute()
                    .top(px((hit - SLIDER_THUMB) / 2.))
                    .left(gpui::relative(value))
                    .ml(px(-SLIDER_THUMB / 2.))
                    .size(px(SLIDER_THUMB))
                    .rounded_full()
                    .bg(rgb(0xFFFFFF))
                    .shadow(pill_shadow())
                    // Klavye odağında tutamaç görünür ve vurgu renginde halka alır.
                    .when(ring, |el| el.border_2().border_color(rgb(fill_ring(fill))))
                    .opacity(if ring { 1. } else { 0. })
                    .group_hover(group, |s| s.opacity(1.)),
            )
        })
        .into_any_element()
}

/// Fare tıklamasının çubuk üzerindeki oranı (0.0–1.0).
pub(super) fn fraction(
    bounds: &Rc<Cell<Option<Bounds<Pixels>>>>,
    position: Point<Pixels>,
) -> Option<f32> {
    let b = bounds.get()?;
    let x = (position.x - b.origin.x) / b.size.width;
    Some(x.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_row_is_styled_as_an_action_only_when_it_is_one() {
        use crate::engine::LoginFailReason;
        let look = offline_look;
        // Giriş gerekli: eylem (vurgu + yarı kalın), simge sakin.
        assert_eq!(
            look(&Connection::NeedsLogin),
            OfflineLook {
                accent_detail: true,
                danger_icon: false
            }
        );
        // Motor durdu / giriş başarısız: eylem ve simge kırmızı.
        assert_eq!(
            look(&Connection::EngineStopped),
            OfflineLook {
                accent_detail: true,
                danger_icon: true
            }
        );
        assert_eq!(
            look(&Connection::LoginFailed(LoginFailReason::Other)),
            OfflineLook {
                accent_detail: true,
                danger_icon: true
            }
        );
        // Premium gerekli: tıklanacak bir şey yok, görünüm değişmez.
        assert_eq!(
            look(&Connection::NeedsPremium),
            OfflineLook {
                accent_detail: false,
                danger_icon: false
            }
        );
        // Çevrimdışı ve ses yok da değişmez.
        assert_eq!(
            look(&Connection::Failed(String::new())),
            OfflineLook {
                accent_detail: false,
                danger_icon: false
            }
        );
        assert_eq!(
            look(&Connection::NoAudio),
            OfflineLook {
                accent_detail: false,
                danger_icon: false
            }
        );
        assert_eq!(
            look(&Connection::LoggingIn),
            OfflineLook {
                accent_detail: false,
                danger_icon: false
            }
        );
    }

    #[test]
    fn pill_follows_narrow_windows() {
        assert_eq!(pill_width(1180.), PILL_MAX_WIDTH);
        // En küçük pencere: iki yanda 32 px.
        assert_eq!(pill_width(760.), 696.);
        assert_eq!(pill_width(864.), PILL_MAX_WIDTH);
    }
}
