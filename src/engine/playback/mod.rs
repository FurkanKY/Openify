//! Çalma motoru: çalar dikişi, deste, çalma durumu ve sürücü (bkz.
//! `PLAN-MOTOR-CONNECT.md` §3).
//!
//! Katmanlar (alttan üste):
//! - `port.rs`: motorun çalardan istedikleri (`PlayerPort`) ve çaldan gelen
//!   olaylar (`PlayerSignal`). Gerçek uç `LibrespotPort`, sınama ucu
//!   `testkit::FakePort`.
//! - `deck.rs`: çalar komutları ile kendi ses çıkışımızın eşgüdümü (susturma,
//!   kuyruk atma, işaret; sıra değişmez).
//! - `Playback` (burası): çalma durumu ve kararları: yükleme, sırada ilerleme,
//!   önyükleme, konum, kurtarma (anahtar hatası, takılma, yeniden bağlanma),
//!   boşluksuz geçişte tutma (`handover.rs`), şarkılar arası geçişin (crossfade)
//!   politikası (`crossfade.rs`); komutlar `driver.rs`'teki sürücüden
//!   (`TransportCmd`) geçer.
//!
//! `Playback` motorun başka hiçbir şeyine dokunmaz. Sırayı ve şarkı bilgilerini
//! çağrı başına `Cx` ile ödünç alır, yan etkileri (arayüz olayı, kayıt, kapak,
//! bildirim, yeniden bağlanma) `Cx::fx`e SIRAYLA yazar; `Engine` her çağrıdan
//! sonra listeyi baştan sona boşaltır (`bridge.rs`), yani olayların sırası
//! doğrudan çağırmakla aynıdır.

mod bridge;
mod connect;
mod crossfade;
mod deck;
mod driver;
mod handover;
mod load;
mod persist;
mod port;
mod recover;
mod signal;
#[cfg(test)]
pub(super) mod testkit;
#[cfg(test)]
mod connect_tests;
#[cfg(test)]
mod tests;

use std::{collections::HashMap, time::{Duration, Instant}};

pub(super) use deck::{Deck, volume_to_u16};
pub(super) use connect::Link;
pub(super) use driver::{Driver, Local, LocalSnapshot, RemoteDrive, SpircDrive, TransportCmd};
pub(super) use handover::Gate;
pub(super) use port::{LibrespotPort, PlayerPort, PlayerSignal};

use super::{Event, NoticeKind, TrackInfo, TrackRow, output::Analysis, queue::Queue, resume};

/// Üst üste bu kadar şarkı çalınamazsa çalma durdurulur.
pub(super) const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// Adresi çözülemeyen şarkılar atlanırken en çok kaç tanesi denenir.
pub(super) const MAX_UNPARSEABLE_SKIPS: usize = 8;

/// Sırada ilerleme biçimi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine) enum Advance {
    /// Kullanıcının Sonraki'si (ve çalınamayan şarkıyı atlama): tekrar "tek"
    /// olsa da ilerler.
    Next,
    Previous,
    /// Şarkı kendiliğinden bitti: tekrar "tek"se aynı şarkı.
    Auto,
}

/// Motorun bağlantı durumundan `Playback`in kurtarma kararları için bilmesi
/// gerekenler (bağlantıyı motor yönetir).
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::engine) struct LinkFacts {
    /// Yeniden bağlanma şu an sürüyor.
    pub reconnecting: bool,
    /// Son bağlanma denemesinden bu yana geçen süre (`None`: hiç denenmedi).
    pub since_reconnect: Option<Duration>,
    /// Art arda başarısız yeniden bağlanma sayısı.
    pub reconnect_failures: u32,
}

/// `Playback`in motordan ödünç aldıkları, tek çağrı için.
pub(in crate::engine) struct Cx<'a> {
    pub queue: &'a mut Queue,
    /// Sıradaki şarkıların bilgileri (kaldığı yer kaydı için).
    pub meta: &'a HashMap<String, TrackRow>,
    pub link: LinkFacts,
    pub fx: &'a mut Vec<Effect>,
}

impl Cx<'_> {
    pub fn emit(&mut self, event: Event) {
        self.fx.push(Effect::Emit(event));
    }

    pub fn notice(&mut self, kind: NoticeKind, message: &'static str) {
        self.fx.push(Effect::Notice { kind, message });
    }

    /// Sırayı ve konumu kaydet (boşluksuz geçişte tutma sürerken atlanır).
    pub fn persist(&mut self) {
        self.fx.push(Effect::Persist(None));
    }

    /// Sıra değişti: arayüze bildir, önyüklemeyi yenile, kaydet.
    pub fn queue_changed(&mut self) {
        self.fx.push(Effect::QueueChanged);
    }
}

