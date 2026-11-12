use super::*;
use librespot::{
    core::{SpotifyId, SpotifyUri},
    playback::player::PlayerEvent,
};
use tokio::sync::mpsc as tmpsc;

struct Rig {
    ctl: Arc<OutputCtl>,
    clock: Arc<AtomicU64>,
    src: RingSource,
    w: Writer,
}

impl Rig {
    fn new() -> Self {
        let clock = Arc::new(AtomicU64::new(1_000_000_000));
        let ctl = OutputCtl::with_clock(AudioLevels::new(), clock.clone());
        let src = RingSource::new(ctl.clone());
        let w = Writer::new(ctl.clone());
        Rig { ctl, clock, src, w }
    }

    fn advance_ms(&self, ms: f64) {
        self.clock.fetch_add((ms * 1e6) as u64, Relaxed);
    }

    /// `frames` kare çeker (L, R çiftleri).
    fn pull(&mut self, frames: usize) -> Vec<(f32, f32)> {
        (0..frames)
            .map(|_| (self.src.next().unwrap(), self.src.next().unwrap()))
            .collect()
    }

    /// Gerçek bir geri çağrı gibi: saati 10 ms ilerlet, 441 kare çek.
    fn callback(&mut self) -> Vec<(f32, f32)> {
        self.advance_ms(10.0);
        self.pull(441)
    }

    fn write(&mut self, samples: &[f64]) {
        self.w.sync();
        if self.w.accept((samples.len() / 2) as u64) {
            self.w.push(samples).unwrap();
        }
    }
}

/// L = kare numarası / 1e6 (kimlik), R = sabit 0,5.
fn ramp(first: u64, frames: u64) -> Vec<f64> {
    (first..first + frames)
        .flat_map(|i| [i as f64 / 1e6, 0.5])
        .collect()
}

fn dc(frames: usize, v: f64) -> Vec<f64> {
    vec![v; frames * 2]
}

fn id_of(sample: f32) -> u64 {
    (sample as f64 * 1e6).round() as u64
}

/// Zarflı karenin kimliği: R kanalı (0,5 × zarf) zarfı verir. Sessiz
/// karede `None`.
fn id_env(s: (f32, f32)) -> Option<u64> {
    (s.1 > 0.0).then(|| (s.0 as f64 / (s.1 as f64 / 0.5) * 1e6).round() as u64)
}

#[test]
fn never_returns_none_and_underrun_is_counted_once() {
    let mut rig = Rig::new();
    rig.w.start();
    // Veri yok, çalma başı: sıfır, xrun değil.
    assert!(rig.pull(100_000).iter().all(|s| *s == (0.0, 0.0)));
    assert_eq!(rig.ctl.xruns(), 0);
    rig.write(&dc(2048, 0.25));
    let got = rig.pull(4096);
    assert!(got[1000].0 > 0.24, "çalmadı: {:?}", got[1000]);
    assert!(got[3000..].iter().all(|s| *s == (0.0, 0.0)), "veri bitince sıfır");
    assert_eq!(rig.ctl.xruns(), 0, "veri gelmeden xrun sayılmaz");
    // Kesinti bitti: bir xrun, hedef büyüdü.
    let before = rig.ctl.target_frames();
    rig.write(&dc(2048, 0.25));
    rig.pull(1024);
    assert_eq!(rig.ctl.xruns(), 1);
    assert!(rig.ctl.target_frames() > before);
    assert!(rig.ctl.underrun_frames.load(Relaxed) >= 1000);
}

#[test]
fn plays_exact_samples_in_order_at_full_volume() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&ramp(0, 20_000));
    let got = rig.pull(20_000);
    // Giriş zarfından (8 ms) sonra bit bit aynı.
    for (i, s) in got.iter().enumerate().skip(FADE_FRAMES as usize) {
        assert_eq!(id_of(s.0), i as u64, "kare {i}");
        assert_eq!(s.1, 0.5);
    }
    assert_eq!(rig.ctl.played_frames(), 20_000);
}

#[test]
fn write_fills_to_target_blocks_then_releases() {
    let rig = Rig::new();
    let ctl = rig.ctl.clone();
    let mut src = rig.src;
    let mut w = rig.w;
    w.start();
    let target = ctl.target_frames();
    let (tx, rx) = mpsc::channel();
    let writer = thread::spawn(move || {
        for k in 0..40u64 {
            w.sync();
            w.push(&ramp(k * 1024, 1024)).unwrap();
            w.wait().unwrap();
            tx.send(k).unwrap();
        }
    });
    // Hedef dolana dek beklemeden yazar, sonra durur.
    let mut done = 0;
    while rx.recv_timeout(Duration::from_millis(300)).is_ok() {
        done += 1;
    }
    let queued = ctl.queued_frames();
    assert!(queued > target && queued <= target + 1024, "kuyruk {queued}, hedef {target}");
    assert!(done < 40, "yazar beklemedi");
    let waiting = (0..200).any(|_| {
        thread::sleep(Duration::from_millis(1));
        ctl.writer_waiting.load(SeqCst)
    });
    assert!(waiting, "yazar uyumuyor");
    // Çekince bırakır; sonuna dek bütün paketler yazılır. Kareler sıralı,
    // atlanan / yinelenen yok (yazar geç uyanıp kuyruk boşalırsa sessiz
    // kareler olabilir; onlar sayılmaz).
    let mut ids = Vec::new();
    for _ in 0..200 {
        for _ in 0..441 {
            let s = (src.next().unwrap(), src.next().unwrap());
            ids.extend(id_env(s));
        }
        thread::sleep(Duration::from_millis(2));
    }
    writer.join().unwrap();
    done += rx.try_iter().count();
    assert_eq!(done, 40);
    assert_eq!(ids.len(), 40 * 1024);
    assert!(ids.windows(2).all(|p| p[1] == p[0] + 1), "sıra bozuldu");
}

#[test]
fn duck_silences_fast_and_resume_is_bit_continuous() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&ramp(0, 30_000));
    let a = rig.pull(5000);
    let mark = rig.ctl.duck();
    assert!(mark > 0);
    let b = rig.pull(2000);
    // İniş (o an çekilmiş blok + zarf boyu) içinde biter, sonrası tam sıfır.
    let silent_from = b.iter().position(|s| *s == (0.0, 0.0)).unwrap();
    assert!(silent_from <= BLOCK + FADE_FRAMES as usize, "{silent_from}");
    assert!(b[silent_from..].iter().all(|s| *s == (0.0, 0.0)));
    // Kuyruk korunuyor: çalınan son kare + 1'den sürer.
    let last_played = id_env(b[silent_from - 1]).unwrap();
    let r = rig.ctl.read.load(SeqCst);
    assert_eq!(r, last_played + 1);
    assert_eq!(id_of(a[4999].0), 4999);
    // Devam (çalar `start`): bit bit kaldığı yerden, girişle.
    rig.w.stop();
    rig.w.start();
    let c = rig.pull(1000);
    // (Önce o an çekilmiş, sessiz bloğun kalanı.)
    let first = c.iter().position(|s| s.1 > 0.0).unwrap();
    assert!(first < BLOCK);
    assert_eq!(id_env(c[first]), Some(r), "ilk kare");
    assert!(c[first].1 < 0.01, "giriş zarfı yok");
    let full = first + FADE_FRAMES as usize + 10;
    assert_eq!(id_of(c[full].0), r + FADE_FRAMES as u64 + 10);
    assert_eq!(c[full].1, 0.5);
}

#[test]
fn fades_have_no_discontinuity() {
    // Tam ölçekli DC: ardışık örnekler arası en büyük adım ~1/353.
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&dc(40_000, 1.0));
    let mut out = rig.pull(3000);
    rig.ctl.duck();
    out.extend(rig.pull(1000));
    rig.ctl.unduck();
    out.extend(rig.pull(3000));
    rig.ctl.flush();
    rig.w.sync();
    out.extend(rig.pull(1000));
    let eps = 1.0 / FADE_FRAMES as f32 + 1e-4;
    let max = out.windows(2).map(|p| (p[1].0 - p[0].0).abs()).fold(0.0, f32::max);
    assert!(max <= eps, "en büyük adım {max} > {eps}");
    assert!(out.iter().any(|s| s.0 == 1.0) && out.last().unwrap().0 == 0.0);
}

#[test]
fn flush_drops_queue_and_new_audio_follows() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&ramp(0, 20_000));
    rig.pull(3000);
    rig.ctl.flush();
    // Yazar onaylamadan: iniş, sonra sessizlik; eski ses atılmayı bekler.
    let b = rig.pull(3000);
    assert!(b[BLOCK + FADE_FRAMES as usize..].iter().all(|s| *s == (0.0, 0.0)));
    assert_eq!(rig.ctl.queued_frames(), 0, "bekleyen flush: kuyruk 0 sayılır");
    // Yazar onaylar (olay kanalı yok: kapı açık), yeni ses yazılır.
    rig.write(&ramp(1_000_000, 5000));
    let c = rig.pull(3000);
    let first = c.iter().position(|s| s.0 != 0.0).unwrap();
    let id = id_of(c[first + FADE_FRAMES as usize].0);
    assert!(id >= 1_000_000 && id < 1_000_000 + 2 * FADE_FRAMES as u64, "eski ses çaldı: {id}");
    // (Ses iş parçacığının o an doldurduğu blok da okunmuş sayılır.)
    let expect = 5000 - (3000 - first) as u64;
    let q = rig.ctl.queued_frames();
    assert!(q <= expect && q + BLOCK as u64 > expect, "kuyruk {q}, beklenen ~{expect}");
}

fn uri() -> SpotifyUri {
    SpotifyUri::Track { id: SpotifyId::from_base62("2WUy2Uywcj5cP0IXQagO3z").unwrap() }
}

