use std::{mem, path::PathBuf, str::FromStr, time::Duration};

pub use crate::dither::{DithererBuilder, TriangularDitherer, mk_ditherer};
use crate::{convert::i24, player::duration_to_coefficient};

#[derive(Clone, Copy, Debug, Hash, PartialOrd, Ord, PartialEq, Eq, Default)]
pub enum Bitrate {
    Bitrate96,
    #[default]
    Bitrate160,
    Bitrate320,
}

impl FromStr for Bitrate {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "96" => Ok(Self::Bitrate96),
            "160" => Ok(Self::Bitrate160),
            "320" => Ok(Self::Bitrate320),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialOrd, Ord, PartialEq, Eq, Default)]
pub enum AudioFormat {
    F64,
    F32,
    S32,
    S24,
    S24_3,
    #[default]
    S16,
}

impl FromStr for AudioFormat {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_ref() {
            "F64" => Ok(Self::F64),
            "F32" => Ok(Self::F32),
            "S32" => Ok(Self::S32),
            "S24" => Ok(Self::S24),
            "S24_3" => Ok(Self::S24_3),
            "S16" => Ok(Self::S16),
            _ => Err(()),
        }
    }
}

impl AudioFormat {
    // not used by all backends
    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        match self {
            Self::F64 => mem::size_of::<f64>(),
            Self::F32 => mem::size_of::<f32>(),
            Self::S24_3 => mem::size_of::<i24>(),
            Self::S16 => mem::size_of::<i16>(),
            _ => mem::size_of::<i32>(), // S32 and S24 are both stored in i32
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalisationType {
    Album,
    Track,
    #[default]
    Auto,
}

impl FromStr for NormalisationType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "album" => Ok(Self::Album),
            "track" => Ok(Self::Track),
            "auto" => Ok(Self::Auto),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalisationMethod {
    Basic,
    #[default]
    Dynamic,
}

impl FromStr for NormalisationMethod {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "basic" => Ok(Self::Basic),
            "dynamic" => Ok(Self::Dynamic),
            _ => Err(()),
        }
    }
}

/// Openify: çalışırken değiştirilebilen ses ayarları (ses kalitesi, ses
/// düzeyi eşitleme). `PlayerConfig` çalar kurulurken sabitleniyor; kullanıcı
/// ayarlardan değiştirince sıradaki şarkı yüklenirken buradaki değer okunur
/// (bkz. `vendor/librespot-playback/OPENIFY.md`).
pub mod live {
    use super::Bitrate;
    use std::sync::atomic::{AtomicI32, AtomicU8, AtomicU32, Ordering::Relaxed};

    static KEY_FAILURES: AtomicU32 = AtomicU32::new(0);

    /// Çalar bir şarkının anahtarını alamadı (zaman aşımı, ret, oturum düştü).
    /// Engine bunu "çalınamayan şarkının sebebi oturumdur" kanıtı sayar
    /// (bkz. `vendor/librespot-playback/OPENIFY.md`).
    pub fn note_key_failure() {
        KEY_FAILURES.fetch_add(1, Relaxed);
    }

    /// Bugüne kadarki anahtar hatası sayısı (artışı izlenir).
    pub fn key_failures() -> u32 {
        KEY_FAILURES.load(Relaxed)
    }

    static BITRATE: AtomicU8 = AtomicU8::new(0);
    static NORMALISATION: AtomicU8 = AtomicU8::new(0);

    /// `None`: ayar verilmedi, çaların kendi yapılandırması geçerli.
    pub fn set_bitrate(bitrate: Option<Bitrate>) {
        BITRATE.store(
            match bitrate {
                None => 0,
                Some(Bitrate::Bitrate96) => 1,
                Some(Bitrate::Bitrate160) => 2,
                Some(Bitrate::Bitrate320) => 3,
            },
            Relaxed,
        );
    }

    /// `None`: ayar verilmedi, çaların kendi yapılandırması geçerli.
    pub fn set_normalisation(on: Option<bool>) {
        NORMALISATION.store(
            match on {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            },
            Relaxed,
        );
    }

