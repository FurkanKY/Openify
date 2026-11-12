//! Şarkı yükleme, sırada ilerleme ve önyükleme.

use librespot::core::SpotifyUri;

use super::{Advance, Cx, MAX_UNPARSEABLE_SKIPS, Playback};
use crate::engine::queue::Queue;

impl Playback {
    /// Şarkıyı çaları yükler (`user`: bkz. `Deck::load`). Adres geçersizse
    /// `false`. Yeni şarkıyla önyükleme ve konum durumu sıfırlanır.
    pub fn load_uri(&mut self, uri: &str, position_ms: u32, user: bool) -> bool {
        let Ok(parsed) = SpotifyUri::from_uri(uri) else {
            return false;
        };
        let Some(deck) = &self.deck else { return false };
        deck.load(parsed, true, position_ms, user);
        if user {
            // Kullanıcı yeniden istedi: bu şarkının yeniden deneme hakkı tazelenir.
            self.retried_uri = None;
        }
        self.stall_fired = false;
        self.load_started = Some(std::time::Instant::now());
        self.preload_armed = false;
        self.preloaded = None;
        // Yeni şarkı: eski konum geçersiz (yeniden bağlanma ve kayıt, yeni
        // şarkının konumunu eskisinden sanmasın).
        self.last_position = None;
        self.load_position = position_ms;
        true
    }

    /// Yeni bağlamın ilk şarkısını çalar (`Engine::start_queue`'un çalar
    /// tarafı: sıra zaten kuruldu, `uri` çalan şarkı).
    pub fn begin(&mut self, cx: &mut Cx<'_>, uri: String, position_ms: u32) {
        self.restored = None;
        if self.load_uri(&uri, position_ms, true) {
            cx.queue_changed();
        } else {
            // Adres çözülemedi (bozuk kayıt): sıradakine geç.
            log::warn!("Çalınamayan adres atlandı: {uri}");
            if !self.advance(cx, Advance::Next, true) {
                cx.queue_changed();
            }
        }
    }

    /// Sırada ilerle ya da geri git. Gidilecek şarkı yoksa `false`. `user`:
    /// kullanıcı istedi (kuyruktaki eski ses atılır). Adresi çözülemeyen
    /// şarkılar (nadir) atlanır.
    pub fn advance(&mut self, cx: &mut Cx<'_>, how: Advance, user: bool) -> bool {
        if self.deck.is_none() {
            return false;
        }
        // Geçiş (crossfade) kararı için yüklemeden önce çalan şarkı.
        let prev = Self::current_track(cx);
        let mut moved = false;
        for _ in 0..MAX_UNPARSEABLE_SKIPS {
            let uri = match how {
                Advance::Next => cx.queue.next(),
                Advance::Previous => cx.queue.previous(),
                Advance::Auto => cx.queue.next_auto(),
            };
            let Some(uri) = uri else { break };
            moved = true;
            self.arm_transition(cx, &prev, &uri, how);
            if self.load_uri(&uri, 0, user) {
                self.restored = None;
                cx.queue_changed();
                return true;
            }
            log::warn!("Çalınamayan adres atlandı: {uri}");
        }
        if moved {
            cx.queue_changed();
        }
        false
    }

    /// Görünür listedeki `i`. şarkıya atla.
    pub fn jump_to(&mut self, cx: &mut Cx<'_>, i: usize) {
        let Some(uri) = cx.queue.jump_to(i) else { return };
        self.restored = None;
        if self.load_uri(&uri, 0, true) {
            cx.queue_changed();
        } else if !self.advance(cx, Advance::Next, true) {
            cx.queue_changed();
        }
    }

    /// Sıradaki şarkıyı çalara önyüklettir (boşluksuz geçiş için); aynısı
    /// zaten önyüklendiyse boş.
    pub fn preload_next(&mut self, queue: &Queue) {
        let Some(deck) = &self.deck else { return };
        let Some(uri) = queue.peek_next_auto() else { return };
        if self.preloaded.as_deref() == Some(uri.as_str()) {
            return;
        }
        if let Ok(parsed) = SpotifyUri::from_uri(&uri) {
            deck.preload(parsed);
            self.preloaded = Some(uri);
        }
    }

    /// Önyükleme zaten istenmişken sıra değiştiyse (sıraya ekle, karıştırma,
    /// tekrar…) doğru sıradakini yeniden önyükle.
    pub fn refresh_preload(&mut self, queue: &Queue) {
        if self.preload_armed {
            self.preload_next(queue);
        }
    }

    /// Geri yüklenen şarkıyı kaldığı konumdan çalmaya başlar (kullanıcı Çal'a
    /// bastı; şarkı anahtarı isteği ilk kez şimdi gider).
    pub fn play_restored(&mut self, cx: &mut Cx<'_>, position_ms: u32) {
        let Some(uri) = cx.queue.current_uri().map(str::to_owned) else {
            self.restored = None;
            return;
        };
        if self.load_uri(&uri, position_ms, true) {
            self.restored = None;
        } else if !self.advance(cx, Advance::Next, true) {
            self.restored = None;
        }
    }

    /// Geri yüklenen (çalara yüklenmemiş) şarkıda sarma: yalnız kayıtlı konum.
    pub fn seek_restored(&mut self, cx: &mut Cx<'_>, position_ms: u32) {
        let position_ms = match self.track_info.as_ref().map(|t| t.duration_ms) {
            Some(d) if d > 0 => position_ms.min(d),
            _ => position_ms,
        };
        self.restored = Some(position_ms);
        cx.emit(crate::engine::Event::Position { position_ms });
        cx.persist();
    }
}