#[test]
fn gate_drops_stale_packets_until_marker_then_confirmation() {
    let mut rig = Rig::new();
    let (tx, rx) = tmpsc::unbounded_channel();
    rig.ctl.attach_events(rx);
    rig.w.start();
    rig.write(&ramp(0, 5000));
    rig.pull(2000);
    // Flush'tan ÖNCE işlenmiş eski bir komutun olayı kanalda bekliyor.
    tx.send(PlayerEvent::PlayRequestIdChanged { play_request_id: 7 }).unwrap();
    let mark = rig.ctl.flush();
    // Çalar sarmayı işlemeden eski konumdan bir paket daha yazar: atılır.
    rig.write(&ramp(5000, 1024));
    // İşaret geldi ama komutun sonucu henüz yok: hâlâ atılır.
    tx.send(mark_event(mark)).unwrap();
    rig.write(&ramp(6024, 1024));
    assert_eq!(rig.w.written(), 5000, "eski paket kuyruğa girdi");
    // Sarma işlendi: yeni ses.
    tx.send(PlayerEvent::Seeked { play_request_id: 7, track_id: uri(), position_ms: 90_000 }).unwrap();
    rig.write(&ramp(2_000_000, 4000));
    assert_eq!(rig.w.written(), 9000);
    let out = rig.pull(6000);
    let ids: Vec<u64> = out.iter().filter_map(|s| id_env(*s)).collect();
    let fade_end = 2000 + (BLOCK + FADE_FRAMES as usize) as u64;
    assert!(ids.iter().all(|i| *i < fade_end || *i >= 2_000_000), "eski ses sızdı");
    assert!(ids.iter().any(|i| *i >= 2_000_000 + 3000));
}

/// Spirc (Spotify Connect) telefondan gelen ses düzeyini `VolumeChanged` ile
/// bildirir. Sayısı işaretin sayısına denk gelse bile kapıyı açmamalı ve
/// duraklatmanın susturmasını kaldırmamalı (7.3: işaret `SessionClientChanged`).
#[test]
fn a_volume_event_with_the_same_number_never_counts_as_the_marker() {
    let mut rig = Rig::new();
    let (tx, rx) = tmpsc::unbounded_channel();
    rig.ctl.attach_events(rx);
    rig.w.start();
    rig.write(&ramp(0, 5000));
    rig.pull(2000);
    let mark = rig.ctl.flush();
    // Aynı sayıyı taşıyan gerçek bir ses düzeyi olayı (telefon ses düzeyini oynattı).
    tx.send(PlayerEvent::VolumeChanged { volume: mark as u16 }).unwrap();
    // Komutun sonucu (işaret hiç gelmedi): eski ses hâlâ atılmalı.
    tx.send(PlayerEvent::Seeked { play_request_id: 7, track_id: uri(), position_ms: 90_000 }).unwrap();
    rig.write(&ramp(5000, 1024));
    assert_eq!(rig.w.written(), 5000, "ses düzeyi olayı işaret sayıldı, kapı erken açıldı");
    // Gerçek işaret (başka bir kimlikle gelen aynı sayı da sayılmaz) ardından sonuç.
    tx.send(PlayerEvent::SessionClientChanged {
        client_id: "baska-istemci".into(),
        client_name: mark.to_string(),
        client_brand_name: String::new(),
        client_model_name: String::new(),
    })
    .unwrap();
    rig.write(&ramp(6024, 1024));
    assert_eq!(rig.w.written(), 5000, "başka istemcinin olayı işaret sayıldı");
    tx.send(mark_event(mark)).unwrap();
    tx.send(PlayerEvent::Seeked { play_request_id: 7, track_id: uri(), position_ms: 90_000 }).unwrap();
    rig.write(&ramp(2_000_000, 1024));
    assert_eq!(rig.w.written(), 6024, "gerçek işaret ve sonuç kapıyı açar");
}

/// İşaret artık 64 bit: 65 536'dan sonraki işaretler eskiden başa sarardı
/// (`as u16`) ve eski bir işaretle karışabilirdi.
#[test]
fn marks_beyond_sixteen_bits_round_trip_through_the_event() {
    for mark in [1u64, 65_535, 65_536, 65_537, 1 << 40, u64::MAX] {
        assert_eq!(mark_of(&mark_event(mark)), Some(mark));
    }
    assert_eq!(mark_of(&PlayerEvent::VolumeChanged { volume: 7 }), None);
    let other = PlayerEvent::SessionClientChanged {
        client_id: "x".into(),
        client_name: "7".into(),
        client_brand_name: String::new(),
        client_model_name: String::new(),
    };
    assert_eq!(mark_of(&other), None, "yalnız openify-mark kimliği işarettir");
    let junk = PlayerEvent::SessionClientChanged {
        client_id: MARK_CLIENT_ID.into(),
        client_name: "sayı değil".into(),
        client_brand_name: String::new(),
        client_model_name: String::new(),
    };
    assert_eq!(mark_of(&junk), None);
    // Gerçek işaretler birbirinden farklı ve artan (aynı sayaçtan).
    let rig = Rig::new();
    let (a, b) = (rig.ctl.flush(), rig.ctl.duck());
    assert!(b > a && a >= 1);
}

#[test]
fn duck_cleared_when_player_starts_playing_after_marker() {
    let mut rig = Rig::new();
    let (tx, rx) = tmpsc::unbounded_channel();
    rig.ctl.attach_events(rx);
    rig.w.start();
    // Eski bir Playing olayı (işaretten önce) susturmayı kaldırmaz.
    tx.send(PlayerEvent::Playing { play_request_id: 1, track_id: uri(), position_ms: 0 }).unwrap();
    let mark = rig.ctl.duck();
    rig.w.sync();
    assert!(rig.ctl.duck.load(SeqCst));
    // Duraklatma çalarda boşa düştü, sıradaki şarkı çalmaya başladı.
    tx.send(mark_event(mark)).unwrap();
    tx.send(PlayerEvent::Playing { play_request_id: 2, track_id: uri(), position_ms: 0 }).unwrap();
    rig.w.sync();
    assert!(!rig.ctl.duck.load(SeqCst));
}

#[test]
fn volume_ramps_in_ten_ms_and_mute_is_exact_zero() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&dc(20_000, 0.8));
    rig.pull(1000);
    rig.ctl.set_gain(0.25);
    let out = rig.pull(1000);
    // Rampa: 441 karede tek yönlü, adım küçük; sonrası tam 0,8 × 0,25.
    let l: Vec<f32> = out.iter().map(|s| s.0).collect();
    assert!(l.windows(2).all(|p| p[1] <= p[0] + 1e-6));
    assert!(l.windows(2).all(|p| (p[1] - p[0]).abs() < 0.8 * 0.75 / 400.0));
    assert!((l[GAIN_RAMP_FRAMES as usize + 64] - 0.2).abs() < 1e-6);
    assert!((l[100] - 0.2).abs() > 0.05, "rampa yok");
    rig.ctl.set_gain(0.0);
    let out = rig.pull(1000);
    assert!(out[GAIN_RAMP_FRAMES as usize + 64..].iter().all(|s| *s == (0.0, 0.0)));
}

#[test]
fn audible_position_corrects_for_queued_audio() {
    let mut rig = Rig::new();
    rig.w.start();
    // 10 paket × 1024 kare yazıldı; çalar son paketin başını (9216.
    // kare = 208,98 ms) bildirir.
    for k in 0..10 {
        rig.write(&dc(1024, 0.1));
        let _ = k;
    }
    rig.pull(4410); // 100 ms çalındı
    rig.ctl.duck();
    rig.w.stop();
    let reported = (9216u64 * 1000 / RATE as u64) as u32;
    // İniş bitmeden: en çok bir zarf boyu daha çalınacak sayılır.
    let early = rig.ctl.audible_position_ms(reported);
    rig.pull(1000);
    let heard_frames = rig.ctl.read.load(SeqCst);
    let heard_ms = (heard_frames * 1000 / RATE as u64) as i64;
    let pos = rig.ctl.audible_position_ms(reported) as i64;
    // (Ms'ye yuvarlamalar: en çok 2 ms.)
    assert!((pos - heard_ms).abs() <= 2, "konum {pos}, duyulan {heard_ms}");
    assert!((early as i64 - heard_ms).abs() <= 2, "erken {early}, duyulan {heard_ms}");
    // Devamda aynı konum.
    rig.w.start();
    assert!((rig.ctl.audible_position_ms(reported) as i64 - heard_ms).abs() <= 2);
    // Flush bekliyor (paused iken sarma): düzeltme yok.
    rig.ctl.flush();
    assert_eq!(rig.ctl.audible_position_ms(5000), 5000);
}

#[test]
fn anchor_predicts_pull_time_of_queued_frames() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&dc(30_000, 0.1));
    // Isınma; aygıta verilen kare sayısı `out`.
    let mut out = 0u64;
    for _ in 0..20 {
        rig.callback();
        out += 441;
    }
    // Gerçek: F. kare, onu içeren geri çağrının anında + çağrı içindeki
    // yeri / fs (sanal çekilme anı; duyulma = bu + D).
    let now = rig.ctl.now_ns();
    let r = rig.ctl.read.load(SeqCst);
    for ahead in [10u64, 500, 1323, 4000] {
        let f = r + ahead;
        let predicted = rig.ctl.pull_ns(f, now) as f64;
        let (mut t, mut o) = (now, out);
        while o + 441 <= f {
            t += 10_000_000;
            o += 441;
        }
        let truth = (t + 10_000_000) as f64 + frames_to_ns((f - o) as f64);
        let err_ms = (predicted - truth) / 1e6;
        // Blok taneciği: ±BLOCK/2 kare = ±0,73 ms.
        assert!(err_ms.abs() < 0.8, "{ahead} kare ileride: hata {err_ms} ms");
    }
}

/// Söz saati: duyulan kare, ses kartının saatinden (bir geri çağrı ve
/// aygıt gecikmesi içinde); boşluksuz geçişte yeni şarkı henüz
/// duyulmuyorsa eksi; sarma sürerken bilinmiyor.
#[test]
fn audible_track_clock_follows_the_device() {
    let mut rig = Rig::new();
    rig.w.start();
    assert_eq!(rig.ctl.audible_track_ms(), None, "şarkı başlamadan bilinmiyor");
    rig.ctl.set_track_base(0);
    rig.write(&dc(30_000, 0.1));
    for _ in 0..20 {
        rig.callback();
    }
    let device_ms = rig.ctl.device_ns as f64 / 1e6;
    let expect = |rig: &Rig, base: f64| {
        let r = rig.ctl.read.load(SeqCst) as f64;
        (r - base) * 1000. / RATE as f64 - device_ms
    };
    let got = rig.ctl.audible_track_ms().expect("saat") as f64;
    assert!((got - expect(&rig, 0.)).abs() <= 12., "duyulan {got}, beklenen {}", expect(&rig, 0.));
    // Boşluksuz geçiş: sıradaki şarkı yazılmaya başladı (30 000. kareden),
    // önceki şarkının sonu hâlâ kuyrukta: yeni şarkının saati eksi.
    rig.ctl.set_track_base(0);
    let got = rig.ctl.audible_track_ms().expect("saat") as f64;
    assert!(got < 0. && (got - expect(&rig, 30_000.)).abs() <= 12., "geçişte {got}");
    // Sarma (flush) sürerken bilinmiyor; yazar onaylayınca da yeni konum
    // gelene dek.
    rig.ctl.flush();
    assert_eq!(rig.ctl.audible_track_ms(), None);
    rig.w.sync();
    assert_eq!(rig.ctl.audible_track_ms(), None);
}

