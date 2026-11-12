//! Geri / ileri gezinme (Adım 12): Alt+← / Alt+→, farenin yan tuşları ve
//! başlık çubuğundaki düğmeler.
//!
//! Her kayıt bir yer (liste, sanatçı, albüm, ana sayfa, arama) ve oradan
//! ayrılırken kaydırma konumu: geri dönünce sayfa bırakıldığı yerde açılır.

use crate::engine::CollectionId;

/// En çok bu kadar geri adım tutulur.
const MAX_BACK: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// Kütüphanedeki (ya da aramadan açılan) liste: raf + liste görünümü.
    Collection(CollectionId),
    /// Sanatçı sayfası.
    Artist(String),
    /// Albüm sayfası (kütüphanede olmayan albüm; kütüphanedeki albüm
    /// `Collection` ile açılır).
    Album(String),
    Home,
    /// Keşfet: yeni çıkanlar, trend listeler, türler (Adım 12, 7 Kasım).
    Explore,
    /// Bir türün sayfası (`spotify:page:<id>`).
    Category(String),
    /// Kütüphane: kapak rafı + kütüphanedeki her şey (süzgeç, sıralama,
    /// ızgara / liste); raf ve liste yerine bu görünüm.
    Library,
    /// Tam arama sonuçları.
    Search(String),
    /// Ayarlar (Adım 15): tek sayfa, kısa bölümler alt alta.
    Settings,
    /// Son çalınan liste ve albümlerin tam sayfası (ana sayfadan ve paletten).
    Recent,
    /// Kütüphane araçları: yinelenenler, birleştir / böl, sıralama, yedek,
    /// istatistik (kütüphane araç çubuğundan ve paletten).
    Tools,
}

/// Başlık çubuğundaki sekmeler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Explore,
    Library,
}

impl Route {
    /// Raf içeren görünüm mü (kütüphane sekmesi: açık liste ya da kütüphane
    /// ızgarası; değilse sayfa)?
    pub fn is_collection(&self) -> bool {
        matches!(self, Route::Collection(_) | Route::Library)
    }

    /// Bu yer hangi sekmenin içinde (sanatçı, albüm, arama: hiçbiri)?
    pub fn tab(&self) -> Option<Tab> {
        match self {
            // Son çalınanlar ana sayfanın "tümünü gör" sayfası: Ana sayfa sekmesi seçili kalır.
            Route::Home | Route::Recent => Some(Tab::Home),
            Route::Explore | Route::Category(_) => Some(Tab::Explore),
            Route::Collection(_) | Route::Library | Route::Tools => Some(Tab::Library),
            Route::Artist(_) | Route::Album(_) | Route::Search(_) | Route::Settings => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub route: Route,
    /// Ayrılırken kaydırma konumu (px; listede raf kapanması + liste).
    pub scroll: f32,
}

pub struct History {
    back: Vec<Entry>,
    current: Route,
    forward: Vec<Entry>,
}

impl History {
    pub fn new(start: Route) -> Self {
        Self {
            back: Vec::new(),
            current: start,
            forward: Vec::new(),
        }
    }

    pub fn current(&self) -> &Route {
        &self.current
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Yeni bir yere git; şimdiki yer (kaydırmasıyla) geriye eklenir, ileri
    /// silinir. Zaten oradaysak `false`.
    pub fn push(&mut self, route: Route, scroll: f32) -> bool {
        if route == self.current {
            return false;
        }
        let from = std::mem::replace(&mut self.current, route);
        self.back.push(Entry {
            route: from,
            scroll,
        });
        if self.back.len() > MAX_BACK {
            self.back.remove(0);
        }
        self.forward.clear();
        true
    }

    /// Geri: gidilecek kayıt (kaydırma konumuyla).
    pub fn back(&mut self, scroll: f32) -> Option<Entry> {
        let to = self.back.pop()?;
        let from = std::mem::replace(&mut self.current, to.route.clone());
        self.forward.push(Entry {
            route: from,
            scroll,
        });
        Some(to)
    }

    pub fn forward(&mut self, scroll: f32) -> Option<Entry> {
        let to = self.forward.pop()?;
        let from = std::mem::replace(&mut self.current, to.route.clone());
        self.back.push(Entry {
            route: from,
            scroll,
        });
        Some(to)
    }

    /// Bir yer başka bir adla değişti (geçici liste adresi gerçeğine) ya da
    /// artık yok (silinen liste: `to` yerine konur).
    pub fn rename(&mut self, from: &Route, to: Route) {
        for r in self
            .back
            .iter_mut()
            .chain(self.forward.iter_mut())
            .map(|e| &mut e.route)
            .chain(std::iter::once(&mut self.current))
        {
            if r == from {
                *r = to.clone();
            }
        }
        // Art arda aynı yer kalmasın.
        self.back.dedup_by(|a, b| a.route == b.route);
        if self.back.last().is_some_and(|e| e.route == self.current) {
            self.back.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(n: u8) -> Route {
        Route::Artist(format!("spotify:artist:{n}"))
    }

    #[test]
    fn back_and_forward_keep_scroll() {
        let mut h = History::new(Route::Collection(CollectionId::Liked));
        assert!(h.push(artist(1), 300.));
        assert!(h.push(artist(2), 120.));
        assert!(!h.push(artist(2), 0.), "aynı yere ikinci kez gidilmez");
        let to = h.back(40.).unwrap();
        assert_eq!(
            to,
            Entry {
                route: artist(1),
                scroll: 120.
            }
        );
        let to = h.back(0.).unwrap();
        assert_eq!(to.scroll, 300.);
        assert!(!h.can_back());
        let to = h.forward(10.).unwrap();
        assert_eq!(to.route, artist(1));
        let to = h.forward(0.).unwrap();
        assert_eq!(
            to,
            Entry {
                route: artist(2),
                scroll: 40.
            }
        );
        // Yeni yere gitmek ileriyi siler.
        h.back(0.);
        h.push(Route::Home, 0.);
        assert!(!h.can_forward());
    }

    #[test]
    fn renamed_routes_follow() {
        let temp = Route::Collection(CollectionId::Playlist("local:playlist:1".into()));
        let real = Route::Collection(CollectionId::Playlist("spotify:playlist:x".into()));
        let mut h = History::new(Route::Home);
        h.push(temp.clone(), 0.);
        h.push(artist(1), 0.);
        h.rename(&temp, real.clone());
        assert_eq!(h.back(0.).unwrap().route, real);
    }
}
