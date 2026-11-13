//! Ayarlar sayfası, profil menüsü ve onay kutusu (Adım 15).
//!
//! - Başlık çubuğunun sağında profil düğmesi (baş harf ya da simge; Spotify
//!   profil fotoğrafı oturum verisinde yok, bkz. DESIGN.md) ve menüsü:
//!   hesap, Ayarlar, Hakkında, Çıkış yap.
//! - Ayarlar: tek sayfa, kısa bölümler alt alta (Ses, Görünüm ve davranış,
//!   Başlangıç, Depolama ve hesap, Hakkında). Her satır: ad, tek cümle
//!   açıklama, sağda anahtar / seçenek düğmesi / liste.
//! - Çıkış ve önbellek temizleme uygulama içi onay kutusuyla.
//!
//! Değerler `Settings`te (`prefs`); her değişiklik hemen uygulanır
//! (`apply_prefs`) ve diske yazılır. Sınama kopyaları gerçek kayıt defterine,
//! önbelleğe ve ayar dosyasına dokunmaz (bkz. `autostart`, `engine::cache`,
//! `OPENIFY_SETTINGS_PATH`).

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, Bounds, ClickEvent, Context, ElementId, FontWeight, KeyDownEvent, MouseButton,
    Pixels, SharedString, Window, canvas, deferred, div, prelude::*, px, rgb,
};

use super::{
    Shell, TITLEBAR_HEIGHT,
    components::{
        Look, MENU_ITEM_HEIGHT, MenuItem, Tooltip, menu_item, menu_panel, menu_separator,
        primary_button_sized, secondary_button, segment, segmented_track, select_field, switch,
    },
    history::Route,
    player::Connection,
    shortcuts::Shortcut,
    text::line_with_tip,
    theme::*,
};
use crate::{
    autostart::Autostart,
    engine,
    settings::{CROSSFADE_CHOICES_MS, Settings, ThemePreference},
    update,
};
use crate::{t, tf};

/// Sayfadaki içerik sütununun en büyük eni (satırlar geniş ekranda uzamasın).
const SETTINGS_WIDTH: f32 = 720.;
const PROFILE_MENU_WIDTH: f32 = 272.;
/// Başlıktaki profil hapı: avatar çapı, adın dışındaki payı (kenarlar, boşluklar,
/// ok), ad alanının en çok / en az eni; sekmelerin (yaklaşık) toplam eni.
const PROFILE_AVATAR: f32 = 24.;
const PROFILE_CHROME: f32 = 70.;
const PROFILE_NAME_MAX: f32 = 140.;
const PROFILE_NAME_MIN: f32 = 48.;
const TABS_WIDTH_LABELS: f32 = 300.;
const TABS_WIDTH_ICONS: f32 = 116.;
const CONFIRM_WIDTH: f32 = 400.;
/// Çıkış aygıtı listesinin en çok görünen satırı.
const DEVICE_ROWS: usize = 8;

/// Onay isteyen işlemler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    SignOut,
    ClearCache,
}

/// Ayarlar sayfasının ve profil menüsünün ekran durumu.
pub struct SettingsUi {
    pub profile_open: bool,
    profile_cursor: Option<usize>,
    /// Profil düğmesinin ekrandaki yeri (menü altına açılır).
    profile_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub confirm: Option<Confirm>,
    device_open: bool,
    device_cursor: usize,
    device_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Bilinen çıkış aygıtları (arka planda bulunur) ve sistem varsayılanı.
    devices: Option<Vec<String>>,
    default_device: Option<String>,
    autostart: Autostart,
    autostart_on: bool,
    autostart_failed: bool,
    /// Kısayol alınamadıysa söylenecek metin.
    hotkey_note: Option<String>,
    /// Önbellek boyutu (arka planda hesaplanır) ve temizlendikten sonraki not.
    cache_bytes: Option<u64>,
    cache_note: Option<String>,
    licenses_open: bool,
    license_text: Option<usize>,
    /// Hakkında'dan gelindi: sayfa en alta kaydırılsın.
    pub jump_about: bool,
    /// Kısayol yardımı katmanı (bkz. `shortcuts`).
    pub help: super::shortcuts::HelpState,
    /// Şarkı künyesi penceresi (bkz. `credits`).
    pub credits: super::credits::CreditsUi,
    /// Ekolayzer bölümü: sürgü yerleri, kulaklık araması (bkz. `equalizer`).
    pub eq: super::equalizer::EqUi,
    /// Motora son bildirilen Connect durumu (açık mı, hangi ad): Connect
    /// kapalıyken (varsayılan) motora hiçbir Connect komutu gitmez.
    connect_sent: (bool, String),
}

impl SettingsUi {
    pub fn new() -> Self {
        let autostart = Autostart::system();
        let autostart_on = autostart.is_enabled();
        Self {
            profile_open: false,
            profile_cursor: None,
            profile_bounds: Rc::default(),
            confirm: None,
            device_open: false,
            device_cursor: 0,
            device_bounds: Rc::default(),
            devices: None,
            default_device: None,
            autostart,
            autostart_on,
            autostart_failed: false,
            hotkey_note: None,
            cache_bytes: None,
            cache_note: None,
            licenses_open: false,
            license_text: None,
            jump_about: false,
            help: Default::default(),
            credits: super::credits::CreditsUi::new(),
            eq: super::equalizer::EqUi::new(),
            connect_sent: (false, String::new()),
        }
    }

    /// Klavyeyi ve tekerleği kendine alan bir katman açık mı?
    pub fn modal(&self) -> bool {
        self.profile_open
            || self.device_open
            || self.confirm.is_some()
            || self.help.open
            || self.credits.open
    }

    /// Menüleri kapat (onay kutusu kalır).
    pub(super) fn close_menus(&mut self) {
        self.profile_open = false;
        self.profile_cursor = None;
        self.device_open = false;
    }
}

// ── Saf yardımcılar (test edilir) ───────────────────────────────────────

/// Spotify'ın kendi ürettiği kullanıcı kimliği mi (uzun, harf-rakam karışık)?
/// Bunlar bir ad değil; baş harfi anlamsız.
fn looks_generated(name: &str) -> bool {
    name.len() >= 20
        && name.chars().all(|c| c.is_ascii_alphanumeric())
        && name.chars().any(|c| c.is_ascii_digit())
}

/// Profil düğmesindeki baş harf; ad yoksa ya da bir kimlik gibiyse `None` (simge).
pub fn profile_initial(username: &str) -> Option<char> {
    let name = username.trim();
    let first = name.chars().next()?;
    if !first.is_alphabetic() || looks_generated(name) {
        return None;
    }
    crate::i18n::upper(&first.to_string()).chars().next()
}

