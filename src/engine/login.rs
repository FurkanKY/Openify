//! Bağlantı ve giriş durumu: saf kararlar (motorun durumundan bağımsız, sınanır).
//!
//! Durumlar ve geçişler:
//!
//! ```text
//! Connecting ──ağ yok──────────────▶ Offline ──(geri çekilmeyle yeniden dener)──▶ Connecting
//! Connecting ──kayıt yok / ret─────▶ NeedsLogin ──StartLogin──▶ LoggingIn
//! Connecting ──hesap ücretsiz──────▶ NeedsPremium ──(Yeniden dene: tekrar bağlanır)──▶ Connecting
//! LoggingIn ──başarı───────────────▶ Connected
//! LoggingIn ──CancelLogin──────────▶ NeedsLogin
//! LoggingIn ──zaman aşımı / hata───▶ LoginFailed(neden) ──StartLogin──▶ LoggingIn
//! LoggingIn ──hesap ücretsiz───────▶ NeedsPremium
//! Connected ──oturum reddedildi────▶ NeedsLogin   (tarayıcı KENDİLİĞİNDEN açılmaz)
//! Connected ──oturum koptu (ağ)────▶ Connected (arka planda geri çekilmeyle yeniden bağlanır)
//! ```
//!
//! Tarayıcıyı yalnız `StartLogin` (kullanıcı eylemi) açar.

use crate::auth::LoginError;

/// Giriş neden tamamlanamadı? Motor ARAYÜZE KOD yollar (metin değil): yazıya
/// çevirme arayüzde (`msg::login_failed_text`, sonra i18n).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFailReason {
    /// 3 dakikada tamamlanmadı.
    TimedOut,
    /// Yerel yönlendirme portu (8898) başka uygulamada açık.
    PortBusy,
    /// Jeton alındı ama Spotify'a ulaşılamadı.
    Network,
    /// Başka hata (ayrıntı yalnız günlükte).
    Other,
}

/// Çalma komutu neden düştü? (Kod; yazısı arayüzde: `msg::play_block_text`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayBlock {
    NeedsLogin,
    LoggingIn,
    LoginFailed,
    NeedsPremium,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// Bağlanılıyor (kayıtlı anahtarla).
    Connecting,
    Connected,
    /// Ağ yok / Spotify'a ulaşılamıyor: kendiliğinden yeniden denenir.
    Offline,
    /// Kayıtlı giriş yok ya da reddedildi: kullanıcının girmesi gerekir.
    NeedsLogin,
    /// Tarayıcıda giriş bekleniyor.
    LoggingIn,
    /// Giriş tamamlanamadı.
    LoginFailed(LoginFailReason),
    /// Hesap ücretsiz: Spotify Premium olmadan ses akıtılamaz.
    NeedsPremium,
}

impl LinkState {
    /// `StartLogin` bu durumda tarayıcıyı açabilir mi?
    pub fn can_start_login(&self) -> bool {
        matches!(self, Self::NeedsLogin | Self::LoginFailed(_))
    }

    /// Kendiliğinden (arka planda) yeniden bağlanmak serbest mi? Giriş ya da
    /// Premium gerektiren durumlarda denemek boşuna; Spotify'ı da yorar.
    pub fn may_reconnect(&self) -> bool {
        matches!(self, Self::Connecting | Self::Connected | Self::Offline)
    }

    /// Çalma komutları bu durumda reddedilir; nedeni (kod).
    /// (`Connecting` / `Offline`: komutlar bağlanınca çalışmak üzere bekler ya
    /// da arayüz önceden söyler; burada engel yok.)
    pub fn playback_block(&self) -> Option<PlayBlock> {
        match self {
            Self::NeedsLogin => Some(PlayBlock::NeedsLogin),
            Self::LoggingIn => Some(PlayBlock::LoggingIn),
            Self::LoginFailed(_) => Some(PlayBlock::LoginFailed),
            Self::NeedsPremium => Some(PlayBlock::NeedsPremium),
            Self::Connecting | Self::Connected | Self::Offline => None,
        }
    }

