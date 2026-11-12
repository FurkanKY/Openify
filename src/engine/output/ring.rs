//! Halka belleği (`RingBuf`): kare (stereo örnek çifti) numarasına göre
//! adreslenen, hücreleri `AtomicU32` (f32 bitleri) olan 2'nin kuvveti
//! kapasiteli dizi. Çıkışın halkası bir ya da iki yuvadan oluşur (bkz.
//! `OutputCtl::reserve`, `mod.rs` başlığındaki "Büyüyebilen halka"): ses iş
//! parçacığı yalnız `load` çağırır (atomik okuma, kilit ve bellek ayırma yok),
//! yazar `store` ile yazar.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use super::CHANNELS;

pub(super) struct RingBuf {
    cells: Box<[AtomicU32]>,
    mask: u64,
}

impl RingBuf {
    /// `frames`: kapasite (kare); 2'nin kuvveti olmalı. Bütün hücreler 0.
    pub fn new(frames: u64) -> Self {
        assert!(
            frames.is_power_of_two(),
            "halka kapasitesi 2'nin kuvveti olmalı"
        );
        Self {
            cells: (0..frames as usize * CHANNELS)
                .map(|_| AtomicU32::new(0))
                .collect(),
            mask: frames - 1,
        }
    }

    /// Kapasite (kare).
    pub fn frames(&self) -> u64 {
        self.mask + 1
    }

    /// `frame` numaralı karenin (L, R) örnekleri. Numara kapasiteye göre sarılır.
    #[inline]
    pub fn load(&self, frame: u64) -> (f32, f32) {
        let cell = ((frame & self.mask) as usize) * CHANNELS;
        (
            f32::from_bits(self.cells[cell].load(Relaxed)),
            f32::from_bits(self.cells[cell + 1].load(Relaxed)),
        )
    }

    /// Testler / ölçüm: ham hücreler.
    #[cfg(test)]
    pub fn raw(&self) -> &[AtomicU32] {
        &self.cells
    }

    #[inline]
    pub fn store(&self, frame: u64, left: f32, right: f32) {
        let cell = ((frame & self.mask) as usize) * CHANNELS;
        self.cells[cell].store(left.to_bits(), Relaxed);
        self.cells[cell + 1].store(right.to_bits(), Relaxed);
    }
}