#[test]
fn writer_stall_returns_error_instead_of_hanging() {
    let rig = Rig::new();
    let mut w = rig.w;
    w.start();
    w.sync();
    w.push(&dc(20_000, 0.1)).unwrap();
    let t = Instant::now();
    let r = w.wait();
    assert!(r.is_err());
    assert!(t.elapsed() >= STALL && t.elapsed() < STALL + Duration::from_secs(1));
}

#[test]
fn wait_returns_immediately_when_ducked_or_stopped() {
    let rig = Rig::new();
    let mut w = rig.w;
    w.start();
    w.sync();
    w.push(&dc(20_000, 0.1)).unwrap();
    rig.ctl.duck();
    let t = Instant::now();
    assert!(!w.wait().unwrap());
    rig.ctl.unduck();
    w.stop();
    assert!(!w.wait().unwrap());
    assert!(t.elapsed() < Duration::from_millis(50));
}

// ── Şarkı sonu payı ve boşluksuz geçiş ──────────────────────────────

/// Çaların `wait`i gibi: kuyruk hedefin üstüne çıkana dek 1024 karelik
/// paketler (kimlikli rampa) yazar.
fn fill(rig: &mut Rig, next: &mut u64) {
    while rig.ctl.queued_raw() <= rig.ctl.target_frames() {
        rig.write(&ramp(*next, 1024));
        *next += 1024;
    }
}

/// Önceki şarkı 3 sn çalar (pay açıksa son saniyeleri önden çözülür),
/// `EndOfTrack`, yeni şarkının ilk paketi `stall_ms` gecikir (7 Kasım: disk),
/// sonra 3 sn daha çalınır. Dönüş: xrun sayısı.
fn boundary_stall(tail: bool, stall_ms: u64) -> u64 {
    let mut rig = Rig::new();
    rig.w.start();
    rig.ctl.set_tail(tail);
    let mut next = 0;
    for _ in 0..300 {
        fill(&mut rig, &mut next);
        rig.callback();
    }
    fill(&mut rig, &mut next);
    rig.ctl.begin_handover();
    for _ in 0..stall_ms / 10 {
        rig.callback();
    }
    for _ in 0..300 {
        fill(&mut rig, &mut next);
        rig.callback();
    }
    rig.ctl.xruns()
}

#[test]
fn tail_buffer_hides_a_three_second_stall_at_the_boundary() {
    // Pay açık: 3 sn (ve çıta 3,5 sn) takılma duyulmaz.
    assert_eq!(boundary_stall(true, 3000), 0, "payla 3 sn takılma xrun verdi");
    assert_eq!(boundary_stall(true, 3500), 0, "payla 3,5 sn takılma xrun verdi");
    // Pay yok (eski davranış, 120 ms): aynı takılma kesinti yapar.
    assert!(boundary_stall(false, 3000) >= 1, "paysız 3 sn takılma xrun vermedi");
}

#[test]
fn tail_buffer_returns_to_normal_target_after_new_track_refills() {
    let mut rig = Rig::new();
    rig.w.start();
    let normal = rig.ctl.target_frames();
    rig.ctl.set_tail(true);
    assert_eq!(rig.ctl.target_frames(), ms_to_frames(TAIL_MS));
    let mut next = 0;
    fill(&mut rig, &mut next);
    assert!(rig.ctl.queued_raw() >= ms_to_frames(TAIL_MS), "pay dolmadı");
    // Yeni şarkı (yazar `TrackChanged` + `Playing`'i görünce koyar).
    rig.w.refill = Some((rig.ctl.written.load(SeqCst), rig.ctl.tail_gen.load(SeqCst)));
    for _ in 0..1000 {
        fill(&mut rig, &mut next);
        rig.callback();
        if !rig.ctl.tail.load(SeqCst) {
            break;
        }
    }
    assert!(!rig.ctl.tail.load(SeqCst), "pay kapanmadı");
    assert_eq!(rig.ctl.target_frames(), normal);
    // Kuyruk kendiliğinden erir, kesinti olmadan.
    for _ in 0..500 {
        fill(&mut rig, &mut next);
        rig.callback();
    }
    assert!(rig.ctl.queued_raw() <= normal + 1024, "kuyruk erimedi: {}", rig.ctl.queued_raw());
    assert_eq!(rig.ctl.xruns(), 0);
}

#[test]
fn deep_queue_keeps_user_actions_instant() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.ctl.set_tail(true);
    let mut next = 0;
    fill(&mut rig, &mut next);
    rig.pull(5000);
    let queued = rig.ctl.queued_frames();
    assert!(queued > ms_to_frames(3900), "kuyruk {queued}");
    // Duraklat: kuyruk derin olsa da blok + zarf içinde sessiz, kuyruk korunur.
    rig.ctl.duck();
    let out = rig.pull(2000);
    let silent = out.iter().position(|s| *s == (0.0, 0.0)).unwrap();
    assert!(silent <= BLOCK + FADE_FRAMES as usize, "duraklat {silent} kare sürdü");
    assert!(rig.ctl.queued_frames() + BLOCK as u64 + FADE_FRAMES as u64 >= queued);
    rig.ctl.unduck();
    rig.pull(2000);
    // Ses düzeyi: 10 ms'lik rampayla, kuyruğun önünde değil.
    rig.ctl.set_gain(0.0);
    let out = rig.pull(1000);
    assert!(out[GAIN_RAMP_FRAMES as usize + BLOCK..].iter().all(|s| *s == (0.0, 0.0)), "ses düzeyi gecikti");
    rig.ctl.set_gain(1.0);
    // Sar / şarkı değiştir: 4 sn'lik eski ses beklemeden atılır.
    rig.ctl.flush();
    assert_eq!(rig.ctl.queued_frames(), 0);
    let out = rig.pull(2000);
    assert!(out[BLOCK + FADE_FRAMES as usize..].iter().all(|s| *s == (0.0, 0.0)));
    rig.write(&ramp(5_000_000, 4096));
    let out = rig.pull(3000);
    let first = out.iter().find_map(|s| id_env(*s)).expect("yeni ses çalmadı");
    assert!(first >= 5_000_000, "eski ses çaldı: {first}");
}

/// Sınırın duyulduğu an (`handover_state` → `Audible`), ses kartının
/// geri çağrı modeline göre gerçek andan en çok 3 ms sapar (çapa blok
/// taneciği ~0,7 ms + 1 ms yoklama aralığı; ölçülen 2,1); o ana dek
/// önceki şarkının konumu verilir.
#[test]
fn handover_becomes_audible_when_the_boundary_is_heard() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.ctl.set_tail(true);
    rig.ctl.set_track_base(0);
    let mut next = 0;
    for _ in 0..200 {
        fill(&mut rig, &mut next);
        rig.callback();
    }
    fill(&mut rig, &mut next);
    let boundary = rig.ctl.written.load(SeqCst);
    rig.ctl.begin_handover();
    // Yeni şarkı hemen başlar (takılma yok); çalar tabanı sınıra koyar.
    rig.ctl.set_track_base(0);
    let device_ns = rig.ctl.device_ns as f64;
    let mut truth = None;
    let mut seen = None;
    let mut last_old = 0u32;
    rig.advance_ms(10.0);
    for _ in 0..600 {
        // Geri çağrı: 10 ms'lik dilimin başında 441 kare çekilir; araya
        // 1 ms'lik yoklamalar.
        let t_cb = rig.clock.load(SeqCst) as f64;
        let before = rig.ctl.read.load(SeqCst);
        rig.pull(441);
        fill(&mut rig, &mut next);
        if truth.is_none() && rig.ctl.read.load(SeqCst) > boundary {
            // Bu çağrının F. karesi çağrı anı + (F - ilk) / fs'de verilir
            // (bkz. `anchor_predicts_pull_time_of_queued_frames`), D sonra duyulur.
            truth = Some(t_cb + frames_to_ns((boundary - before) as f64) + device_ns);
        }
        for _ in 0..10 {
            match rig.ctl.handover_state() {
                Handover::Audible { .. } if seen.is_none() => seen = Some(rig.clock.load(SeqCst) as f64),
                Handover::Pending { .. } => {
                    let old = rig.ctl.handover_old_ms(false).expect("önceki şarkının konumu");
                    assert!(old + 1 >= last_old, "önceki şarkının konumu geri gitti");
                    last_old = old;
                }
                _ => {}
            }
            rig.advance_ms(1.0);
        }
        if seen.is_some() {
            break;
        }
    }
    let (truth, seen) = (truth.expect("sınır çekilmedi"), seen.expect("sınır duyulmadı"));
    let err_ms = (seen - truth) / 1e6;
    assert!(err_ms.abs() <= 3.0, "sınır {err_ms:.2} ms sapmayla bildirildi");
    // Önceki şarkının son konumu sınıra (boundary karesi) yakın.
    let end_ms = boundary * 1000 / RATE as u64;
    assert!((last_old as i64 - end_ms as i64).abs() <= 12, "önceki şarkı {last_old} ms, sonu {end_ms} ms");
    // Sarma (flush) geçişi iptal eder.
    rig.ctl.begin_handover();
    rig.ctl.flush();
    assert_eq!(rig.ctl.handover_state(), Handover::None);
}