/// `Playback`in kendi başına yapamadığı, motorun yapacağı işler. Sırayla,
/// yazıldıkları sırayla yürütülür.
#[derive(Debug)]
pub(in crate::engine) enum Effect {
    /// Arayüze olay (boşluksuz geçişte kapıdan geçer).
    Emit(Event),
    /// Kullanıcıya kısa bilgi.
    Notice { kind: NoticeKind, message: &'static str },
    /// Kaldığı yer kaydı; `Some(ms)`: konum olarak bu yazılır (ör. biten
    /// sırada 0), `None`: tahmin edilen konum, boşluksuz geçişte tutma
    /// sürerken yazılmaz.
    Persist(Option<u32>),
    /// Çalan şarkının kapağını indir / diskten oku.
    FetchCover { uri: String, url: String },
    /// Sıra değişti (`Cx::queue_changed`).
    QueueChanged,
    /// Yeniden bağlan (`retry_current`: bağlanınca çalan şarkıyı yeniden yükle).
    /// Başladıysa (ya da sürüyorsa) `started` bildirimi gösterilir; başlamadıysa
    /// ve `skip_if_refused` ise çalınamayan şarkı atlanır.
    Reconnect { retry_current: bool, skip_if_refused: bool, started: Option<&'static str> },
    /// AutoMix: çalınan şarkının çözümlemesini önbelleğe yaz (`engine/automix.rs`).
    SaveAnalysis { uri: String, analysis: Analysis },
    /// Spotify Connect durumu değişti (`PlaybackEvent::Connect`).
    Connect(crate::engine::ConnectStatus),
}

/// Çalma durumu: çalar + çıkış (deste), ses düzeyi, çalan şarkı, konum ve
/// kurtarma sayaçları.
pub(in crate::engine) struct Playback {
    /// Çalmayı yürüten sürücü (bkz. `driver.rs`).
    pub driver: Driver,
    /// Çalar + kendi ses çıkışımız; Spotify'a bağlanınca kurulur.
    pub deck: Option<Deck>,
    /// Arayüze giden çalma olaylarının kapısı (bkz. `handover.rs`).
    pub gate: Gate,
    /// 0..1; bağlanmadan önce de saklanır (çalar kurulunca uygulanır).
    pub volume: f32,
    /// Son `TrackChanged`in bilgisi (kaldığı yer kaydı için).
    pub track_info: Option<TrackInfo>,
    /// Açılışta geri yüklenen şarkı çalara hiç yüklenmedi: Çal bu konumdan
    /// yükler (Spotify'a açılışta anahtar isteği gitmesin diye).
    pub restored: Option<u32>,
    /// Çalar şarkıyı bu konumdan yükledi, `Playing` henüz gelmedi.
    pub load_position: u32,
    /// Son bilinen konum, o anın zamanı ve çalıyor muydu: yeniden
    /// bağlanınca şarkı kaldığı yerden sürsün.
    pub last_position: Option<(u32, Instant, bool)>,
    /// Çalar sıradakini önyüklemeye çağırdı mı (`PreloadDue`) ve hangi şarkı
    /// önyüklendi? Sıra değişirse önyükleme yenilenir.
    pub preload_armed: bool,
    pub preloaded: Option<String>,
    /// Üst üste çalınamayan şarkı sayısı (bir şarkı çalınca sıfırlanır).
    pub failures: u32,
    /// Yeniden bağlanınca o anki şarkı yeniden yüklensin mi?
    pub retry_after_reconnect: bool,
    /// Bağlanıp yeniden yüklemesi yapılmış şarkı (şarkı başına en çok 1 kez);
    /// şarkı çalınca ya da kullanıcı yeniden yükleyince silinir.
    pub retried_uri: Option<String>,
    /// Çalar şarkıyı yüklemeye ne zaman başladı (`Playing` / `Paused` /
    /// `Unavailable` gelene dek) ve takılma için bağlanma zaten denendi mi?
    pub load_started: Option<Instant>,
    pub stall_fired: bool,
    /// Bağlanıp yeniden yükledik, çaların yeni `Loading` olayı henüz gelmedi:
    /// aradaki `Unavailable` eski yüklemeden kalmadır, yok sayılır.
    pub ignore_stale_failure: bool,
    /// Çaların bildirdiği anahtar hatası sayısının son görülen değeri.
    pub seen_key_failures: u32,
    /// Halkanın ve şarkı sonu payının dayandığı geçiş süresi, ms (0 kapalı; bkz.
    /// `crossfade.rs`): sabit kipte ayardaki süre, AutoMix'te en uzun bindirme.
    pub crossfade_ms: u32,
    /// Ayardaki sabit geçiş süresi (ms) ve AutoMix açık mı (`crossfade.rs`).
    pub fixed_ms: u32,
    pub automix: bool,
    /// AutoMix: bellekteki şarkı çözümlemeleri (açılışta önbellekten, çalındıkça
    /// çıkıştan; bkz. `engine/automix.rs`). AutoMix kapalıyken boş.
    pub beats: HashMap<String, Analysis>,
    /// Spotify Connect tarafı: Spirc tutamağı, küme görünümü, son bildirilen durum
    /// (bkz. `connect.rs`). Connect kapalıyken (varsayılan) boş kalır.
    pub link: Link,
}

impl Playback {
    pub fn new(gate: Gate, seen_key_failures: u32) -> Self {
        Self {
            driver: Driver::Local(Local),
            deck: None,
            gate,
            volume: 1.0,
            track_info: None,
            restored: None,
            load_position: 0,
            last_position: None,
            preload_armed: false,
            preloaded: None,
            failures: 0,
            retry_after_reconnect: false,
            retried_uri: None,
            load_started: None,
            stall_fired: false,
            ignore_stale_failure: false,
            seen_key_failures,
            crossfade_ms: 0,
            fixed_ms: 0,
            automix: false,
            beats: HashMap::new(),
            link: Link::default(),
        }
    }

    /// Ses düzeyi (0..1); çalar varsa hemen uygulanır.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(deck) = &self.deck {
            deck.set_volume(self.volume);
        }
    }

