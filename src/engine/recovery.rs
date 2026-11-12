//! Çalma kurtarma ilkesi: oturum sessizce ölünce (TCP kopmuş ama librespot
//! bunu 5-10 sn sonra fark ediyor) şarkı anahtarı istekleri zaman aşımına
//! uğrar, şarkı "anahtarsız" yüklenir ve ya sessizlik ya geç başlama olur.
//!
//! Burada yalnız karar var (saf işlevler, motorun durumundan bağımsız, sınanır):
//! hangi hatada ne yapılır, yeniden bağlanma en erken ne zaman, yeniden
//! denemede hangi konumdan sürülür. Uygulama `mod.rs`'te (`on_unavailable`,
//! `on_load_stall`, `on_reconnected`).
//!
//! Değişmezler:
//! - Bir şarkı için bağlanıp-yeniden-yükleme EN FAZLA 1 kez yapılır; o da
//!   olmazsa atlanır / durulur (döngü yok).
//! - Yeniden bağlanmalar arasında her zaman geri çekilme var (Spotify'ı
//!   boğma); art arda başarısız bağlanmada aralık uzar.
//! - Önyüklenen (sıradaki) şarkının hatası çalan şarkıyı atlatmaz.

use std::time::Duration;

/// İki yeniden bağlanma denemesi arasında en az bu kadar beklenir (ilk hata).
pub const RECONNECT_BACKOFF: Duration = Duration::from_secs(10);
/// Art arda başarısız bağlanmalarda geri çekilmenin tavanı.
pub const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Şarkı yüklemesi bu kadar sürerse oturum ölü sayılır. Sağlıklı yükleme
/// bir saniyenin altında; anahtar cevabı gecikse de (Spotify yoğunlukta
/// ~3,5 sn gördük) bir deneme 4 sn (`vendor/librespot-core/OPENIFY.md`).
/// Beklemezsek anahtar istekleri 3 deneme + aralar ≈ 15 sn sürüyordu.
pub const LOAD_STALL: Duration = Duration::from_secs(5);

/// `failed` kez üst üste bağlanılamadıktan sonraki en küçük bekleme:
/// 10, 20, 40, 60, 60… sn.
pub fn backoff_for(failed_attempts: u32) -> Duration {
    let factor = 1u32 << failed_attempts.min(3);
    (RECONNECT_BACKOFF * factor).min(RECONNECT_BACKOFF_MAX)
}

/// Açılışta (ağ yokken) bağlanılamayınca ilk yeniden deneme aralığı: internet
/// gelince çabuk bağlansın diye `RECONNECT_BACKOFF`'tan kısa.
pub const CONNECT_RETRY_FIRST: Duration = Duration::from_secs(5);

/// Açılıştaki bağlanma `failed` kez üst üste başarısız olduktan sonra ne kadar
/// beklenir: 5, 10, 20, 40, 60, 60… sn (tavan `RECONNECT_BACKOFF_MAX`).
pub fn connect_backoff_for(failed_attempts: u32) -> Duration {
    let factor = 1u32 << failed_attempts.min(4);
    (CONNECT_RETRY_FIRST * factor).min(RECONNECT_BACKOFF_MAX)
}

/// Son bağlanma denemesinden bu yana geçen süreyle yeni deneme serbest mi?
/// `None`: hiç denenmedi.
pub fn backoff_elapsed(since_last: Option<Duration>, failed_attempts: u32) -> bool {
    since_last.is_none_or(|t| t >= backoff_for(failed_attempts))
}

/// Çalınamayan bir şarkının durumu.
#[derive(Debug, Clone, Copy)]
pub struct Failure {
    /// Çalar anahtar alamadığını bildirdi (zaman aşımı, ret, oturum düştü).
    pub key_failure: bool,
    /// Çalan (yüklenen) şarkı mı; `false`: önyüklenen sıradaki şarkı.
    pub current: bool,
    /// Bu şarkı için bağlanıp yeniden yükleme zaten yapıldı.
    pub retried: bool,
    /// Yeniden bağlanma şu an sürüyor.
    pub reconnecting: bool,
    pub since_reconnect: Option<Duration>,
    pub reconnect_failures: u32,
    /// Üst üste çalınamayan şarkı sayısı (bu dahil) ve izin verilen en çok.
    pub consecutive: u32,
    pub max_consecutive: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Yeniden bağlan (sürüyorsa ona kat) ve AYNI şarkıyı bir kez daha yükle.
    RetryAfterReconnect,
    /// Önyükleme anahtar alamadı: oturum şüpheli, yeniden bağlan; çalan
    /// şarkıya dokunma, sıradakini bağlanınca yeniden önyükle.
    ReconnectOnly,
    /// Hiçbir şey yapma (önyükleme hatası ve oturum şüphesi yok).
    Ignore,
    /// Bu şarkıyı bırak, sıradakine geç.
    Skip,
    /// Üst üste çok şarkı çalınamadı: dur ve söyle.
    GiveUp,
}