#[test]
fn stall_hook_sleeps_before_first_packet_of_new_track() {
    let rig = Rig::new();
    let mut w = Writer::with_stall(rig.ctl.clone(), Some(Duration::from_millis(60)));
    w.start();
    w.sync();
    let t = Instant::now();
    w.push(&dc(1024, 0.1)).unwrap();
    assert!(t.elapsed() < Duration::from_millis(30), "şarkı ortasında uyudu");
    w.first_write_pending = true;
    let t = Instant::now();
    w.push(&dc(1024, 0.1)).unwrap();
    assert!(t.elapsed() >= Duration::from_millis(60), "yeni şarkıda uyumadı");
}

// ── Aygıt ───────────────────────────────────────────────────────────

#[test]
fn device_loss_stops_writer_at_once_and_reopen_resumes_exactly() {
    let mut rig = Rig::new();
    rig.w.start();
    for k in 0..20u64 {
        rig.write(&ramp(k * 1024, 1024));
    }
    for _ in 0..12 {
        rig.callback();
    }
    // Akış hatası (Bluetooth kulaklık gitti): yazar 2 sn beklemeden hata.
    rig.ctl.signal_device_lost("sınama");
    let t = Instant::now();
    assert!(rig.w.wait().is_err());
    assert!(t.elapsed() < Duration::from_millis(50), "yazar {:?} bekledi", t.elapsed());
    assert_eq!(rig.ctl.take_device_notice(), Some(DeviceNotice::Lost));
    // Çalar duraklar; aygıt öldü, iniş olmayacak: konum = son çekilen kare.
    rig.w.stop();
    let reported = (19 * 1024 * 1000 / RATE as u64) as u32;
    let read = rig.ctl.read.load(SeqCst);
    let paused = rig.ctl.audible_position_ms(reported);
    assert!((paused as i64 - (read * 1000 / RATE as u64) as i64).abs() <= 1, "duraklatma {paused}");
    // Ölü aygıtın kaynağı artık çağrılmıyor. Çal: o anki varsayılan açılır.
    let mut device = Device::headless(rig.ctl.clone());
    device.ensure_for_start().unwrap();
    assert!(!rig.ctl.is_device_lost());
    let resumed = rig.ctl.audible_position_ms(reported);
    assert_eq!(paused, resumed, "devam konumu duraklatmadan saptı");
    rig.w.start();
    // (Ölü akış yeniden açarken kapandı, eski kaynak bir daha çağrılmaz.)
    // Yeni aygıtın kaynağı tam kaldığı kareden, girişle sürer.
    let mut src = RingSource::new(rig.ctl.clone());
    let out: Vec<(f32, f32)> = (0..1000).map(|_| (src.next().unwrap(), src.next().unwrap())).collect();
    let first = out.iter().find_map(|s| id_env(*s)).expect("çalmadı");
    assert_eq!(first, read, "kaldığı kareden sürmedi");
    assert_eq!(rig.ctl.xruns(), 0);
}

#[test]
fn default_device_switch_hands_over_without_gap_or_overlap() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&ramp(0, 30_000));
    let mut ids: Vec<u64> = rig.pull(5000).into_iter().filter_map(id_env).collect();
    // Yeni aygıtın akışı bekleme kipinde açıldı: halkaya dokunmaz.
    let mut new = RingSource::with_gen(rig.ctl.clone(), 2);
    assert!((0..2000).all(|_| new.next().unwrap() == 0.0));
    let read = rig.ctl.read.load(SeqCst);
    // Geçiş: eski kaynak 8 ms'lik inişle bırakır…
    rig.ctl.source_gen.store(2, SeqCst);
    let fade: Vec<(f32, f32)> = rig.pull(1000);
    assert!(rig.ctl.released_gen.load(SeqCst) >= 1, "eski kaynak bırakmadı");
    let tail = fade.iter().filter(|s| s.1 > 0.0).count();
    assert!(tail <= BLOCK + FADE_FRAMES as usize, "iniş {tail} kare");
    assert!(fade.last().unwrap().1 == 0.0);
    ids.extend(fade.into_iter().filter_map(id_env));
    assert!(rig.ctl.read.load(SeqCst) >= read);
    // …yenisi aynı kareden girişle sürer: atlanan / yinelenen kare yok.
    let out: Vec<(f32, f32)> = (0..3000).map(|_| (new.next().unwrap(), new.next().unwrap())).collect();
    ids.extend(out.iter().filter_map(|s| id_env(*s)));
    assert!(ids.windows(2).all(|p| p[1] == p[0] + 1), "sıra bozuldu");
    assert!(out.iter().any(|s| s.1 == 0.5), "yeni aygıt tam sese çıkmadı");
}

// ── Altın test (PLAN-MOTOR-CONNECT.md §4, 6.0) ──────────────────────

/// FNV-1a 64.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_bits().to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.u64(v as u64);
    }
}

/// xorshift64*: platformdan bağımsız, belirli gürültü (libm yok).
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// Sanal saatli, tek iş parçacıklı senaryo düzeneği: her çekilen kare
/// özete girer.
struct Scn {
    ctl: Arc<OutputCtl>,
    clock: Arc<AtomicU64>,
    src: RingSource,
    w: Writer,
    tx: tmpsc::UnboundedSender<PlayerEvent>,
    noise: Noise,
    h: Fnv,
}

impl Scn {
    fn new(prep: impl FnOnce(&Arc<OutputCtl>)) -> Self {
        let clock = Arc::new(AtomicU64::new(1_000_000_000));
        let ctl = OutputCtl::with_clock(AudioLevels::new(), clock.clone());
        prep(&ctl);
        let (tx, rx) = tmpsc::unbounded_channel();
        ctl.attach_events(rx);
        let src = RingSource::new(ctl.clone());
        let w = Writer::new(ctl.clone());
        Scn { ctl, clock, src, w, tx, noise: Noise(0x9E37_79B9_7F4A_7C15), h: Fnv::new() }
    }

    fn eat(h: &mut Fnv, src: &mut RingSource, frames: usize) {
        for _ in 0..frames * 2 {
            h.f32(src.next().unwrap());
        }
    }

    /// `frames` kare çeker ve özete katar.
    fn pull(&mut self, frames: usize) {
        Self::eat(&mut self.h, &mut self.src, frames);
    }

    /// Geri çağrı: saat 10 ms, 441 kare.
    fn cb(&mut self) {
        self.clock.fetch_add(10_000_000, Relaxed);
        self.pull(441);
    }

    fn write(&mut self, frames: usize) {
        let pk: Vec<f64> = (0..frames * 2).map(|_| self.noise.next() * 0.8).collect();
        self.w.sync();
        if self.w.accept(frames as u64) {
            self.w.push(&pk).unwrap();
        }
    }

    /// Çaların `wait`i gibi: kuyruk hedefin üstüne çıkana dek paket yazar.
    fn fill_to_target(&mut self) {
        while self.ctl.queued_raw() <= self.ctl.target_frames() {
            self.write(1024);
        }
    }

    /// Sayaçlar ve zamanlama değerleri de özete girer (çapa, konum,
    /// hedef): yalnız örnekler değil, zaman hesabı da sabit kalmalı.
    fn state(&mut self) {
        let c = &self.ctl;
        self.h.u64(c.read.load(SeqCst));
        self.h.u64(c.written.load(SeqCst));
        self.h.u64(c.queued_frames());
        self.h.u64(c.xruns());
        self.h.u64(c.underrun_frames.load(Relaxed));
        self.h.u64(c.target_frames());
        self.h.u64(c.played_frames());
        self.h.i64(c.anchor.load(SeqCst));
        self.h.i64(c.audible_offset_ms());
        self.h.i64(c.audible_track_ms().unwrap_or(i64::MIN));
    }
}

