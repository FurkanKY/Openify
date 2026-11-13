//! Windows'un UYGULAMA renk modu. Görev çubuğunun `SystemUsesLightTheme`
//! ayarından ayrıdır; bu modül kayıt defterini yalnız okur.

use crate::settings::ThemePreference;

/// Sınama zorlaması yalnız yalıtılmış koşuda geçerli; kalıcı tercihe yazılmaz.
pub fn light(preference: ThemePreference) -> bool {
    let test_run = std::env::var_os("OPENIFY_TEST_DISPLAY").is_some()
        || std::env::var_os("OPENIFY_SETTINGS_PATH").is_some();
    let forced = std::env::var("OPENIFY_TEST_THEME").ok();
    resolve_light(
        preference,
        forced.as_deref(),
        test_run,
        if preference == ThemePreference::System {
            apps_use_light_theme()
        } else {
            None
        },
    )
}

/// Öncelik: yalıtılmış sınama > açık/koyu tercih > Windows uygulama modu.
/// Windows değeri okunamazsa eski davranış (koyu) korunur.
fn resolve_light(
    preference: ThemePreference,
    forced: Option<&str>,
    test_run: bool,
    system_light: Option<bool>,
) -> bool {
    if test_run {
        match forced {
            Some("light") => return true,
            Some("dark") => return false,
            _ => {}
        }
    }
    match preference {
        ThemePreference::Dark => false,
        ThemePreference::Light => true,
        ThemePreference::System => system_light.unwrap_or(false),
    }
}

fn apps_use_light_theme() -> Option<bool> {
    use windows::{
        Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        core::w,
    };
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    match (result.is_ok(), value) {
        (true, 0) => Some(false),
        (true, 1) => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_ignore_windows_changes() {
        for system in [None, Some(false), Some(true)] {
            assert!(!resolve_light(ThemePreference::Dark, None, false, system));
            assert!(resolve_light(ThemePreference::Light, None, false, system));
        }
    }

    #[test]
    fn system_preference_follows_app_mode_and_falls_back_to_dark() {
        assert!(!resolve_light(ThemePreference::System, None, false, None));
        assert!(!resolve_light(
            ThemePreference::System,
            None,
            false,
            Some(false)
        ));
        assert!(resolve_light(
            ThemePreference::System,
            None,
            false,
            Some(true)
        ));
    }

    #[test]
    fn isolated_test_override_wins_over_every_preference() {
        for preference in [
            ThemePreference::Dark,
            ThemePreference::Light,
            ThemePreference::System,
        ] {
            for system in [None, Some(false), Some(true)] {
                assert!(resolve_light(preference, Some("light"), true, system));
                assert!(!resolve_light(preference, Some("dark"), true, system));
            }
        }
    }

    #[test]
    fn override_is_ignored_outside_tests_and_invalid_values_are_harmless() {
        assert!(!resolve_light(
            ThemePreference::Dark,
            Some("light"),
            false,
            Some(true)
        ));
        assert!(resolve_light(
            ThemePreference::Light,
            Some("dark"),
            false,
            Some(false)
        ));
        assert!(resolve_light(
            ThemePreference::System,
            Some("invalid"),
            true,
            Some(true)
        ));
    }
}
