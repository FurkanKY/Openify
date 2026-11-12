//! Çaların arayüz tarafındaki durumu.
//!
//! Ses motorundan gelen olayların kopyası burada tutulur. Konum her
//! milisaniye motordan sorulmaz: son bilinen konum ve o anın zamanı
//! saklanır, çalarken aradan geçen süre eklenerek hesaplanır.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use super::theme::{DARK_ROOM, DEFAULT_ACCENT, accent_fade, ease_out, mix};
use crate::engine::{Event, NoticeKind, QueueState, TrackInfo, palette::Accent};
use crate::t;

/// Çalar hapında gösterilen kısa bildirim.
#[derive(Debug, Clone)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    pub at: Instant,
}

/// Bilgi bildirimi bu kadar görünür, sonra kendiliğinden kalkar.
pub const INFO_NOTICE: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq)]
pub enum Connection {
    Connecting,
    Connected(String),
    /// Çevrimdışı (ağ yok); motor kendiliğinden yeniden dener.
    Failed(String),
    /// Bağlı ama ses çıkışı yok (hoparlör, kulaklık).
    NoAudio,
    /// Kayıtlı giriş yok ya da reddedildi: "Giriş yap" gerekir (Adım 17).
    NeedsLogin,
    /// Tarayıcıda giriş bekleniyor ("Girişi iptal et" mümkün).
    LoggingIn,
    /// Giriş tamamlanamadı (neden kodu); "Giriş yap" yeniden denenebilir.
    LoginFailed(crate::engine::LoginFailReason),
    /// Hesap ücretsiz: Openify yalnız Spotify Premium'la çalar.
    NeedsPremium,
    /// Ses motoru çöktü: "Yeniden başlat" gerekir.
    EngineStopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Henüz şarkı yok.
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
    Unavailable,
}

/// Yüklenmekte olan şarkının ekranda gösterilecek bilgisi.
#[derive(Clone)]
pub struct PendingTrack {
    pub name: String,
    /// Küçük kapak adresi (kapak önbelleğinden).
    pub image: String,
}

pub struct PlayerState {
    pub connection: Connection,
    pub track: Option<TrackInfo>,
    pub cover: Option<Arc<gpui::Image>>,
    /// Yerini yenisine bırakmış kapaklar (bkz. `take_retired`).
    retired: Vec<Arc<gpui::Image>>,
    pub status: Status,
    /// 0.0–1.0
    pub volume: f32,
    /// Kullanıcıya gösterilecek kısa bildirim (çalar hapında görünür).
    pub notice: Option<Notice>,
    /// Hangi listenin kaçıncı şarkısı çalıyor.
    pub queue: QueueState,
    /// Çal'a basılan ama henüz yüklenmemiş şarkı: çalar hapı yükleme
    /// bitene kadar onu "Yükleniyor…" ile gösterir. (Spotify anahtarı geç
    /// verince yükleme saniyeler sürebiliyor; çubuk boş kalıyordu.)
    pub pending: Option<PendingTrack>,
    /// Çalan şarkının kapağından türetilen vurgu rengi (hedef).
    accent: Accent,
    /// Geçiş başlamadan önceki renk ve geçişin başladığı an.
    accent_from: Accent,
    accent_changed: Instant,
    anchor_ms: u32,
    anchor_at: Instant,
}

impl PlayerState {
    pub fn new(volume: f32) -> Self {
        Self {
            connection: Connection::Connecting,
            track: None,
            cover: None,
            retired: Vec::new(),
            status: Status::Idle,
            volume,
            notice: None,
            queue: QueueState::default(),
            pending: None,
            accent: DARK_ROOM,
            accent_from: DARK_ROOM,
            accent_changed: Instant::now(),
            anchor_ms: 0,
            anchor_at: Instant::now(),
        }
    }

