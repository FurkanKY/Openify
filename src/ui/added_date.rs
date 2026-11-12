//! "Eklenme tarihi" sütununun yazısı: yakınsa göreli ("3 gün önce"), bir
//! aydan eskiyse tarih ("3 Kas 2025" / "Nov 3, 2025"). Tarih yerel saate
//! göre (Windows'un o anki saat dilimi); ay adları i18n tablosundan.

use crate::i18n::K;
use crate::{t, tf, tp};

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// Bir aydan (30 gün) eski eklenmeler tarih olarak yazılır.
const RELATIVE_UNTIL_DAYS: i64 = 30;

/// `at_ms` (unix ms) anının, `now_ms`'ye göre yazısı. Gelecekteki an (saat
/// kayması) "Az önce" sayılır.
pub fn added_label(now_ms: i64, at_ms: i64) -> String {
    added_label_in(now_ms, at_ms, local_offset_minutes())
}

/// `added_label`, yerel saat farkı (UTC'den dakika) verilerek: sınanabilir.
pub fn added_label_in(now_ms: i64, at_ms: i64, offset_minutes: i64) -> String {
    let secs = ((now_ms - at_ms) / 1000).max(0);
    if secs < MINUTE {
        t!(AddedNow).into()
    } else if secs < HOUR {
        tp!(AddedMinutes, secs / MINUTE)
    } else if secs < DAY {
        tp!(AddedHours, secs / HOUR)
    } else if secs < 7 * DAY {
        tp!(AddedDays, secs / DAY)
    } else if secs < RELATIVE_UNTIL_DAYS * DAY {
        tp!(AddedWeeks, secs / (7 * DAY))
    } else {
        date_label(at_ms, offset_minutes)
    }
}

/// "3 Kas 2025" / "Nov 3, 2025".
pub fn date_label(at_ms: i64, offset_minutes: i64) -> String {
    let local_secs = at_ms.div_euclid(1000) + offset_minutes * 60;
    let (y, m, d) = civil_from_days(local_secs.div_euclid(DAY));
    const MONTHS: [K; 12] = [
        crate::k!(MonthJan),
        crate::k!(MonthFeb),
        crate::k!(MonthMar),
        crate::k!(MonthApr),
        crate::k!(MonthMay),
        crate::k!(MonthJun),
        crate::k!(MonthJul),
        crate::k!(MonthAug),
        crate::k!(MonthSep),
        crate::k!(MonthOct),
        crate::k!(MonthNov),
        crate::k!(MonthDec),
    ];
    let month = crate::i18n::t(MONTHS[(m as usize - 1).min(11)]);
    tf!(DateMonthDayYear, month, d, y)
}

/// 1970-01-01'den gün sayısı → (yıl, ay, gün); Howard Hinnant'ın algoritması.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
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

/// (yıl, ay, gün) → 1970-01-01'den gün sayısı.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Yerel saatin UTC'den farkı (dakika; Türkiye için 180). Windows'un o anki
/// saat dilimi ve yaz saati durumu; okunamazsa 0 (UTC).
pub fn local_offset_minutes() -> i64 {
    use windows::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::{GetLocalTime, GetSystemTime}};
    let minutes = |t: &SYSTEMTIME| {
        days_from_civil(i64::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay)) * 1440
            + i64::from(t.wHour) * 60
            + i64::from(t.wMinute)
    };
    // SAFETY: parametresiz, yalnız saat okur.
    let (local, utc) = unsafe { (GetLocalTime(), GetSystemTime()) };
    minutes(&local) - minutes(&utc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, with_lang};

    const NOW: i64 = 1_762_300_800_000; // 2025-11-05 00:00:00 UTC

    fn ago(secs: i64) -> i64 {
        NOW - secs * 1000
    }

    fn label(lang: Lang, secs: i64) -> String {
        with_lang(lang, || added_label_in(NOW, ago(secs), 180))
    }

    #[test]
    fn recent_additions_are_relative_in_both_languages() {
        let cases: [(i64, &str, &str); 11] = [
            (0, "Just now", "Az önce"),
            (59, "Just now", "Az önce"),
            (60, "1 minute ago", "1 dk önce"),
            (59 * 60, "59 minutes ago", "59 dk önce"),
            (HOUR, "1 hour ago", "1 saat önce"),
            (23 * HOUR + 59 * 60, "23 hours ago", "23 saat önce"),
            (DAY, "1 day ago", "1 gün önce"),
            (3 * DAY, "3 days ago", "3 gün önce"),
            (7 * DAY, "1 week ago", "1 hafta önce"),
            (13 * DAY, "1 week ago", "1 hafta önce"),
            (29 * DAY, "4 weeks ago", "4 hafta önce"),
        ];
        for (secs, en, tr) in cases {
            assert_eq!(label(Lang::En, secs), en, "{secs} s");
            assert_eq!(label(Lang::Tr, secs), tr, "{secs} s");
        }
        // Gelecekteki an (saat kayması) "Az önce".
        assert_eq!(with_lang(Lang::Tr, || added_label_in(NOW, NOW + 5_000, 180)), "Az önce");
    }

    #[test]
    fn old_additions_are_dates_in_local_time() {
        // 30 gün ve ötesi tarih; 2025-11-05 − 30 gün = 2025-10-06 00:00 UTC = 03:00 yerel.
        assert_eq!(label(Lang::En, 30 * DAY), "Oct 6, 2025");
        assert_eq!(label(Lang::Tr, 30 * DAY), "6 Eki 2025");
        assert_eq!(label(Lang::Tr, 400 * DAY), "1 Eki 2024");
        // Yerel saat farkı gün sınırını geçirir: 22:30 UTC → Türkiye'de ertesi gün.
        let at = 1_730_759_400_000; // 2024-11-04 22:30:00 UTC
        assert_eq!(with_lang(Lang::En, || date_label(at, 0)), "Nov 4, 2024");
        assert_eq!(with_lang(Lang::En, || date_label(at, 180)), "Nov 5, 2024");
        assert_eq!(with_lang(Lang::Tr, || date_label(at, -300)), "4 Kas 2024");
    }

    #[test]
    fn civil_conversions_agree_both_ways() {
        for days in [-800_000, -1, 0, 1, 59, 60, 365, 11_000, 19_000, 20_400, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_722), (2023, 12, 31));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn the_local_offset_is_sane() {
        let o = local_offset_minutes();
        assert!((-14 * 60..=14 * 60).contains(&o), "{o}");
    }
}