/// Bayt sayısı okunur biçimde: "340 KB", "1,2 GB".
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    let b = bytes as f64;
    let text = if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * 100. {
        format!("{:.1} MB", b / (KB * KB))
    } else if b < KB * KB * KB {
        format!("{:.0} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    };
    crate::i18n::localize_decimal(&text)
}

/// Güncelleme durumunun düz cümlesi.
fn update_text(status: &update::Status) -> String {
    match status {
        update::Status::NotInstalled => t!(UpdNotInstalled).into(),
        update::Status::Off => t!(UpdOff).into(),
        update::Status::Waiting => t!(UpdWaiting).into(),
        update::Status::Checking => t!(UpdChecking).into(),
        update::Status::UpToDate => t!(UpdUpToDate).into(),
        update::Status::Ready(v) => tf!(UpdReady, v),
        update::Status::Failed => t!(UpdFailed).into(),
        update::Status::RolledBack(v) => tf!(UpdRolledBack, v),
    }
}

/// Çıkış aygıtı alanında görünen ad.
fn device_label(selected: &Option<String>, known: Option<&Vec<String>>) -> String {
    match selected {
        None => t!(SystemDefault).into(),
        Some(name) if known.is_some_and(|k| !k.contains(name)) => tf!(DeviceDisconnected, name),
        Some(name) => name.clone(),
    }
}

/// Kullanılan açık kaynak bileşenler: ad, lisans, ne için. Yazı tipleri ve
/// simgelerin lisans metni uygulamaya gömülü (OFL / ISC bunu ister).
pub fn notices() -> [(
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
); 14] {
    [
        ("librespot", "MIT", t!(LicLibrespot), None),
        ("gpui (Zed)", "Apache-2.0", t!(LicUi), None),
        ("Velopack", "MIT", t!(LicInstaller), None),
        ("cpal ve rodio", "Apache-2.0, MIT", t!(LicAudioOut), None),
        ("SQLite (rusqlite)", t!(LicSqliteTerms), t!(Cache), None),
        ("Tokio, reqwest", "MIT, Apache-2.0", t!(LicNetwork), None),
        (
            "serde, protobuf",
            "MIT, Apache-2.0",
            t!(LicDataFormats),
            None,
        ),
        ("resvg, image", "Apache-2.0, MIT", t!(LicImages), None),
        ("nucleo-matcher", "MPL-2.0", t!(LicMatcher), None),
        (
            "keyring, windows-rs",
            "MIT, Apache-2.0",
            t!(LicWindows),
            None,
        ),
        (
            "Figtree",
            "SIL OFL 1.1",
            t!(LicTitleFont),
            Some(include_str!("../../assets/licenses/Figtree-OFL.txt")),
        ),
        (
            "Plus Jakarta Sans",
            "SIL OFL 1.1",
            t!(LicFallbackFont),
            Some(include_str!(
                "../../assets/licenses/PlusJakartaSans-OFL.txt"
            )),
        ),
        (
            "Lucide",
            "ISC",
            t!(LicIcons),
            Some(include_str!("../../assets/licenses/Lucide-ISC.txt")),
        ),
        (
            "OPRA",
            "CC BY-SA 4.0",
            t!(LicOpra),
            Some(include_str!("../../assets/licenses/OPRA-NOTICE.txt")),
        ),
    ]
}

// ── Eylemler ────────────────────────────────────────────────────────────

impl Shell {
    /// Ayarlar sayfasını aç (Ctrl+,, profil menüsü, palet).
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.sui.close_menus();
        self.navigate(Route::Settings, cx);
    }

    /// Profil menüsündeki "Hakkında": Ayarlar'ın en altına.
    fn open_about(&mut self, cx: &mut Context<Self>) {
        self.sui.close_menus();
        self.sui.jump_about = true;
        self.navigate(Route::Settings, cx);
        cx.notify();
    }

    /// Sayfaya girerken: durum yeniden okunur, aygıtlar ve önbellek boyutu arkada bulunur.
    pub(super) fn enter_settings(&mut self, cx: &mut Context<Self>) {
        self.sui.autostart_on = self.sui.autostart.is_enabled();
        self.sui.cache_note = None;
        let devices = cx.background_spawn(async {
            (
                engine::output_device_names(),
                engine::default_output_device_name(),
            )
        });
        cx.spawn(async move |this, cx| {
            let (names, default) = devices.await;
            let _ = this.update(cx, |shell, cx| {
                shell.sui.devices = Some(names);
                shell.sui.default_device = default;
                cx.notify();
            });
        })
        .detach();
        self.refresh_cache_size(cx);
        self.eq_enter(cx);
    }

    fn refresh_cache_size(&mut self, cx: &mut Context<Self>) {
        let dir = engine::settings_cache_dir();
        let size = cx.background_spawn(async move { dir.map_or(0, |d| engine::dir_size(&d)) });
        cx.spawn(async move |this, cx| {
            let bytes = size.await;
            let _ = this.update(cx, |shell, cx| {
                shell.sui.cache_bytes = Some(bytes);
                cx.notify();
            });
        })
        .detach();
    }

    /// Bir ayarı değiştir: hemen uygulanır, diske yazılır, ekran yenilenir.
    fn set_pref(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        change(&mut self.prefs);
        self.apply_prefs(false);
        self.apply_theme(cx);
        self.save_settings();
        // Arkadaş etkinliği ayarı değiştiyse (panel, yenileme, bellekteki liste).
        self.after_friend_pref(cx);
        cx.notify();
    }

    /// Söz çevirisini aç / kapat (sözler paneli ve tam ekran; kalıcı tercih).
    pub(super) fn toggle_lyrics_translation(&mut self, cx: &mut Context<Self>) {
        let on = !self.prefs.lyrics_translation;
        self.set_pref(cx, |p| p.lyrics_translation = on);
        self.notify_lyric_views(cx);
    }

    /// Karaoke'yu aç / kapat (sözler paneli ve tam ekran; kalıcı tercih).
    pub(super) fn toggle_karaoke(&mut self, cx: &mut Context<Self>) {
        let on = !self.prefs.karaoke_enabled;
        self.set_pref(cx, |p| p.karaoke_enabled = on);
        self.notify_lyric_views(cx);
    }

    /// Ayarların motora, temaya ve sisteme uygulanması. Açılışta (`startup`)
    /// kısayol ve pencere işleri `Desk` kurulurken zaten yapıldı.
    pub(super) fn apply_prefs(&mut self, startup: bool) {
        engine::apply_audio(self.prefs.kbps(), self.prefs.normalise);
        engine::apply_normalisation_level(self.prefs.normalise_pregain_db());
        self.engine
            .send(engine::Command::SetQueueRules(self.prefs.queue_rules()));
        engine::set_output_device(self.prefs.output_device.clone());
        self.engine
            .send(engine::Command::SetCrossfade(self.prefs.crossfade_ms));
        self.engine
            .send(engine::Command::SetAutoMix(self.prefs.automix));
        self.eq_send();
        self.karaoke_send();
        self.connect_send();
        engine::set_resume(self.prefs.resume);
        super::theme::set_reduce_motion(&self.prefs.reduce_motion);
        crate::i18n::apply_preference(&self.prefs.language);
        self.sync_discord();
        self.desk.set_close_to_tray(self.prefs.close_to_tray);
        if !startup {
            self.sui.hotkey_note = self.desk.set_global_hotkey(self.prefs.global_hotkey);
        }
    }

    /// Spotify Connect ayarlarını motora bildirir (bkz. `engine::connect`).
    /// Kapalıyken (varsayılan) motora HİÇBİR Connect komutu gitmez: davranış
    /// Connect'ten önceki gibi. Açıkken önce ad, sonra "aç"; kapatınca "kapat".
    /// Kalıcı aygıt kimliği (oturum bununla açılır) her seferinde güncellenir.
    fn connect_send(&mut self) {
        engine::apply_connect_identity(self.prefs.connect_enabled, &self.prefs.device_id);
        let (on, name) = (self.prefs.connect_enabled, self.prefs.device_name.clone());
        let sent = &mut self.sui.connect_sent;
        if on && sent.1 != name {
            sent.1 = name.clone();
            self.engine
                .send(engine::Command::Connect(engine::ConnectCommand::SetName(
                    name,
                )));
        }
        let sent = &mut self.sui.connect_sent;
        if on != sent.0 {
            sent.0 = on;
            self.engine.send(engine::Command::Connect(
                engine::ConnectCommand::SetEnabled(on),
            ));
        }
    }

    /// Cihaz adı penceresini aç (Spotify'ın cihaz listesinde görünen ad).
    fn ask_device_name(&mut self, cx: &mut Context<Self>) {
        use super::manage::{Prompt, PromptKind};
        self.manage.prompt = Some(Prompt {
            kind: PromptKind::DeviceName,
            text: self.prefs.device_name.clone(),
        });
        self.refocus_anchor = true;
        cx.notify();
    }

    /// Cihaz adı penceresi onaylandı (`manage::confirm_prompt`).
    pub(super) fn set_device_name(&mut self, name: String, cx: &mut Context<Self>) {
        let name: String = name
            .trim()
            .chars()
            .take(crate::settings::DEVICE_NAME_MAX)
            .collect();
        if name.is_empty() {
            return;
        }
        self.set_pref(cx, |p| p.device_name = name);
    }

    /// Bir sanatçıyı kara listeye al / listeden çıkar (sağ tık menüleri ve
    /// Ayarlar'daki liste): hemen uygulanır, kaydedilir, bir bildirimle söylenir.
    pub(super) fn set_artist_blocked(
        &mut self,
        name: String,
        uri: String,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        self.set_pref(cx, |p| changed = p.set_blocked(&name, &uri, on));
        if changed {
            let text = if on {
                tf!(ToastArtistBlocked, name)
            } else {
                tf!(ToastArtistAllowed, name)
            };
            self.show_toast(text, false, false, cx);
        }
    }

    /// Sağ tık menüsüne konan "Bu sanatçıyı hiç çalma" satırları: tek sanatçı
    /// bir satır, birden çok sanatçı bir alt menü. Kara listedeki sanatçı için
    /// "yeniden izin ver". `named`: tek sanatçılı satır adı da söyler (menü
    /// zaten tek sanatçıyı göstermiyorsa); uzun adlar menüye sığmaz, bu yüzden
    /// satır tek sanatçıyı gösteriyorsa genel yazı kullanılır. Boş adlar atlanır.
    pub(super) fn never_play_entries(
        &self,
        names: &[&str],
        uri: &str,
        named: bool,
    ) -> Vec<super::manage::Entry> {
        use super::manage::{Act, Entry};
        let act = |name: &str, named: bool| {
            let blocked = self.prefs.is_blocked(name);
            let label = match (blocked, named) {
                (true, true) => tf!(MenuAllowNamed, name),
                (false, true) => tf!(MenuNeverPlayNamed, name),
                (true, false) => t!(MenuAllowArtist).to_string(),
                (false, false) => t!(MenuNeverPlayArtist).to_string(),
            };
            (
                blocked,
                label,
                Act::BlockArtist {
                    name: name.to_string(),
                    uri: uri.to_string(),
                    on: !blocked,
                },
            )
        };
        let names: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !n.trim().is_empty())
            .collect();
        match names.len() {
            0 => Vec::new(),
            1 => {
                let (_, label, act) = act(names[0], named);
                vec![Entry::Item {
                    icon: "user-x",
                    label,
                    shortcut: None,
                    danger: false,
                    act: Some(act),
                    sub: Vec::new(),
                }]
            }
            _ => vec![Entry::Item {
                icon: "user-x",
                label: t!(MenuNeverPlayArtist).into(),
                shortcut: None,
                danger: false,
                act: None,
                // Alt menüde yalnız ad (kara listedekinde "yeniden izin ver"): üst satır anlatır.
                sub: names
                    .iter()
                    .map(|n| {
                        let (blocked, label, a) = act(n, true);
                        (if blocked { label } else { (*n).to_string() }, a)
                    })
                    .collect(),
            }],
        }
    }

    fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        let want = !self.sui.autostart_on;
        match self.sui.autostart.set_enabled(want) {
            Ok(()) => {
                self.sui.autostart_on = want;
                self.sui.autostart_failed = false;
            }
            Err(e) => {
                log::warn!("Açılışta başlatma ayarı değiştirilemedi: {e}");
                self.sui.autostart_failed = true;
            }
        }
        cx.notify();
    }

    pub(super) fn ask_sign_out(&mut self, cx: &mut Context<Self>) {
        self.sui.close_menus();
        self.sui.confirm = Some(Confirm::SignOut);
        cx.notify();
    }

    fn ask_clear_cache(&mut self, cx: &mut Context<Self>) {
        self.sui.close_menus();
        self.sui.confirm = Some(Confirm::ClearCache);
        cx.notify();
    }

    /// Kayıtlı Spotify girişini sil ve Openify'ı kapat. Bir sonraki açılışta
    /// giriş yeniden istenir.
    pub(super) fn sign_out(&mut self, cx: &mut Context<Self>) {
        // Sınama kopyası kullanıcının gerçek girişine (Kimlik Bilgisi
        // Yöneticisi) dokunmaz: yalnız kapanır.
        if std::env::var_os("OPENIFY_TEST_DISPLAY").is_some() {
            log::info!("sınama kopyası: kayıtlı giriş silinmedi, yalnız kapanıyor");
            cx.quit();
            return;
        }
        if let Err(e) = crate::auth::clear() {
            log::warn!("Çıkış yapılamadı: {e:#}");
            self.state
                .notify(engine::NoticeKind::Problem, t!(SignOutFailed));
            cx.notify();
            return;
        }
        self.save_settings();
        cx.quit();
    }

    /// Önbelleği temizle (şarkı dosyaları ve kapaklar). Sınama kopyasında
    /// gerçek önbelleğe dokunulmaz (`engine::clearing_allowed`).
    fn clear_cache(&mut self, cx: &mut Context<Self>) {
        if !engine::clearing_allowed() {
            return;
        }
        let Some(dir) = engine::settings_cache_dir() else {
            return;
        };
        let freed =
            cx.background_spawn(async move { engine::clear_subdirs(&dir, &engine::CLEARABLE) });
        cx.spawn(async move |this, cx| {
            let freed = freed.await;
            let _ = this.update(cx, |shell, cx| {
                // Silinen kapaklar yeniden istenebilsin.
                shell.images.forget_all();
                shell.sui.cache_note = Some(tf!(CacheCleared, format_size(freed)));
                shell.refresh_cache_size(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn run_confirm(&mut self, confirm: Confirm, cx: &mut Context<Self>) {
        self.sui.confirm = None;
        match confirm {
            Confirm::SignOut => self.sign_out(cx),
            Confirm::ClearCache => self.clear_cache(cx),
        }
        cx.notify();
    }

    pub(super) fn toggle_profile(&mut self, cx: &mut Context<Self>) {
        self.sui.device_open = false;
        self.sui.profile_open = !self.sui.profile_open;
        self.sui.profile_cursor = None;
        cx.notify();
    }

    fn toggle_devices(&mut self, cx: &mut Context<Self>) {
        self.sui.profile_open = false;
        self.sui.device_open = !self.sui.device_open;
        // Seçili aygıttan başla.
        self.sui.device_cursor = self.sui.devices.as_ref().map_or(0, |d| {
            self.prefs
                .output_device
                .as_ref()
                .and_then(|s| d.iter().position(|n| n == s))
                .map_or(0, |i| i + 1)
        });
        cx.notify();
    }

    fn pick_device(&mut self, index: usize, cx: &mut Context<Self>) {
        // 0: sistem varsayılanı, sonrakiler listedeki aygıtlar.
        let name = index
            .checked_sub(1)
            .and_then(|i| self.sui.devices.as_ref()?.get(i).cloned());
        self.sui.device_open = false;
        self.set_pref(cx, |p| p.output_device = name);
    }

    /// Açık katman varken tuşlar (Esc, oklar, Enter) ve Ctrl+, . İşlendiyse `true`.
    pub(super) fn settings_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let ks = &event.keystroke;
        let key = ks.key.as_str();
        if let Some(confirm) = self.sui.confirm {
            match key {
                "escape" => self.sui.confirm = None,
                "enter" => self.run_confirm(confirm, cx),
                _ => {}
            }
            cx.notify();
            return true;
        }
        if self.sui.device_open {
            let count = self.sui.devices.as_ref().map_or(0, |d| d.len()) + 1;
            match key {
                "escape" => self.sui.device_open = false,
                "up" => self.sui.device_cursor = self.sui.device_cursor.saturating_sub(1),
                "down" => self.sui.device_cursor = (self.sui.device_cursor + 1).min(count - 1),
                "enter" => {
                    let i = self.sui.device_cursor;
                    self.pick_device(i, cx);
                }
                _ => {}
            }
            cx.notify();
            return true;
        }
        if self.sui.profile_open {
            const ITEMS: usize = 4;
            match key {
                "escape" => self.sui.profile_open = false,
                "up" => {
                    self.sui.profile_cursor = Some(
                        self.sui
                            .profile_cursor
                            .map_or(ITEMS - 1, |c| c.saturating_sub(1)),
                    )
                }
                "down" => {
                    self.sui.profile_cursor = Some(
                        self.sui
                            .profile_cursor
                            .map_or(0, |c| (c + 1).min(ITEMS - 1)),
                    )
                }
                "enter" => match self.sui.profile_cursor {
                    Some(0) => self.open_settings(cx),
                    Some(1) => self.toggle_private_from_menu(cx),
                    Some(2) => self.open_about(cx),
                    Some(_) => self.ask_sign_out(cx),
                    None => {}
                },
                _ => {}
            }
            cx.notify();
            return true;
        }
        if Shortcut::from_keystroke(ks) == Some(Shortcut::Settings) {
            self.open_settings(cx);
            return true;
        }
        false
    }

    /// Tekerlek bir katmanın üstünde: menüler kapanır, sayfa kaymaz.
    pub(super) fn settings_wheel(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.sui.modal() {
            return false;
        }
        if self.sui.confirm.is_none() {
            self.sui.close_menus();
            cx.notify();
        }
        true
    }

    // ── Başlık çubuğu: profil düğmesi ────────────────────────────────────

    fn account_name(&self) -> Option<&str> {
        // Geliştirme denemesi: OPENIFY_TEST_USER=ad verilirse oturum o adla
        // "bağlı" gösterilir (profil düğmesi ve menüsü; Spotify'a gitmeden).
        static TEST_USER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        if let Some(user) = TEST_USER.get_or_init(|| std::env::var("OPENIFY_TEST_USER").ok()) {
            return Some(user.as_str());
        }
        match &self.state.connection {
            Connection::Connected(user) => Some(user.as_str()),
            _ => None,
        }
    }

    /// Başlık çubuğunun en solundaki profil hapı: baş harf (ya da kişi
    /// simgesi) + ad + aşağı ok. Dar pencerede yalnız avatar + ok.
    pub(super) fn render_profile_button(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (stop, look) = self.stop_look("profile-button", super::tab::PROFILE, true, window, cx);
        let open = self.sui.profile_open;
        let rest = if open { VEIL_SELECTED } else { VEIL_HOVER };
        let raw = self.account_name();
        let avatar = div()
            .size(px(PROFILE_AVATAR))
            .flex_none()
            .rounded_full()
            .bg(veil(VEIL_PRESSED))
            .flex()
            .items_center()
            .justify_center()
            .child(match raw.and_then(profile_initial) {
                Some(initial) => div()
                    .type_role(T_SECONDARY)
                    .font_weight(FontWeight::BOLD)
                    .text_color(text())
                    .child(initial.to_string())
                    .into_any_element(),
                None => icon("user", 14., soft()).into_any_element(),
            });
        let name: SharedString = match raw {
            Some(u) if !looks_generated(u) => u.to_string().into(),
            Some(_) => t!(YourSpotifyAccount).into(),
            None => t!(Account).into(),
        };
        let name_w = self.profile_name_width(window);
        let bounds = self.sui.profile_bounds.clone();
        div()
            .id("profile-button")
            .relative()
            .h(px(32.))
            .flex_none()
            .ml(px(S5))
            .pl(px(S1))
            .pr(px(if name_w.is_some() { S3 } else { S2 }))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(S2))
            .bg(veil(rest))
            .cursor_pointer()
            .border_2()
            .border_color(if look == Look::Focus {
                rgb(self.accent().text).into()
            } else {
                gpui::transparent_black()
            })
            .when_some(stop, |el, h| el.track_focus(&h))
            // Menü açıkken ipucu yok: menünün üstünde kalırdı.
            .when(!open, |el| el.tooltip(Tooltip::text(t!(ProfileTip), None)))
            .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                this.leave_button(ev, window);
                this.toggle_profile(cx)
            }))
            .child(super::tween::hover_fill(
                veil(VEIL_SELECTED),
                veil(VEIL_PRESSED),
                rest,
                16.,
            ))
            .child(avatar)
            .when_some(name_w, |el, w| {
                el.child(
                    div()
                        .flex()
                        .max_w(px(w))
                        .flex_none()
                        .type_role(T_SECONDARY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(text())
                        // Kısa ad içeriğe göre boyutlanır (Line öğesi
                        // tüm genişliği kaplar, hap gereksiz uzardı);
                        // uzun ad kısaltılır, ipucunda tamamı.
                        .map(|el| {
                            // ~8 px/harf: bu boyda harf genişliğinin üst sınırı.
                            if name.chars().count() as f32 * 8. <= w {
                                el.child(name.clone())
                            } else {
                                el.w(px(w)).child(
                                    line_with_tip("profile-pill-name", name.clone())
                                        .flex()
                                        .min_w_0(),
                                )
                            }
                        }),
                )
            })
            .child(icon("chevron-down", 14., muted()))
            .child(
                canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .into_any_element()
    }

    /// Hapın ad alanının eni; yer dar olunca `None` (yalnız avatar + ok).
    /// Arama pencerenin ortasında kaldığı için sol yarının payı:
    /// (pencere - arama) / 2; ondan gezinme düğmeleri ve sekmeler düşülür.
    fn profile_name_width(&self, window: &Window) -> Option<f32> {
        let w = f32::from(window.viewport_size().width);
        let half = (w - self.search_width()) / 2.;
        let tabs = if w >= super::explore::TABS_LABELS_FROM {
            TABS_WIDTH_LABELS
        } else {
            TABS_WIDTH_ICONS
        };
        let fixed = S5 + PROFILE_CHROME + super::pages::NAV_WIDTH + tabs + S4 + S2;
        let room = (half - fixed).min(PROFILE_NAME_MAX);
        (room >= PROFILE_NAME_MIN).then_some(room)
    }

    // ── Katmanlar: profil menüsü, aygıt listesi, onay ─────────────────────

    pub(super) fn render_settings_overlays(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if self.sui.profile_open {
            out.push(self.render_profile_menu(window, cx));
        }
        if self.sui.device_open {
            out.push(self.render_device_menu(window, cx));
        }
        if let Some(confirm) = self.sui.confirm {
            out.push(self.render_confirm(confirm, cx));
        }
        if self.sui.help.open {
            out.push(self.render_help(window, cx));
        }
        if self.sui.credits.open {
            out.push(self.render_credits(window, cx));
        }
        out
    }

    /// Menünün arkasındaki, pencereyi kaplayan görünmez katman: dışarı
    /// tıklamak (ya da tekerlek) menüyü kapatır, arkadaki öğe seçilmez.
    fn dismiss_layer(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.sui.close_menus();
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.sui.close_menus();
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                this.sui.close_menus();
                cx.notify();
            }))
            // Başlık çubuğunun altını kaplayan engelleyici: menüyü açan düğme
            // (ve arkadaki her şey) "üzerine gelinmiş" sayılmaz, ipucu çıkmaz.
            // Katmanın dinleyicisi bu bölgede çalışmaz, o yüzden aynıları burada.
            .child(
                super::manage::layer_blocker()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.sui.close_menus();
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.sui.close_menus();
                            cx.notify();
                        }),
                    )
                    .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                        this.sui.close_menus();
                        cx.notify();
                    })),
            )
    }

    fn render_profile_menu(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let win_w = f32::from(window.viewport_size().width);
        let anchor = self.sui.profile_bounds.get();
        let (left, bottom) = anchor.map_or((S5, TITLEBAR_HEIGHT), |b| {
            (f32::from(b.origin.x), f32::from(b.origin.y + b.size.height))
        });
        // Hapın sol kenarına hizalı.
        let x = left.clamp(S2, win_w - PROFILE_MENU_WIDTH - S2);
        let y = bottom + S1;

        let raw = self.account_name();
        let (name, detail): (SharedString, SharedString) = match raw {
            Some(u) if !looks_generated(u) => (u.to_string().into(), t!(SpotifyAccount).into()),
            Some(u) => (t!(YourSpotifyAccount).into(), u.to_string().into()),
            None => (t!(SpotifyAccount).into(), t!(ShownWhenConnected).into()),
        };
        let face = match raw.and_then(profile_initial) {
            Some(c) => div()
                .type_role(T_BODY)
                .font_weight(FontWeight::BOLD)
                .text_color(text())
                .child(c.to_string())
                .into_any_element(),
            None => icon("user", 18., soft()).into_any_element(),
        };
        let header = div()
            .px(px(S2))
            .py(px(S2))
            .flex()
            .items_center()
            .gap(px(S3))
            .child(
                div()
                    .size(px(S7 - S2))
                    .flex_none()
                    .rounded_full()
                    .bg(veil(VEIL_SELECTED))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(face),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .type_role(T_BODY)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(text())
                            .child(line_with_tip("profile-name", name).flex().min_w_0()),
                    )
                    .child(
                        div()
                            .flex()
                            .type_role(T_LABEL)
                            .text_color(label())
                            .child(line_with_tip("profile-detail", detail).flex().min_w_0()),
                    ),
            );

        let item = |i: usize, entry: MenuItem, act: fn(&mut Shell, &mut Context<Shell>)| {
            let look = if self.sui.profile_cursor == Some(i) {
                Look::Hover
            } else {
                Look::Live
            };
            menu_item(i, entry.look(look), accent)
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && this.sui.profile_cursor != Some(i) {
                        this.sui.profile_cursor = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| act(this, cx)))
        };
        let panel = menu_panel("profile-menu")
            .w(px(PROFILE_MENU_WIDTH))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(menu_separator())
            .child(item(
                0,
                MenuItem::new(t!(Settings))
                    .icon("settings")
                    .shortcut("Ctrl ,"),
                |s, cx| s.open_settings(cx),
            ))
            // Gizli oturum: açıkken simge "gizli göz" ve "Açık" yazar (bkz. `listening`).
            .child(item(
                1,
                {
                    let entry =
                        MenuItem::new(t!(MenuPrivateSession)).icon(if self.private_session() {
                            "eye-off"
                        } else {
                            "eye"
                        });
                    if self.private_session() {
                        entry.shortcut(t!(On))
                    } else {
                        entry
                    }
                },
                |s, cx| s.toggle_private_from_menu(cx),
            ))
            .child(item(2, MenuItem::new(t!(About)).icon("info"), |s, cx| {
                s.open_about(cx)
            }))
            .child(menu_separator())
            .child(item(
                3,
                MenuItem::new(t!(SignOut)).icon("log-out").danger(),
                |s, cx| s.ask_sign_out(cx),
            ));
        deferred(
            self.dismiss_layer(cx)
                .child(div().absolute().left(px(x)).top(px(y)).child(panel)),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn render_device_menu(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (win_w, win_h) = {
            let v = window.viewport_size();
            (f32::from(v.width), f32::from(v.height))
        };
        let anchor = self.sui.device_bounds.get();
        let devices = self.sui.devices.clone().unwrap_or_default();
        let rows = devices.len() + 1;
        let shown = rows.min(DEVICE_ROWS);
        let height = shown as f32 * MENU_ITEM_HEIGHT + 2. * S1 + 2.;
        let width = anchor.map_or(320., |b| f32::from(b.size.width).max(320.));
        let (left, top, field_h) = anchor.map_or((win_w - width - S5, TITLEBAR_HEIGHT, 0.), |b| {
            (
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.height),
            )
        });
        let x = (left + anchor.map_or(width, |b| f32::from(b.size.width)) - width)
            .clamp(S2, win_w - width - S2);
        // Altına sığmazsa üstüne açılır.
        let below = top + field_h + S1;
        let y = if below + height > win_h - S2 {
            (top - height - S1).max(S2)
        } else {
            below
        };

        let selected = self
            .prefs
            .output_device
            .as_ref()
            .and_then(|s| devices.iter().position(|n| n == s))
            .map_or(0, |i| i + 1);
        let mut list = div()
            .id("device-list")
            .flex()
            .flex_col()
            .max_h(px(DEVICE_ROWS as f32 * MENU_ITEM_HEIGHT))
            .overflow_y_scroll();
        for i in 0..rows {
            let label_text: SharedString = if i == 0 {
                t!(SystemDefault).into()
            } else {
                devices[i - 1].clone().into()
            };
            let mut entry = MenuItem::new(label_text);
            if i == selected {
                entry = entry.icon("check");
            }
            if self.sui.device_cursor == i {
                entry = entry.look(Look::Hover);
            }
            list = list.child(
                menu_item(i, entry, accent)
                    .flex_shrink_0()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.sui.device_cursor != i {
                            this.sui.device_cursor = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_device(i, cx))),
            );
        }
        let panel = menu_panel("device-menu")
            .w(px(width))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(list);
        deferred(
            self.dismiss_layer(cx)
                .child(div().absolute().left(px(x)).top(px(y)).child(panel)),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn render_confirm(&self, confirm: Confirm, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (title, body, ok_label, ok_icon, danger) = match confirm {
            Confirm::SignOut => (
                t!(ConfirmSignOutTitle),
                t!(ConfirmSignOutBody).to_string(),
                t!(SignOut),
                "log-out",
                true,
            ),
            Confirm::ClearCache => (
                t!(ConfirmClearTitle),
                tf!(
                    ConfirmClearBody,
                    self.sui
                        .cache_bytes
                        .map_or(String::new(), |b| format!(" ({})", format_size(b)))
                ),
                t!(Clear),
                "trash-2",
                false,
            ),
        };
        let ok_color = if danger {
            palette().danger_text
        } else {
            accent
        };
        let panel = div()
            .id("confirm")
            .occlude()
            .w(px(CONFIRM_WIDTH))
            .p(px(S5))
            .flex()
            .flex_col()
            .gap(px(S4))
            .rounded(px(R_XL))
            .bg(rgb(palette().bar))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow(float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .font_family(font())
                    .type_role(T_TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(text())
                    .child(title),
            )
            .child(div().type_role(T_BODY).text_color(muted()).child(body))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(S2))
                    .child(
                        secondary_button("confirm-cancel", "x", t!(Cancel), Look::Live, accent)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sui.confirm = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        primary_button_sized(
                            "confirm-ok",
                            ok_icon,
                            ok_label,
                            Look::Live,
                            ok_color,
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.run_confirm(confirm, cx))),
                    ),
            );
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(shade(SCRIM))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.sui.confirm = None;
                        cx.notify();
                    }),
                )
                .child(super::manage::layer_blocker().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.sui.confirm = None;
                        cx.notify();
                    }),
                ))
                .child(panel),
        )
        .with_priority(4)
        .into_any_element()
    }
}

