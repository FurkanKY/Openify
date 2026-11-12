//! Motorun gönderdiği KODLARIN kullanıcı yazısına çevrilmesi.
//!
//! Motor yazı yollamaz, yalnız kod (`PlayBlock`, `LoginFailReason`); yazılar
//! `src/i18n/table.rs`'te (İngilizce + Türkçe) ve o anki dilde çözülür. Giriş /
//! Premium / motor durumlarının düğme, hap ve durum satırı yazıları da aynı
//! tabloda (`t!(BtnSignIn)`, `t!(PillNeedsLoginTitle)` …).

use crate::engine::{LoginFailReason, PlayBlock};
use crate::t;

/// Çalma neden engellendi (bildirim yazısı).
pub fn play_block_text(block: PlayBlock) -> &'static str {
    match block {
        PlayBlock::NeedsLogin => t!(PlayNeedsLogin),
        PlayBlock::LoggingIn => t!(PlayLoggingIn),
        PlayBlock::LoginFailed => t!(PlayLoginFailed),
        PlayBlock::NeedsPremium => t!(PlayNeedsPremium),
    }
}

/// Giriş neden tamamlanamadı (başlangıç ekranının durum satırı).
pub fn login_failed_text(reason: LoginFailReason) -> &'static str {
    match reason {
        LoginFailReason::TimedOut => t!(LoginTimedOut),
        LoginFailReason::PortBusy => t!(LoginPortBusy),
        LoginFailReason::Network => t!(LoginNetwork),
        LoginFailReason::Other => t!(LoginFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, with_lang};

    #[test]
    fn every_code_has_a_text_in_both_languages() {
        for lang in [Lang::Tr, Lang::En] {
            with_lang(lang, || {
                for b in [
                    PlayBlock::NeedsLogin,
                    PlayBlock::LoggingIn,
                    PlayBlock::LoginFailed,
                    PlayBlock::NeedsPremium,
                ] {
                    assert!(!play_block_text(b).is_empty());
                }
                for r in [
                    LoginFailReason::TimedOut,
                    LoginFailReason::PortBusy,
                    LoginFailReason::Network,
                    LoginFailReason::Other,
                ] {
                    assert!(!login_failed_text(r).is_empty());
                }
            });
        }
    }
}