pub fn on_failure(f: Failure) -> Recovery {
    let may_reconnect = f.reconnecting || backoff_elapsed(f.since_reconnect, f.reconnect_failures);
    if !f.current {
        // Sıradaki şarkı önyüklenemedi. Çalan şarkı kesilmez; yalnız anahtar
        // hatasıysa oturum şüpheli: yenile, çalan şarkı yarıda kalmasın.
        return if f.key_failure && !f.reconnecting && may_reconnect {
            Recovery::ReconnectOnly
        } else {
            Recovery::Ignore
        };
    }
    if f.consecutive > f.max_consecutive {
        return Recovery::GiveUp;
    }
    if f.retried {
        return Recovery::Skip;
    }
    if may_reconnect {
        Recovery::RetryAfterReconnect
    } else {
        Recovery::Skip
    }
}

/// Yükleme `elapsed` süredir sürüyor ve hâlâ ne çalma ne hata geldi: oturumu
/// yenileyip şarkıyı yeniden yüklemek gerekir mi? (Tek seferlik; ikinci
/// yükleme takılırsa çaların kendi hatası beklenir.)
pub fn stall_should_reconnect(
    elapsed: Duration,
    retried: bool,
    reconnecting: bool,
    since_reconnect: Option<Duration>,
    reconnect_failures: u32,
) -> bool {
    elapsed >= LOAD_STALL
        && !retried
        && !reconnecting
        && backoff_elapsed(since_reconnect, reconnect_failures)
}