// ── Sayfa ───────────────────────────────────────────────────────────────

/// Geçiş süresi seçeneklerinin anahtarları (saniye), `CROSSFADE_CHOICES_MS` ile aynı sırada.
const CROSSFADE_KEYS: [&str; 5] = ["0", "2", "5", "8", "12"];

/// AutoMix seçeneğinin anahtarı (süreler sayı, bu "auto").
const AUTOMIX_KEY: &str = "auto";

/// Kayıtlı süreye (ms) en yakın seçeneğin anahtarı; 0 yalnız tam 0 için.
fn crossfade_key(ms: u32) -> &'static str {
    if ms == 0 {
        return CROSSFADE_KEYS[0];
    }
    let nearest = (1..CROSSFADE_KEYS.len())
        .min_by_key(|i| CROSSFADE_CHOICES_MS[*i].abs_diff(ms))
        .unwrap_or(2);
    CROSSFADE_KEYS[nearest]
}

/// Denetimde seçili görünen anahtar: AutoMix açıksa o, değilse süreye en yakın.
fn crossfade_choice(p: &Settings) -> &'static str {
    if p.automix {
        AUTOMIX_KEY
    } else {
        crossfade_key(p.crossfade_ms)
    }
}

/// Bir seçeneği ayara işler: AutoMix, sabit süreyi (`crossfade_ms`) bozmadan açılır;
/// bir süre seçmek AutoMix'i kapatır.
fn pick_crossfade(p: &mut Settings, key: &str) {
    if key == AUTOMIX_KEY {
        p.automix = true;
    } else {
        p.automix = false;
        p.crossfade_ms = crossfade_ms(key);
    }
}

