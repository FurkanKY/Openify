//! Çalar dikişi: motorun çalardan istediği komutlar (`PlayerPort`) ve çaldan
//! gelen olayların motor dili (`PlayerSignal`).
//!
//! Motor librespot'un `Player`ını doğrudan görmez; komutları `PlayerPort`
//! üzerinden verir, olayları `PlayerSignal` olarak alır. Böylece çalma
//! kararları (sıra, kurtarma, boşluksuz geçiş) çalar olmadan, sahte bir
//! porta (`testkit::FakePort`) karşı sınanır. Gerçek uç `LibrespotPort`.

use std::sync::Arc;

use librespot::{
    core::{SpotifyUri, session::Session},
    metadata::audio::{AudioItem, UniqueFields},
    playback::player::{Player, PlayerEvent},
};

use crate::engine::TrackInfo;

/// Motorun çalara verdiği komutlar. Hepsi beklemeden döner (çalar kendi iş
/// parçacığında işler).
pub(in crate::engine) trait PlayerPort: Send {
    /// Şarkıyı yükle (`play`: yüklenince çalsın, `position_ms`: başlangıç).
    fn load(&self, uri: SpotifyUri, play: bool, position_ms: u32);
    /// Sıradaki şarkıyı önyükle (boşluksuz geçiş için).
    fn preload(&self, uri: SpotifyUri);
    fn play(&self);
    fn pause(&self);
    fn stop(&self);
    fn seek(&self, position_ms: u32);
    /// Çıkış kapısının işareti (bkz. `output/mod.rs`, "Eski ses nasıl
    /// ayıklanıyor"): çalar bunu kendi olay kanalından geri yollar,
    /// `SessionClientChanged { client_id: "openify-mark", client_name: işaret }`
    /// olarak (`VolumeChanged` Spirc'in ses düzeyi bildirimiyle karışırdı).
    fn mark(&self, mark: u64);
    /// Yeniden bağlanınca yeni oturumu çalara ver.
    fn rebind(&self, session: &Session);
    /// Çaların bugüne kadar bildirdiği anahtar hatası sayısı (artışı izlenir).
    fn key_failures(&self) -> u32;
    /// Gerçek librespot çaları (Spirc'i kurmak için; sahte portta yok).
    fn player(&self) -> Option<Arc<Player>> {
        None
    }
}

/// Çaldan gelen olayların motor dili. Konum bildirenler (`Seeked`,
/// `PositionCorrection`) tek `Position`da toplanır; motorun işlemediği
/// olaylar (`from_librespot` → `None`) hiç oluşmaz.
#[derive(Debug, Clone)]
pub(in crate::engine) enum PlayerSignal {
    Loading,
    Playing { position_ms: u32 },
    Paused { position_ms: u32 },
    Position { position_ms: u32 },
    /// Sıradaki şarkıyı önyüklemenin zamanı (`TimeToPreloadNextTrack`).
    PreloadDue,
    EndOfTrack,
    /// Şarkı çalınamadı; `uri`: hangi şarkı (adresi çözülemediyse `None`).
    Unavailable { uri: Option<String> },
    TrackChanged(TrackInfo),
    /// Başka bir cihaz (telefon) bu aygıtı sürmeye başladı / bıraktı. Bant
    /// içi: Spirc etkinleşirken / etkinlikten çıkarken çaldan geçen
    /// `SessionConnected` / `SessionDisconnected` olayıdır; çalar olay
    /// sırasında Spirc'in o andan sonraki komutlarının ÖNÜNDE gelir (bkz.
    /// `connect.rs`, `PLAN-MOTOR-CONNECT.md` 7.5).
    RemoteAttached,
    RemoteDetached,
}

impl PlayerSignal {
    pub fn from_librespot(event: PlayerEvent) -> Option<Self> {
        Some(match event {
            PlayerEvent::Loading { .. } => Self::Loading,
            PlayerEvent::Playing { position_ms, .. } => Self::Playing { position_ms },
            PlayerEvent::Paused { position_ms, .. } => Self::Paused { position_ms },
            PlayerEvent::Seeked { position_ms, .. } | PlayerEvent::PositionCorrection { position_ms, .. } => {
                Self::Position { position_ms }
            }
            PlayerEvent::TimeToPreloadNextTrack { .. } => Self::PreloadDue,
            PlayerEvent::EndOfTrack { .. } => Self::EndOfTrack,
            PlayerEvent::Unavailable { track_id, .. } => Self::Unavailable { uri: track_id.to_uri().ok() },
            PlayerEvent::TrackChanged { audio_item } => Self::TrackChanged(track_info(&audio_item)),
            PlayerEvent::SessionConnected { .. } => Self::RemoteAttached,
            PlayerEvent::SessionDisconnected { .. } => Self::RemoteDetached,
            _ => return None,
        })
    }
}

/// Gerçek uç: librespot'un çaları.
pub(in crate::engine) struct LibrespotPort(pub Arc<Player>);

impl PlayerPort for LibrespotPort {
    fn load(&self, uri: SpotifyUri, play: bool, position_ms: u32) {
        self.0.load(uri, play, position_ms);
    }

    fn preload(&self, uri: SpotifyUri) {
        self.0.preload(uri);
    }

    fn play(&self) {
        self.0.play();
    }

    fn pause(&self) {
        self.0.pause();
    }

    fn stop(&self) {
        self.0.stop();
    }

    fn seek(&self, position_ms: u32) {
        self.0.seek(position_ms);
    }

    fn mark(&self, mark: u64) {
        self.0.emit_session_client_changed_event(
            crate::engine::output::MARK_CLIENT_ID.to_string(),
            mark.to_string(),
            String::new(),
            String::new(),
        );
    }

    fn rebind(&self, session: &Session) {
        self.0.set_session(session.clone());
    }

    fn key_failures(&self) -> u32 {
        librespot::playback::config::live::key_failures()
    }

    fn player(&self) -> Option<Arc<Player>> {
        Some(self.0.clone())
    }
}

pub(in crate::engine) fn track_info(item: &AudioItem) -> TrackInfo {
    let artists = match &item.unique_fields {
        UniqueFields::Track { artists, .. } => artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };

    let cover_url = item
        .covers
        .iter()
        .max_by_key(|c| c.width)
        .map(|c| c.url.clone());

    TrackInfo {
        uri: item.uri.clone(),
        name: item.name.clone(),
        artists,
        duration_ms: item.duration_ms,
        cover_url,
    }
}