/// Yeniden bağlanınca şarkı hangi konumdan, çalarak mı yoksa duraklatılmış
/// mı yüklensin? `None`: yeniden yüklenmez (boştayken, hiçbir şey yüklü ya da
/// çalmıyorken oturum düşerse kullanıcı istemeden müzik başlamasın, Spotify'a
/// anahtar isteği gitmesin).
///
/// - `last`: son bilinen konum (ms), bundan geçen süre, çalıyor muydu.
/// - `load_position`: şarkı çalara en son bu konumdan yüklendi (kaldığı yerden
///   sürme, arama); `Playing` henüz gelmediyse `last` boştur.
/// - `loading`: bir yükleme sürüyordu (kullanıcı çalmak istedi).
pub fn retry_target(
    last: Option<(u32, Duration, bool)>,
    load_position: u32,
    loading: bool,
) -> Option<(u32, bool)> {
    match last {
        Some((ms, age, true)) => Some((ms.saturating_add(age.as_millis() as u32), true)),
        Some((ms, _, false)) => Some((ms, false)),
        None if loading => Some((load_position, true)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn base() -> Failure {
        Failure {
            key_failure: true,
            current: true,
            retried: false,
            reconnecting: false,
            since_reconnect: None,
            reconnect_failures: 0,
            consecutive: 1,
            max_consecutive: 3,
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff_for(0), S(10));
        assert_eq!(backoff_for(1), S(20));
        assert_eq!(backoff_for(2), S(40));
        assert_eq!(backoff_for(3), S(60));
        assert_eq!(backoff_for(50), S(60));
    }

    #[test]
    fn startup_connect_backoff_starts_short_and_is_capped() {
        let secs: Vec<u64> = (0..8).map(|n| connect_backoff_for(n).as_secs()).collect();
        assert_eq!(secs, [5, 10, 20, 40, 60, 60, 60, 60]);
        assert_eq!(
            connect_backoff_for(u32::MAX),
            RECONNECT_BACKOFF_MAX,
            "taşma yok"
        );
    }

    #[test]
    fn backoff_elapsed_edges() {
        assert!(backoff_elapsed(None, 0), "hiç denenmediyse serbest");
        assert!(!backoff_elapsed(Some(S(9)), 0));
        assert!(backoff_elapsed(Some(S(10)), 0));
        assert!(!backoff_elapsed(Some(S(19)), 1));
        assert!(backoff_elapsed(Some(S(20)), 1));
    }

    #[test]
    fn first_key_failure_reconnects_and_retries_same_track() {
        assert_eq!(on_failure(base()), Recovery::RetryAfterReconnect);
    }

    #[test]
    fn second_failure_of_same_track_is_skipped_not_looped() {
        let f = Failure {
            retried: true,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::Skip);
        // Bağlanma serbest olsa bile.
        let f = Failure {
            retried: true,
            since_reconnect: Some(S(600)),
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::Skip);
    }

    #[test]
    fn recent_reconnect_blocks_another_one() {
        let f = Failure {
            since_reconnect: Some(S(3)),
            ..base()
        };
        assert_eq!(
            on_failure(f),
            Recovery::Skip,
            "geri çekilme: Spotify boğulmaz"
        );
        let f = Failure {
            since_reconnect: Some(S(11)),
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::RetryAfterReconnect);
    }

    #[test]
    fn failed_reconnects_stretch_the_backoff() {
        let f = Failure {
            since_reconnect: Some(S(15)),
            reconnect_failures: 2,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::Skip);
        let f = Failure {
            since_reconnect: Some(S(40)),
            reconnect_failures: 2,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::RetryAfterReconnect);
    }

    #[test]
    fn failure_during_a_running_reconnect_joins_it() {
        // Geri çekilme süresi dolmamış olsa da süren bağlanmaya katılınır.
        let f = Failure {
            reconnecting: true,
            since_reconnect: Some(S(1)),
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::RetryAfterReconnect);
    }

    #[test]
    fn too_many_consecutive_failures_give_up() {
        let f = Failure {
            consecutive: 4,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::GiveUp);
        let f = Failure {
            consecutive: 3,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::RetryAfterReconnect);
    }

    #[test]
    fn preload_failure_never_skips_the_playing_track() {
        let f = Failure {
            current: false,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::ReconnectOnly);
        // Anahtar hatası değilse (ör. şarkı bölgede yok): sessizce geç.
        let f = Failure {
            current: false,
            key_failure: false,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::Ignore);
        // Geri çekilme sürüyorsa ya da zaten bağlanılıyorsa: dokunma.
        let f = Failure {
            current: false,
            since_reconnect: Some(S(2)),
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::Ignore);
        let f = Failure {
            current: false,
            reconnecting: true,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::Ignore);
        // Önyükleme hatası "üst üste hata" sayısına bakmaz.
        let f = Failure {
            current: false,
            consecutive: 99,
            ..base()
        };
        assert_eq!(on_failure(f), Recovery::ReconnectOnly);
    }

    #[test]
    fn stall_reconnects_only_once_and_after_the_threshold() {
        assert!(!stall_should_reconnect(S(4), false, false, None, 0));
        assert!(stall_should_reconnect(S(5), false, false, None, 0));
        assert!(
            !stall_should_reconnect(S(30), true, false, None, 0),
            "tek yeniden deneme"
        );
        assert!(
            !stall_should_reconnect(S(30), false, true, None, 0),
            "bağlanma sürüyor"
        );
        assert!(
            !stall_should_reconnect(S(30), false, false, Some(S(5)), 0),
            "geri çekilme"
        );
        assert!(stall_should_reconnect(S(30), false, false, Some(S(10)), 0));
    }

    #[test]
    fn stall_threshold_is_shorter_than_the_key_retry_chain() {
        // Anahtar yöneticisi 3 deneme × 4 sn + 1 + 2 sn aralar = 15 sn bekler.
        assert!(LOAD_STALL < S(15));
        // Ama tek denemeden (4 sn) uzun: yavaş ama canlı oturum hemen kesilmez.
        assert!(LOAD_STALL > S(4));
    }

    #[test]
    fn retry_resumes_at_the_same_position_with_the_same_intent() {
        // Çalıyordu: geçen süre eklenir, çalarak devam.
        assert_eq!(
            retry_target(Some((10_000, S(2), true)), 0, false),
            Some((12_000, true))
        );
        // Duraklatılmıştı: aynı konumda duraklatılmış.
        assert_eq!(
            retry_target(Some((10_000, S(60), false)), 0, false),
            Some((10_000, false))
        );
        // Yükleme sürerken takıldı (kaldığı yerden sürme): yükleme konumu korunur.
        assert_eq!(retry_target(None, 134_959, true), Some((134_959, true)));
        // Yüklemesi olmayan boş durumda yeniden yükleme yok (kendiliğinden çalmasın).
        assert_eq!(retry_target(None, 0, false), None);
    }

    #[test]
    fn retry_position_does_not_overflow() {
        assert_eq!(
            retry_target(
                Some((u32::MAX - 5, Duration::from_secs(10), true)),
                0,
                false
            ),
            Some((u32::MAX, true))
        );
    }
}