/// Altın senaryo: çal, ses rampası, duck / unduck, flush + kapı, xrun,
/// pay + sınır, aygıt değişimi. `prep` çıkış kurulduktan hemen sonra
/// çağrılır (EQ testleri buradan ayar verir). Dönen değer: her çıkan
/// f32'nin bitleri ve bütün zaman / sayaç değerlerinin FNV özeti.
fn golden_scenario(prep: impl FnOnce(&Arc<OutputCtl>)) -> u64 {
    let mut s = Scn::new(prep);

    // 1. Çal: giriş zarfı, kararlı akış.
    s.w.start();
    for _ in 0..6 {
        s.write(1024);
    }
    for _ in 0..8 {
        s.cb();
    }
    s.state();

    // 2. Ses rampası: ortasında yeniden hedef, sessizlik, geri dönüş.
    for _ in 0..4 {
        s.write(1024);
    }
    s.ctl.set_gain(0.3);
    s.pull(300);
    s.ctl.set_gain(0.8);
    s.pull(200);
    s.ctl.set_gain(0.0);
    s.pull(700);
    s.ctl.set_gain(0.65);
    for _ in 0..3 {
        s.cb();
    }
    s.state();

    // 3. duck / unduck, durdur / başlat.
    for _ in 0..4 {
        s.write(1024);
    }
    s.ctl.duck();
    s.pull(700);
    s.state();
    s.ctl.unduck();
    s.pull(1500);
    s.w.stop();
    s.pull(600);
    s.state();
    s.w.start();
    s.pull(900);
    s.state();

    // 4. flush + kapı: eski paketler atılır, işaret + komut sonucu
    // gelince yeni ses akar.
    for _ in 0..3 {
        s.write(1024);
    }
    s.pull(500);
    let mark = s.ctl.flush();
    s.pull(100);
    s.w.sync();
    let written = s.w.written();
    s.write(1024); // kapı kapalı: atılır
    s.tx.send(mark_event(mark)).unwrap();
    s.write(777); // işaret var, komut sonucu yok: atılır
    assert_eq!(s.w.written(), written, "eski paket kuyruğa girdi");
    s.tx.send(PlayerEvent::Seeked { play_request_id: 1, track_id: uri(), position_ms: 12_345 }).unwrap();
    for _ in 0..3 {
        s.write(1024);
    }
    s.pull(3000);
    s.state();

    // 5. xrun: kuyruk boşalır, akış ortasında kesinti; veri gelince say.
    let xruns = s.ctl.xruns();
    let target = s.ctl.target_frames();
    s.pull(9000);
    s.write(1024);
    s.write(2048);
    s.pull(2500);
    assert!(s.ctl.xruns() > xruns, "xrun sayılmadı");
    assert!(s.ctl.target_frames() > target, "hedef büyümedi");
    s.state();

    // 6. Şarkı sonu payı + sınır: pay dolar, sınır duyulur, yeni şarkı
    // payı doldurunca hedef normale döner.
    s.ctl.set_tail(true);
    s.ctl.set_track_base(0);
    for _ in 0..150 {
        s.fill_to_target();
        s.cb();
    }
    s.state();
    s.fill_to_target();
    s.ctl.begin_handover();
    s.w.refill = Some((s.ctl.written.load(SeqCst), s.ctl.tail_gen.load(SeqCst)));
    s.ctl.set_track_base(0);
    let mut audible = None;
    for k in 0..800u32 {
        s.fill_to_target();
        s.cb();
        match s.ctl.handover_state() {
            Handover::Pending { eta } => {
                s.h.u64(eta.as_nanos() as u64);
                s.h.u64(s.ctl.handover_old_ms(false).unwrap_or(u32::MAX) as u64);
            }
            Handover::Audible { new_ms } => {
                if audible.is_none() {
                    audible = Some(k);
                    s.h.u64(new_ms as u64);
                    s.h.u64(k as u64);
                    s.ctl.end_handover();
                }
            }
            Handover::None => {}
        }
        if k % 50 == 0 {
            s.state();
        }
        if audible.is_some() && !s.ctl.tail.load(SeqCst) {
            s.h.u64(k as u64);
            break;
        }
    }
    assert!(audible.is_some(), "sınır duyulmadı");
    assert!(!s.ctl.tail.load(SeqCst), "pay kapanmadı");
    s.state();
    // Sarma geçişi iptal eder.
    s.ctl.begin_handover();
    let mark = s.ctl.flush();
    assert_eq!(s.ctl.handover_state(), Handover::None);
    s.pull(800);
    s.w.sync();
    s.tx.send(mark_event(mark)).unwrap();
    s.tx.send(PlayerEvent::Seeked { play_request_id: 2, track_id: uri(), position_ms: 500 }).unwrap();
    for _ in 0..4 {
        s.write(1024);
    }
    for _ in 0..3 {
        s.cb();
    }
    s.state();

    // 7. Aygıt değişimi: yeni akış bekleme kipinde, eskisi inişle
    // bırakır, yenisi aynı kareden sürer.
    let mut new = RingSource::with_gen(s.ctl.clone(), 2);
    Scn::eat(&mut s.h, &mut new, 500);
    s.ctl.source_gen.store(2, SeqCst);
    s.pull(1000);
    Scn::eat(&mut s.h, &mut new, 3000);
    s.state();

    // 8. Aygıt gitti: yazar hemen hata döner, Çal yeniden açar, yeni
    // kaynak kaldığı kareden sürer.
    s.ctl.signal_device_lost("altın");
    assert!(s.w.wait().is_err());
    s.w.stop();
    s.state();
    let mut device = Device::headless(s.ctl.clone());
    device.ensure_for_start().unwrap();
    s.w.start();
    let mut again = RingSource::new(s.ctl.clone());
    Scn::eat(&mut s.h, &mut again, 1500);
    s.state();
    s.h.0
}

/// Bugünkü çıkışın (EQ yok, geçiş yok) ürettiği her f32'nin FNV özeti.
/// Bu sabit bölme, `fill`'in üç aşamaya ayrılması ve EQ'nun eklenmesi
/// boyunca DEĞİŞMEMELİ: değişirse çıkış bit düzeyinde bozulmuş demektir.
const GOLDEN: u64 = 0x0ee8402968c4d7f3;

#[test]
fn golden_output_is_bit_stable() {
    let got = golden_scenario(|_| {});
    assert_eq!(got, golden_scenario(|_| {}), "senaryo kendi içinde belirli değil");
    assert_eq!(got, GOLDEN, "altın özet değişti: {got:#018x}");
}
/// Ölçüm düzeneği: halka gürültüyle tam dolu, çıkış çalıyor.
fn bench_rig() -> Rig {
    let mut rig = Rig::new();
    rig.w.start();
    let mut noise = Noise(0x1234_5678_9ABC_DEF1);
    let big: Vec<f64> = (0..CAP_FRAMES as usize * 2).map(|_| noise.next() * 0.8).collect();
    rig.w.sync();
    rig.w.push(&big).unwrap();
    rig.pull(2000);
    rig
}

/// `blocks` blok `fill` çağırır, blok başına ns döner. Halka sürekli dolu
/// tutulur (okuma ucundan 200 000 kare ileri).
fn fill_ns(rig: &mut Rig, blocks: usize, fill: fn(&mut RingSource)) -> f64 {
    let t = Instant::now();
    for i in 0..blocks {
        if i % 1024 == 0 {
            let r = rig.ctl.read.load(SeqCst);
            rig.ctl.written.store(r + 200_000, SeqCst);
            rig.clock.fetch_add(10_000_000, Relaxed);
        }
        fill(&mut rig.src);
        std::hint::black_box(&rig.src.buf);
    }
    t.elapsed().as_nanos() as f64 / blocks as f64
}

fn median_min(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[0])
}

/// Dışarıdan ölçüm: EQ kapalıyken `fill` blok başına maliyeti (ns). Çalıştırma:
/// `cargo test --release fill_cost -- --ignored --nocapture`. Referans (6.0,
/// bölme ve üç aşamadan önceki tek parça `fill`, aynı düzenek): ortanca
/// ~240-250 ns/blok; bölmeden sonra EQ kapalı +%1,6 … +%2,8 (çıta ≤ +%5).
#[test]
#[ignore]
fn fill_cost_bench() {
    const BLOCKS: usize = 400_000;
    let mut rig = bench_rig();
    let runs: Vec<f64> = (0..9).map(|_| fill_ns(&mut rig, BLOCKS, RingSource::fill)).collect();
    let (med, min) = median_min(runs);
    println!("fill_cost: ortanca {med:.1} ns/blok (en az {min:.1}; 9 koşu)");
}

// ── Ekolayzır (6.6) ─────────────────────────────────────────────────────

use super::eq::{BANDS, CENTERS_HZ, Eq, EqCtl, GLIDE_BLOCKS, Params};

/// Kapalı ayarlar: kazançlar sıfırdan farklı ama ekolayzır kapalı.
const EQ_GAINS: [f32; BANDS] = [6.0, -3.0, 0.0, 4.5, 2.0, -6.0, 3.0, 9.0, -2.0, 5.0];

fn gains_at(band: usize, db: f32) -> [f32; BANDS] {
    let mut g = [0.0; BANDS];
    g[band] = db;
    g
}

/// `freq` Hz'te genliği `amp` bir sinüsün [`start`, `start + frames`) kareleri
/// (iki kanal aynı), f64 yazar paketi.
fn sine_pcm(freq: f64, amp: f64, start: usize, frames: usize) -> Vec<f64> {
    let w = 2.0 * std::f64::consts::PI * freq / RATE as f64;
    (start..start + frames)
        .flat_map(|n| {
            let s = amp * (w * n as f64).sin();
            [s, s]
        })
        .collect()
}

/// `Eq`yu doğrudan sürer: ayarın oturması için 0,5 sn atılır, sonraki 1 sn'de
/// (giriş enerjisi, çıkış enerjisi, çıkış tepesi) ölçülür.
fn eq_measure(gains: [f32; BANDS], preamp_db: f32, freq: f64, amp: f64) -> (f64, f64, f32) {
    let ctl = EqCtl::new();
    ctl.set(true, &gains, preamp_db);
    let mut eq = Eq::new();
    eq.poll(&ctl);
    let w = 2.0 * std::f64::consts::PI * freq / RATE as f64;
    let (mut pin, mut pout, mut peak) = (0.0f64, 0.0f64, 0.0f32);
    let skip = RATE as usize / 2;
    let total = RATE as usize * 3 / 2;
    let mut n = 0usize;
    while n < total {
        let mut blk = [0f32; BLOCK * CHANNELS];
        for f in 0..BLOCK {
            let s = (amp * (w * (n + f) as f64).sin()) as f32;
            blk[f * 2] = s;
            blk[f * 2 + 1] = s;
        }
        let input = blk;
        if eq.running() {
            eq.process(&mut blk, BLOCK);
        }
        if n >= skip {
            for f in 0..BLOCK {
                pin += (input[f * 2] as f64).powi(2);
                pout += (blk[f * 2] as f64).powi(2);
                peak = peak.max(blk[f * 2].abs()).max(blk[f * 2 + 1].abs());
            }
        }
        n += BLOCK;
    }
    (pin, pout, peak)
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.log10()
}

/// EQ kapalıyken (ayarlar sıfırdan farklı olsa da) çıkış altın senaryoyla bit
/// bit aynı.
#[test]
fn eq_off_is_bit_identical() {
    let got = golden_scenario(|ctl| ctl.set_eq(false, EQ_GAINS, 3.0));
    assert_eq!(got, GOLDEN, "kapalı ekolayzır çıkışı değiştirdi: {got:#018x}");
}

/// Açık ama bütün bantlar 0 dB ve ön kazanç 0 dB: dal hiç çalışmaz, çıkış
/// bit bit aynı.
#[test]
fn all_zero_gains_bypass_exactly() {
    let got = golden_scenario(|ctl| ctl.set_eq(true, [0.0; BANDS], 0.0));
    assert_eq!(got, GOLDEN, "sıfır kazançlı ekolayzır çıkışı değiştirdi: {got:#018x}");
    let ctl = EqCtl::new();
    ctl.set(true, &[0.0; BANDS], 0.0);
    let mut eq = Eq::new();
    eq.poll(&ctl);
    assert!(!eq.running());
    // 0,004 dB de sıfır sayılır (birim katsayı).
    ctl.set(true, &[0.004; BANDS], 0.0);
    eq.poll(&ctl);
    assert!(!eq.running());
}

/// Açıp kapatınca (süzülme bitince) çıkış yeniden bit bit ham × çarpan.
#[test]
fn eq_disabled_after_use_returns_to_exact_passthrough() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&ramp(0, 60_000));
    rig.ctl.set_eq(true, EQ_GAINS, 0.0);
    rig.pull(6016);
    assert!(rig.src.eq.running());
    rig.ctl.set_eq(false, EQ_GAINS, 0.0);
    rig.pull(BLOCK * (GLIDE_BLOCKS as usize + 2));
    assert!(!rig.src.eq.running(), "kapatınca dal durmadı");
    assert!(rig.src.eq.state_is_zero());
    // Bundan sonrası ham kare (ramp kimliği) × 1,0: bit bit.
    let r = rig.ctl.read.load(SeqCst);
    let out = rig.pull(1984);
    for (i, s) in out.iter().enumerate() {
        let frame = r + i as u64;
        assert_eq!(s.0.to_bits(), ((frame as f64 / 1e6) as f32).to_bits(), "kare {frame}");
        assert_eq!(s.1, 0.5);
    }
}

