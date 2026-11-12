//! Kapak resimleri: diskteki önbellekten gösterilir, yoksa motordan istenir.
//!
//! Çizim sırasında bir resme ihtiyaç olunca `get` çağrılır. Diskte varsa
//! yolu döner; yoksa istek listesine eklenir ve bir sonraki fırsatta
//! motora `FetchImages` gider. Motor indirince `ImageReady` gelir, ekran
//! yeniden çizilir. Aynı resim bir kez istenir; ama indirme başarısız olursa
//! (ağ koptu, bağlantı henüz yok) motor haber vermez: `RETRY_AFTER` sonra
//! resim yine çizilirken (`get`) bir kez daha istenir.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use super::theme::{animations_enabled, cover_fade, ease_out};
use crate::engine::cover_path;

/// İstenen resim bu kadar sürede gelmediyse yeniden istenir.
const RETRY_AFTER: Duration = Duration::from_secs(20);

pub struct ImageStore {
    ready: RefCell<HashSet<String>>,
    /// İstenen ama henüz gelmeyen resimler ve en son ne zaman istendikleri.
    requested: RefCell<HashMap<String, Instant>>,
    pending: RefCell<Vec<String>>,
    retry_after: Duration,
    /// Bu oturumda indirilip yeni gelen resimlerin belirme anı (açılma geçişi
    /// için). Diskte hazır bulunanlar buraya girmez; geçiş bitince kayıt silinir.
    fades: RefCell<HashMap<String, Instant>>,
    /// Bu karede çizilen bir kapak hâlâ açılıyor: çizen görünüm bir kare daha ister.
    fading: Cell<bool>,
    /// Kapaktan çıkarılan renkler (bir kez hesaplanır; ana sayfa kartları).
    tints: RefCell<HashMap<String, Option<u32>>>,
}

impl Default for ImageStore {
    fn default() -> Self {
        Self::with_retry(RETRY_AFTER)
    }
}

impl ImageStore {
    fn with_retry(retry_after: Duration) -> Self {
        Self {
            ready: RefCell::default(),
            requested: RefCell::default(),
            pending: RefCell::default(),
            retry_after,
            fades: RefCell::default(),
            fading: Cell::new(false),
            tints: RefCell::default(),
        }
    }

    /// Kapağın baskın rengi (kapak ışığının koyu tonu, `engine::palette`):
    /// ana sayfa kartlarının zemin tonu için. Resim diskte değilse `None`
    /// (sonra yine sorulur); gri tonlu kapakta da `None` (kalıcı). Kapak bir
    /// kez çözülür (24 × 24'e küçültülüp), sonuç saklanır.
    pub fn tint(&self, url: &str) -> Option<u32> {
        if let Some(known) = self.tints.borrow().get(url) {
            return *known;
        }
        let path = self.get(url)?;
        let color = std::fs::read(path)
            .ok()
            .and_then(|bytes| crate::engine::palette::accent_from_image(&bytes))
            .map(|a| a.glow);
        self.tints.borrow_mut().insert(url.to_string(), color);
        color
    }

    /// Resim diskteyse yolu; değilse `None` (ve indirme isteği sıraya girer).
    pub fn get(&self, url: &str) -> Option<PathBuf> {
        if url.is_empty() {
            return None;
        }
        let path = cover_path(url)?;
        if self.ready.borrow().contains(url) {
            return Some(path);
        }
        let now = Instant::now();
        let asked_at = self.requested.borrow().get(url).copied();
        if let Some(at) = asked_at {
            if now.duration_since(at) < self.retry_after {
                return None;
            }
            // Uzun süredir gelmedi: indirme büyük olasılıkla başarısız oldu.
            // (Arada gelmişse aşağıdaki disk denetimi yakalar.)
        }
        if path.exists() {
            self.requested.borrow_mut().remove(url);
            self.ready.borrow_mut().insert(url.to_string());
            return Some(path);
        }
        self.requested.borrow_mut().insert(url.to_string(), now);
        self.pending.borrow_mut().push(url.to_string());
        None
    }

    /// Hazır / istenmiş kayıtlarını unut (önbellek temizlenince): disk
    /// yeniden denetlenir, eksik kapaklar yeniden istenir.
    pub fn forget_all(&self) {
        self.ready.borrow_mut().clear();
        self.requested.borrow_mut().clear();
        self.tints.borrow_mut().clear();
    }

    pub fn mark_ready(&self, url: &str) {
        self.requested.borrow_mut().remove(url);
        self.ready.borrow_mut().insert(url.to_string());
        // İndirme şimdi bitti: kapak ilk görünüşünde yumuşakça açılsın.
        if animations_enabled() {
            self.fades
                .borrow_mut()
                .insert(url.to_string(), Instant::now());
        }
    }

    /// Yeni inen kapağın açılma ilerlemesi (yavaşlayan 0..1); geçiş yoksa ya da
    /// bittiyse `None`. Sürerken `take_fading` için işaret bırakır.
    pub fn fade(&self, url: &str) -> Option<f32> {
        let start = self.fades.borrow().get(url).copied()?;
        let t = start.elapsed().as_secs_f32() / cover_fade().as_secs_f32();
        if t >= 1.0 {
            self.fades.borrow_mut().remove(url);
            return None;
        }
        self.fading.set(true);
        Some(ease_out(t))
    }

    /// Bu karede açılan kapak vardı mı? (Okuyunca işaret sıfırlanır.)
    pub fn take_fading(&self) -> bool {
        self.fading.replace(false)
    }

    /// Motora gönderilecek yeni istekler.
    pub fn take_pending(&self) -> Vec<String> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.borrow().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_each_missing_image_once() {
        let store = ImageStore::default();
        let url = "https://example.invalid/openify-test-image-does-not-exist";
        assert!(store.get(url).is_none());
        assert!(store.get(url).is_none());
        assert_eq!(store.take_pending(), [url]);
        assert!(store.take_pending().is_empty());

        store.mark_ready(url);
        assert!(store.get(url).is_some());
        assert!(store.get("").is_none());
    }

    #[test]
    fn failed_download_is_requested_again_later() {
        // Bekleme süresi 0: hiç gelmeyen (indirilemeyen) resim her çizimde
        // yeniden istenir; varsayılan sürede ise bir kez istenip beklenir.
        let store = ImageStore::with_retry(Duration::ZERO);
        let url = "https://example.invalid/openify-test-image-failed-download";
        assert!(store.get(url).is_none());
        assert_eq!(store.take_pending(), [url]);
        assert!(store.get(url).is_none());
        assert_eq!(store.take_pending(), [url]);

        // İndi: artık istenmez.
        store.mark_ready(url);
        assert!(store.get(url).is_some());
        assert!(store.take_pending().is_empty());
    }
}
