//! Çaların gördüğü ses çıkışı (`Sink`): kendi çıkışımızı (`output.rs`) ve
//! seviye çözümleyicisini (`levels.rs`) birleştirir.
//!
//!   çalar ──write──▶ LevelSink ──▶ kapı (eski ses mi?) ──▶ LevelTap
//!                                                     └──▶ Writer ──▶ halka
//!
//! # Ses düzeyi nerede uygulanıyor
//!
//! librespot çalarında örnekler çıkışa gelmeden önce yazılım ses düzeyiyle
//! (`VolumeGetter::attenuation_factor`) çarpılır; öyle kalsa hem çubuklar ses
//! kısılınca düzleşirdi hem de değişiklik kuyruk boyu (eskiden ~0,6 sn)
//! gecikirdi. Bu yüzden `attach` çalara sabit 1,0 döndüren bir `NoOpVolume`
//! verir; gerçek ses düzeyi ses kartının geri çağrısında, kuyruktan SONRA
//! uygulanır (`OutputCtl::set_gain`, ~10 ms'lik rampa). Çözümleme ses
//! düzeyinden önceki örneklerde: çubuklar ses düzeyinden bağımsız,
//! normalleştirme (şarkı başına kazanç) ve sınırlayıcıdan SONRAKİ düzeye
//! bakar.
//!
//! `Raw` paketler (geçiş kipi; Openify'da kapalı) çalınamaz, atılır.

use std::sync::Arc;

use librespot::playback::{
    NUM_CHANNELS, SAMPLE_RATE,
    audio_backend::{Sink, SinkError, SinkResult},
    convert::Converter,
    decoder::AudioPacket,
};

use super::{
    levels::{self, LevelTap},
    output::{self, BeatTap, Device, OutputCtl, Writer},
};

// levels.rs / output.rs librespot'u bilmez; sabit varsayımlar burada
// doğrulanır.
const _: () = assert!(SAMPLE_RATE == 44_100 && NUM_CHANNELS == 2);
const _: () = assert!(levels::SAMPLE_RATE == SAMPLE_RATE as f64);
const _: () = assert!(output::RATE == SAMPLE_RATE && output::CHANNELS == NUM_CHANNELS as usize);

pub struct LevelSink {
    out: Writer,
    tap: LevelTap,
    /// Ses kartı akışı (çalar iş parçacığında yaşar). Açılamadıysa ya da
    /// aygıt gittiyse `start` o anki varsayılanı yeniden açar; olmazsa hata
    /// döner, çalar duraklar.
    device: Device,
    boosted: bool,
    raw_warned: bool,
    /// AutoMix çözümleyicisi (`output/beat.rs`); ilk kullanımda kurulur, AutoMix
    /// kapalıyken hiç kurulmaz / çalışmaz.
    beat: Option<Box<BeatTap>>,
}

impl LevelSink {
    /// Varsayılan ses aygıtını açar. `Player::new`'in `sink_builder`'ında,
    /// yani çalar iş parçacığında çağrılır.
    pub fn open(ctl: Arc<OutputCtl>) -> Self {
        let device = Device::open(Arc::clone(&ctl));
        Self::with_device(ctl, device)
    }

    /// Aygıtsız (testler).
    #[cfg(test)]
    fn new(ctl: Arc<OutputCtl>) -> Self {
        let device = Device::headless(Arc::clone(&ctl));
        Self::with_device(ctl, device)
    }

    fn with_device(ctl: Arc<OutputCtl>, device: Device) -> Self {
        let tap = LevelTap::new(ctl.levels(), levels::device_delay_ns());
        Self {
            out: Writer::new(ctl),
            tap,
            device,
            boosted: false,
            raw_warned: false,
            beat: None,
        }
    }

    /// Motor komutlarını işle; kuyruk atıldıysa çözümleyici de sıfırlanır.
    fn sync(&mut self) {
        if self.out.sync() {
            self.flushed();
        }
    }

    /// Kuyruk atıldı (sarma, şarkı değiştirme): seviye çözümleyici sıfırlanır, AutoMix
    /// çözümleyicisi akışın kesildiğini öğrenir.
    fn flushed(&mut self) {
        self.tap.on_flush(self.out.now_ns());
        if let Some(beat) = &mut self.beat {
            beat.discontinuity();
        }
    }

