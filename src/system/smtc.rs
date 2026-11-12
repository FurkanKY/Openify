//! Windows medya kontrolü (System Media Transport Controls): ses
//! düzeyi göstergesinin yanındaki kart, kilit ekranı ve medya tuşları.
//!
//! Masaüstü uygulaması kontrolü pencereden alır
//! (`ISystemMediaTransportControlsInterop::GetForWindow`). Düğme olayları
//! Windows'un iş parçacığında gelir: yalnız kanala bırakılır.

use std::time::Instant;

use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
    Win32::{Foundation::HWND, System::WinRT::ISystemMediaTransportControlsInterop},
    core::{HSTRING, Result},
};

use super::{MediaButton, NowPlaying, PlayStatus, SystemEvent, emit};

pub struct Smtc {
    controls: SystemMediaTransportControls,
}

impl Smtc {
    pub fn new(hwnd: HWND) -> Result<Self> {
        let interop = windows::core::factory::<
            SystemMediaTransportControls,
            ISystemMediaTransportControlsInterop,
        >()?;
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
        controls.SetIsEnabled(true)?;
        controls.SetIsPlayEnabled(false)?;
        controls.SetIsPauseEnabled(false)?;
        controls.SetIsNextEnabled(false)?;
        controls.SetIsPreviousEnabled(false)?;
        controls.SetIsStopEnabled(false)?;
        controls.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
        controls.ButtonPressed(&TypedEventHandler::new(
            |_, args: windows::core::Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    let button = match args.Button()? {
                        SystemMediaTransportControlsButton::Play => Some(MediaButton::Play),
                        SystemMediaTransportControlsButton::Pause => Some(MediaButton::Pause),
                        SystemMediaTransportControlsButton::Next => Some(MediaButton::Next),
                        SystemMediaTransportControlsButton::Previous => Some(MediaButton::Previous),
                        SystemMediaTransportControlsButton::Stop => Some(MediaButton::Stop),
                        _ => None,
                    };
                    if let Some(button) = button {
                        emit(SystemEvent::Media(button));
                    }
                }
                Ok(())
            },
        ))?;
        controls.PlaybackPositionChangeRequested(&TypedEventHandler::new(
            |_, args: windows::core::Ref<PlaybackPositionChangeRequestedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    // TimeSpan 100 ns biriminde.
                    let ms = (args.RequestedPlaybackPosition()?.Duration / 10_000).max(0);
                    emit(SystemEvent::SeekTo(ms.min(u32::MAX as i64) as u32));
                }
                Ok(())
            },
        ))?;
        Ok(Smtc { controls })
    }

    /// Şarkı adı, sanatçı, albüm ve kapak.
    pub fn set_metadata(&mut self, now: &NowPlaying) {
        let t = Instant::now();
        if let Err(e) = self.try_set_metadata(now) {
            log::warn!("Medya kontrolüne şarkı bilgisi verilemedi: {e}");
        }
        log::info!(
            "sistem: medya kontrolü bilgisi \"{}\" ({}) {:.2} ms",
            now.title,
            now.cover
                .as_ref()
                .map_or("kapaksız".to_string(), |c| format!(
                    "kapak {} bayt",
                    c.bytes.len()
                )),
            t.elapsed().as_secs_f64() * 1000.
        );
    }

    fn try_set_metadata(&mut self, now: &NowPlaying) -> Result<()> {
        let updater = self.controls.DisplayUpdater()?;
        updater.ClearAll()?;
        if now.title.is_empty() {
            return updater.Update();
        }
        updater.SetType(MediaPlaybackType::Music)?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&HSTRING::from(&now.title))?;
        music.SetArtist(&HSTRING::from(&now.artist))?;
        music.SetAlbumTitle(&HSTRING::from(&now.album))?;
        if let Some(cover) = &now.cover {
            match thumbnail(&cover.bytes) {
                Ok(reference) => updater.SetThumbnail(&reference)?,
                Err(e) => log::warn!("Medya kontrolüne kapak verilemedi: {e}"),
            }
        }
        updater.Update()
    }

    /// Hangi düğmeler kullanılabilir ve çalma durumu.
    pub fn set_buttons(&mut self, now: &NowPlaying) {
        let r = (|| -> Result<()> {
            let c = &self.controls;
            let has_track = now.status != PlayStatus::Closed;
            c.SetIsPlayEnabled(now.can_play || has_track)?;
            c.SetIsPauseEnabled(has_track)?;
            c.SetIsStopEnabled(has_track)?;
            c.SetIsNextEnabled(now.can_next)?;
            c.SetIsPreviousEnabled(now.can_previous)?;
            c.SetPlaybackStatus(match now.status {
                PlayStatus::Closed => MediaPlaybackStatus::Closed,
                PlayStatus::Stopped => MediaPlaybackStatus::Stopped,
                PlayStatus::Changing => MediaPlaybackStatus::Changing,
                PlayStatus::Playing => MediaPlaybackStatus::Playing,
                PlayStatus::Paused => MediaPlaybackStatus::Paused,
            })
        })();
        if let Err(e) = r {
            log::warn!("Medya kontrolünün durumu verilemedi: {e}");
        }
    }

    /// Konum çizelgesi: şarkı değişince, çal / duraklat / sarmada (her
    /// saniye değil; Windows aradaki konumu kendisi ilerletir).
    pub fn set_timeline(&mut self, now: &NowPlaying) {
        let r = (|| -> Result<()> {
            let span = |ms: u32| TimeSpan {
                Duration: ms as i64 * 10_000,
            };
            let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
            timeline.SetStartTime(span(0))?;
            timeline.SetEndTime(span(now.duration_ms))?;
            timeline.SetMinSeekTime(span(0))?;
            timeline.SetMaxSeekTime(span(now.duration_ms))?;
            timeline.SetPosition(span(now.position_ms.min(now.duration_ms)))?;
            self.controls.UpdateTimelineProperties(&timeline)
        })();
        if let Err(e) = r {
            log::warn!("Medya kontrolünün konumu verilemedi: {e}");
        }
    }

    pub fn disable(&self) {
        let _ = self.controls.SetIsEnabled(false);
    }
}

/// Kapak baytlarından medya kontrolünün okuyacağı akış.
fn thumbnail(bytes: &[u8]) -> Result<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.get()?;
    writer.FlushAsync()?.get()?;
    // Yazıcı bırakılırken akışı kapatmasın.
    writer.DetachStream()?;
    stream.Seek(0)?;
    RandomAccessStreamReference::CreateFromStream(&stream)
}