    pub fn play(&self) {
        if let Some(deck) = &self.deck {
            deck.play();
        }
    }

    pub fn pause(&self) {
        if let Some(deck) = &self.deck {
            deck.pause();
        }
    }

    pub fn seek(&self, position_ms: u32) {
        if let Some(deck) = &self.deck {
            deck.seek(position_ms);
        }
    }

    /// Yeniden bağlanınca yeni oturumu çalara ver.
    pub fn rebind(&self, session: &librespot::core::session::Session) {
        if let Some(deck) = &self.deck {
            deck.port.rebind(session);
        }
    }

    /// Çaların bildirdiği anahtar hatası sayısı (çalar yoksa `None`).
    pub fn key_failures(&self) -> Option<u32> {
        self.deck.as_ref().map(|d| d.port.key_failures())
    }

    /// Çalarda şu an çalan (ya da duraklatılmış) konumun tahmini.
    pub fn position_now(&self) -> u32 {
        let position = match (self.restored, self.last_position) {
            (Some(position), _) => position,
            (None, Some((ms, at, true))) => ms.saturating_add(at.elapsed().as_millis() as u32),
            (None, Some((ms, _, false))) => ms,
            (None, None) => self.load_position,
        };
        match &self.track_info {
            Some(t) if t.duration_ms > 0 => position.min(t.duration_ms),
            _ => position,
        }
    }
}

impl Playback {
    /// Kaldığı yer kaydının konumu ve şarkısı (`position`: verilirse o konum).
    /// Şarkı: çaların son bildirdiği, yoksa sıradaki satır, yoksa önbellekteki.
    pub fn resume_state(&self, cx: &Cx<'_>, position: Option<u32>) -> (u32, Option<resume::SnapTrack>) {
        let position = position.unwrap_or_else(|| self.position_now());
        let current = cx.queue.current_uri();
        let track = self
            .track_info
            .as_ref()
            .filter(|t| Some(t.uri.as_str()) == current)
            .map(|t| resume::SnapTrack {
                uri: t.uri.clone(),
                name: t.name.clone(),
                artists: t.artists.clone(),
                duration_ms: t.duration_ms,
                cover_url: t.cover_url.clone(),
            })
            .or_else(|| {
                let row = cx.queue.current_row().or_else(|| cx.meta.get(current?).cloned())?;
                Some(resume::SnapTrack {
                    uri: row.uri,
                    name: row.name,
                    artists: row.artists,
                    duration_ms: row.duration_ms,
                    // Küçük kapak adresi; büyüğü çalınca öğrenilir.
                    cover_url: (!row.image.is_empty()).then_some(row.image),
                })
            });
        (position, track)
    }
}
