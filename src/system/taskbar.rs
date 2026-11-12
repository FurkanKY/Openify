//! Görev çubuğu önizlemesindeki önceki / çal-duraklat / sonraki düğmeleri
//! (`ITaskbarList3`).
//!
//! Düğmeler ancak pencerenin görev çubuğu düğmesi oluştuktan sonra eklenir
//! ("TaskbarButtonCreated"). Pencere gizlenip yeniden gösterilince ya da
//! Explorer yeniden başlayınca bu ileti yeniden gelir: düğmeler yeniden
//! eklenir. Tıklama `WM_COMMAND` ile gelir (`hook.rs`).

use windows::Win32::{
    Foundation::HWND,
    System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    UI::Shell::{
        ITaskbarList3, THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_DISABLED, THBF_ENABLED, THUMBBUTTON,
        TaskbarList,
    },
};

use super::{
    NowPlaying, PlayStatus,
    hook::{THUMB_NEXT, THUMB_PLAY, THUMB_PREVIOUS},
    icons::{self, Icon},
    tray::copy_wide,
};
use crate::t;

struct Icons {
    previous: Icon,
    play: Icon,
    pause: Icon,
    next: Icon,
}

impl Icons {
    fn render(light: bool) -> Option<Self> {
        let size = icons::small_icon_size();
        let ink = icons::ink(light);
        Some(Icons {
            previous: icons::render(icons::PREVIOUS, size, ink)?,
            play: icons::render(icons::PLAY, size, ink)?,
            pause: icons::render(icons::PAUSE, size, ink)?,
            next: icons::render(icons::NEXT, size, ink)?,
        })
    }
}

pub struct Taskbar {
    list: ITaskbarList3,
    icons: Icons,
    /// Düğmeler bu görev çubuğu düğmesine eklendi mi?
    added: bool,
}

impl Taskbar {
    pub fn new(light: bool) -> windows::core::Result<Self> {
        let list: ITaskbarList3 =
            unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
        unsafe { list.HrInit()? };
        let icons = Icons::render(light).ok_or_else(|| windows::core::Error::from_win32())?;
        Ok(Taskbar {
            list,
            icons,
            added: false,
        })
    }

    fn buttons(&self, now: &NowPlaying) -> [THUMBBUTTON; 3] {
        let playing = matches!(now.status, PlayStatus::Playing | PlayStatus::Changing);
        let button = |id: u32, icon: &Icon, tip: &str, enabled: bool| {
            let mut b = THUMBBUTTON {
                dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                iId: id,
                hIcon: icon.0,
                dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
                ..Default::default()
            };
            copy_wide(&mut b.szTip, tip);
            b
        };
        [
            button(
                THUMB_PREVIOUS,
                &self.icons.previous,
                t!(PrevTrack),
                now.can_previous,
            ),
            if playing {
                button(THUMB_PLAY, &self.icons.pause, t!(Pause), true)
            } else {
                button(THUMB_PLAY, &self.icons.play, t!(Play), now.can_play)
            },
            button(THUMB_NEXT, &self.icons.next, t!(NextTrack), now.can_next),
        ]
    }

    /// Görev çubuğu düğmesi (yeniden) oluştu: düğmeleri ekle.
    pub fn on_button_created(&mut self, hwnd: HWND, now: &NowPlaying) {
        let buttons = self.buttons(now);
        let added = unsafe { self.list.ThumbBarAddButtons(hwnd, &buttons) };
        match added {
            Ok(()) => self.added = true,
            // Aynı düğmeye ikinci kez eklenemez: güncelle.
            Err(_) => {
                self.added = unsafe { self.list.ThumbBarUpdateButtons(hwnd, &buttons) }
                    .inspect_err(|e| log::warn!("Görev çubuğu düğmeleri eklenemedi: {e}"))
                    .is_ok();
            }
        }
        log::info!(
            "sistem: görev çubuğu düğmeleri {}",
            if self.added { "eklendi" } else { "yok" }
        );
    }

    pub fn set_state(&mut self, hwnd: HWND, now: &NowPlaying) {
        if !self.added {
            return;
        }
        let buttons = self.buttons(now);
        // Pencere gizliyken görev çubuğu düğmesi yok: hata olağan, gösterilince
        // "TaskbarButtonCreated" ile yeniden eklenir.
        let _ = unsafe { self.list.ThumbBarUpdateButtons(hwnd, &buttons) };
    }

    pub fn set_theme(&mut self, hwnd: HWND, light: bool, now: &NowPlaying) {
        if let Some(icons) = Icons::render(light) {
            self.icons = icons;
            self.set_state(hwnd, now);
        }
    }
}