    pub fn apply(&mut self, event: Event, now: Instant) {
        match event {
            Event::Connecting => self.connection = Connection::Connecting,
            Event::Connected { username } => {
                // Bağlantı geri geldi: "yeniden deneniyor" bildirimi bitti.
                if self
                    .notice
                    .as_ref()
                    .is_some_and(|n| n.kind == NoticeKind::Working)
                {
                    self.notice = None;
                }
                self.connection = Connection::Connected(username);
            }
            Event::Reconnecting => self.notify(NoticeKind::Working, reconnecting()),
            Event::Notice { kind, message } => self.notify(kind, message),
            Event::ConnectFailed(e) => {
                // Bağlantı yokken bekleyen şarkı hiç yüklenmeyecek.
                self.pending = None;
                self.connection = Connection::Failed(e);
            }
            Event::NoAudioOutput => {
                self.pending = None;
                self.connection = Connection::NoAudio;
            }
            // Adım 17: giriş / Premium / motor durumları. Bekleyen şarkı hiç
            // yüklenmeyecek (çalma komutu motorda düştü).
            Event::NeedsLogin => {
                self.pending = None;
                self.connection = Connection::NeedsLogin;
            }
            Event::LoggingIn => self.connection = Connection::LoggingIn,
            Event::LoginFailed(reason) => {
                self.pending = None;
                self.connection = Connection::LoginFailed(reason);
            }
            Event::NeedsPremium => {
                self.pending = None;
                self.connection = Connection::NeedsPremium;
            }
            // Giriş / Premium gerektiği için düşen çalma komutu: motor KOD yollar,
            // yazıya burada çevrilir.
            Event::PlaybackBlocked(block) => {
                self.notify(NoticeKind::Problem, crate::msg::play_block_text(block))
            }
            Event::EngineStopped(reason) => {
                log::error!("ses motoru durdu: {reason}");
                self.pending = None;
                self.connection = Connection::EngineStopped;
            }
            Event::QueueChanged(queue) => self.queue = queue,
            // Kütüphane olayları çaları ilgilendirmiyor (Shell işliyor).
            Event::LibraryLoaded(_)
            | Event::LibraryFailed(_)
            | Event::CollectionLoaded { .. }
            | Event::CollectionFailed { .. }
            | Event::SearchIndex(_)
            | Event::SearchResults { .. }
            | Event::SearchFailed { .. }
            | Event::ImageReady(_)
            | Event::Library(_)
            | Event::Lyrics { .. } => {}
            Event::TrackChanged(info) => {
                self.pending = None;
                // Işık kapakla yanar (`Cover`); kapağı olmayan şarkıda
                // varsayılan ışıkla.
                if info.cover_url.is_none() && self.accent == DARK_ROOM {
                    self.set_accent(DEFAULT_ACCENT, now);
                }
                self.track = Some(info);
                self.retire_cover();
                // Yeni şarkı geldi: süren ve sorun bildirimleri bitti. Bilgi
                // ("önceki şarkı çalınamadı") tam bu an için: süresi dolana
                // kadar kalır.
                if self
                    .notice
                    .as_ref()
                    .is_some_and(|n| n.kind != NoticeKind::Info)
                {
                    self.notice = None;
                }
                self.set_position(0, now);
            }
            Event::Cover {
                track_uri,
                bytes,
                accent,
            } => {
                if self.track.as_ref().is_some_and(|t| t.uri == track_uri) {
                    self.retire_cover();
                    self.cover = Some(Arc::new(gpui::Image::from_bytes(
                        gpui::ImageFormat::Jpeg,
                        bytes,
                    )));
                    self.set_accent(accent.unwrap_or(DEFAULT_ACCENT), now);
                }
            }
            Event::Loading => self.status = Status::Loading,
            Event::Playing { position_ms } => {
                self.status = Status::Playing;
                self.set_position(position_ms, now);
            }
            Event::Paused { position_ms } => {
                self.status = Status::Paused;
                self.set_position(position_ms, now);
            }
            Event::Position { position_ms } => self.set_position(position_ms, now),
            Event::Ended => {
                self.pending = None;
                self.status = Status::Ended;
                let end = self.duration_ms();
                self.set_position(end, now);
            }
            Event::Unavailable => {
                self.pending = None;
                self.status = Status::Unavailable;
                if self.notice.is_none() {
                    self.notify(NoticeKind::Problem, t!(PlayDenied));
                }
            }
        }
    }

    /// Çalan kapağı bırakır; ama gpui'nin varlık önbelleği (çözülmüş piksel
    /// verisi) resmi kendiliğinden bırakmaz, her şarkı için bir kapak daha
    /// birikir. Bu yüzden eski kapak `take_retired` ile arayüze verilir ve
    /// orada `Image::remove_asset` ile önbellekten atılır.
    fn retire_cover(&mut self) {
        if let Some(old) = self.cover.take() {
            self.retired.push(old);
        }
    }

    /// Artık gösterilmeyen kapaklar: çağıran her biri için
    /// `image.remove_asset(cx)` çağırmalı (`apply`'dan hemen sonra).
    pub fn take_retired(&mut self) -> Vec<Arc<gpui::Image>> {
        std::mem::take(&mut self.retired)
    }

