//! Çalma kurtarma: yükleme takılması, çalınamayan şarkı, yeniden bağlanınca
//! şarkıyı kaldığı yerden sürme. Kararlar `recovery.rs`'te (saf); burada
//! uygulanışı. Bağlantıyı (yeniden bağlanmayı) motor yönetir: istek
//! `Effect::Reconnect` ile gider.

use std::time::Instant;

use librespot::core::SpotifyUri;

use super::{Advance, Cx, Effect, MAX_CONSECUTIVE_FAILURES, Playback};
use crate::{
    engine::{Event, NoticeKind, recovery},
    t,
};

impl Playback {
    /// Yükleme takıldıysa (anahtar istekleri yanıtsız: oturum sessizce ölmüş)
    /// uyanılacak an. Yalnız bağlıyken, bağlanma sürmüyorken ve bu yükleme için
    /// henüz işlem yapılmadıysa. `current`: sıradaki çalan şarkı.
    pub fn stall_deadline(&self, current: Option<&str>, reconnecting: bool) -> Option<Instant> {
        if self.deck.is_none() || reconnecting || self.stall_fired || self.retried_current(current) {
            return None;
        }
        self.load_started.map(|t| t + recovery::LOAD_STALL)
    }

    /// Yükleme `LOAD_STALL` süredir sürüyor: oturumu yenile, aynı şarkıyı
    /// kaldığı yerden bir kez daha yükle (TCP kopması 5-10 sn sonra fark
    /// edilmeden; bkz. `recovery.rs`).
    pub fn on_load_stall(&mut self, cx: &mut Cx<'_>) {
        self.stall_fired = true;
        let Some(started) = self.load_started else { return };
        if !recovery::stall_should_reconnect(
            started.elapsed(),
            self.retried_current(cx.queue.current_uri()),
            cx.link.reconnecting,
            cx.link.since_reconnect,
            cx.link.reconnect_failures,
        ) {
            return;
        }
        log::warn!("Şarkı yüklemesi {} sn'dir sürüyor, oturum ölü sayıldı", started.elapsed().as_secs());
        cx.fx.push(Effect::Reconnect {
            retry_current: true,
            skip_if_refused: false,
            started: Some(t!(NoticeSlowReconnecting)),
        });
    }

    /// Şu anki şarkı için bağlanıp-yeniden-yükleme zaten yapıldı mı?
    pub fn retried_current(&self, current: Option<&str>) -> bool {
        match (&self.retried_uri, current) {
            (Some(retried), Some(current)) => retried == current,
            _ => false,
        }
    }

    /// Şarkı çalınamadı (`failed`: hangi şarkı). Karar `recovery.rs`'te:
    /// anahtar alınamadıysa oturum şüphelidir, hemen yeniden bağlanıp AYNI
    /// şarkı bir kez daha denenir; o da olmazsa (ya da geri çekilme sürüyorsa)
    /// sıradakine geçilir, üst üste çoksa durulur. Önyüklenen sıradaki şarkının
    /// hatası çalan şarkıyı atlatmaz.
    pub fn on_unavailable(&mut self, cx: &mut Cx<'_>, failed: Option<String>) {
        let current = match (&failed, cx.queue.current_uri()) {
            (Some(failed), Some(current)) => failed == current,
            _ => true,
        };
        // Çalar anahtar alamadı mı (bu hata ya da ondan önceki)? Sayaç artışı.
        let key_total = self.key_failures().unwrap_or(self.seen_key_failures);
        let key_failure = key_total != self.seen_key_failures;
        self.seen_key_failures = key_total;

        if current && self.ignore_stale_failure {
            // Yeniden yüklemeden önceki yüklemenin geç gelen hatası.
            log::debug!("Eski yüklemenin hatası yok sayıldı");
            return;
        }
        if current {
            self.load_started = None;
            self.failures += 1;
        }
        let action = recovery::on_failure(recovery::Failure {
            key_failure,
            current,
            retried: self.retried_current(cx.queue.current_uri()),
            reconnecting: cx.link.reconnecting,
            since_reconnect: cx.link.since_reconnect,
            reconnect_failures: cx.link.reconnect_failures,
            consecutive: self.failures,
            max_consecutive: MAX_CONSECUTIVE_FAILURES,
        });
        log::info!("Şarkı çalınamadı (anahtar hatası: {key_failure}, çalan: {current}) → {action:?}");
        match action {
            recovery::Recovery::Ignore => {}
            recovery::Recovery::ReconnectOnly => {
                cx.fx.push(Effect::Reconnect { retry_current: false, skip_if_refused: false, started: None });
            }
            recovery::Recovery::GiveUp => {
                self.failures = 0;
                self.retry_after_reconnect = false;
                cx.emit(Event::Unavailable);
                cx.notice(NoticeKind::Problem, t!(NoticeSeveralFailed));
            }
            recovery::Recovery::RetryAfterReconnect => {
                // "Bağlantı koptu" değil: sorun çoğunlukla Spotify'ın şarkı
                // anahtarını vermemesi (bkz. vendor/librespot-core/OPENIFY.md).
                cx.fx.push(Effect::Reconnect {
                    retry_current: true,
                    skip_if_refused: true,
                    started: Some(t!(NoticeRetryingSong)),
                });
            }
            recovery::Recovery::Skip => self.skip_failed(cx),
        }
    }

    /// Çalınamayan şarkıyı bırakıp sıradakine geç.
    pub fn skip_failed(&mut self, cx: &mut Cx<'_>) {
        self.retry_after_reconnect = false;
        // Kendiliğinden atlama: önceki şarkının kuyruktaki sonu çalsın.
        if self.advance(cx, Advance::Next, false) {
            cx.notice(NoticeKind::Info, t!(NoticePrevFailed));
        } else {
            cx.emit(Event::Unavailable);
        }
    }

    /// Yeniden bağlanıldı, çalar yeni oturumu aldı: çalınamayan / takılan
    /// şarkıyı kaldığı yerden (duraklatılmışsa duraklatılmış olarak) yeniden
    /// yükle; boşta ve hiçbir şey yüklü değilken yeniden yükleme yok (bkz.
    /// `recovery.rs`). Çalan şarkıya dokunulmadıysa sıradakini yeni oturumla
    /// yeniden önyükle.
    pub fn after_reconnect(&mut self, cx: &mut Cx<'_>) {
        let retry = std::mem::take(&mut self.retry_after_reconnect);
        let target = if retry {
            recovery::retry_target(
                self.last_position.map(|(ms, at, playing)| (ms, at.elapsed(), playing)),
                self.load_position,
                self.load_started.is_some(),
            )
        } else {
            None
        };
        let current = cx.queue.current_uri().map(str::to_owned);
        if let (Some((position, playing)), Some(current)) = (target, current) {
            if let (Some(deck), Ok(uri)) = (&self.deck, SpotifyUri::from_uri(&current)) {
                // Şarkı başına tek yeniden deneme: bu da olmazsa atlanır.
                self.retried_uri = Some(current);
                self.ignore_stale_failure = true;
                self.stall_fired = false;
                self.load_started = playing.then(Instant::now);
                deck.load(uri, playing, position, true);
                self.load_position = position;
                self.preload_armed = false;
                self.preloaded = None;
            }
        } else {
            // Çalan şarkıya dokunulmadı: yeni oturumla sıradakini yeniden önyükle
            // (eski oturumdaki önyükleme anahtarsız kalmış olabilir).
            self.preloaded = None;
            if self.preload_armed {
                self.preload_next(cx.queue);
            }
        }
    }
}