    /// AutoMix çözümlemesi: yeni şarkının ilk paketinde öncekini kapatır (canlı sonucu
    /// yazara verir, motora yayınlar), yenisini başlatır; paketi çözümleyiciye besler.
    /// AutoMix kapalıyken hiçbir şey yapmaz.
    fn analyze_beats(&mut self, samples: &[f64]) {
        if !self.out.ctl().automix_on() {
            if let Some(beat) = &mut self.beat {
                beat.abort();
            }
            return;
        }
        let beat = self.beat.get_or_insert_with(|| Box::new(BeatTap::new()));
        if let Some(track) = self.out.pending_track() {
            let prev = beat.finish(track.ended);
            let live = prev.as_ref().map(|(_, a)| *a).filter(|a| a.has_tail());
            if let Some((uri, analysis)) = prev {
                self.out.ctl().publish_analysis(uri, analysis);
            }
            self.out.set_prev_analysis(live);
            beat.begin(track.uri, track.from_start);
        }
        beat.feed(samples);
    }
}

impl Sink for LevelSink {
    fn start(&mut self) -> SinkResult<()> {
        // Aygıt gittiyse / varsayılan değiştiyse o anki varsayılan açılır
        // (7 Kasım: Bluetooth kulaklık gidince "Devam" ölü aygıta yazıyordu).
        if let Err(e) = self.device.ensure_for_start() {
            log::error!("Ses çıkışı açılamadı: {e}");
            return Err(SinkError::ConnectionRefused(format!(
                "ses çıkışı açılamadı: {e}"
            )));
        }
        self.sync();
        // Duraklatmada kesilen, kuyrukta bekleyen sesin çubukları yeni
        // anlarıyla geri gelsin.
        let read = self.out.read_frame();
        self.tap
            .resume(self.out.ctl().pull_ns(read, self.out.now_ns()), read);
        self.out.start();
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        // Beklemez: ses ~8 ms'lik inişle susar, kuyruk korunur (devamda
        // kaldığı yerden sürer).
        self.sync();
        self.out.stop();
        self.tap.hold();
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
        let AudioPacket::Samples(samples) = &packet else {
            if !self.raw_warned {
                self.raw_warned = true;
                log::warn!("Ses çıkışı: ham (geçiş kipi) paket çalınamaz, atılıyor");
            }
            return Ok(());
        };
        if !self.boosted {
            self.boosted = true;
            #[cfg(not(test))]
            boost_thread();
        }
        // Varsayılan aygıt değiştiyse yeni aygıta geç; aygıt gittiyse dur
        // (çalar duraklar, kuyruk ve konum korunur).
        self.device.follow_default();
        self.out.check_device()?;
        self.sync();
        let frames = (samples.len() / NUM_CHANNELS as usize) as u64;
        if !self.out.accept(frames) {
            // `flush`tan önce çözülmüş eski ses.
            return Ok(());
        }
        if self.out.is_live() {
            // Susturma `start` olmadan kalktıysa (örn. yükleme sürerken
            // duraklat + çal) kesme burada kapanır, çubuklar sürer.
            let read = self.out.read_frame();
            self.tap
                .resume(self.out.ctl().pull_ns(read, self.out.now_ns()), read);
        }
        // AutoMix çözümlemesi (kapalıyken hiçbir şey): yeni şarkıysa öncekini kapat
        // (canlı sonucu yazar geçişi kurarken kullanır), paketi besle.
        self.analyze_beats(samples);
        // Yeni şarkının ilk paketi: geçiş (bindirme) burada kurulur.
        self.out.begin_packet();
        // AutoMix: gelen şarkının baştaki, giden şarkının sondaki sessizliği atılır
        // (kapalıyken paketin kendisi).
        let packet = self.out.prepare(samples);
        let samples: &[f64] = &packet;
        let frames = (samples.len() / NUM_CHANNELS as usize) as u64;
        if frames == 0 {
            return Ok(());
        }
        // Bindirilen baş kareler halkada duran giden sesin üstüne karışır ve
        // yazma ucunu ilerletmez; seviye çözümlemesi onları atlar (giden
        // şarkının çubukları bindirme boyunca sürer, yeni şarkınınkiler
        // bindirme bitince, doğru kare numarasıyla başlar).
        let skip = self.out.overlap_in(frames) as usize;
        // Çözümleme ses düzeyinden ÖNCE (çalar 1,0 gördü).
        let first = self.out.written();
        let now = self.out.now_ns();
        let pull = self.out.ctl().pull_ns(first, now);
        if skip < frames as usize {
            self.tap
                .analyze(&samples[skip * NUM_CHANNELS as usize..], now, pull, first);
        }
        if self.out.push(samples)? {
            self.flushed();
            return Ok(());
        }
        if self.out.wait()? {
            self.flushed();
        }
        Ok(())
    }
}