/// Her bantın merkezinde (çok bantlı değil, tek bant, ön kazanç otomatik
/// payı elle geri verilerek) ölçülen kazanç ayarlanan dB'den en çok 0,2 dB
/// sapar.
#[test]
fn band_gain_at_center_within_0_2_db() {
    for band in 0..BANDS {
        for g in [6.0f32, -6.0, 12.0, -12.0] {
            // Otomatik ön kazanç −g (g > 0): elle +g vererek 0 dB'e getir.
            let (pin, pout, _) = eq_measure(gains_at(band, g), g.max(0.0), CENTERS_HZ[band] as f64, 0.1);
            let got = db(pout / pin);
            assert!((got - g as f64).abs() <= 0.2, "bant {band} ({} Hz), {g} dB: ölçülen {got:.3} dB", CENTERS_HZ[band]);
        }
    }
}

/// Tüm yol: halkadan okuyan kaynakta 1 kHz bandı +12 dB (ön kazanç elle
/// +12 ile otomatik payı götürür): çıkış enerjisi EQ kapalıya göre +12 dB.
#[test]
fn eq_is_applied_in_the_signal_path() {
    let run = |on: bool| -> f64 {
        let mut rig = Rig::new();
        if on {
            rig.ctl.set_eq(true, gains_at(5, 12.0), 12.0);
        }
        rig.w.start();
        let mut energy = 0.0f64;
        for k in 0..30 {
            rig.write(&sine_pcm(1000.0, 0.1, k * 1024, 1024));
            for s in rig.callback() {
                if k >= 20 {
                    energy += (s.0 as f64).powi(2);
                }
            }
        }
        energy
    };
    let gain = db(run(true) / run(false));
    assert!((gain - 12.0).abs() <= 0.3, "yol boyunca kazanç {gain:.2} dB");
}

/// Ayar iki iş parçacığı arasında asla yırtık görünmez: yazar bütün
/// değerleri aynı sayıya ayarlar, okur hepsinin aynı olduğunu görmeli.
#[test]
fn coef_update_is_never_torn() {
    const N: u32 = 200_000;
    let ctl = Arc::new(EqCtl::new());
    let w = ctl.clone();
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let writer = thread::spawn(move || {
        for k in 1..=N {
            let v = k as f32;
            let p = Params { coef: [[v; 5]; BANDS], pre: v };
            w.publish(k % 2 == 1, &p);
        }
        d.store(true, SeqCst);
    });
    let (mut reads, mut last) = (0u64, 0u32);
    let mut check = |ctl: &EqCtl| {
        if let Some((seq, enabled, p)) = ctl.read() {
            assert!(seq >= last, "sürüm geri gitti");
            last = seq;
            if seq == 0 {
                return;
            }
            let v = p.pre;
            assert!(p.coef.iter().flatten().all(|x| *x == v), "yırtık okuma: {p:?}");
            assert_eq!(enabled, (v as u32) % 2 == 1, "açık bayrağı başka sürümden");
            reads += 1;
        }
    };
    while !done.load(SeqCst) {
        check(&ctl);
    }
    writer.join().unwrap();
    check(&ctl);
    assert_eq!(last, 2 * N, "son sürüm okunmadı");
    assert!(reads > 0);
}

