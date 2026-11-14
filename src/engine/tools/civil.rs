//! Takvim yardımcıları (UTC'den dakika farkı verilerek yerel saat): yıl, saat,
//! tarih damgası. Bağımlılık yok; Howard Hinnant'ın algoritması.

const DAY_SECS: i64 = 86_400;

/// 1970-01-01'den gün sayısı → (yıl, ay, gün).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn local_secs(at_ms: i64, offset_minutes: i64) -> i64 {
    at_ms.div_euclid(1000) + offset_minutes * 60
}

/// Yerel takvim yılı.
pub fn year_of(at_ms: i64, offset_minutes: i64) -> i64 {
    civil_from_days(local_secs(at_ms, offset_minutes).div_euclid(DAY_SECS)).0
}

/// Yerel saat (0..=23).
pub fn hour_of(at_ms: i64, offset_minutes: i64) -> usize {
    (local_secs(at_ms, offset_minutes).rem_euclid(DAY_SECS) / 3600) as usize
}

/// Haftanın günü, Pazartesi 0 ile Pazar 6 arasında.
pub fn weekday_of(at_ms: i64, offset_minutes: i64) -> usize {
    (local_secs(at_ms, offset_minutes).div_euclid(DAY_SECS) + 3).rem_euclid(7) as usize
}

/// UTC takvim tarih ve saatini Unix milisaniyesine çevirir.
pub fn unix_ms(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millis: u32,
) -> i64 {
    let days = days_from_civil(year, month, day);
    ((days * DAY_SECS + i64::from(hour * 3600 + minute * 60 + second)) * 1000) + i64::from(millis)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2026-11-11" (dosya adı için).
pub fn date_stamp(at_ms: i64, offset_minutes: i64) -> String {
    let (y, m, d) = civil_from_days(local_secs(at_ms, offset_minutes).div_euclid(DAY_SECS));
    format!("{y:04}-{m:02}-{d:02}")
}

/// (yıl, ay, gün) → 1970-01-01'den gün sayısı.
/// Windows'un o anki saat dilimi farkı (UTC'den dakika; Türkiye için 180).
/// Okunamazsa ya da Windows dışında 0.
pub fn local_offset_minutes() -> i64 {
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::SYSTEMTIME,
            System::SystemInformation::{GetLocalTime, GetSystemTime},
        };
        let minutes = |t: &SYSTEMTIME| {
            days_from_civil(i64::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay)) * 1440
                + i64::from(t.wHour) * 60
                + i64::from(t.wMinute)
        };
        // SAFETY: parametresiz, yalnız saat okur.
        let (local, utc) = unsafe { (GetLocalTime(), GetSystemTime()) };
        minutes(&local) - minutes(&utc)
    }
    #[cfg(not(windows))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates_and_hours() {
        // 2026-11-11 12:34:56 UTC.
        let ms = 1_794_400_496_000;
        assert_eq!(date_stamp(ms, 0), "2026-11-11");
        assert_eq!(year_of(ms, 0), 2026);
        assert_eq!(hour_of(ms, 0), 12);
        // +03:00 (Türkiye): akşam 15.
        assert_eq!(hour_of(ms, 180), 15);
        // Gün sınırı: UTC 22:00 + 3 sa = ertesi gün 01:00.
        let late = ms + (22 * 3600 - (12 * 3600 + 34 * 60 + 56)) * 1000;
        assert_eq!(
            (date_stamp(late, 0), date_stamp(late, 180)),
            ("2026-11-11".into(), "2026-11-12".into())
        );
        assert_eq!(hour_of(late, 180), 1);
        // Yıl sınırı ve eski (1970 öncesi değil ama sıfır) an.
        assert_eq!(year_of(0, 0), 1970);
        assert_eq!(year_of(1_767_225_599_000, 0), 2025); // 2025-12-31 23:59:59 UTC
        assert_eq!(year_of(1_767_225_599_000, 180), 2026);
        let monday_midnight = unix_ms(2023, 5, 1, 0, 0, 0, 0);
        assert_eq!(weekday_of(monday_midnight, 0), 0);
        assert_eq!(
            weekday_of(monday_midnight, -60),
            6,
            "UTC'de Pazartesi, -1 saat yerelde Pazar"
        );
        assert_eq!(hour_of(monday_midnight, 180), 3);
    }

    #[test]
    fn local_offset_is_a_sane_timezone() {
        let o = local_offset_minutes();
        assert!((-14 * 60..=14 * 60).contains(&o), "{o}");
    }
}
