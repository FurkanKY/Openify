//! Çalma testlerinin sahte çaları: çağrıları sırayla kaydeder, anahtar
//! hatası sayacı elle artırılır. Gerçek ses çıkışı gerekmez (`OutputCtl`
//! aygıtsız da çalışır).

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

use librespot::{
    core::{SpotifyUri, session::Session},
    playback::mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
};

use super::{Deck, PlayerPort};
use crate::engine::{OutputCtl, levels::AudioLevels};

/// `FakePort`un kaydettiği çağrı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::engine) enum Call {
    Load {
        uri: String,
        play: bool,
        position_ms: u32,
    },
    Preload(String),
    Play,
    Pause,
    Stop,
    Seek(u32),
    Mark(u64),
    Rebind,
}

/// Testin elinde kalan tutamak: çağrılar ve anahtar hatası sayacı.
#[derive(Clone, Default)]
pub(in crate::engine) struct Spy {
    calls: Arc<Mutex<Vec<Call>>>,
    key_failures: Arc<AtomicU32>,
}

impl Spy {
    /// Kaydı alıp boşaltır.
    pub fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }

    /// Çalar bir anahtar hatası bildirmiş gibi sayacı artırır.
    pub fn fail_key(&self) {
        self.key_failures.fetch_add(1, Ordering::SeqCst);
    }
}

pub(in crate::engine) struct FakePort(pub Spy);

impl FakePort {
    pub fn new() -> (Self, Spy) {
        let spy = Spy::default();
        (Self(spy.clone()), spy)
    }

    fn record(&self, call: Call) {
        self.0.calls.lock().unwrap().push(call);
    }
}

impl PlayerPort for FakePort {
    fn load(&self, uri: SpotifyUri, play: bool, position_ms: u32) {
        self.record(Call::Load {
            uri: uri.to_uri().unwrap_or_default(),
            play,
            position_ms,
        });
    }

    fn preload(&self, uri: SpotifyUri) {
        self.record(Call::Preload(uri.to_uri().unwrap_or_default()));
    }

    fn play(&self) {
        self.record(Call::Play);
    }

    fn pause(&self) {
        self.record(Call::Pause);
    }

    fn stop(&self) {
        self.record(Call::Stop);
    }

    fn seek(&self, position_ms: u32) {
        self.record(Call::Seek(position_ms));
    }

    fn mark(&self, mark: u64) {
        self.record(Call::Mark(mark));
    }

    fn rebind(&self, _session: &Session) {
        self.record(Call::Rebind);
    }

    fn key_failures(&self) -> u32 {
        self.0.key_failures.load(Ordering::SeqCst)
    }
}

/// Sahte çaları ve aygıtsız (sanal saatli) çıkışı olan bir deste. Dönenler:
/// deste, çağrı casusu, çıkış (tutma / konum sınamaları için).
pub(in crate::engine) fn fake_deck() -> (Deck, Spy, Arc<OutputCtl>) {
    fake_deck_for(0)
}

/// `fake_deck`; çıkışın halkası `crossfade_ms` geçişine göre boyutlu.
pub(in crate::engine) fn fake_deck_for(crossfade_ms: u32) -> (Deck, Spy, Arc<OutputCtl>) {
    let clock = Arc::new(std::sync::atomic::AtomicU64::new(1_000_000_000));
    let out = OutputCtl::with_clock_for(AudioLevels::new(), clock, crossfade_ms);
    let (port, spy) = FakePort::new();
    let mixer = SoftMixer::open(MixerConfig::default()).expect("software mixer must open");
    let volume = mixer.get_soft_volume();
    (
        Deck::new(Box::new(port), out.clone(), mixer, volume),
        spy,
        out,
    )
}