/// Katsayılar hedefe tam 8 blokta varır; süzülme sırasında çıkış sıçramaz.
#[test]
fn coef_glides_over_eight_blocks_without_a_click() {
    let ctl = EqCtl::new();
    ctl.set(true, &gains_at(5, 6.0), 0.0);
    let mut eq = Eq::new();
    eq.poll(&ctl);
    assert!(eq.running() && !eq.settled());
    // Sabit (DC) giriş: tepe süzgeci DC'de 1 kazançlı, yani yalnız ön
    // kazanç (−6 dB) çıkışı değiştirir; o da kareden kareye süzülür.
    let mut out = Vec::new();
    for blk in 0..GLIDE_BLOCKS + 2 {
        let mut b = [0.5f32; BLOCK * CHANNELS];
        eq.process(&mut b, BLOCK);
        out.extend(b.iter().step_by(2).copied());
        if blk + 1 < GLIDE_BLOCKS {
            assert!(!eq.settled(), "{} blokta oturdu", blk + 1);
        } else {
            assert!(eq.settled(), "{} blokta oturmadı", blk + 1);
        }
    }
    let max_step = out.windows(2).map(|p| (p[1] - p[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.005, "çıkış sıçradı: {max_step}");
    assert!((out.last().unwrap() - 0.25).abs() < 0.01, "son değer {}", out.last().unwrap());
}

/// Kuyruk atılınca (flush) ekolayzırın durumu sıfırlanır: eski sesin
/// çınlaması yeni (sessiz) sese sızmaz.
#[test]
fn eq_state_resets_on_flush() {
    let mut rig = Rig::new();
    rig.ctl.set_eq(true, gains_at(5, 12.0), 12.0);
    rig.w.start();
    rig.write(&sine_pcm(1000.0, 0.5, 0, 8192));
    rig.pull(4000);
    assert!(rig.src.eq.running());
    assert!(!rig.src.eq.state_is_zero(), "durum hiç dolmadı");
    rig.ctl.flush();
    rig.pull(1000);
    // Yazar onaylamadan eski durum durur; onayla birlikte sıfırlanır.
    assert!(!rig.src.eq.state_is_zero());
    rig.w.sync();
    rig.pull(200);
    assert!(rig.src.eq.state_is_zero(), "flush durumu sıfırlamadı");
    // Yeni (sıfır) ses: çınlama olmamalı, tam sıfır.
    rig.write(&dc(4096, 0.0));
    let out = rig.pull(3000);
    assert!(out.iter().all(|s| *s == (0.0, 0.0)), "eski durum yeni sese sızdı");
}

/// Otomatik ön kazanç: bileşik yanıtın gerçek tepesi kadar kısar (en büyük
/// artı banttan, komşu eteklerin payı kadar fazla); tepelerde çıkış 1,0'ı
/// aşmaz. Elle ön kazançla otomatik pay geri verilirse aşardı.
#[test]
fn preamp_prevents_clipping() {
    let mut g = [0.0; BANDS];
    g[4] = 10.0;
    g[9] = 12.0;
    let p = Params::from_gains(&g, 0.0);
    let peak = super::eq::peak_response_db(&g);
    let got_db = 20.0 * (p.pre as f64).log10();
    assert!((got_db + peak).abs() < 1e-3, "otomatik pay {got_db} dB, tepe {peak} dB");
    // Uzak iki bantta tepe en büyük banttan çok az fazladır.
    assert!((12.0..12.2).contains(&peak), "tepe {peak} dB");
    // Artı bant yoksa ön kazanç 1,0 (kısma yok).
    assert_eq!(Params::from_gains(&gains_at(3, -6.0), 0.0).pre, 1.0);
    // Genliği 0,99 sinüs, bütün merkezlerde ve aralarında: çıkış ≤ 1,0.
    let mut worst = 0.0f32;
    let mut freqs: Vec<f64> = CENTERS_HZ.iter().map(|f| *f as f64).collect();
    freqs.extend((0..40).map(|i| 25.0 * (18_000.0f64 / 25.0).powf(i as f64 / 39.0)));
    for f in freqs {
        let (_, _, peak) = eq_measure(g, 0.0, f, 0.99);
        worst = worst.max(peak);
    }
    assert!(worst <= 1.0, "çıkış tepesi {worst}");
    // Ön kazanç payı elle geri verilirse (otomatik pay iptal) aynı giriş
    // keser: payın gerçekten işe yaradığının kanıtı.
    let (_, _, peak) = eq_measure(g, 12.0, CENTERS_HZ[9] as f64, 0.99);
    assert!(peak > 2.0, "paysız tepe {peak}");
}

/// Denormal kesme: dürtü sonrası sessizlikte hiçbir durum denormal kalmaz,
/// sonunda hepsi sıfırlanır.
#[test]
fn eq_flushes_denormals() {
    let ctl = EqCtl::new();
    ctl.set(true, &EQ_GAINS, 0.0);
    let mut eq = Eq::new();
    eq.poll(&ctl);
    let mut b = [0.0f32; BLOCK * CHANNELS];
    b[0] = 1.0;
    b[1] = -1.0;
    eq.process(&mut b, BLOCK);
    for _ in 0..(RATE as usize * 20 / BLOCK) {
        let mut z = [0.0f32; BLOCK * CHANNELS];
        eq.process(&mut z, BLOCK);
        let m = eq.min_nonzero_state();
        assert!(m.is_infinite() || m >= f32::MIN_POSITIVE, "denormal durum {m:e}");
    }
    assert!(eq.state_is_zero(), "durum hiç sıfırlanmadı");
}

/// Ölçüm: ekolayzır 10 bant açıkken blok başına maliyet (çıta < 3 µs).
/// `cargo test --release eq_cost -- --ignored --nocapture`
#[test]
#[ignore]
fn eq_cost_bench() {
    const BLOCKS: usize = 200_000;
    let ctl = EqCtl::new();
    ctl.set(true, &EQ_GAINS, 0.0);
    let mut eq = Eq::new();
    eq.poll(&ctl);
    let mut noise = Noise(0xABCD_EF01_2345_6789);
    let mut blk = [0f32; BLOCK * CHANNELS];
    for v in blk.iter_mut() {
        *v = (noise.next() * 0.5) as f32;
    }
    for _ in 0..GLIDE_BLOCKS + 1 {
        eq.process(&mut blk.clone(), BLOCK);
    }
    let mut runs = Vec::new();
    for _ in 0..9 {
        let t = Instant::now();
        for _ in 0..BLOCKS {
            let mut b = blk;
            eq.process(&mut b, BLOCK);
            std::hint::black_box(&b);
        }
        runs.push(t.elapsed().as_nanos() as f64 / BLOCKS as f64);
    }
    let (med, min) = median_min(runs);
    println!("eq_cost: Eq::process 10 bant {med:.0} ns/blok (en az {min:.0})");

    // Tüm yolda: aynı halka, EQ kapalı / açık dönüşümlü.
    let mut off = bench_rig();
    let mut on = bench_rig();
    on.ctl.set_eq(true, EQ_GAINS, 0.0);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    fill_ns(&mut on, 5_000, RingSource::fill);
    for _ in 0..9 {
        a.push(fill_ns(&mut off, BLOCKS, RingSource::fill));
        b.push(fill_ns(&mut on, BLOCKS, RingSource::fill));
    }
    let (am, an) = median_min(a);
    let (bm, bn) = median_min(b);
    println!("eq_cost: fill EQ kapalı {am:.0} ns/blok (en az {an:.0}); EQ açık {bm:.0} ns/blok (en az {bn:.0}); EQ'nun payı {:.0} ns/blok", bm - am);
    assert!(on.src.eq.running());
}

// ── Karaoke (vokal azaltma) ─────────────────────────────────────────────

use super::karaoke::{GLIDE_BLOCKS as KARAOKE_GLIDE, Karaoke, KaraokeCtl};

/// Karaoke'yu doğrudan sürer (ses iş parçacığının `poll` / `process` düzeni).
/// `ctl` her blok başında okunur; `at(blok, ctl)` ayarı oynatabilir. Giriş
/// (L, R) ve çıkış (L, R) döner.
fn karaoke_drive(
    frames: usize,
    input: impl Fn(usize) -> (f32, f32),
    mut at: impl FnMut(usize, &KaraokeCtl),
) -> (Vec<(f32, f32)>, Vec<(f32, f32)>, Karaoke) {
    let ctl = KaraokeCtl::new();
    let mut k = Karaoke::new();
    let (mut inp, mut out) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    for (b, start) in (0..frames).step_by(BLOCK).enumerate() {
        at(b, &ctl);
        k.poll(&ctl);
        let mut blk = [0f32; BLOCK * CHANNELS];
        for f in 0..BLOCK {
            let (l, r) = input(start + f);
            blk[f * 2] = l;
            blk[f * 2 + 1] = r;
        }
        inp.extend(blk.chunks_exact(2).map(|c| (c[0], c[1])));
        if k.running() {
            k.process(&mut blk, BLOCK);
        }
        out.extend(blk.chunks_exact(2).map(|c| (c[0], c[1])));
    }
    (inp, out, k)
}

/// Bir dizideki `freq` Hz bileşeninin gücü (tek frekanslı DFT; pencere tam
/// sayıda döngü içerirse diğer bileşenler sızmaz).
fn tone_power(x: impl Iterator<Item = f32>, freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / RATE as f64;
    let (mut re, mut im, mut n) = (0.0f64, 0.0f64, 0usize);
    for (i, v) in x.enumerate() {
        re += v as f64 * (w * i as f64).cos();
        im += v as f64 * (w * i as f64).sin();
        n += 1;
    }
    (re * re + im * im) / (n as f64 * n as f64)
}

fn sin_at(freq: f64, amp: f64, n: usize) -> f32 {
    (amp * (2.0 * std::f64::consts::PI * freq * n as f64 / RATE as f64).sin()) as f32
}

/// Ölçüm penceresi: ilk saniye (rampa + süzgeç oturması) atılır, ikinci saniye.
const K_SKIP: usize = RATE as usize;
const K_FRAMES: usize = 2 * RATE as usize;

/// `freq` Hz'te sol kanalın çıkış / giriş gücü (dB), ikinci saniyede.
fn karaoke_change_db(inp: &[(f32, f32)], out: &[(f32, f32)], freq: f64) -> f64 {
    let p_in = tone_power(inp[K_SKIP..K_SKIP + RATE as usize].iter().map(|s| s.0), freq);
    let p_out = tone_power(out[K_SKIP..K_SKIP + RATE as usize].iter().map(|s| s.0), freq);
    db(p_out / p_in)
}

/// Orta: `center_hz` sinüsü; yan: `side_hz` sinüsü (stereo olduğu belli olsun).
/// L = M + S, R = M − S.
fn center_and_side(center_hz: f64, side_hz: f64) -> impl Fn(usize) -> (f32, f32) {
    move |n| {
        let (c, s) = (sin_at(center_hz, 0.25, n), sin_at(side_hz, 0.25, n));
        (c + s, c - s)
    }
}

/// Karaoke kapalıyken (ayar tutulsa da) ve açık ama miktar 0 iken çıkış altın
/// senaryoyla bit bit aynı.
#[test]
fn karaoke_off_is_bit_identical() {
    let got = golden_scenario(|ctl| ctl.set_karaoke(false, 1.0));
    assert_eq!(got, GOLDEN, "kapalı karaoke çıkışı değiştirdi: {got:#018x}");
    let got = golden_scenario(|ctl| ctl.set_karaoke(true, 0.0));
    assert_eq!(got, GOLDEN, "miktarı 0 olan karaoke çıkışı değiştirdi: {got:#018x}");
    // Açıkken altın senaryo (stereo girişte) gerçekten değişir: dal çalışıyor.
    let on = golden_scenario(|ctl| ctl.set_karaoke(true, 1.0));
    assert_ne!(on, GOLDEN, "açık karaoke hiçbir şeyi değiştirmedi");
}

/// Merkezdeki sinüs en az 15 dB azalır (250 Hz – 5 kHz); yan kanala yerleşmiş
/// ayrı bir ton (stereo olduğunu gösterir) 1 dB'den az oynar.
#[test]
fn karaoke_cuts_the_center_and_spares_the_side() {
    for hz in [250.0, 400.0, 1000.0, 3000.0, 5000.0] {
        let (inp, out, k) = karaoke_drive(K_FRAMES, center_and_side(hz, 7000.0), |_, c| c.set(true, 1.0));
        assert!(!k.is_mono());
        let cut = karaoke_change_db(&inp, &out, hz);
        println!("karaoke: merkez {hz:>6} Hz → {cut:+.1} dB");
        assert!(cut <= -15.0, "{hz} Hz merkez yalnız {cut:.1} dB azaldı");
        let side = karaoke_change_db(&inp, &out, 7000.0);
        assert!(side.abs() <= 1.0, "{hz} Hz merkezle yan ton {side:+.2} dB oynadı");
    }
}

/// Yalnız yan kanalda olan ton (L = −R) 1 dB'den az değişir; merkezde başka bir
/// ton varken de.
#[test]
fn karaoke_leaves_pure_side_tones_alone() {
    for hz in [250.0, 1000.0, 5000.0] {
        let side_only = move |n: usize| {
            let s = sin_at(hz, 0.3, n);
            (s, -s)
        };
        let (inp, out, _) = karaoke_drive(K_FRAMES, side_only, |_, c| c.set(true, 1.0));
        let d = karaoke_change_db(&inp, &out, hz);
        assert!(d.abs() <= 1.0, "yalnız yan {hz} Hz {d:+.2} dB oynadı");
        let (inp, out, _) = karaoke_drive(K_FRAMES, center_and_side(440.0, hz), |_, c| c.set(true, 1.0));
        let d = karaoke_change_db(&inp, &out, hz);
        assert!(d.abs() <= 1.0, "orta 440 Hz varken yan {hz} Hz {d:+.2} dB oynadı");
    }
}

/// Bas (150 Hz altı) ortada da korunur: 40 / 60 / 80 Hz en çok 1 dB oynar.
#[test]
fn karaoke_keeps_the_bass() {
    for hz in [40.0, 60.0, 80.0] {
        let (inp, out, _) = karaoke_drive(K_FRAMES, center_and_side(hz, 7000.0), |_, c| c.set(true, 1.0));
        let d = karaoke_change_db(&inp, &out, hz);
        println!("karaoke: bas {hz:>4} Hz → {d:+.2} dB");
        assert!(d.abs() <= 1.0, "{hz} Hz bas {d:+.2} dB oynadı");
    }
}

/// Miktar orantılıdır: yarı miktar merkezi yaklaşık yarı genlikte (−6 dB) bırakır.
#[test]
fn karaoke_amount_scales_the_cut() {
    let (inp, out, _) = karaoke_drive(K_FRAMES, center_and_side(1000.0, 7000.0), |_, c| c.set(true, 0.5));
    let d = karaoke_change_db(&inp, &out, 1000.0);
    assert!((d + 6.0).abs() <= 0.5, "yarı miktar {d:.2} dB (beklenen ≈ −6)");
}

/// Mono kaynak (L = R) hiç değişmez: bit bit aynı, karışım hiç yükselmez.
#[test]
fn karaoke_does_nothing_on_a_mono_source() {
    let mono = |n: usize| {
        let s = sin_at(300.0, 0.2, n) + sin_at(1800.0, 0.2, n) + sin_at(90.0, 0.1, n);
        (s, s)
    };
    let (inp, out, mut k) = karaoke_drive(K_FRAMES, mono, |_, c| c.set(true, 1.0));
    assert_eq!(inp, out, "mono kaynak değişti");
    assert!(k.is_mono());
    assert_eq!(k.engage(), 0.0);
    // Mono bir kaynağın durumu bayat kalmaz.
    assert!(k.state_is_zero());
}

/// Karar eşiği: yan / orta −50 dB (neredeyse mono) mono sayılır, −10 dB stereo.
#[test]
fn karaoke_mono_detection_follows_the_side_level() {
    let mix = |side_amp: f64| {
        move |n: usize| {
            let (c, s) = (sin_at(500.0, 0.3, n), sin_at(3100.0, side_amp, n));
            (c + s, c - s)
        }
    };
    let (_, _, k) = karaoke_drive(K_SKIP, mix(0.001), |_, c| c.set(true, 1.0));
    assert!(k.is_mono(), "−50 dB yan kanal mono sayılmadı");
    let (inp, out, k) = karaoke_drive(K_SKIP, mix(0.1), |_, c| c.set(true, 1.0));
    assert!(!k.is_mono(), "−10 dB yan kanal mono sayıldı");
    assert!(k.engage() > 0.99);
    assert_ne!(inp[K_SKIP - 1], out[K_SKIP - 1]);
    // Mono başlayıp stereo olan kaynakta karaoke sonradan devreye girer.
    let switch = |n: usize| {
        let c = sin_at(500.0, 0.3, n);
        let s = if n >= K_SKIP / 2 { sin_at(3100.0, 0.2, n) } else { 0.0 };
        (c + s, c - s)
    };
    let (_, _, k) = karaoke_drive(K_SKIP * 3 / 2, switch, |_, c| c.set(true, 1.0));
    assert!(!k.is_mono() && k.engage() > 0.99);
}

/// Boşluksuz geçişte stereo şarkıdan sayısal mono şarkıya geçilince (akış
/// kesilmeden, `reset` yok) karaoke birkaç blokta çekilir ve çıkış yeniden
/// girişle bit bit aynı olur (yavaş sızıntılı ölçüm ~1 sn beklemez).
#[test]
fn karaoke_gives_way_quickly_when_a_mono_track_follows_a_stereo_one() {
    let cut = 2 * K_SKIP;
    let input = move |n: usize| {
        if n < cut {
            center_and_side(500.0, 3100.0)(n)
        } else {
            let m = sin_at(500.0, 0.25, n) + sin_at(1700.0, 0.2, n);
            (m, m)
        }
    };
    let (inp, out, k) = karaoke_drive(cut + RATE as usize / 2, input, |_, c| c.set(true, 1.0));
    assert!(k.is_mono());
    assert_eq!(k.engage(), 0.0, "mono şarkıda karışım inmedi");
    // Geçişten 100 ms sonra çıkış girişin aynısı.
    let from = cut + RATE as usize / 10;
    assert_eq!(inp[from..], out[from..]);
}

/// Geçiş sıçramasız: açma, miktar değişimi, kapama sırasında ardışık örnekler
/// arasındaki en büyük fark, girişin kendi (kuramsal) en büyük farkını aşmaz;
/// ani bir basamak (tıkırtı) bunun kat kat üstünde olurdu.
#[test]
fn karaoke_transitions_do_not_click() {
    // Düşük frekanslar: girişin kendi örnek farkı küçük (≈ 0,03), bir basamak
    // (tıkırtı) onun çok üstünde kalır (karaoke açılırken sönen orta ton 0,2).
    let tones = [(300.0, 0.2), (450.0, 0.2), (200.0, 0.2)];
    let input = move |n: usize| {
        let c = sin_at(tones[0].0, tones[0].1, n) + sin_at(tones[1].0, tones[1].1, n);
        let s = sin_at(tones[2].0, tones[2].1, n);
        (c + s, c - s)
    };
    // Kuramsal en büyük örnek farkı: Σ genlik · 2π f / fs.
    let bound: f64 = tones.iter().map(|(f, a)| a * 2.0 * std::f64::consts::PI * f / RATE as f64).sum();
    let blocks_per_s = RATE as usize / BLOCK;
    let script = move |b: usize, c: &KaraokeCtl| match b {
        x if x == blocks_per_s / 2 => c.set(true, 1.0),
        x if x == blocks_per_s * 3 / 2 => c.set(true, 0.3),
        x if x == blocks_per_s * 5 / 2 => c.set(false, 0.3),
        x if x == blocks_per_s * 7 / 2 => c.set(true, 0.7),
        x if x == blocks_per_s * 7 / 2 + 10 => c.set(true, 0.0),
        _ => {}
    };
    let (_, out, mut k) = karaoke_drive(RATE as usize * 5, input, script);
    let mut worst = 0.0f32;
    for w in out.windows(2) {
        worst = worst.max((w[1].0 - w[0].0).abs()).max((w[1].1 - w[0].1).abs());
    }
    println!("karaoke geçiş: en büyük örnek farkı {worst:.5} (girişin kuramsal sınırı {bound:.5})");
    assert!(f64::from(worst) <= bound * 1.05 + 0.002, "geçişte sıçrama: {worst} > {bound}");
    // Duyarlılık: giriş eğimi küçük olduğundan anlık bir kuru→ıslak geçiş bu sınırı aşardı.
    assert!(bound < 0.04, "test girişinin eğimi büyümüş: {bound}");
    // Miktar 0'a inince dal durdu (kuru, durum sıfır).
    assert!(!k.running() && k.state_is_zero());
}

/// Açma rampası gerçekten rampa: `KARAOKE_GLIDE` blokta tam ıslak.
#[test]
fn karaoke_engages_over_the_glide() {
    let (_, _, k) = karaoke_drive(BLOCK * (KARAOKE_GLIDE as usize / 2), center_and_side(1000.0, 7000.0), |_, c| c.set(true, 1.0));
    assert!(k.engage() > 0.4 && k.engage() < 0.6, "yarı yolda karışım {}", k.engage());
    let (_, _, k) = karaoke_drive(BLOCK * (KARAOKE_GLIDE as usize + 1), center_and_side(1000.0, 7000.0), |_, c| c.set(true, 1.0));
    assert_eq!(k.engage(), 1.0);
}

/// Kuyruk atılınca (flush) karaoke durumu sıfırlanır: eski sesin çınlaması yeni
/// (sessiz) sese sızmaz.
#[test]
fn karaoke_state_resets_on_flush() {
    let mut rig = Rig::new();
    rig.ctl.set_karaoke(true, 1.0);
    rig.w.start();
    let stereo: Vec<f64> = (0..8192)
        .flat_map(|n| {
            let (c, s) = (sin_at(1000.0, 0.25, n) as f64, sin_at(7000.0, 0.25, n) as f64);
            [c + s, c - s]
        })
        .collect();
    rig.write(&stereo);
    rig.pull(4000);
    assert!(rig.src.karaoke.running());
    assert!(!rig.src.karaoke.state_is_zero(), "durum hiç dolmadı");
    rig.ctl.flush();
    rig.pull(1000);
    assert!(!rig.src.karaoke.state_is_zero());
    rig.w.sync();
    rig.pull(200);
    assert!(rig.src.karaoke.state_is_zero(), "flush durumu sıfırlamadı");
    rig.write(&dc(4096, 0.0));
    let out = rig.pull(3000);
    assert!(out.iter().all(|s| *s == (0.0, 0.0)), "eski durum yeni sese sızdı");
}

/// Tüm yol: halkadan okuyan kaynakta karaoke açılıp kapanınca, kapanma bitince
/// çıkış yeniden bit bit ham × çarpan (dal durur).
#[test]
fn karaoke_disabled_after_use_returns_to_exact_passthrough() {
    let mut rig = Rig::new();
    rig.w.start();
    rig.write(&ramp(0, 60_000));
    rig.ctl.set_karaoke(true, 1.0);
    rig.pull(6016);
    assert!(rig.src.karaoke.running());
    rig.ctl.set_karaoke(false, 1.0);
    rig.pull(BLOCK * (KARAOKE_GLIDE as usize + 2));
    assert!(!rig.src.karaoke.running(), "kapatınca dal durmadı");
    assert!(rig.src.karaoke.state_is_zero());
    let r = rig.ctl.read.load(SeqCst);
    let out = rig.pull(1984);
    for (i, s) in out.iter().enumerate() {
        let frame = r + i as u64;
        assert_eq!(s.0.to_bits(), ((frame as f64 / 1e6) as f32).to_bits(), "kare {frame}");
        assert_eq!(s.1, 0.5);
    }
}

/// Ayar sonlu olmayan ve sınır dışı değerlerde güvenli: derinlik 0..1.
#[test]
fn karaoke_setting_is_sanitised() {
    let ctl = KaraokeCtl::new();
    let mut k = Karaoke::new();
    for (enabled, amount, running) in [(true, f32::NAN, false), (true, -4.0, false), (true, 9.0, true), (false, 1.0, false)] {
        ctl.set(enabled, amount);
        k.poll(&ctl);
        // Önceki çağrıdan kalan karışım yok (process çağrılmadı): yalnız derinlik.
        assert_eq!(k.running(), running, "{enabled} {amount}");
    }
}

/// Ölçüm: `Karaoke::process` ve tam yolda (`fill`) karaoke açıkken blok başına
/// maliyet; EQ ölçümüyle aynı yöntem (aynı halka, kapalı / açık dönüşümlü,
/// ortanca). `cargo test --release karaoke_cost -- --ignored --nocapture`
#[test]
#[ignore]
fn karaoke_cost_bench() {
    const BLOCKS: usize = 200_000;
    let ctl = KaraokeCtl::new();
    ctl.set(true, 1.0);
    let mut k = Karaoke::new();
    k.poll(&ctl);
    let mut noise = Noise(0xABCD_EF01_2345_6789);
    let mut blk = [0f32; BLOCK * CHANNELS];
    for v in blk.iter_mut() {
        *v = (noise.next() * 0.5) as f32;
    }
    for _ in 0..KARAOKE_GLIDE + 1 {
        k.process(&mut blk.clone(), BLOCK);
    }
    let mut runs = Vec::new();
    for _ in 0..9 {
        let t = Instant::now();
        for _ in 0..BLOCKS {
            let mut b = blk;
            k.process(&mut b, BLOCK);
            std::hint::black_box(&b);
        }
        runs.push(t.elapsed().as_nanos() as f64 / BLOCKS as f64);
    }
    let (med, min) = median_min(runs);
    println!("karaoke_cost: Karaoke::process {med:.0} ns/blok (en az {min:.0})");

    // Tüm yolda: aynı halka, karaoke kapalı / açık dönüşümlü. Halka gürültü
    // dolu (stereo): açıkken mono sayılmaz, tam ıslak çalışır.
    let mut off = bench_rig();
    let mut on = bench_rig();
    on.ctl.set_karaoke(true, 1.0);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    fill_ns(&mut on, 5_000, RingSource::fill);
    for _ in 0..9 {
        a.push(fill_ns(&mut off, BLOCKS, RingSource::fill));
        b.push(fill_ns(&mut on, BLOCKS, RingSource::fill));
    }
    let (am, an) = median_min(a);
    let (bm, bn) = median_min(b);
    println!(
        "karaoke_cost: fill karaoke kapalı {am:.0} ns/blok (en az {an:.0}); açık {bm:.0} ns/blok (en az {bn:.0}); karaoke'nin payı {:.0} ns/blok",
        bm - am
    );
    assert!(on.src.karaoke.running() && on.src.karaoke.engage() > 0.99, "dal tam ıslak çalışmadı");
}