    pub fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Some(Notice {
            kind,
            text: text.into(),
            at: Instant::now(),
        });
    }

    /// Şu an görünmesi gereken bildirim (süresi dolan bilgi görünmez).
    pub fn visible_notice(&self, now: Instant) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|n| n.kind != NoticeKind::Info || now.duration_since(n.at) < INFO_NOTICE)
    }

    fn set_accent(&mut self, accent: Accent, now: Instant) {
        if accent == self.accent {
            return;
        }
        // Geçişin ortasında yeni renk gelirse o anki karışımdan devam et.
        self.accent_from = self.accent_at(now);
        self.accent = accent;
        self.accent_changed = now;
    }

    /// O anki vurgu rengi (geçiş sürüyorsa iki rengin karışımı).
    pub fn accent_at(&self, now: Instant) -> Accent {
        let t = ease_out(
            now.saturating_duration_since(self.accent_changed)
                .as_secs_f32()
                / accent_fade().as_secs_f32(),
        );
        Accent {
            text: mix(self.accent_from.text, self.accent.text, t),
            glow: mix(self.accent_from.glow, self.accent.glow, t),
        }
    }

    /// Renk geçişi sürüyor mu? (Sürüyorsa her karede yeniden çizilmeli.)
    pub fn accent_animating(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.accent_changed) < accent_fade()
    }

    /// Kullanıcı sardığında motorun cevabını beklemeden çubuğu hemen taşı.
    pub fn set_position(&mut self, position_ms: u32, now: Instant) {
        self.anchor_ms = position_ms;
        self.anchor_at = now;
    }

    pub fn duration_ms(&self) -> u32 {
        self.track.as_ref().map_or(0, |t| t.duration_ms)
    }

    pub fn position_ms(&self, now: Instant) -> u32 {
        let elapsed = if self.status == Status::Playing {
            now.saturating_duration_since(self.anchor_at).as_millis() as u32
        } else {
            0
        };
        let position = self.anchor_ms.saturating_add(elapsed);
        match self.duration_ms() {
            0 => position,
            d => position.min(d),
        }
    }

    /// 0.0–1.0
    pub fn progress(&self, now: Instant) -> f32 {
        match self.duration_ms() {
            0 => 0.0,
            d => self.position_ms(now) as f32 / d as f32,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.status == Status::Playing
    }
}

fn reconnecting() -> &'static str {
    t!(Reconnecting)
}

pub fn fmt_time(ms: u32) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn track(duration_ms: u32) -> TrackInfo {
        TrackInfo {
            uri: "spotify:track:x".into(),
            name: "Şarkı".into(),
            artists: "Sanatçı".into(),
            duration_ms,
            cover_url: None,
        }
    }

    #[test]
    fn position_advances_only_while_playing() {
        let t0 = Instant::now();
        let mut s = PlayerState::new(0.8);
        s.apply(Event::TrackChanged(track(200_000)), t0);
        s.apply(
            Event::Playing {
                position_ms: 10_000,
            },
            t0,
        );
        assert_eq!(s.position_ms(t0 + Duration::from_secs(5)), 15_000);

        s.apply(
            Event::Paused {
                position_ms: 15_000,
            },
            t0 + Duration::from_secs(5),
        );
        assert_eq!(s.position_ms(t0 + Duration::from_secs(60)), 15_000);
    }

    #[test]
    fn position_never_passes_duration() {
        let t0 = Instant::now();
        let mut s = PlayerState::new(0.8);
        s.apply(Event::TrackChanged(track(3_000)), t0);
        s.apply(Event::Playing { position_ms: 0 }, t0);
        assert_eq!(s.position_ms(t0 + Duration::from_secs(10)), 3_000);
        assert_eq!(s.progress(t0 + Duration::from_secs(10)), 1.0);
    }

    #[test]
    fn cover_for_previous_track_is_ignored() {
        let t0 = Instant::now();
        let mut s = PlayerState::new(0.8);
        s.apply(Event::TrackChanged(track(1_000)), t0);
        s.apply(
            Event::Cover {
                track_uri: "spotify:track:baska".into(),
                bytes: vec![1, 2, 3],
                accent: None,
            },
            t0,
        );
        assert!(s.cover.is_none());
    }

    #[test]
    fn replaced_cover_is_handed_back_for_release() {
        let t0 = Instant::now();
        let mut s = PlayerState::new(0.8);
        let cover = |uri: &str| Event::Cover {
            track_uri: uri.into(),
            bytes: vec![1, 2, 3],
            accent: None,
        };
        let first = track(1_000);
        let uri = first.uri.clone();
        s.apply(Event::TrackChanged(first), t0);
        s.apply(cover(&uri), t0);
        assert!(s.cover.is_some());
        assert!(s.take_retired().is_empty());

        // Aynı şarkının kapağı yeniden gelirse eskisi bırakılır.
        s.apply(cover(&uri), t0);
        assert_eq!(s.take_retired().len(), 1);

        // Şarkı değişince çalan kapak bırakılır.
        s.apply(Event::TrackChanged(track(2_000)), t0);
        assert!(s.cover.is_none());
        assert_eq!(s.take_retired().len(), 1);
        assert!(s.take_retired().is_empty());
    }

    #[test]
    fn formats_time() {
        assert_eq!(fmt_time(0), "0:00");
        assert_eq!(fmt_time(65_400), "1:05");
        assert_eq!(fmt_time(3_600_000), "60:00");
    }
}