/// Seçenek anahtarının milisaniyesi.
fn crossfade_ms(key: &str) -> u32 {
    key.parse::<u32>().map_or(0, |s| s * 1000)
}

/// Satırın sol yarısı: ad ve tek cümle açıklama. Açıklama tek satırdır, sığmazsa
/// "…" ile kısalır, tamamı ipucunda (yanında denetim olan satırlar).
pub(super) fn row_text(
    title: &str,
    desc: impl Into<SharedString>,
    note: Option<(SharedString, bool)>,
) -> gpui::Div {
    row_text_inner(title, desc.into(), note, false)
}

/// Denetimi olmayan satır: açıklama tam enle sarar, kısalmaz.
pub(super) fn row_text_wrap(title: &str, desc: impl Into<SharedString>) -> gpui::Div {
    row_text_inner(title, desc.into(), None, true)
}

fn row_text_inner(
    title: &str,
    desc: SharedString,
    note: Option<(SharedString, bool)>,
    wrap: bool,
) -> gpui::Div {
    let desc_el = if wrap {
        div()
            .min_w_0()
            .type_role(T_SECONDARY)
            .text_color(muted())
            .child(desc)
    } else {
        div()
            .flex()
            .min_w_0()
            .type_role(T_SECONDARY)
            .text_color(muted())
            .child(
                line_with_tip(ElementId::Name(format!("row-desc-{title}").into()), desc)
                    .flex()
                    .min_w_0(),
            )
    };
    div()
        .flex_grow()
        .flex_shrink()
        .flex_basis(px(0.))
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(S0))
        .child(
            div()
                .type_role(T_BODY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(text())
                .child(title.to_string()),
        )
        .child(desc_el)
        .children(note.map(|(note, problem)| {
            div()
                .pt(px(S1))
                .type_role(T_SECONDARY)
                .text_color(if problem {
                    rgb(palette().danger_text)
                } else {
                    label()
                })
                .child(note)
        }))
}

