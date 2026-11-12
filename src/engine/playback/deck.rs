//! Deste: çaların komutları ile kendi ses çıkışımızın (`OutputCtl`) eşgüdümü.
//!
//! Kullanıcı komutları çıkışa ÖNCE dokunur (susturma, kuyruk atma), çalara
//! SONRA gider; arada çıkış kapısının işareti (`PlayerPort::mark`) çalar
//! iş parçacığının eski ses yazışını kesinleştirir (bkz. `output/mod.rs`).
//! Sıra değişmez; testleri `tests.rs`'te.

use std::sync::Arc;

use librespot::{
    core::SpotifyUri,
    playback::mixer::{Mixer, VolumeGetter, softmixer::SoftMixer},
};

use super::PlayerPort;
use crate::engine::OutputCtl;

pub(in crate::engine) struct Deck {
    pub port: Box<dyn PlayerPort>,
    /// Kendi ses çıkışımız: susturma, kuyruk atma, ses düzeyi.
    pub out: Arc<OutputCtl>,
    /// Ses düzeyi eğrisi (librespot'un logaritmik eğrisi) için; çarpanı
    /// çıkış uygular.
    mixer: SoftMixer,
    volume: Box<dyn VolumeGetter + Send>,
}

impl Deck {
    pub fn new(
        port: Box<dyn PlayerPort>,
        out: Arc<OutputCtl>,
        mixer: SoftMixer,
        volume: Box<dyn VolumeGetter + Send>,
    ) -> Self {
        Self { port, out, mixer, volume }
    }

    /// Kullanıcının duraklatması: ses beklemeden susar (çalarınki kuyruğun
    /// ardından gelir).
    pub fn pause(&self) {
        self.out.note("pause");
        let mark = self.out.duck();
        self.port.mark(mark);
        self.port.pause();
    }

    pub fn play(&self) {
        self.out.note("play");
        self.out.unduck();
        self.port.play();
    }

    /// Kullanıcının sarması: kuyruktaki eski konumun sesi atılır.
    pub fn seek(&self, position_ms: u32) {
        self.out.note("seek");
        let mark = self.out.flush();
        self.port.mark(mark);
        self.port.seek(position_ms);
    }

    /// Şarkı yükle. `user`: kullanıcı istedi (ya da yeniden deneme), kuyruktaki
    /// eski ses atılır. Şarkı kendiliğinden bitip sıradakine geçerken DEĞİL:
    /// önceki şarkının kuyruktaki sonu boşluksuz çalmalı.
    pub fn load(&self, uri: SpotifyUri, play: bool, position_ms: u32, user: bool) {
        self.out.note(if user { "load" } else { "auto_next" });
        if user {
            // Başka şarkı: şarkı sonu payı, o şarkı önyükleyince yeniden açılır.
            self.out.set_tail(false);
            let mark = self.out.flush();
            self.port.mark(mark);
            if play {
                self.out.unduck();
            }
        }
        self.port.load(uri, play, position_ms);
    }

    pub fn preload(&self, uri: SpotifyUri) {
        self.port.preload(uri);
    }

    pub fn stop(&self) {
        self.port.stop();
    }

    // ── Spirc (telefon) sürerken: çalara KOMUT yok, yalnız çıkış ──────────
    // Spirc çalara kendi komutlarını verir (bkz. `connect.rs`); deste yalnız
    // çıkışa dokunur (susturma, kuyruk atma) ve işaret gönderir.

    /// Çıkış kuyruğundaki eski sesi at (işaret çaldan geri dönene dek yeni
    /// paketler de atılır, bkz. `output/mod.rs`). `foreign_flush`: komut
    /// ÖNCE değil SONRA fark edildi (telefon aktardı / şarkı değiştirdi / geri
    /// aldı); kapı bir sonraki "komut sonucu" olayında (`Playing`,
    /// `TrackChanged`, `Stopped`…) açılır. Kullanıcının kendi komutundan
    /// ÖNCE de (`user_flush`) aynı çift kullanılır.
    pub fn foreign_flush(&self) {
        self.out.note("foreign_flush");
        let mark = self.out.flush();
        self.port.mark(mark);
    }

    /// Telefonun duraklatması (ya da kendi duraklatmamız Spirc'e verilmeden
    /// önce): ses beklemeden susar. `Paused` olayını beklemez.
    pub fn silence(&self) {
        self.out.note("silence");
        let mark = self.out.duck();
        self.port.mark(mark);
    }

    /// `silence`ın tersi.
    pub fn unsilence(&self) {
        self.out.note("unsilence");
        self.out.unduck();
    }

    /// Ses düzeyi çarpanını Spirc'in karıştırıcısıyla paylaşmak için
    /// (`OutMixer`; ortak durum, kopya yeni durum açmaz).
    pub fn mixer(&self) -> SoftMixer {
        self.mixer.clone()
    }

    pub fn set_volume(&self, volume: f32) {
        self.mixer.set_volume(volume_to_u16(volume));
        self.out.set_gain(self.volume.attenuation_factor() as f32);
    }
}

pub(in crate::engine) fn volume_to_u16(volume: f32) -> u16 {
    (volume.clamp(0.0, 1.0) * u16::MAX as f32) as u16
}