/// Çalar iş parçacığının önceliğini yükselt: kuyruk 120 ms; işlemci
/// doluyken (derleme, arayüz canlandırması) çözme gecikip kuyruk boşalmasın.
/// Ses kartının iş parçacığı zaten "zaman kritik" (cpal). Testlerde
/// çağrılmaz (test iş parçacığının önceliği değişmesin).
#[cfg_attr(test, allow(dead_code))]
fn boost_thread() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
        };
        // SAFETY: geçerli iş parçacığının sözde tutamacı; yan etkisi yalnız öncelik.
        if let Err(e) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } {
            log::debug!("Çalar iş parçacığının önceliği yükseltilemedi: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;
    use crate::engine::levels::AudioLevels;

    fn rig() -> (LevelSink, output::RingSource, Arc<OutputCtl>) {
        let clock = Arc::new(AtomicU64::new(1_000_000_000));
        let ctl = OutputCtl::with_clock(AudioLevels::new(), clock);
        let src = output::RingSource::new(ctl.clone());
        (LevelSink::new(ctl.clone()), src, ctl)
    }

    fn pull(src: &mut output::RingSource, frames: usize) -> Vec<f32> {
        (0..frames * 2).map(|_| src.next().unwrap()).collect()
    }

    fn tone(freq: f64, amp: f64, frames: usize) -> Vec<f64> {
        (0..frames)
            .flat_map(|i| {
                let s = amp * (2.0 * std::f64::consts::PI * freq * i as f64 / 44_100.0).sin();
                [s, -s * 0.5]
            })
            .collect()
    }

    /// 1024 karelik paketlerle yazar (toplam kuyruk hedefini aşmamalı:
    /// çeken yok, aşarsa `write` bekler).
    fn write_all(sink: &mut LevelSink, samples: &[f64]) {
        let mut conv = Converter::new(None);
        for c in samples.chunks(2048) {
            sink.write(AudioPacket::Samples(c.to_vec()), &mut conv)
                .unwrap();
        }
    }

    #[test]
    fn passes_samples_through_unchanged_at_full_volume() {
        let (mut sink, mut src, _) = rig();
        sink.start().unwrap();
        let input = tone(440.0, 0.4, 4096);
        write_all(&mut sink, &input);
        let out = pull(&mut src, 4096);
        // Giriş zarfından sonra f32'ye çevrilmiş girişin aynısı.
        let skip = output::FADE_FRAMES as usize * 2;
        for (i, (o, x)) in out.iter().zip(&input).enumerate().skip(skip) {
            assert_eq!(*o, *x as f32, "örnek {i}");
        }
    }

    #[test]
    fn volume_applies_at_output_and_mute_is_silent() {
        let (mut sink, mut src, ctl) = rig();
        sink.start().unwrap();
        ctl.set_gain(0.25);
        let input = tone(440.0, 0.4, 4096);
        write_all(&mut sink, &input);
        let out = pull(&mut src, 4096);
        for (o, x) in out.iter().zip(&input).skip(1000) {
            assert!((*o - (*x as f32) * 0.25).abs() < 1e-6);
        }
        ctl.set_gain(0.0);
        write_all(&mut sink, &input);
        let out = pull(&mut src, 4096);
        assert!(out[2000..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn levels_ignore_volume() {
        // Aynı ses %100, %5 ve sessizde: çubuklar aynı (ses düzeyi çıkışta
        // uygulanır, analiz öncesinde).
        let input = tone(60.0, 0.5, 44_100);
        let read = |gain: f32| {
            let (mut sink, mut src, ctl) = rig();
            ctl.set_gain(gain);
            sink.start().unwrap();
            let mut conv = Converter::new(None);
            for c in input.chunks(2048) {
                sink.write(AudioPacket::Samples(c.to_vec()), &mut conv)
                    .unwrap();
                pull(&mut src, 1024);
            }
            ctl.levels().last_entry().expect("girdi yok").1[0]
        };
        let full = read(1.0);
        assert!(full > 0.8, "{full}");
        assert_eq!(read(0.05), full);
        assert_eq!(read(0.0), full);
    }

    #[test]
    fn stop_does_not_block_and_keeps_the_queue() {
        let (mut sink, mut src, ctl) = rig();
        sink.start().unwrap();
        write_all(&mut sink, &tone(440.0, 0.4, 4096));
        pull(&mut src, 1000);
        let t = std::time::Instant::now();
        sink.stop().unwrap();
        assert!(
            t.elapsed() < std::time::Duration::from_millis(5),
            "stop bekledi"
        );
        pull(&mut src, 1000);
        let queued = ctl.queued_frames();
        assert!(queued > 2000, "kuyruk atıldı: {queued}");
        sink.start().unwrap();
        pull(&mut src, 4000);
        assert_eq!(ctl.queued_frames(), 0);
    }

    #[test]
    fn flush_resets_levels_and_discards() {
        let (mut sink, mut src, ctl) = rig();
        sink.start().unwrap();
        write_all(&mut sink, &tone(60.0, 0.5, 4096));
        assert!(ctl.levels().last_entry().unwrap().1[0] > 0.5);
        pull(&mut src, 1000);
        ctl.flush();
        pull(&mut src, 1000);
        // Yeni ses (sessizlik) yazılır; eski kuyruk atılır, çözümleyici sıfırdan.
        write_all(&mut sink, &vec![0.0; 4096 * 2]);
        pull(&mut src, 200);
        let q = ctl.queued_frames();
        assert!(q <= 4096 && q >= 4096 - 200, "eski ses kuyrukta: {q}");
        assert_eq!(ctl.levels().last_entry().unwrap().1, [0.0; 3]);
    }

    #[test]
    fn duck_without_sink_restart_does_not_freeze_bars() {
        // Yükleme sürerken duraklat + çal: çalar `stop`/`start` demez.
        let (mut sink, mut src, ctl) = rig();
        sink.start().unwrap();
        let input = tone(60.0, 0.5, 2048);
        write_all(&mut sink, &input);
        pull(&mut src, 2048);
        ctl.duck();
        ctl.unduck();
        write_all(&mut sink, &input);
        let (t, v) = ctl.levels().last_entry().unwrap();
        let seen = ctl.levels().at_ns(t).expect("çubuklar dondu");
        assert!(
            (seen[0] - v[0]).abs() < 0.01 && v[0] > 0.5,
            "{seen:?} / {v:?}"
        );
    }

    /// Şarkı sonu payıyla kuyruk ~4 sn: duyulan anın çubuk girdileri, 4 sn
    /// sonrası yazılırken ezilmemeli (seviye halkası 3 sn iken eziliyordu).
    #[test]
    fn bars_survive_the_deep_tail_queue() {
        let (mut sink, _src, ctl) = rig();
        sink.start().unwrap();
        ctl.set_tail(true);
        let now = ctl.now_ns();
        write_all(&mut sink, &tone(60.0, 0.5, 170_000));
        assert!(ctl.queued_frames() >= 169_000);
        // İlk pencerelerin duyulacağı an (çekilme + D + pencere ortası).
        let t = ctl.pull_ns(2048, now) + levels::device_delay_ns() + 12_000_000;
        let v = ctl
            .levels()
            .at_ns(t)
            .expect("duyulacak anın girdisi ezildi");
        assert!(v[0] > 0.5, "{v:?}");
    }

    /// 7 Kasım: Bluetooth kulaklık gitti, "Devam" ölü aygıta yazıp 2 sn sonra
    /// yine durdu. Şimdi: yazma hemen hata (çalar duraklar), `start` o anki
    /// varsayılanı açar, kaldığı kareden sürer.
    #[test]
    fn device_loss_pauses_and_start_reopens_on_current_default() {
        let (mut sink, mut src, ctl) = rig();
        sink.start().unwrap();
        write_all(&mut sink, &tone(440.0, 0.4, 2048));
        pull(&mut src, 1000);
        ctl.signal_device_lost("sınama");
        let mut conv = Converter::new(None);
        let t = std::time::Instant::now();
        assert!(
            sink.write(AudioPacket::Samples(tone(440.0, 0.4, 1024)), &mut conv)
                .is_err()
        );
        assert!(t.elapsed() < std::time::Duration::from_millis(50));
        assert_eq!(ctl.take_device_notice(), Some(output::DeviceNotice::Lost));
        let queued = ctl.queued_frames();
        sink.stop().unwrap();
        sink.start().unwrap();
        assert!(!ctl.is_device_lost(), "yeniden açılmadı");
        assert_eq!(ctl.queued_frames(), queued, "kuyruk korunmadı");
        // Yeni aygıtın kaynağı kalan sesi çalar; yazma yine çalışır.
        let mut src = output::RingSource::new(ctl.clone());
        let out = pull(&mut src, 2000);
        assert!(out.iter().any(|x| x.abs() > 0.1), "çalmadı");
        write_all(&mut sink, &tone(440.0, 0.4, 2048));
    }

    #[test]
    fn raw_packets_are_dropped() {
        let (mut sink, _, ctl) = rig();
        sink.write(AudioPacket::Raw(vec![1, 2, 3]), &mut Converter::new(None))
            .unwrap();
        assert_eq!(ctl.queued_frames(), 0);
    }
}
