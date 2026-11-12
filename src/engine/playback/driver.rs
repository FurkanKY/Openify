//! Sürücü: çalmayı kim yürütüyor, komutlar kime gidiyor?
//!
//! Karar (PLAN-MOTOR-CONNECT §0.1): "kim başlattıysa o sürer".
//! - `Local`: Openify'da başlatılan çalma. Sıra (`Queue`), kurallar ve dengeli
//!   karıştırma bizde, çalar bizim çaladır.
//! - `Spirc`: telefondan başlatılan / aktarılan çalma (Spotify Connect, 7.5).
//!   Sıra ve ilerleme telefonun (Spirc'in); bizde yalnız gözlem ve
//!   çıkış. Çalara KOMUT vermeyiz (komutu Spirc verir); duraklat, sar, sonraki
//!   Spirc'e iletilir, çıkış önceden susturulur / kuyruğu atılır.
//! - `Remote`: başka bir aygıtta çalıyor, bizde çalan yok (7.6). Yalnız
//!   gözlem ve "buraya aktar"; uzak aygıta başka komut yok.
//!
//! Geçişler `connect.rs`'te (saf, sahte porta karşı sınanır); `TransportCmd`
//! ortak dildir. Bağlantı kapalıysa (varsayılan) hep `Local`: davranış
//! Connect'ten önceki gibidir.
//!
//! `Local`, motorun "Çal / Duraklat / Sar / Sonraki / Önceki / listeden seç /
//! yeni bağlamı başlat / ses düzeyi" komutlarını `Playback`in yükleme ve sıra
//! işlerine çevirir. Sırayı `Cx` ile ödünç alır (`Queue` motorda kalır).

use super::{Advance, Cx, Playback};
use crate::engine::TrackInfo;

/// Sürücüden bağımsız taşıma komutları (arayüzün `Command`larının çalmaya
/// dönük hâli).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::engine) enum TransportCmd {
    Play,
    Pause,
    Seek(u32),
    Next,
    Previous,
    /// Görünür listedeki `i`. şarkıya atla.
    JumpTo(usize),
    /// Sıra zaten kuruldu: `uri` şarkısını `position_ms`'den çalmaya başla.
    Start { uri: String, position_ms: u32 },
    /// 0..1.
    SetVolume(f32),
}

/// Yerel sürücü: çalma sırası ve çalar bizde. Durumu `Playback`in alanlarında
/// (çalar, konum, kurtarma sayaçları) ve motorun `Queue`sunda durur.
#[derive(Debug, Default)]
pub(in crate::engine) struct Local;

/// Spirc'e geçerken yerelde gösterilen şarkı ve konum: Spirc bırakılınca
/// (telefon geri aldı) arayüz Openify'ın kendi durumuna döner.
#[derive(Debug, Default, Clone)]
pub(in crate::engine) struct LocalSnapshot {
    pub track: Option<TrackInfo>,
    pub position_ms: u32,
}

/// Spirc sürücüsü: telefon (ya da Spotify) çalmayı yürütüyor.
#[derive(Debug)]
pub(in crate::engine) struct SpircDrive {
    pub snapshot: LocalSnapshot,
    /// Çıkış duraklatma için susturuldu (çalma sürünce kalkar).
    pub silenced: bool,
    /// Sıradaki `Loading` zaten hesaba katıldı (kendiliğinden geçiş, kendi
    /// komutumuz ya da etkinleşme yıkaması): kuyruk yeniden atılmaz.
    pub load_expected: bool,
}

/// Başka bir aygıtta çalıyor (7.6).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::engine) struct RemoteDrive {
    pub device_id: String,
    pub device_name: String,
}

pub(in crate::engine) enum Driver {
    Local(Local),
    Spirc(SpircDrive),
    Remote(RemoteDrive),
}

impl Playback {
    /// Taşıma komutunu etkin sürücüye ver.
    pub fn transport(&mut self, cx: &mut Cx<'_>, cmd: TransportCmd) {
        match &self.driver {
            Driver::Local(_) => self.local_transport(cx, cmd),
            Driver::Spirc(_) => self.spirc_transport(cx, cmd),
            Driver::Remote(_) => self.remote_transport(cx, cmd),
        }
    }

    pub(super) fn local_transport(&mut self, cx: &mut Cx<'_>, cmd: TransportCmd) {
        match cmd {
            TransportCmd::Play => match self.restored {
                // Geri yüklenen şarkı ilk kez çalacak: kaldığı konumdan yükle.
                Some(position) => self.play_restored(cx, position),
                None => self.play(),
            },
            // Geri yüklenen şarkı çalara yüklü değil: duraklatma zaten
            // duraklatılmış, sarma yalnız kayıtlı konumu değiştirir.
            TransportCmd::Pause if self.restored.is_some() => {}
            TransportCmd::Pause => self.pause(),
            TransportCmd::Seek(ms) if self.restored.is_some() => self.seek_restored(cx, ms),
            TransportCmd::Seek(ms) => self.seek(ms),
            TransportCmd::Next => {
                self.advance(cx, Advance::Next, true);
            }
            TransportCmd::Previous => {
                self.advance(cx, Advance::Previous, true);
            }
            TransportCmd::JumpTo(i) => self.jump_to(cx, i),
            TransportCmd::Start { uri, position_ms } => self.begin(cx, uri, position_ms),
            TransportCmd::SetVolume(volume) => self.set_volume(volume),
        }
    }
}