    pub fn bitrate(default: Bitrate) -> Bitrate {
        match BITRATE.load(Relaxed) {
            1 => Bitrate::Bitrate96,
            2 => Bitrate::Bitrate160,
            3 => Bitrate::Bitrate320,
            _ => default,
        }
    }

    pub fn normalisation(default: bool) -> bool {
        match NORMALISATION.load(Relaxed) {
            1 => false,
            2 => true,
            _ => default,
        }
    }

    /// Eşitleme ön kazancı, 0,01 dB biriminde (`i32::MIN`: ayar verilmedi).
    static PREGAIN_CENTI_DB: AtomicI32 = AtomicI32::new(i32::MIN);

    /// Ses düzeyi eşitleme düzeyi (Sessiz / Normal / Yüksek) için ön kazanç,
    /// dB. `None`: ayar verilmedi, çaların kendi yapılandırması geçerli.
    /// Etkisi sıradaki şarkıdan başlar.
    pub fn set_normalisation_pregain_db(db: Option<f64>) {
        PREGAIN_CENTI_DB.store(db.map_or(i32::MIN, |d| (d.clamp(-30.0, 30.0) * 100.0).round() as i32), Relaxed);
    }

    pub fn normalisation_pregain_db(default: f64) -> f64 {
        match PREGAIN_CENTI_DB.load(Relaxed) {
            i32::MIN => default,
            v => v as f64 / 100.0,
        }
    }
}

#[derive(Clone)]
pub struct PlayerConfig {
    pub bitrate: Bitrate,
    pub gapless: bool,
    pub passthrough: bool,

    pub normalisation: bool,
    pub normalisation_type: NormalisationType,
    pub normalisation_method: NormalisationMethod,
    pub normalisation_pregain_db: f64,
    pub normalisation_threshold_dbfs: f64,
    pub normalisation_attack_cf: f64,
    pub normalisation_release_cf: f64,
    pub normalisation_knee_db: f64,

    pub local_file_directories: Vec<PathBuf>,

    // pass function pointers so they can be lazily instantiated *after* spawning a thread
    // (thereby circumventing Send bounds that they might not satisfy)
    pub ditherer: Option<DithererBuilder>,
    /// Setting this will enable periodically sending events during playback informing about the playback position
    /// To consume the PlayerEvent::PositionChanged event, listen to events via `Player::get_player_event_channel()``
    pub position_update_interval: Option<Duration>,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            bitrate: Bitrate::default(),
            gapless: true,
            normalisation: false,
            normalisation_type: NormalisationType::default(),
            normalisation_method: NormalisationMethod::default(),
            normalisation_pregain_db: 0.0,
            normalisation_threshold_dbfs: -2.0,
            normalisation_attack_cf: duration_to_coefficient(Duration::from_millis(5)),
            normalisation_release_cf: duration_to_coefficient(Duration::from_millis(100)),
            normalisation_knee_db: 5.0,
            passthrough: false,
            ditherer: Some(mk_ditherer::<TriangularDitherer>),
            position_update_interval: None,
            local_file_directories: Vec::new(),
        }
    }
}

// fields are intended for volume control range in dB
#[derive(Clone, Copy, Debug)]
pub enum VolumeCtrl {
    Cubic(f64),
    Fixed,
    Linear,
    Log(f64),
}

impl FromStr for VolumeCtrl {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_with_range(s, Self::DEFAULT_DB_RANGE)
    }
}

impl Default for VolumeCtrl {
    fn default() -> VolumeCtrl {
        VolumeCtrl::Log(Self::DEFAULT_DB_RANGE)
    }
}

impl VolumeCtrl {
    pub const MAX_VOLUME: u16 = u16::MAX;

    // Taken from: https://www.dr-lex.be/info-stuff/volumecontrols.html
    pub const DEFAULT_DB_RANGE: f64 = 60.0;

    pub fn from_str_with_range(s: &str, db_range: f64) -> Result<Self, <Self as FromStr>::Err> {
        use self::VolumeCtrl::*;
        match s.to_lowercase().as_ref() {
            "cubic" => Ok(Cubic(db_range)),
            "fixed" => Ok(Fixed),
            "linear" => Ok(Linear),
            "log" => Ok(Log(db_range)),
            _ => Err(()),
        }
    }
}