/// Bir ayar satırı: solda ad ve açıklama, sağda denetim. İlk satır dışında üstte ince çizgi.
pub(super) fn setting_row(first: bool, left: gpui::Div, control: AnyElement) -> AnyElement {
    div()
        .min_h(px(S8))
        .py(px(S3))
        .flex()
        .items_center()
        .gap(px(S5))
        .when(!first, |el| el.border_t_1().border_color(veil(LINE)))
        .child(left)
        .child(div().flex_none().flex().items_center().child(control))
        .into_any_element()
}

impl Shell {
    /// Bölüm: başlık ve satırları (içerik sütunu en çok `SETTINGS_WIDTH`).
    pub(super) fn settings_section(&self, title: &str, rows: Vec<AnyElement>) -> AnyElement {
        div()
            .px(px(self.margin()))
            .pb(px(S6))
            .child(
                div()
                    .max_w(px(SETTINGS_WIDTH))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(S7))
                            .pb(px(S2))
                            .flex()
                            .items_end()
                            .font_family(font())
                            .type_role(T_TITLE)
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .text_color(text())
                            .child(title.to_string()),
                    )
                    .children(rows),
            )
            .into_any_element()
    }

    /// Seçenek düğmesi satırı (ses kalitesi, animasyon, kapat düğmesi).
    pub(super) fn choice_control(
        &self,
        id: &'static str,
        options: &[(&'static str, &'static str)],
        current: &str,
        pick: fn(&mut Settings, &'static str),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = self.accent().text;
        let mut track = segmented_track();
        for (key, label_text) in options {
            let key: &'static str = key;
            // Anahtar `seg:grup:seçenek`: ← / → grubun içinde gezer.
            let (stop, look) = self.stop_look(
                format!("seg:{id}:{key}"),
                super::tab::PAGE,
                true,
                window,
                cx,
            );
            track = track.child(
                segment(
                    ElementId::Name(format!("{id}-{key}").into()),
                    *label_text,
                    current == key,
                    look,
                    accent,
                )
                .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                .on_click(cx.listener(move |this, _, _, cx| this.set_pref(cx, |p| pick(p, key)))),
            );
        }
        track.into_any_element()
    }

    /// Açık / kapalı satırı.
    pub(super) fn switch_control(
        &self,
        id: &'static str,
        on: bool,
        pick: fn(&mut Settings, bool),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (stop, look) = self.stop_look(id, super::tab::PAGE, true, window, cx);
        switch(id, on, look, self.accent().text)
            .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
            .on_click(cx.listener(move |this, _, _, cx| this.set_pref(cx, |p| pick(p, !on))))
            .into_any_element()
    }

    fn audio_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let p = &self.prefs;
        let quality = self.choice_control(
            "quality",
            &[
                ("normal", "Normal"),
                ("high", t!(QualityHigh)),
                ("very_high", t!(QualityVeryHigh)),
            ],
            &p.quality,
            |p, k| p.quality = k.into(),
            window,
            cx,
        );
        // Kapalı / Sessiz / Normal / Yüksek: "Kapalı" `normalise`'i kapatır, öbürleri
        // açar ve düzeyi seçer (`Settings::normalise_pregain_db`).
        let normalise = self.choice_control(
            "normalise",
            &[
                ("off", t!(Off)),
                ("quiet", t!(NormaliseQuiet)),
                ("normal", t!(NormaliseNormal)),
                ("loud", t!(NormaliseLoud)),
            ],
            if p.normalise {
                p.normalise_level.as_str()
            } else {
                "off"
            },
            |p, k| {
                p.normalise = k != "off";
                if k != "off" {
                    p.normalise_level = k.into();
                }
            },
            window,
            cx,
        );
        // Kapalı / 2 / 5 / 8 / 12 sn (`Settings::crossfade_ms`) / AutoMix (`Settings::automix`).
        let crossfade = self.choice_control(
            "crossfade",
            &[
                (CROSSFADE_KEYS[0], t!(Off)),
                (CROSSFADE_KEYS[1], t!(CrossfadeSec2)),
                (CROSSFADE_KEYS[2], t!(CrossfadeSec5)),
                (CROSSFADE_KEYS[3], t!(CrossfadeSec8)),
                (CROSSFADE_KEYS[4], t!(CrossfadeSec12)),
                (AUTOMIX_KEY, t!(AutoMixName)),
            ],
            crossfade_choice(p),
            pick_crossfade,
            window,
            cx,
        );
        let bounds = self.sui.device_bounds.clone();
        let (device_stop, device_look) =
            self.stop_look("device-field", super::tab::PAGE, true, window, cx);
        let field = div()
            .relative()
            .w(px(280.))
            .child(
                select_field(
                    "device-field",
                    device_label(&p.output_device, self.sui.devices.as_ref()),
                    self.sui.device_open,
                    device_look,
                    accent,
                )
                .when_some(device_stop, |el, h| {
                    el.child(self.focus_probe(&h)).track_focus(&h)
                })
                .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                    this.leave_button(ev, window);
                    this.toggle_devices(cx)
                })),
            )
            .child(
                canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .into_any_element();
        self.settings_section(
            t!(SectionSound),
            vec![
                setting_row(
                    true,
                    row_text(t!(RowQuality), t!(RowQualityDesc), None),
                    quality,
                ),
                setting_row(
                    false,
                    row_text(t!(RowNormalise), t!(RowNormaliseDesc), None),
                    normalise,
                ),
                setting_row(
                    false,
                    row_text(
                        t!(RowCrossfade),
                        t!(RowCrossfadeDesc),
                        p.automix.then(|| (t!(AutoMixNote).into(), false)),
                    ),
                    crossfade,
                ),
                setting_row(
                    false,
                    row_text(t!(RowDevice), t!(RowDeviceDesc), None),
                    field,
                ),
            ],
        )
    }

    /// Spotify Connect: telefondan çalma aktarmak için (varsayılan açık,
    /// deneysel). Açıkken telefonda görünen aygıt adı da ayarlanır.
    fn connect_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let p = &self.prefs;
        let on = self.switch_control(
            "connect",
            p.connect_enabled,
            |p, v| p.connect_enabled = v,
            window,
            cx,
        );
        let mut rows = vec![setting_row(
            true,
            row_text(
                t!(RowConnect),
                t!(RowConnectDesc),
                Some((t!(ConnectExperimental).into(), false)),
            ),
            on,
        )];
        if p.connect_enabled {
            // Ad uzunsa düğmeyi şişirmesin (tam ad pencerede düzenlenir).
            let shown: String = if p.device_name.chars().count() > 28 {
                p.device_name.chars().take(27).chain(['…']).collect()
            } else {
                p.device_name.clone()
            };
            let (stop, look) = self.stop_look("connect-name", super::tab::PAGE, true, window, cx);
            let name = secondary_button("connect-name", "pencil", shown, look, accent)
                .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
                .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                    this.leave_button(ev, window);
                    this.ask_device_name(cx);
                }))
                .into_any_element();
            rows.push(setting_row(
                false,
                row_text(t!(RowConnectName), t!(RowConnectNameDesc), None),
                name,
            ));
        }
        self.settings_section(t!(SectionConnect), rows)
    }

    /// Karıştırma ve sıra: sürüm eleme, açık sözlüleri atla, "Hiç çalma" listesi.
    fn shuffle_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let p = &self.prefs;
        let dedupe = self.switch_control(
            "dedupe",
            p.dedupe_versions,
            |p, v| p.dedupe_versions = v,
            window,
            cx,
        );
        let explicit = self.switch_control(
            "skip-explicit",
            p.skip_explicit,
            |p, v| p.skip_explicit = v,
            window,
            cx,
        );
        let mut rows = vec![
            setting_row(
                true,
                row_text(t!(RowDedupe), t!(RowDedupeDesc), None),
                dedupe,
            ),
            setting_row(
                false,
                row_text(t!(RowSkipExplicit), t!(RowSkipExplicitDesc), None),
                explicit,
            ),
            // Başlık satırı: denetimi yok, liste altında.
            setting_row(
                false,
                row_text_wrap(t!(RowNeverPlay), t!(RowNeverPlayDesc)),
                div().into_any_element(),
            ),
        ];
        if p.blocked_artists.is_empty() {
            rows.push(
                div()
                    .pb(px(S3))
                    .type_role(T_SECONDARY)
                    .text_color(label())
                    .child(t!(NeverPlayEmpty))
                    .into_any_element(),
            );
        }
        for (i, artist) in p.blocked_artists.iter().enumerate() {
            let (stop, look) = self.stop_look(
                format!("allow-artist-{i}"),
                super::tab::PAGE,
                true,
                window,
                cx,
            );
            let name = artist.name.clone();
            let uri = artist.uri.clone();
            let allow = secondary_button(
                ElementId::Name(format!("allow-artist-{i}").into()),
                "user-check",
                t!(NeverPlayAllow),
                look,
                accent,
            )
            .when_some(stop, |el, h| el.child(self.focus_probe(&h)).track_focus(&h))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.leave_button(ev, window);
                this.set_artist_blocked(name.clone(), uri.clone(), false, cx);
            }))
            .into_any_element();
            let left = div()
                .flex_1()
                .min_w_0()
                .flex()
                .type_role(T_BODY)
                .text_color(text())
                .child(
                    line_with_tip(
                        ElementId::Name(format!("blocked-artist-{i}").into()),
                        artist.name.clone(),
                    )
                    .flex()
                    .min_w_0(),
                );
            rows.push(setting_row(false, left, allow));
        }
        // Yerel olarak gizlenen şarkılar (bkz. `listening`).
        rows.extend(self.hidden_settings_rows(window, cx));
        rows.push(
            div()
                .pt(px(S4))
                .type_role(T_SECONDARY)
                .text_color(label())
                .child(t!(ShuffleSpreadNote))
                .into_any_element(),
        );
        self.settings_section(t!(SectionShuffle), rows)
    }

    fn behavior_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.prefs;
        let appearance = self.choice_control(
            "theme",
            &[
                ("dark", t!(ThemeDark)),
                ("light", t!(ThemeLight)),
                ("system", t!(ThemeSystem)),
            ],
            p.theme.key(),
            |p, k| p.theme = ThemePreference::from_key(k),
            window,
            cx,
        );
        // Dil: "Sistem" Windows'un görüntü dilini izler; seçim hemen uygulanır
        // (`apply_prefs` → `i18n::apply_preference`, ardından kök yenilenir).
        let language = self.choice_control(
            "language",
            &[
                ("system", t!(LangSystem)),
                ("en", t!(LangEnglish)),
                ("tr", t!(LangTurkish)),
            ],
            &p.language,
            |p, k| p.language = k.into(),
            window,
            cx,
        );
        let motion = self.choice_control(
            "motion",
            &[("system", t!(LangSystem)), ("on", t!(On)), ("off", t!(Off))],
            &p.reduce_motion,
            |p, k| p.reduce_motion = k.into(),
            window,
            cx,
        );
        let panel =
            self.switch_control("now-panel", p.now_panel, |p, v| p.now_panel = v, window, cx);
        let friends = self.switch_control(
            "friend-activity",
            p.friend_activity,
            |p, v| p.friend_activity = v,
            window,
            cx,
        );
        let close = self.choice_control(
            "close",
            &[("tray", t!(CloseToTray)), ("quit", t!(CloseQuit))],
            if p.close_to_tray { "tray" } else { "quit" },
            |p, k| p.close_to_tray = k == "tray",
            window,
            cx,
        );
        let hotkey = self.switch_control(
            "hotkey",
            p.global_hotkey,
            |p, v| p.global_hotkey = v,
            window,
            cx,
        );
        let private_row = self.private_setting_row(window, cx);
        let hotkey_note = self
            .sui
            .hotkey_note
            .clone()
            .map(|n| (SharedString::from(n), true));
        self.settings_section(
            t!(SectionBehavior),
            vec![
                setting_row(
                    true,
                    row_text(t!(RowTheme), t!(RowThemeDesc), None),
                    appearance,
                ),
                setting_row(
                    false,
                    row_text(t!(SettingsLanguageTitle), t!(SettingsLanguageDesc), None),
                    language,
                ),
                setting_row(
                    false,
                    row_text(t!(RowMotion), t!(RowMotionDesc), None),
                    motion,
                ),
                setting_row(
                    false,
                    row_text(t!(RowNowPanel), t!(RowNowPanelDesc), None),
                    panel,
                ),
                setting_row(
                    false,
                    row_text(t!(RowFriendActivity), t!(RowFriendActivityDesc), None),
                    friends,
                ),
                private_row,
                setting_row(false, row_text(t!(RowClose), t!(RowCloseDesc), None), close),
                setting_row(
                    false,
                    row_text(t!(RowHotkey), t!(RowHotkeyDesc), hotkey_note),
                    hotkey,
                ),
            ],
        )
    }

    fn startup_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let (auto_stop, auto_look) =
            self.stop_look("autostart", super::tab::PAGE, true, window, cx);
        let auto = switch("autostart", self.sui.autostart_on, auto_look, accent)
            .when_some(auto_stop, |el, h| {
                el.child(self.focus_probe(&h)).track_focus(&h)
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_autostart(cx)))
            .into_any_element();
        let auto_note = self
            .sui
            .autostart_failed
            .then(|| (SharedString::from(t!(AutostartFailed)), true));
        let resume =
            self.switch_control("resume", self.prefs.resume, |p, v| p.resume = v, window, cx);
        self.settings_section(
            t!(SectionStartup),
            vec![
                setting_row(
                    true,
                    row_text(t!(RowAutostart), t!(RowAutostartDesc), auto_note),
                    auto,
                ),
                setting_row(
                    false,
                    row_text(t!(RowResume), t!(RowResumeDesc), None),
                    resume,
                ),
            ],
        )
    }

    fn storage_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let allowed = engine::clearing_allowed();
        let size = match (&self.sui.cache_note, self.sui.cache_bytes) {
            (Some(note), _) => note.clone(),
            (None, Some(b)) => tf!(CacheUsing, format_size(b)),
            (None, None) => t!(CacheSizing).into(),
        };
        let dir = engine::settings_cache_dir();
        let dir_text = dir
            .as_ref()
            .map_or_else(|| t!(NotFound).to_string(), |d| d.display().to_string());
        let (clear_stop, clear_look) =
            self.stop_look("clear-cache", super::tab::PAGE, allowed, window, cx);
        let clear = secondary_button("clear-cache", "trash-2", t!(ClearCache), clear_look, accent)
            .when_some(clear_stop, |el, h| {
                el.child(self.focus_probe(&h)).track_focus(&h)
            })
            .when(allowed, |el| {
                el.on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                    this.leave_button(ev, window);
                    this.ask_clear_cache(cx)
                }))
            })
            .into_any_element();
        let (open_stop, open_look) =
            self.stop_look("open-cache", super::tab::PAGE, dir.is_some(), window, cx);
        let open = secondary_button(
            "open-cache",
            "folder-open",
            t!(OpenFolder),
            open_look,
            accent,
        )
        .when_some(open_stop, |el, h| {
            el.child(self.focus_probe(&h)).track_focus(&h)
        })
        .when(
            dir.is_some() && std::env::var_os("OPENIFY_TEST_DISPLAY").is_none(),
            |el| {
                el.on_click(cx.listener(|_, _, _, _| {
                    if let Some(dir) = engine::settings_cache_dir() {
                        let _ = std::process::Command::new("explorer").arg(dir).spawn();
                    }
                }))
            },
        )
        .into_any_element();
        let (account, account_detail): (SharedString, SharedString) = match self.account_name() {
            Some(u) => (t!(SpotifyAccount).into(), tf!(ConnectedAs, u).into()),
            None => (t!(SpotifyAccount).into(), t!(ShownHereWhenConnected).into()),
        };
        let (out_stop, out_look) = self.stop_look("sign-out", super::tab::PAGE, true, window, cx);
        // Adım 17: giriş gerekirken / bekleyen girişte / motor çökünce bu düğme
        // o adımı yapar; bağlıyken "Çıkış yap".
        let (sign_icon, sign_label) = match &self.state.connection {
            Connection::NeedsLogin | Connection::LoginFailed(_) => ("user", t!(BtnSignIn)),
            Connection::LoggingIn => ("x", t!(BtnCancelLogin)),
            Connection::EngineStopped => ("refresh-cw", t!(BtnRestart)),
            _ => ("log-out", t!(SignOut)),
        };
        let sign_out = secondary_button("sign-out", sign_icon, sign_label, out_look, accent)
            .when_some(out_stop, |el, h| {
                el.child(self.focus_probe(&h)).track_focus(&h)
            })
            .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                this.leave_button(ev, window);
                match this.state.connection {
                    Connection::NeedsLogin | Connection::LoginFailed(_) => this.start_login(cx),
                    Connection::LoggingIn => this.cancel_login(cx),
                    Connection::EngineStopped => this.restart_engine(cx),
                    _ => this.ask_sign_out(cx),
                }
            }))
            .into_any_element();
        // Adım 17: kayıt klasörünü aç (günlük + çökme raporları).
        let log_dir = crate::crash::log_dir();
        let (log_stop, log_look) =
            self.stop_look("open-logs", super::tab::PAGE, log_dir.is_some(), window, cx);
        let open_logs = secondary_button(
            "open-logs",
            "folder-open",
            t!(BtnOpenLogFolder),
            log_look,
            accent,
        )
        .when_some(log_stop, |el, h| {
            el.child(self.focus_probe(&h)).track_focus(&h)
        })
        .when(
            log_dir.is_some() && std::env::var_os("OPENIFY_TEST_DISPLAY").is_none(),
            |el| {
                el.on_click(cx.listener(|_, _, _, _| {
                    if let Some(dir) = crate::crash::log_dir() {
                        let _ = std::process::Command::new("explorer").arg(dir).spawn();
                    }
                }))
            },
        )
        .into_any_element();
        self.settings_section(
            t!(SectionStorage),
            vec![
                setting_row(true, row_text(t!(Cache), tf!(CacheDesc, size), None), clear),
                setting_row(false, row_text(t!(CacheLocation), dir_text, None), open),
                setting_row(
                    false,
                    row_text(t!(SettingsLogTitle), t!(SettingsLogDetail), None),
                    open_logs,
                ),
                setting_row(false, row_text(&account, account_detail, None), sign_out),
            ],
        )
    }

    fn about_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.accent().text;
        let status = update::status();
        let can_check = !matches!(
            status,
            update::Status::NotInstalled | update::Status::Off | update::Status::Checking
        );
        let (check_stop, check_look) =
            self.stop_look("check-update", super::tab::PAGE, can_check, window, cx);
        let check = secondary_button(
            "check-update",
            "refresh-cw",
            t!(CheckNow),
            check_look,
            accent,
        )
        .when_some(check_stop, |el, h| {
            el.child(self.focus_probe(&h)).track_focus(&h)
        })
        .when(can_check, |el| {
            el.on_click(cx.listener(|_, _, _, cx| {
                update::check_now(|_| {});
                // Sonuç birkaç saniye sonra: bitene dek sayfa saniyede bir yenilenir.
                cx.spawn(async move |this, cx| {
                    for _ in 0..30 {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(1))
                            .await;
                        let done = update::status() != update::Status::Checking;
                        let _ = this.update(cx, |_, cx| cx.notify());
                        if done {
                            break;
                        }
                    }
                })
                .detach();
                cx.notify();
            }))
        })
        .into_any_element();
        let (lic_stop, lic_look) =
            self.stop_look("licenses-toggle", super::tab::PAGE, true, window, cx);
        let licenses_button = secondary_button(
            "licenses-toggle",
            "info",
            if self.sui.licenses_open {
                t!(Hide)
            } else {
                t!(Show)
            },
            lic_look,
            accent,
        )
        .when_some(lic_stop, |el, h| {
            el.child(self.focus_probe(&h)).track_focus(&h)
        })
        .on_click(cx.listener(|this, _, _, cx| {
            this.sui.licenses_open = !this.sui.licenses_open;
            this.sui.license_text = None;
            cx.notify();
        }))
        .into_any_element();

        let mut rows = vec![
            setting_row(
                true,
                row_text(
                    &format!("{} {}", crate::brand::APP_NAME, update::version()),
                    update_text(&status),
                    None,
                ),
                check,
            ),
            setting_row(
                false,
                row_text(t!(Licenses), t!(LicensesDesc), None),
                licenses_button,
            ),
        ];
        if self.sui.licenses_open {
            rows.push(self.licenses_list(cx));
        }
        rows.push(
            div()
                .pt(px(S4))
                .type_role(T_SECONDARY)
                .text_color(label())
                .child(t!(AboutDisclaimer))
                .into_any_element(),
        );
        self.settings_section(t!(About), rows)
    }

    /// Açık kaynak bileşenler; yazı tipi ve simge satırlarına tıklayınca lisans metni.
    fn licenses_list(&self, cx: &mut Context<Self>) -> AnyElement {
        // Satır içindeki boşluk (ton katmanı için) dışarı taşar: yazılar
        // satırların yazısıyla hizalı kalır.
        let mut list = div().mx(px(-S2)).pb(px(S2)).flex().flex_col();
        for (i, (name, license, role, full)) in notices().iter().enumerate() {
            let open = self.sui.license_text == Some(i);
            let has_text = full.is_some();
            let row = div()
                .id(ElementId::Name(format!("license-{i}").into()))
                .min_h(px(S6))
                .px(px(S2))
                .rounded(px(R_MD))
                .flex()
                .items_center()
                .gap(px(S3))
                .type_role(T_SECONDARY)
                .when(has_text, |el| {
                    el.cursor_pointer()
                        .child(super::tween::hover_fill(
                            veil(VEIL_HOVER),
                            veil(VEIL_PRESSED),
                            0.,
                            R_MD,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sui.license_text = if this.sui.license_text == Some(i) {
                                None
                            } else {
                                Some(i)
                            };
                            cx.notify();
                        }))
                })
                .child(
                    div()
                        .w(px(176.))
                        .flex_none()
                        .text_color(text())
                        .child(*name),
                )
                .child(
                    div()
                        .w(px(160.))
                        .flex_none()
                        .text_color(muted())
                        .child(*license),
                )
                .child(div().flex_1().min_w_0().text_color(label()).child(*role))
                .when(has_text, |el| {
                    el.child(icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        14.,
                        label(),
                    ))
                });
            list = list.child(row);
            if open && let Some(full) = full {
                list = list.child(
                    div()
                        .my(px(S1))
                        .p(px(S3))
                        .rounded(px(R_MD))
                        .bg(shade(SHADE_INPUT))
                        .type_role(T_LABEL)
                        .text_color(muted())
                        .child(full.trim().to_string()),
                );
            }
        }
        list.into_any_element()
    }

    /// Optional sharing integration, hidden until the application ID is configured.
    #[cfg(windows)]
    fn discord_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let control = self.switch_control(
            "discord-presence",
            self.prefs.discord_presence,
            |prefs, on| prefs.discord_presence = on,
            window,
            cx,
        );
        self.settings_section(
            t!(SectionIntegrations),
            vec![setting_row(
                true,
                row_text(t!(RowDiscordPresence), t!(RowDiscordPresenceDesc), None),
                control,
            )],
        )
    }

    /// Ayarlar sayfasının gövdesi.
    pub(super) fn settings_body(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let meta = div().child(t!(SettingsSubtitle)).into_any_element();
        div()
            .flex()
            .flex_col()
            .child(self.page_header(None, t!(Settings), meta, Vec::new()))
            .child(self.audio_section(window, cx))
            .child(self.eq_section(window, cx))
            .child(self.shuffle_section(window, cx))
            .child(self.connect_section(window, cx))
            .when(cfg!(windows), |view| {
                #[cfg(windows)]
                let view = view.when(crate::discord::available(), |view| {
                    view.child(self.discord_section(window, cx))
                });
                view
            })
            .child(self.behavior_section(window, cx))
            .child(self.startup_section(window, cx))
            .child(self.storage_section(window, cx))
            .child(self.about_section(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{MOTION_CHOICES, QUALITIES};

    #[test]
    fn initial_comes_from_a_real_name_only() {
        assert_eq!(profile_initial("furkan"), Some('F'));
        assert_eq!(
            profile_initial("  ışık "),
            Some('I'),
            "Türkçe harf büyütülür"
        );
        assert_eq!(profile_initial(""), None);
        // Spotify'ın ürettiği kimlikler ad değil.
        assert_eq!(profile_initial("31abc4defghij5klmnopqrstuv"), None);
        assert_eq!(profile_initial("abcdefghijklmnopqrstuvwxy12"), None);
        assert_eq!(
            profile_initial("123456"),
            None,
            "rakamla başlayan harf değil"
        );
    }

    #[test]
    fn sizes_read_naturally_with_a_decimal_comma() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(900), "900 B");
        assert_eq!(format_size(340 * 1024), "340 KB");
        assert_eq!(format_size(1_300_000), "1,2 MB");
        assert_eq!(format_size(250 * 1024 * 1024), "250 MB");
        assert_eq!(format_size(1_288_490_189), "1,2 GB");
    }

    #[test]
    fn every_update_status_has_a_plain_sentence() {
        use update::Status::*;
        for s in [
            NotInstalled,
            Off,
            Waiting,
            Checking,
            UpToDate,
            Ready("0.2.0".into()),
            Failed,
            RolledBack("0.2.0".into()),
        ] {
            let t = update_text(&s);
            assert!(!t.is_empty() && !t.contains("Status"), "{t}");
        }
        assert!(update_text(&Ready("0.2.0".into())).contains("0.2.0"));
        assert!(update_text(&RolledBack("0.2.0".into())).contains("0.2.0"));
    }

    #[test]
    fn device_label_marks_missing_devices() {
        let known = vec!["Hoparlör".to_string(), "Kulaklık".to_string()];
        assert_eq!(device_label(&None, Some(&known)), "Sistem varsayılanı");
        assert_eq!(
            device_label(&Some("Kulaklık".into()), Some(&known)),
            "Kulaklık"
        );
        assert_eq!(
            device_label(&Some("Eski".into()), Some(&known)),
            "Eski (bağlı değil)"
        );
        // Liste henüz gelmediyse adı olduğu gibi göster.
        assert_eq!(device_label(&Some("Eski".into()), None), "Eski");
    }

    #[test]
    fn page_options_cover_every_stored_value() {
        // Sayfadaki seçenekler Settings'in kabul ettiği değerlerle aynı.
        let ui_quality = ["normal", "high", "very_high"];
        assert_eq!(ui_quality.len(), QUALITIES.len());
        assert!(QUALITIES.iter().all(|(k, _)| ui_quality.contains(k)));
        let ui_motion = ["system", "on", "off"];
        assert!(MOTION_CHOICES.iter().all(|k| ui_motion.contains(k)));
    }

    #[test]
    fn crossfade_choices_round_trip_and_odd_values_show_the_nearest() {
        for (key, ms) in CROSSFADE_KEYS.iter().zip(CROSSFADE_CHOICES_MS) {
            assert_eq!(crossfade_ms(key), ms);
            assert_eq!(crossfade_key(ms), *key);
        }
        assert_eq!(crossfade_key(3_400), "2");
        assert_eq!(crossfade_key(3_600), "5");
        assert_eq!(crossfade_key(12_000), "12");
        assert_eq!(crossfade_key(1), "2", "kapalı yalnız tam 0");
        // AutoMix: açıkken seçili görünen o; bir süre seçmek onu kapatır, açmak süreyi bozmaz.
        let mut p = Settings {
            crossfade_ms: 8_000,
            ..Settings::default()
        };
        assert_eq!(crossfade_choice(&p), "8");
        pick_crossfade(&mut p, AUTOMIX_KEY);
        assert_eq!((crossfade_choice(&p), p.crossfade_ms), (AUTOMIX_KEY, 8_000));
        pick_crossfade(&mut p, "5");
        assert_eq!(
            (crossfade_choice(&p), p.automix, p.crossfade_ms),
            ("5", false, 5_000)
        );
        pick_crossfade(&mut p, "0");
        assert_eq!((crossfade_choice(&p), p.crossfade_ms), ("0", 0));
    }

    #[test]
    fn notices_are_complete_and_ship_the_license_texts_they_need() {
        assert!(notices().len() >= 10);
        for (name, license, role, full) in notices() {
            assert!(!name.is_empty() && !license.is_empty() && !role.is_empty());
            if license.contains("OFL") || license.contains("ISC") {
                assert!(
                    full.is_some_and(|t| t.len() > 200),
                    "{name}: lisans metni gömülü olmalı"
                );
            }
        }
    }
}