    /// Kütüphane hiç gelemeyecekse (önbellek yok) arayüzün göstereceği hata
    /// anahtarı (`ui::components::explain_error` bu önekleri tanır).
    pub fn library_error(&self) -> Option<&'static str> {
        match self {
            Self::NeedsLogin | Self::LoginFailed(_) | Self::LoggingIn => {
                Some("auth:login-required")
            }
            Self::NeedsPremium => Some("premium:free-account"),
            Self::Connecting | Self::Connected | Self::Offline => None,
        }
    }
}

/// Tarayıcı girişinin neden bittiği → durum geçişi.
pub fn after_login_error(error: &LoginError) -> LinkState {
    match error {
        // Vazgeçmek hata değil: yeniden giriş yapılabilir durumda kal.
        LoginError::Cancelled => LinkState::NeedsLogin,
        LoginError::NeedsPremium => LinkState::NeedsPremium,
        LoginError::TimedOut => LinkState::LoginFailed(LoginFailReason::TimedOut),
        LoginError::PortBusy => LinkState::LoginFailed(LoginFailReason::PortBusy),
        LoginError::Network => LinkState::LoginFailed(LoginFailReason::Network),
        LoginError::Failed(_) => LinkState::LoginFailed(LoginFailReason::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANY: LoginFailReason = LoginFailReason::Other;

    #[test]
    fn only_needs_login_and_failed_can_open_the_browser() {
        assert!(LinkState::NeedsLogin.can_start_login());
        assert!(LinkState::LoginFailed(ANY).can_start_login());
        for s in [
            LinkState::Connecting,
            LinkState::Connected,
            LinkState::Offline,
            LinkState::LoggingIn,
            LinkState::NeedsPremium,
        ] {
            assert!(!s.can_start_login(), "{s:?}");
        }
    }

    #[test]
    fn no_background_reconnect_when_the_user_must_act() {
        for s in [
            LinkState::NeedsLogin,
            LinkState::LoggingIn,
            LinkState::LoginFailed(ANY),
            LinkState::NeedsPremium,
        ] {
            assert!(!s.may_reconnect(), "{s:?}");
        }
        for s in [
            LinkState::Connecting,
            LinkState::Connected,
            LinkState::Offline,
        ] {
            assert!(s.may_reconnect(), "{s:?}");
        }
    }

    #[test]
    fn playback_is_blocked_with_a_code_except_when_waiting_for_the_network() {
        assert_eq!(
            LinkState::NeedsPremium.playback_block(),
            Some(PlayBlock::NeedsPremium)
        );
        assert_eq!(
            LinkState::NeedsLogin.playback_block(),
            Some(PlayBlock::NeedsLogin)
        );
        assert_eq!(
            LinkState::LoggingIn.playback_block(),
            Some(PlayBlock::LoggingIn)
        );
        assert_eq!(
            LinkState::LoginFailed(ANY).playback_block(),
            Some(PlayBlock::LoginFailed)
        );
        assert_eq!(LinkState::Connected.playback_block(), None);
        assert_eq!(LinkState::Connecting.playback_block(), None);
        assert_eq!(LinkState::Offline.playback_block(), None);
    }

    #[test]
    fn login_errors_map_to_states() {
        assert_eq!(
            after_login_error(&LoginError::Cancelled),
            LinkState::NeedsLogin
        );
        assert_eq!(
            after_login_error(&LoginError::NeedsPremium),
            LinkState::NeedsPremium
        );
        assert_eq!(
            after_login_error(&LoginError::TimedOut),
            LinkState::LoginFailed(LoginFailReason::TimedOut)
        );
        assert_eq!(
            after_login_error(&LoginError::PortBusy),
            LinkState::LoginFailed(LoginFailReason::PortBusy)
        );
        assert_eq!(
            after_login_error(&LoginError::Network),
            LinkState::LoginFailed(LoginFailReason::Network)
        );
        // Teknik ayrıntı arayüze sızmaz.
        let s = after_login_error(&LoginError::Failed("Failed to exchange code (401)".into()));
        assert_eq!(s, LinkState::LoginFailed(LoginFailReason::Other));
    }

    #[test]
    fn library_error_keys_are_recognised_by_the_ui() {
        assert!(
            LinkState::NeedsLogin
                .library_error()
                .unwrap()
                .starts_with("auth:")
        );
        assert!(
            LinkState::NeedsPremium
                .library_error()
                .unwrap()
                .starts_with("premium:")
        );
        assert_eq!(LinkState::Offline.library_error(), None);
    }
}
