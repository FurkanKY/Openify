//! Geçiş (crossfade) ve büyüyebilen halka testleri: aygıtsız, sanal saatli. Düzenek, çaldan gelen olayları
//! (`EndOfTrack`, `TrackChanged`, `Playing`) yazara gerçek kanaldan verir; ses
//! kartı yerine `RingSource` çekilir. Altın test (`tests.rs`) bindirmesiz
//! çıkışın bit düzeyinde değişmediğini ayrıca doğrular.

use std::{
    cell::Cell,
    path::PathBuf,
    sync::{Arc, atomic::AtomicU64},
};

use librespot::{
    core::{SpotifyId, SpotifyUri},
    metadata::audio::{AudioFiles, AudioItem, UniqueFields},
    playback::player::PlayerEvent,
};
use tokio::sync::mpsc as tmpsc;

use super::*;
use crate::engine::levels::AudioLevels;

// ── Sayım yapan ayırıcı: ses iş parçacığında bellek ayırma yok ──────────

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

/// Yalnız sayaç açıkken (bu iş parçacığında) ayırma / yeniden ayırma sayar;
/// ayırmayı `System`e bırakır. `mem-stats` özelliği kendi ayırıcısını kurar.
#[cfg(not(feature = "mem-stats"))]
struct CountingAlloc;

#[cfg(not(feature = "mem-stats"))]
unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        note_alloc(layout.size());
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        note_alloc(layout.size());
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        note_alloc(new_size);
        unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
    }
}

#[cfg(not(feature = "mem-stats"))]
#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

#[cfg(not(feature = "mem-stats"))]
fn note_alloc(bytes: usize) {
    let _ = COUNTING.try_with(|on| {
        if on.get() {
            let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
            let _ = BYTES.try_with(|n| n.set(n.get() + bytes as u64));
        }
    });
}

/// `f` çalışırken bu iş parçacığındaki ayırma sayısı ve toplam bayt.
#[cfg(not(feature = "mem-stats"))]
fn alloc_stats_during(f: impl FnOnce()) -> (u64, u64) {
    ALLOCS.with(|n| n.set(0));
    BYTES.with(|n| n.set(0));
    COUNTING.with(|on| on.set(true));
    f();
    COUNTING.with(|on| on.set(false));
    (ALLOCS.with(|n| n.get()), BYTES.with(|n| n.get()))
}

/// `f` çalışırken bu iş parçacığındaki ayırma sayısı.
#[cfg(not(feature = "mem-stats"))]
fn allocs_during(f: impl FnOnce()) -> u64 {
    alloc_stats_during(f).0
}

// ── Düzenek ─────────────────────────────────────────────────────────────

fn track_uri() -> SpotifyUri {
    SpotifyUri::Track {
        id: SpotifyId::from_base62("0000000000000000000000").unwrap(),
    }
}

fn audio_item(duration_ms: u32) -> Box<AudioItem> {
    Box::new(AudioItem {
        track_id: track_uri(),
        uri: "spotify:track:0000000000000000000000".into(),
        files: AudioFiles::default(),
        name: "test".into(),
        covers: vec![],
        language: vec![],
        duration_ms,
        is_explicit: false,
        availability: Ok(()),
        alternatives: None,
        unique_fields: UniqueFields::Local {
            artists: None,
            album: None,
            album_artists: None,
            number: None,
            disc_number: None,
            path: PathBuf::new(),
        },
    })
}

const A: f64 = 0.25;
const B: f64 = 0.5;
/// Her şarkı sesi sabit (DC): karışım değeri kareden kareye kesin hesaplanır.
const PACKET: usize = 1024;

struct X {
    ctl: Arc<OutputCtl>,
    clock: Arc<AtomicU64>,
    src: RingSource,
    w: Writer,
    tx: tmpsc::UnboundedSender<PlayerEvent>,
    /// Çekilen karelerin sol kanalı, çekilme sırasıyla (aygıt değişimi ve
    /// duraklatma olmayan testlerde dizin = halka kare numarası).
    out: Vec<f32>,
    /// Çekme (ses iş parçacığı yolu) sırasında sayılan bellek ayırma.
    pull_allocs: u64,
}

impl X {
    fn new(crossfade_ms: u32) -> Self {
        let clock = Arc::new(AtomicU64::new(1_000_000_000));
        let ctl = OutputCtl::with_clock_for(AudioLevels::new(), clock.clone(), crossfade_ms);
        let (tx, rx) = tmpsc::unbounded_channel();
        ctl.attach_events(rx);
        let src = RingSource::new(ctl.clone());
        // Ortam değişkeni (`OPENIFY_TEST_TRANSITION_STALL_MS`) gerçek uyku
        // ekler, sanal saati ilerletmez: takılma burada sanal (`stall`).
        let w = Writer::with_stall(ctl.clone(), None);
        X {
            ctl,
            clock,
            src,
            w,
            tx,
            out: Vec::new(),
            pull_allocs: 0,
        }
    }

    fn advance_ms(&self, ms: u64) {
        self.clock
            .fetch_add(ms * 1_000_000, std::sync::atomic::Ordering::Relaxed);
    }

    /// Gerçek geri çağrı gibi: saat 10 ms, 441 kare çek.
    fn cb(&mut self) {
        self.advance_ms(10);
        self.pull(441);
    }

    fn pull(&mut self, frames: usize) {
        // `out.push` testin kendi ayırması; yalnız `RingSource::next` sayılır.
        let mut pulled = Vec::with_capacity(frames);
        #[cfg(not(feature = "mem-stats"))]
        {
            self.pull_allocs += allocs_during(|| {
                for _ in 0..frames {
                    let l = self.src.next().unwrap();
                    self.src.next().unwrap();
                    pulled.push(l);
                }
            });
        }
        #[cfg(feature = "mem-stats")]
        for _ in 0..frames {
            let l = self.src.next().unwrap();
            self.src.next().unwrap();
            pulled.push(l);
        }
        self.out.extend(pulled);
    }

    fn ev(&self, event: PlayerEvent) {
        self.tx.send(event).unwrap();
    }

    /// `TrackChanged` + `Playing { 0 }`: yeni şarkı başladı.
    fn track_started(&self, duration_ms: u32) {
        self.ev(PlayerEvent::TrackChanged {
            audio_item: audio_item(duration_ms),
        });
        self.ev(PlayerEvent::Playing {
            play_request_id: 1,
            track_id: track_uri(),
            position_ms: 0,
        });
    }

    fn end_of_track(&self) {
        self.ev(PlayerEvent::EndOfTrack {
            play_request_id: 1,
            track_id: track_uri(),
        });
    }

    /// `tap::LevelSink::write` sırası: eşitle, kapı, ilk paket işi, yaz.
    fn write_dc(&mut self, v: f64, frames: usize) {
        let samples = vec![v; frames * 2];
        self.w.sync();
        if !self.w.accept(frames as u64) {
            return;
        }
        self.w.begin_packet();
        self.w.push(&samples).unwrap();
    }

    /// Çaların `wait`i gibi: kuyruk hedefin üstüne çıkana dek paket yazar
    /// (en çok `frames` kare).
    fn fill_dc(&mut self, v: f64, frames: usize) -> usize {
        let mut done = 0;
        while done < frames && self.ctl.queued_raw() <= self.ctl.target_frames() {
            let n = PACKET.min(frames - done);
            self.write_dc(v, n);
            done += n;
        }
        done
    }

    /// Giden şarkı (`A`): `secs` sn'lik sesi şarkı sonu payıyla yazar, sanal
    /// ses kartı çeker. Dönen: şarkının son karesinin ucu (`eot`).
    fn play_a(&mut self, secs: usize, spec: TransitionSpec) -> u64 {
        self.w.start();
        self.track_started(secs as u32 * 1000);
        // A'nın başlangıç olayları, pay açılmadan işlensin (gerçekte `PreloadDue`
        // şarkının sonuna yakın gelir; yoksa `refill` payı hemen kapatırdı).
        self.write_dc(A, PACKET);
        // Motorun `PreloadDue` yolu: önce istek, sonra pay.
        self.ctl.arm_transition(spec);
        self.ctl.set_tail(true);
        let total = secs * RATE as usize;
        let mut written = PACKET;
        while written < total {
            written += self.fill_dc(A, total - written);
            self.cb();
        }
        self.ctl.written.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn spec_ms(ms: u32) -> TransitionSpec {
    TransitionSpec::from_ms(ms, Curve::EqualPower)
}

fn ld(c: &AtomicU64) -> u64 {
    c.load(std::sync::atomic::Ordering::SeqCst)
}

/// A'nın sonu, `stall_ms` takılma, B: doğal geçiş. B'nin `b_secs` sn'sini
/// yazar ve her şeyi çeker. Dönen düzenek `out`ta bütün çıkışı taşır.
fn natural(
    crossfade_ms: u32,
    a_secs: usize,
    b_secs: usize,
    stall_ms: u64,
    spec: TransitionSpec,
) -> (X, u64) {
    let mut x = X::new(crossfade_ms);
    let eot = x.play_a(a_secs, spec);
    // Motor, şarkı bitince: sınır karesini al, sıradakini yükle (istek yeniden).
    x.ctl.begin_handover();
    x.end_of_track();
    x.ctl.arm_transition(spec);
    for _ in 0..stall_ms / 10 {
        x.cb();
    }
    x.track_started(b_secs as u32 * 1000);
    let total = b_secs * RATE as usize;
    let mut written = 0;
    while written < total {
        written += x.fill_dc(B, total - written);
        x.cb();
    }
    // Bindirme ve B'nin başı çalınsın (kuyruk boşalmadan: xrun olmasın).
    for _ in 0..10_000 {
        if ld(&x.ctl.read) >= eot + 150_000 {
            break;
        }
        x.cb();
    }
    (x, eot)
}

// ── Eğriler ─────────────────────────────────────────────────────────────

#[test]
fn equal_power_gains_sum_to_unit_power_at_every_frame() {
    // 5 sn'lik bindirmenin her karesi: giden² + gelen² = 1.
    let n = 220_500u64;
    for k in (0..n).step_by(7) {
        let (out, inn) = Curve::EqualPower.gains((k as f64 + 0.5) / n as f64);
        let power = out as f64 * out as f64 + inn as f64 * inn as f64;
        assert!((power - 1.0).abs() < 1e-6, "kare {k}: {power}");
        assert!(out <= 1.0 && inn <= 1.0 && out >= 0.0 && inn >= 0.0);
    }
    // Uçlar: giden 1 → 0, gelen 0 → 1; ortada ikisi de −3 dB.
    let (out0, in0) = Curve::EqualPower.gains(0.0);
    let (out1, in1) = Curve::EqualPower.gains(1.0);
    assert!((out0 - 1.0).abs() < 1e-7 && in0.abs() < 1e-7);
    assert!(out1.abs() < 1e-7 && (in1 - 1.0).abs() < 1e-7);
    let (mid_out, mid_in) = Curve::EqualPower.gains(0.5);
    assert!((mid_out - mid_in).abs() < 1e-6 && (mid_out as f64 - 0.5f64.sqrt()).abs() < 1e-6);
}

#[test]
fn linear_gains_sum_to_one_and_lut_follows_equal_power() {
    for k in 0..=1000 {
        let t = k as f64 / 1000.0;
        let (out, inn) = Curve::Linear.gains(t);
        assert!((out + inn - 1.0).abs() < 1e-6);
        let (lo, li) = Curve::Lut.gains(t);
        let (eo, ei) = Curve::EqualPower.gains(t);
        assert!(
            (lo - eo).abs() < 1e-4 && (li - ei).abs() < 1e-4,
            "t={t}: lut {lo}/{li}, eşit güç {eo}/{ei}"
        );
        let power = lo as f64 * lo as f64 + li as f64 * li as f64;
        assert!((power - 1.0).abs() < 2e-3, "t={t}: güç {power}");
    }
}

#[test]
fn transition_is_one_atomic_with_44_bit_start_and_20_bit_overlap() {
    let t = Transition {
        start: (1 << 44) - 1,
        overlap: (1 << 20) - 1,
    };
    assert_eq!(Transition::unpack(t.pack()), Some(t));
    let t = Transition {
        start: 123_456_789,
        overlap: 529_200,
    };
    assert_eq!(Transition::unpack(t.pack()), Some(t));
    // `overlap` 0: geçiş yok.
    assert_eq!(
        Transition::unpack(
            Transition {
                start: 99,
                overlap: 0
            }
            .pack()
        ),
        None
    );
    assert_eq!(Transition::unpack(0), None);
    // 12 sn bindirme 20 bite sığar.
    assert!(TransitionSpec::from_ms(MAX_OVERLAP_MS, Curve::EqualPower).overlap < (1 << 20));
    for curve in [Curve::Linear, Curve::EqualPower, Curve::Lut] {
        let spec = TransitionSpec::from_ms(7_000, curve);
        assert_eq!(TransitionSpec::unpack(spec.pack()), spec);
    }
    assert!(TransitionSpec::NONE.is_none());
    // Üst sınırın ötesi kırpılır.
    assert_eq!(
        TransitionSpec::from_ms(60_000, Curve::Linear),
        TransitionSpec::from_ms(MAX_OVERLAP_MS, Curve::Linear)
    );
}

#[test]
fn ring_capacity_follows_the_setting() {
    assert_eq!(ring_frames(0), CAP_FRAMES, "kapalı: bugünkü halka (2 MiB)");
    assert_eq!(
        ring_frames(1_000),
        CAP_FRAMES,
        "1 sn bindirme mevcut halkaya sığar"
    );
    assert_eq!(ring_frames(2_000), 1 << 19);
    assert_eq!(ring_frames(5_000), 1 << 19);
    assert_eq!(ring_frames(12_000), 1 << 20, "12 sn: 8 MiB");
    assert_eq!(ring_frames(99_000), 1 << 20);
    // Halka her boyda 12 sn + 4 sn pay + paket payını taşır.
    assert!(ring_frames(12_000) >= ms_to_frames(12_000 + TAIL_MS) + RING_SLACK);
    // Kapalıyken ve kurulumda doğru boyda kurulunca ikinci yuva ayrılmaz.
    let ctl = OutputCtl::with_clock_for(AudioLevels::new(), Arc::new(AtomicU64::new(1)), 12_000);
    assert_eq!(ctl.ring.frames(), 1 << 20);
    ctl.reserve(ring_frames(12_000));
    assert!(
        ctl.big.get().is_none(),
        "halka zaten yeterli: ikinci yuva yok"
    );
    let small = OutputCtl::with_clock(AudioLevels::new(), Arc::new(AtomicU64::new(1)));
    small.reserve(ring_frames(0));
    assert!(small.big.get().is_none(), "geçiş kapalı: bir şey ayrılmaz");
}

// ── Doğal geçiş: bindirme ───────────────────────────────────────────────

/// Bindirme tam eşit güç eğrisinden: giden `A`, gelen `B`, aralarında tam
/// `A·out(t) + B·in(t)`; bindirmeden önce yalnız A, sonra yalnız B; arayüzün
/// sınırı ve söz saati yeni şarkının BAŞLANGICINA (`start`) çekilmiş.
#[test]
fn overlap_mixes_equal_power_between_the_end_of_a_and_the_start_of_b() {
    let spec = spec_ms(5_000);
    let (x, eot) = natural(5_000, 20, 12, 0, spec);
    assert_eq!(x.ctl.xruns(), 0);
    let overlap = spec.overlap as u64;
    // `transition()` bindirme çalarken dolu, bitince boş; başlangıç `eot − overlap`.
    let t = Transition {
        start: eot - overlap,
        overlap: overlap as u32,
    };
    let (a32, b32) = (A as f32, B as f32);
    let skip = (FADE_FRAMES as usize) + BLOCK;
    for f in skip..t.start as usize {
        assert_eq!(x.out[f], a32, "kare {f}: bindirmeden önce yalnız A");
    }
    for k in 0..overlap {
        let (out, inn) = Curve::EqualPower.gains((k as f64 + 0.5) / overlap as f64);
        let f = (t.start + k) as usize;
        assert_eq!(x.out[f], a32 * out + b32 * inn, "kare {f} (k={k})");
    }
    let tail = (eot as usize + 100)..x.out.len().min(eot as usize + 400_000);
    assert!(!tail.is_empty());
    for f in tail {
        assert_eq!(x.out[f], b32, "kare {f}: bindirmeden sonra yalnız B");
    }
    // Değişmez: yazar okuma ucunun GUARD ötesinden öncesine dokunmadı.
    let lead = x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        lead >= GUARD_FRAMES,
        "en yakın değişiklik okumanın {lead} kare önünde (< {GUARD_FRAMES})"
    );
}

#[test]
fn twelve_second_overlap_fits_the_grown_ring_and_stays_equal_power() {
    let spec = spec_ms(12_000);
    let (x, eot) = natural(12_000, 40, 30, 0, spec);
    assert_eq!(x.ctl.xruns(), 0);
    assert_eq!(x.ctl.ring.frames(), 1 << 20);
    let overlap = spec.overlap as u64;
    let start = eot - overlap;
    for k in (0..overlap).step_by(997) {
        let (out, inn) = Curve::EqualPower.gains((k as f64 + 0.5) / overlap as f64);
        assert_eq!(
            x.out[(start + k) as usize],
            A as f32 * out + B as f32 * inn,
            "k={k}"
        );
    }
    assert!(x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed) >= GUARD_FRAMES);
}

#[test]
fn overlap_zero_keeps_the_gapless_append_exactly() {
    let (x, eot) = natural(5_000, 20, 12, 0, TransitionSpec::NONE);
    assert_eq!(x.ctl.xruns(), 0);
    assert_eq!(x.ctl.transition(), None);
    for f in (FADE_FRAMES as usize + BLOCK)..eot as usize {
        assert_eq!(x.out[f], A as f32, "kare {f}");
    }
    for f in (eot as usize)..(eot as usize + 100_000) {
        assert_eq!(x.out[f], B as f32, "kare {f}: B tam sınırda başlar");
    }
}

#[test]
fn track_clock_and_ui_boundary_follow_the_transition_start() {
    let spec = spec_ms(5_000);
    let mut x = X::new(5_000);
    let eot = x.play_a(20, spec);
    x.ctl.begin_handover();
    assert!(
        matches!(x.ctl.handover_state(), Handover::Pending { .. }),
        "geçiş başladı: sınır önce giden şarkının sonu"
    );
    x.end_of_track();
    x.ctl.arm_transition(spec);
    x.track_started(15_000);
    x.write_dc(B, PACKET);
    let t = x.ctl.transition().expect("geçiş kurulmadı");
    assert_eq!(
        (t.start, t.overlap as u64),
        (eot - spec.overlap as u64, spec.overlap as u64)
    );
    // Söz saati: yeni şarkının 0. karesi `start`ta.
    assert_eq!(
        x.ctl.track_base.load(std::sync::atomic::Ordering::SeqCst),
        t.start as i64
    );
    // Sınır (arayüzün şarkı değişimi) `start`a çekildi.
    assert_eq!(
        x.ctl.ho_boundary.load(std::sync::atomic::Ordering::SeqCst),
        t.start as i64
    );

    // Gerçek zamanlı ses kartı modeli: 10 ms'lik geri çağrılar, 1 ms'lik yoklama.
    let device_ns = x.ctl.device_ns as f64;
    let (mut truth, mut seen) = (None, None);
    x.advance_ms(10);
    let mut written_b = PACKET;
    for _ in 0..2_000 {
        let t_cb = ld(&x.clock) as f64;
        let before = ld(&x.ctl.read);
        x.pull(441);
        written_b += x.fill_dc(B, 20 * RATE as usize - written_b);
        if truth.is_none() && ld(&x.ctl.read) > t.start {
            truth = Some(t_cb + frames_to_ns((t.start - before) as f64) + device_ns);
        }
        for _ in 0..10 {
            if let Handover::Audible { new_ms } = x.ctl.handover_state()
                && seen.is_none()
            {
                seen = Some((ld(&x.clock) as f64, new_ms));
                let heard = x.ctl.audible_track_ms().expect("söz saati");
                assert!(
                    (heard - new_ms as i64).abs() <= 2,
                    "söz saati {heard} ms, yeni şarkı {new_ms} ms"
                );
            }
            x.advance_ms(1);
        }
        if seen.is_some() {
            break;
        }
    }
    let (truth, (seen, new_ms)) = (
        truth.expect("başlangıç çekilmedi"),
        seen.expect("başlangıç duyulmadı"),
    );
    let err_ms = (seen - truth) / 1e6;
    assert!(
        err_ms.abs() <= 3.0,
        "arayüz geçişi başlangıçtan {err_ms:.2} ms sapmayla (çıta ±50 ms)"
    );
    assert!(
        new_ms <= 20,
        "yeni şarkının duyulan konumu {new_ms} ms olmalıydı ~0"
    );
}

// ── Takılma ─────────────────────────────────────────────────────────────

/// 7 Kasım: yeni şarkının ilk paketi diskte 2,65 sn takıldı. Bindirmeli
/// geçişte de payın yuttuğu takılma xrun yapmaz (`OPENIFY_TEST_TRANSITION_STALL_MS`
/// = 3000 senaryosu), bindirme kısalmaz.
#[test]
fn a_three_second_stall_at_the_boundary_causes_no_xrun_and_keeps_the_full_overlap() {
    for (ms, stall) in [
        (5_000, 3_000),
        (5_000, 3_500),
        (12_000, 3_000),
        (12_000, 3_500),
    ] {
        let spec = spec_ms(ms);
        let (x, eot) = natural(ms, 40, 30, stall, spec);
        assert_eq!(x.ctl.xruns(), 0, "{ms} ms bindirme, {stall} ms takılma");
        let overlap = spec.overlap as u64;
        // Bindirme tam: B'nin ilk karesi `eot − overlap`'ta, A'nın o ana dek sesi bozulmamış.
        let start = (eot - overlap) as usize;
        assert_eq!(
            x.out[start + 10],
            expected_mix(10, overlap),
            "{ms} ms / {stall} ms: bindirme kısaldı"
        );
        assert_eq!(x.out[start - 10], A as f32);
        assert_eq!(x.out[eot as usize + 10], B as f32);
    }
}

#[test]
fn a_late_first_packet_shortens_the_overlap_to_what_is_still_unplayed() {
    // 5 sn bindirme, payın derinliği 9 sn; B'nin ilk paketi 6,5 sn geç gelir:
    // halkada 2,5 sn kaldı, GUARD'tan sonrası bindirilir. Xrun yok.
    let spec = spec_ms(5_000);
    let (x, eot) = natural(5_000, 30, 15, 6_500, spec);
    assert_eq!(x.ctl.xruns(), 0);
    let lead = x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        lead >= GUARD_FRAMES,
        "okumanın {lead} kare önünde değişiklik (< {GUARD_FRAMES})"
    );
    // Bindirme istenenden kısa ama var: B bir yerde başlıyor, eot'ta tamam.
    let start = (FADE_FRAMES as usize..eot as usize)
        .find(|f| x.out[*f] != A as f32)
        .expect("bindirme hiç olmadı");
    let overlap = eot as usize - start;
    assert!(
        overlap < spec.overlap as usize,
        "bindirme kısalmadı: {overlap}"
    );
    assert!(overlap as u64 >= MIN_OVERLAP_FRAMES);
    assert!(
        overlap as f64 / RATE as f64 > 2.0,
        "kısalan bindirme beklenenden kısa: {overlap}"
    );
    assert_eq!(x.out[eot as usize + 10], B as f32);
}

#[test]
fn no_room_left_falls_back_to_the_gapless_append() {
    // Payın tamamı çalındı (takılma > pay − GUARD): bindirme yok, B sona eklenir.
    let spec = spec_ms(5_000);
    let (x, eot) = natural(5_000, 30, 15, 8_900, spec);
    assert_eq!(x.ctl.transition(), None);
    assert_eq!(x.out[(eot as usize).saturating_sub(10)], A as f32);
    // Kuyruk kısmen boşaldıysa A hâlâ kendi sesinde; B tam sınırda ya da (xrun varsa) sonrasında.
    assert!(x.out[eot as usize..].iter().any(|v| *v == B as f32));
}

// ── Duraklat / sar / aygıt değişimi: bindirmenin ortasında ──────────────

/// A'yı yazıp B'nin bindirmesini `parts` parçada yazan düzenek; her parça
/// sonrasında `between` çalışır. Dönen: (düzenek, eot, bindirme).
fn overlap_in_parts(
    crossfade_ms: u32,
    parts: usize,
    mut between: impl FnMut(&mut X, usize, &Transition),
) -> (X, u64, Transition) {
    let spec = spec_ms(crossfade_ms);
    let mut x = X::new(crossfade_ms);
    let eot = x.play_a(30, spec);
    x.ctl.begin_handover();
    x.end_of_track();
    x.ctl.arm_transition(spec);
    x.track_started(20_000);
    // Birinci paket bindirmeyi kurar.
    x.write_dc(B, PACKET);
    let t = x.ctl.transition().expect("geçiş");
    let per_part = (t.overlap as usize / parts).max(PACKET);
    let mut written_b = PACKET;
    for part in 0..parts {
        let target = ((part + 1) * per_part).min(t.overlap as usize);
        while written_b < target {
            let n = PACKET.min(target - written_b);
            x.write_dc(B, n);
            written_b += n;
        }
        between(&mut x, part, &t);
    }
    (x, eot, t)
}

fn expected_mix(k: u64, overlap: u64) -> f32 {
    let (out, inn) = Curve::EqualPower.gains((k as f64 + 0.5) / overlap as f64);
    A as f32 * out + B as f32 * inn
}

/// Ses kartı, okuma ucu `start + after` karesine gelene dek çeker (bindirmenin
/// ortasına).
fn pull_into_overlap(x: &mut X, t: &Transition, after: u64) {
    for _ in 0..10_000 {
        if ld(&x.ctl.read) >= t.start + after {
            return;
        }
        x.cb();
    }
    panic!("okuma bindirmeye gelmedi");
}

#[test]
fn pause_in_the_middle_of_the_overlap_resumes_bit_exact() {
    let mut paused_at = None;
    let (x, _eot, t) = overlap_in_parts(5_000, 4, |x, part, t| {
        if part == 1 {
            // Ses kartı bindirmenin 0,5 sn içine gelmiş, yazar yarısını yazmış.
            pull_into_overlap(x, t, 22_050);
            let settled = FADE_FRAMES as u64 + 2 * BLOCK as u64;
            // (`read` blok sonunda ilerler: çekilen kare sayısını aşmasın.)
            let upto = ld(&x.ctl.read).min(x.out.len() as u64);
            for f in (t.start + settled)..upto {
                assert_eq!(
                    x.out[f as usize],
                    expected_mix(f - t.start, t.overlap as u64),
                    "duraklatmadan önce kare {f}"
                );
            }
            // Duraklat: ses iner, yazar durur; kuyruk korunur.
            x.ctl.duck();
            x.w.stop();
            let queued = x.ctl.queued_frames();
            for _ in 0..30 {
                x.cb();
            }
            let tail = &x.out[x.out.len() - 441..];
            assert!(tail.iter().all(|v| *v == 0.0), "duraklatılınca ses susmadı");
            assert!(
                x.ctl.queued_frames() + (BLOCK + FADE_FRAMES as usize) as u64 >= queued,
                "kuyruk atıldı"
            );
            paused_at = Some(ld(&x.ctl.read));
            // Devam.
            x.ctl.unduck();
            x.w.start();
        }
    });
    let read_at_pause = paused_at.expect("duraklatılmadı");
    assert!(
        read_at_pause > t.start,
        "duraklatma bindirmenin içinde olmalıydı"
    );
    // Yazar kalan bindirmeyi yazdı: halkadaki karışım hâlâ tam eğriden.
    let end = t.start + t.overlap as u64;
    for pos in (read_at_pause..end).step_by(331) {
        assert_eq!(
            x.ctl.ring.load(pos).0,
            expected_mix(pos - t.start, t.overlap as u64),
            "kare {pos}"
        );
    }
    assert!(x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed) >= GUARD_FRAMES);
    // Devamdan sonra çekilen kareler kaldığı kareden, eğriye uygun (zarf oturunca).
    let mut x = x;
    let r0 = ld(&x.ctl.read);
    assert_eq!(r0, read_at_pause, "duraklatma sırasında okuma ilerledi");
    x.out.clear();
    for _ in 0..8 {
        x.cb();
    }
    // Duraklatırken blokta kalan sessiz kareler önce çıkar: ilk gerçek kare `r0`.
    let lead = x
        .out
        .iter()
        .position(|v| *v != 0.0)
        .expect("devamdan sonra ses gelmedi");
    let settled = lead + FADE_FRAMES as usize + BLOCK;
    for (i, v) in x.out.iter().enumerate().skip(settled) {
        let f = r0 + (i - lead) as u64;
        assert!(
            f >= t.start && f < end,
            "devam bindirmenin içinde olmalıydı"
        );
        assert_eq!(
            *v,
            expected_mix(f - t.start, t.overlap as u64),
            "devamdan sonra kare {f}"
        );
    }
    assert_eq!(x.ctl.xruns(), 0);
}

#[test]
fn seek_in_the_middle_of_the_overlap_discards_the_mix_and_starts_clean() {
    let (mut x, _eot, t) = overlap_in_parts(5_000, 4, |x, part, t| {
        if part == 1 {
            pull_into_overlap(x, t, 22_050);
        }
    });
    // Bindirmenin bir bölümü yazıldı; kullanıcı yeni bir yere sarıyor.
    assert_eq!(x.ctl.transition().map(|c| c.start), Some(t.start));
    let mark = x.ctl.flush();
    assert_eq!(
        x.ctl.handover_state(),
        Handover::None,
        "sarma geçişi iptal eder"
    );
    for _ in 0..3 {
        x.cb();
    }
    x.w.sync();
    assert_eq!(x.ctl.transition(), None, "bindirme sürüyor görünüyor");
    assert_eq!(
        ld(&x.ctl.head),
        ld(&x.ctl.written),
        "yazma ucu yazılanın gerisinde kaldı"
    );
    assert!(x.w.eot.is_none() && !x.w.xf_candidate);
    // Kapı: eski bindirme paketleri atılır, komut sonucu gelince yeni ses akar.
    x.ev(mark_event(mark));
    let before = ld(&x.ctl.written);
    x.write_dc(B, PACKET);
    assert_eq!(
        ld(&x.ctl.written),
        before,
        "kapı kapalı: eski paket atılmalı"
    );
    x.ev(PlayerEvent::Seeked {
        play_request_id: 2,
        track_id: track_uri(),
        position_ms: 90_000,
    });
    x.write_dc(0.125, 4096);
    assert_eq!(
        ld(&x.ctl.written),
        before + 4096,
        "yeni ses bindirilmeden sona eklenmeli"
    );
    x.out.clear();
    for _ in 0..40 {
        x.cb();
    }
    // Yeni ses ve yalnız yeni ses çalar (A = 0,25 ya da karışım artığı yok).
    assert!(
        x.out.iter().all(|v| *v <= 0.125),
        "eski karışımdan artık: {:?}",
        x.out.iter().find(|v| **v > 0.125)
    );
    assert!(x.out.iter().any(|v| *v == 0.125));
}

#[test]
fn device_change_in_the_middle_of_the_overlap_keeps_the_mix_and_the_order() {
    let mut new_src: Option<RingSource> = None;
    let (x, _eot, t) = overlap_in_parts(5_000, 4, |x, part, t| {
        if part == 1 {
            pull_into_overlap(x, t, 22_050);
            // Çalmaya yeni aygıt: yeni kaynak bekleme kipinde, eski iniş yapıp bırakır.
            let new = RingSource::with_gen(x.ctl.clone(), 2);
            x.ctl
                .source_gen
                .store(2, std::sync::atomic::Ordering::SeqCst);
            x.advance_ms(10);
            x.pull(2000);
            assert!(ld(&x.ctl.released_gen) >= 1, "eski kaynak bırakmadı");
            new_src = Some(new);
        }
    });
    // Yeni aygıtın kaynağı, eski kaynağın bıraktığı kareden sürer; her çekilen
    // kare bindirme eğrisine uygun, hiçbiri atlanmadı ya da yinelenmedi.
    let mut new = new_src.expect("aygıt değişmedi");
    let mut r = ld(&x.ctl.read);
    let settled_from = r + (FADE_FRAMES as u64 + 2 * BLOCK as u64);
    let end = t.start + t.overlap as u64;
    let mut checked = 0;
    for block in 0..60 {
        x.advance_ms(1);
        let r0 = ld(&x.ctl.read);
        assert!(r0 >= r, "okuma geri gitti");
        r = r0;
        for i in 0..BLOCK as u64 {
            let l = new.next().unwrap();
            new.next().unwrap();
            let f = r0 + i;
            if f >= settled_from && f < end {
                assert_eq!(
                    l,
                    expected_mix(f - t.start, t.overlap as u64),
                    "blok {block}, kare {f}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 2_000, "yeterince kare doğrulanmadı: {checked}");
    assert_eq!(x.ctl.xruns(), 0);
    assert!(x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed) >= GUARD_FRAMES);
}

#[test]
fn flush_during_the_stall_before_the_first_packet_forgets_the_boundary() {
    // EndOfTrack görüldü, B henüz gelmedi; kullanıcı başka şarkı yükledi (flush).
    // Yazar sınırı ve süren isteği unutmalı: yeni şarkı bindirmesiz başlar.
    let spec = spec_ms(5_000);
    let mut x = X::new(5_000);
    x.play_a(20, spec);
    x.ctl.begin_handover();
    x.end_of_track();
    x.w.sync();
    assert!(x.w.eot.is_some());
    let mark = x.ctl.flush();
    x.cb();
    x.w.sync();
    assert!(x.w.eot.is_none());
    x.ev(mark_event(mark));
    x.ev(PlayerEvent::PlayRequestIdChanged { play_request_id: 3 });
    x.track_started(10_000);
    x.write_dc(B, PACKET);
    assert_eq!(x.ctl.transition(), None);
}

// ── Yazarın değişmezi ───────────────────────────────────────────────────

#[test]
fn writer_only_modifies_frames_after_read_plus_guard() {
    // Bindirme sürerken ses kartı yazardan hızlı koşar (kısa parçalar, çok
    // geri çağrı): değişiklik hiçbir zaman `read + GUARD`ın gerisine inmez;
    // geç kalan kareler atlanır ve ses bozulmaz (NaN / aşırı değer yok).
    let spec = spec_ms(5_000);
    let mut x = X::new(5_000);
    let eot = x.play_a(30, spec);
    x.ctl.begin_handover();
    x.end_of_track();
    x.ctl.arm_transition(spec);
    x.track_started(20_000);
    // Her paketten sonra ses kartı 40 ms çeker; yazar 1024 kare yazar: çekilen
    // (1764 kare) yazılandan (1024) hızlı, bindirme okumayı kovalar.
    let t_total = 30 * RATE as usize;
    let mut sent = 0;
    while sent < 20 * RATE as usize {
        x.write_dc(B, PACKET);
        sent += PACKET;
        for _ in 0..6 {
            x.cb();
        }
        if ld(&x.ctl.read) > eot + 1000 || sent > t_total {
            break;
        }
    }
    let lead = x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        lead >= GUARD_FRAMES,
        "değişmez bozuldu: en yakın değişiklik okumanın {lead} kare önünde"
    );
    assert!(x.out.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
}

// ── Büyüyebilen halka ───────────────────────────────────────────────────

fn ramp_frames(first: u64, frames: usize) -> Vec<f64> {
    (first..first + frames as u64)
        .flat_map(|i| [i as f64 / 1e6, 0.5])
        .collect()
}

fn id_of(sample: f32) -> u64 {
    (sample as f64 * 1e6).round() as u64
}

#[test]
fn growing_the_ring_is_seamless_across_the_split() {
    let mut x = X::new(0);
    x.w.start();
    assert_eq!(x.ctl.write_cap(), CAP_FRAMES);
    let mut next = 0u64;
    let mut write = |x: &mut X, n: usize| {
        let samples = ramp_frames(next, n);
        x.w.sync();
        if x.w.accept(n as u64) {
            x.w.push(&samples).unwrap();
        }
        next += n as u64;
    };
    // Kuyruk sığ: 3000 yazıldı, 2000'i çekildi.
    write(&mut x, 3_000);
    x.pull(2_000);
    let split_expected = ld(&x.ctl.written);
    // Motor halkayı büyütür (ayarlardan açıldı): ikinci yuva ayrılır, geçiş henüz yok.
    x.ctl.reserve(ring_frames(12_000));
    assert!(x.ctl.big.get().is_some());
    assert_eq!(
        ld(&x.ctl.big_from),
        u64::MAX,
        "geçiş yazarın işi, motor yalnız ayırır"
    );
    assert_eq!(x.ctl.write_cap(), CAP_FRAMES);
    // Sonraki paket: yazar ikinci yuvaya geçer. Okuma ucu bölünmenin gerisinde:
    // bir blok iki yuvanın üstünden okur.
    write(&mut x, 1_024);
    assert_eq!(ld(&x.ctl.big_from), split_expected, "geçiş yazma ucundan");
    assert_eq!(x.ctl.write_cap(), MAX_RING_FRAMES);
    // 6 sn: kesintisiz, sıralı.
    for _ in 0..(6 * RATE as usize / 1024) {
        write(&mut x, 1_024);
        x.pull(1_024);
    }
    x.pull(1_000);
    let ids: Vec<u64> = x.out.iter().skip(2_000 + 400).map(|v| id_of(*v)).collect();
    let mut checked = 0;
    for pair in ids.windows(2) {
        assert!(pair[0] > 0, "ses kesildi");
        assert_eq!(
            pair[1],
            pair[0] + 1,
            "halka büyürken kare atlandı / yinelendi: {} → {}",
            pair[0],
            pair[1]
        );
        checked += 1;
    }
    assert!(checked > 200_000);
    assert_eq!(x.ctl.xruns(), 0);
}

#[test]
fn growth_deepens_the_tail_to_the_full_overlap() {
    let ms = 12_000;
    let mut x = X::new(0);
    x.ctl.arm_transition(spec_ms(ms));
    x.ctl.set_tail(true);
    // Halka küçükken pay halkaya sığacak kadar.
    assert_eq!(x.ctl.target_frames(), CAP_FRAMES - RING_SLACK);
    x.ctl.reserve(ring_frames(ms));
    x.w.start();
    x.write_dc(A, PACKET);
    assert_eq!(x.ctl.write_cap(), MAX_RING_FRAMES);
    // Büyüdükten sonra pay tam derinlikte: 12 sn + 4 sn.
    assert_eq!(x.ctl.target_frames(), ms_to_frames(ms as u64 + TAIL_MS));
    // Pay kapanınca (yeni şarkı payı doldurdu / kullanıcı başka şarkı yükledi) istek de silinir.
    x.ctl.set_tail(false);
    assert!(x.ctl.armed_spec().is_none());
}

// ── Ses iş parçacığında ayırma yok ──────────────────────────────────────

/// `RingSource::fill` her yolunda (tek yuva, iki yuvanın üstü, ikinci yuva,
/// bekletme / iniş, flush, ses düzeyi, ekolayzır, aygıt değişimi) bellek
/// AYIRMAZ. (Kilit de yok: yalnız atomik okuma / yazma ve `OnceLock::get`,
/// bkz. `source.rs`.)
#[test]
#[cfg(not(feature = "mem-stats"))]
fn audio_thread_never_allocates() {
    // AutoMix geçişi (sessizlik atlama + planlı bindirme + iki yuva) de: çekme yolu ayırmaz.
    {
        let run = natural_auto(
            auto_spec(5_000),
            None,
            Some(hard_tail(2_000)),
            20,
            2,
            20 * PACKET,
            12,
        );
        assert!(
            Transition::unpack(ld(&run.x.ctl.transition)).is_some(),
            "AutoMix geçişi kurulmadı"
        );
        assert_eq!(
            run.x.pull_allocs, 0,
            "ses iş parçacığı {} kez bellek ayırdı (AutoMix)",
            run.x.pull_allocs
        );
    }
    let mut x = X::new(0);
    x.w.start();
    let mut src = RingSource::new(x.ctl.clone());
    // 3000 kare küçük yuvada; halka büyür; 8000 kare daha ikinci yuvada.
    x.write_dc(A, 3_000);
    x.ctl.reserve(ring_frames(12_000));
    x.write_dc(A, 8_000);
    assert_eq!(ld(&x.ctl.big_from), 3_000);
    // Tek yuva → ikisinin üstü → ikinci yuva.
    let n = allocs_during(|| {
        for _ in 0..(2 * 6_000) {
            std::hint::black_box(src.next());
        }
    });
    assert_eq!(n, 0, "ses iş parçacığı {n} kez bellek ayırdı (yuvalar)");
    x.write_dc(A, 16_000);
    let mut other = RingSource::with_gen(x.ctl.clone(), 2);
    let n = allocs_during(|| {
        for _ in 0..2_000 {
            std::hint::black_box(other.next());
        }
        x.ctl.duck();
        for _ in 0..2_000 {
            std::hint::black_box(src.next());
        }
        x.ctl.unduck();
        x.ctl.set_gain(0.5);
        x.ctl.set_eq(true, [3.0; eq::BANDS], 0.0);
        for _ in 0..2_000 {
            std::hint::black_box(src.next());
        }
        // Karaoke: mono (DC) kaynakta dal ölçer ama süzmez; ıslak yol aşağıda.
        x.ctl.set_karaoke(true, 0.8);
        for _ in 0..2_000 {
            std::hint::black_box(src.next());
        }
        x.ctl.flush();
        for _ in 0..2_000 {
            std::hint::black_box(src.next());
        }
        x.ctl
            .source_gen
            .store(2, std::sync::atomic::Ordering::SeqCst);
        for _ in 0..2_000 {
            std::hint::black_box(src.next());
            std::hint::black_box(other.next());
        }
    });
    assert_eq!(
        n, 0,
        "ses iş parçacığı {n} kez bellek ayırdı (duck / flush / EQ / karaoke / aygıt)"
    );

    // Karaoke'nin ıslak yolu (stereo: orta ve yan ≠ 0): açma, miktar değişimi,
    // kapama, flush; ayırma yok.
    let mut y = X::new(0);
    y.w.start();
    let mut ysrc = RingSource::new(y.ctl.clone());
    let stereo: Vec<f64> = (0..16_384)
        .flat_map(|n| {
            [
                0.3 * ((n % 97) as f64 / 97.0) + 0.1,
                -0.2 * ((n % 53) as f64 / 53.0),
            ]
        })
        .collect();
    y.w.sync();
    assert!(y.w.accept(16_384));
    y.w.begin_packet();
    y.w.push(&stereo).unwrap();
    let mut engaged = 0.0f32;
    let n = allocs_during(|| {
        y.ctl.set_karaoke(true, 1.0);
        for _ in 0..4_000 {
            std::hint::black_box(ysrc.next());
        }
        engaged = ysrc.karaoke.engage();
        y.ctl.set_karaoke(true, 0.4);
        for _ in 0..4_000 {
            std::hint::black_box(ysrc.next());
        }
        y.ctl.set_karaoke(false, 0.4);
        for _ in 0..4_000 {
            std::hint::black_box(ysrc.next());
        }
        y.ctl.flush();
        for _ in 0..2_000 {
            std::hint::black_box(ysrc.next());
        }
    });
    assert!(engaged > 0.0, "ıslak yol hiç çalışmadı (karışım {engaged})");
    assert_eq!(
        n, 0,
        "ses iş parçacığı {n} kez bellek ayırdı (karaoke ıslak yol)"
    );
}

/// Bellek bedeli (çıta: geçiş kapalı < +0,1 MiB, açık ≤ +6,5 MiB): çıkışın
/// kurulurken ayırdığı bayt. Kapalıyken bugünkü 2 MiB'lık halkadan fazlası
/// yalnız birkaç yüz baytlık yeni alan; 12 sn'de halka 8 MiB (+6 MiB).
#[test]
#[cfg(not(feature = "mem-stats"))]
fn ring_memory_cost_stays_within_the_budget() {
    let build = |ms: u32| {
        let levels = AudioLevels::new();
        let clock = Arc::new(AtomicU64::new(1));
        let mut ctl = None;
        let (_, bytes) =
            alloc_stats_during(|| ctl = Some(OutputCtl::with_clock_for(levels, clock, ms)));
        (ctl.unwrap(), bytes)
    };
    let (_off, off) = build(0);
    let ring = CAP_FRAMES * CHANNELS as u64 * 4;
    assert!(off >= ring, "halka ayrılmadı");
    let extra_off = off - ring;
    assert!(
        extra_off < 100 * 1024,
        "kapalıyken halka dışı ayırma {extra_off} bayt (çıta < 0,1 MiB)"
    );
    for (ms, mib) in [(2_000u32, 4u64), (5_000, 4), (8_000, 8), (12_000, 8)] {
        let (ctl, on) = build(ms);
        assert_eq!(
            ctl.ring.frames() * CHANNELS as u64 * 4,
            mib << 20,
            "{ms} ms için halka"
        );
        let delta = on as f64 / (1 << 20) as f64 - off as f64 / (1 << 20) as f64;
        println!("bellek: {ms} ms → +{delta:.3} MiB (kapalıya göre)");
        assert!(delta <= 6.5, "{ms} ms: +{delta:.2} MiB (çıta ≤ +6,5)");
    }
    // Çalışırken açma: ikinci yuva ayrılır (kapalı halka kalır): +8 MiB, yeniden bağlanana dek.
    let (ctl, _) = build(0);
    let (_, grown) = alloc_stats_during(|| ctl.reserve(ring_frames(12_000)));
    println!(
        "bellek: çalışırken 12 sn → +{:.3} MiB",
        grown as f64 / (1 << 20) as f64
    );
    assert!(grown >= 8 << 20 && grown < (8 << 20) + 4096);
}

/// Ölçüm (çıta: EQ kapalı `fill` ≤ +%5): yuva seçimli `fill` ile öncesinin
/// kopyası (`RingSource::fill_baseline`) AYNI süreçte dönüşümlü koşulur; oran
/// makinenin hız durumundan bağımsız. Çalıştırma:
/// `cargo test --release fill_cost_against -- --ignored --nocapture`.
#[test]
#[ignore]
fn fill_cost_against_the_pre_growth_baseline() {
    const BLOCKS: usize = 400_000;
    let mut x = X::new(0);
    x.w.start();
    x.write_dc(A, CAP_FRAMES as usize);
    x.pull(2000);
    let run = |x: &mut X, fill: fn(&mut RingSource)| -> f64 {
        let t = std::time::Instant::now();
        for i in 0..BLOCKS {
            if i % 1024 == 0 {
                let r = ld(&x.ctl.read);
                x.ctl
                    .written
                    .store(r + 200_000, std::sync::atomic::Ordering::SeqCst);
                x.advance_ms(10);
            }
            fill(&mut x.src);
            std::hint::black_box(&x.src.buf);
        }
        t.elapsed().as_nanos() as f64 / BLOCKS as f64
    };
    let (mut new, mut old) = (Vec::new(), Vec::new());
    for _ in 0..9 {
        new.push(run(&mut x, RingSource::fill));
        old.push(run(&mut x, RingSource::fill_baseline));
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (n, o) = (med(&mut new), med(&mut old));
    println!(
        "fill_vs_baseline: yeni {n:.1} ns/blok, önceki {o:.1} ns/blok, oran {:+.2}%",
        (n / o - 1.0) * 100.0
    );
}

// ── AutoMix: sessizlik atlama, planlı bindirme ──────────────────────────

use crate::engine::output::{
    Analysis, Edge, EdgeView, HAS_TAIL,
    automix::{TRIM_KEEP, tempo_compatible},
};

impl X {
    /// `tap::LevelSink::write` sırası (AutoMix): eşitle, kapı, ilk paket işi, sessizlik
    /// atlama, yaz.
    fn write_auto(&mut self, samples: &[f64]) {
        let frames = (samples.len() / CHANNELS) as u64;
        self.w.sync();
        if !self.w.accept(frames) {
            return;
        }
        self.w.begin_packet();
        let packet = self.w.prepare(samples);
        if packet.is_empty() {
            return;
        }
        self.w.push(&packet).unwrap();
    }

    /// `fill_dc` gibi, `write_auto` ile; döner: tüketilen GİRİŞ karesi (atılan
    /// sessizlik de sayılır).
    fn fill_auto(&mut self, v: f64, frames: usize) -> usize {
        let mut done = 0;
        while done < frames && self.ctl.queued_raw() <= self.ctl.target_frames() {
            let n = PACKET.min(frames - done);
            self.write_auto(&vec![v; n * 2]);
            done += n;
        }
        done
    }

    /// Giden şarkı: `secs` sn `A` + `silence_secs` sn dijital sessizlik, pay açıkken.
    /// Dönen: yazılan son kare (`eot`).
    fn play_a_auto(
        &mut self,
        secs: usize,
        silence_secs: usize,
        spec: TransitionSpec,
        incoming: Option<EdgeView>,
    ) -> u64 {
        self.w.start();
        self.ctl.set_automix(true);
        self.track_started(secs as u32 * 1000);
        self.write_auto(&vec![A; PACKET * 2]);
        self.ctl.arm_automix(spec, incoming);
        self.ctl.set_tail(true);
        let mut written = PACKET;
        let total = secs * RATE as usize;
        while written < total {
            written += self.fill_auto(A, total - written);
            self.cb();
        }
        let silence = silence_secs * RATE as usize;
        let mut done = 0;
        while done < silence {
            done += self.fill_auto(0.0, silence - done);
            self.cb();
        }
        ld(&self.ctl.written)
    }
}

/// A (`a_secs` sn + `a_silence` sn sessizlik) biter, `lead` kare sessizlikle B başlar
/// (`b_secs` sn). `prev`: giden şarkının canlı çözümlemesi.
struct AutoRun {
    x: X,
    eot: u64,
}

fn natural_auto(
    spec: TransitionSpec,
    incoming: Option<EdgeView>,
    prev: Option<Analysis>,
    a_secs: usize,
    a_silence: usize,
    lead_frames: usize,
    b_secs: usize,
) -> AutoRun {
    let mut x = X::new(MAX_OVERLAP_MS);
    let eot = x.play_a_auto(a_secs, a_silence, spec, incoming);
    x.ctl.begin_handover();
    x.end_of_track();
    x.ctl.arm_automix(spec, incoming);
    x.w.set_prev_analysis(prev);
    x.track_started(b_secs as u32 * 1000);
    let mut left = lead_frames;
    while left > 0 {
        let n = PACKET.min(left);
        x.write_auto(&vec![0.0; n * 2]);
        left -= n;
    }
    let total = b_secs * RATE as usize;
    let mut written = 0;
    while written < total {
        written += x.fill_auto(B, total - written);
        x.cb();
    }
    for _ in 0..10_000 {
        if ld(&x.ctl.read) >= eot + 150_000 {
            break;
        }
        x.cb();
    }
    AutoRun { x, eot }
}

/// Sert bitişli (sönme yok) giden çözümleme: plan 2 sn, hizalamasız.
fn hard_tail(tail_ms: u32) -> Analysis {
    let mut a = Analysis::EMPTY;
    a.flags = HAS_TAIL;
    a.tail_ms = tail_ms;
    a.tail_env = [-14; crate::engine::output::ENV_SECS];
    a
}

fn auto_spec(ms: u32) -> TransitionSpec {
    TransitionSpec {
        auto: true,
        ..spec_ms(ms)
    }
}

#[test]
fn automix_drops_the_trailing_and_leading_silence_and_blends_for_the_plan() {
    let lead = 64 * PACKET;
    let run = natural_auto(
        auto_spec(5_000),
        None,
        Some(hard_tail(3_000)),
        20,
        3,
        lead,
        12,
    );
    let (x, eot) = (&run.x, run.eot);
    assert_eq!(x.ctl.xruns(), 0);
    // Sondaki 3 sn sessizlik halkaya YAZILMADI: şarkı sonu müziğin bittiği yer (+ TRIM_KEEP).
    assert_eq!(
        eot,
        20 * RATE as u64 + TRIM_KEEP,
        "sondaki sessizlik atılmadı: eot {eot}"
    );
    // Sert bitiş + giriş yok: plan 2 sn.
    let t = x
        .ctl
        .transition()
        .or_else(|| Transition::unpack(ld(&x.ctl.transition)))
        .expect("geçiş kurulmadı");
    assert_eq!(
        (t.start, t.overlap as u64),
        (eot - 2 * RATE as u64, 2 * RATE as u64)
    );
    // Baştaki sessizlik atıldı: yeni şarkının 0. karesi atılan kadar geride.
    assert_eq!(
        x.ctl.track_base.load(std::sync::atomic::Ordering::SeqCst),
        t.start as i64 - lead as i64
    );
    // Çıkış: bindirmeden önce yalnız A, bindirmede eşit güç, hemen ardından B (arada sessizlik YOK).
    let (a32, b32) = (A as f32, B as f32);
    for f in (FADE_FRAMES as usize + BLOCK)..t.start as usize {
        assert_eq!(x.out[f], a32, "kare {f}");
    }
    let overlap = t.overlap as u64;
    // Giden şarkının son TRIM_KEEP karesi bırakılan sessizlik: orada giden 0.
    for k in (0..overlap).step_by(211) {
        let (out, inn) = Curve::EqualPower.gains((k as f64 + 0.5) / overlap as f64);
        let old = if k >= overlap - TRIM_KEEP { 0.0 } else { a32 };
        assert_eq!(
            x.out[(t.start + k) as usize],
            old * out + b32 * inn,
            "k={k}"
        );
    }
    for f in (eot as usize + 100)..(eot as usize + 100_000) {
        assert_eq!(x.out[f], b32, "kare {f}: bindirmeden sonra hemen B");
    }
    assert!(x.ctl.min_lead.load(std::sync::atomic::Ordering::Relaxed) >= GUARD_FRAMES);
}

#[test]
fn automix_without_a_live_analysis_falls_back_to_the_requested_overlap() {
    // Giden şarkının çözümlemesi yok (ör. şarkıya sonlarında girildi): istekteki 5 sn.
    let run = natural_auto(auto_spec(5_000), None, None, 20, 0, 0, 12);
    let t = Transition::unpack(ld(&run.x.ctl.transition)).expect("geçiş kurulmadı");
    assert_eq!(
        (t.start, t.overlap as u64),
        (run.eot - 5 * RATE as u64, 5 * RATE as u64)
    );
    assert_eq!(run.x.ctl.xruns(), 0);
}

#[test]
fn automix_aligns_the_incoming_first_beat_to_an_outgoing_beat() {
    // Giden 120 BPM, son vuruş müzik bitişinden 300 ms önce, 6 sn'lik sönme;
    // gelen 122 BPM (±%6 içinde), ilk vuruşu müzik başlangıcından 100 ms sonra.
    let mut prev = hard_tail(3_000);
    prev.tail = Edge {
        bpm: 120.0,
        conf: 0.8,
        beat_ms: 300,
    };
    prev.tail_env = {
        let mut env = [-14i8; crate::engine::output::ENV_SECS];
        for (k, v) in env.iter_mut().enumerate().take(8) {
            *v = -14 - 4 * (8 - k as i8);
        }
        env
    };
    let inc = EdgeView {
        bpm: 122.0,
        conf: 0.7,
        beat_ms: 100,
        ramp_s: 0,
    };
    assert!(tempo_compatible(120.0, 122.0));
    let run = natural_auto(auto_spec(5_000), Some(inc), Some(prev), 30, 2, 0, 14);
    let t = Transition::unpack(ld(&run.x.ctl.transition)).expect("geçiş kurulmadı");
    let plan = prev.plan(Some(&inc)).unwrap();
    assert!(plan.aligned, "tempolar uyumlu: hizalanmalı");
    assert_eq!(t.overlap, plan.overlap, "yazar planı olduğu gibi uyguladı");
    // Hizalama: halka ucundan (eot) vuruşa geri: son vuruş + n × periyot + gelenin ilk vuruşu.
    let period = 60.0 / 120.0 * RATE as f64;
    let base = 0.3 * RATE as f64 + TRIM_KEEP as f64 + 0.1 * RATE as f64;
    let n = ((t.overlap as f64 - base) / period).round();
    assert!(
        (t.overlap as f64 - (base + n * period)).abs() <= 1.5,
        "bindirme {} kare, ızgara dışı",
        t.overlap
    );
    // Sönme 7 sn: istenen ~7 sn'ye en yakın ölçü.
    let secs = t.overlap as f64 / RATE as f64;
    assert!((secs - 7.0).abs() < 1.2, "{secs} sn");
    assert_eq!(run.x.ctl.xruns(), 0);
}

#[test]
fn automix_flag_off_or_a_none_spec_never_touches_the_written_audio() {
    // AutoMix açık ama istek yok (aynı albüm / kullanıcı geçişi): sessizlik YAZILIR, geçiş yok.
    let mut x = X::new(MAX_OVERLAP_MS);
    let eot = x.play_a_auto(20, 3, TransitionSpec::NONE, None);
    assert_eq!(
        eot,
        23 * RATE as u64,
        "istek yokken sondaki sessizlik korunur (boşluksuz albüm)"
    );
    // AutoMix bayrağı kapalı ama istek AutoMix: pay açıkken de paket aynen geçer.
    let mut y = X::new(MAX_OVERLAP_MILLIS_FOR_TEST);
    y.w.start();
    y.track_started(20_000);
    y.write_dc(A, PACKET);
    y.ctl.arm_automix(auto_spec(5_000), None);
    y.ctl.set_tail(true);
    let samples = vec![0.0f64; 2 * PACKET];
    for _ in 0..8 {
        assert!(matches!(
            y.w.prepare(&samples),
            std::borrow::Cow::Borrowed(_)
        ));
        y.write_dc(0.0, PACKET);
        y.cb();
    }
}

const MAX_OVERLAP_MILLIS_FOR_TEST: u32 = 12_000;

#[test]
fn automix_compresses_a_silent_gap_inside_the_song_but_never_cuts_music() {
    let mut x = X::new(MAX_OVERLAP_MS);
    x.w.start();
    x.ctl.set_automix(true);
    x.track_started(30_000);
    x.write_auto(&vec![A; PACKET * 2]);
    x.ctl.arm_automix(auto_spec(5_000), None);
    x.ctl.set_tail(true);
    let before = ld(&x.ctl.written);
    // 2 sn müzik, 4 sn dijital sessizlik (ortada bir boşluk), 2 sn müzik.
    for (v, secs) in [(A, 2usize), (0.0, 4), (A, 2)] {
        let mut done = 0;
        let total = secs * RATE as usize;
        while done < total {
            done += x.fill_auto(v, total - done);
            x.cb();
        }
    }
    let after = ld(&x.ctl.written);
    // Boşluk 1,5 sn'ye sıkıştı: 2 + 1,5 + 2 sn yazıldı.
    let expect = (2 + 2) * RATE as u64 + super::automix::GAP_MAX;
    assert_eq!(after - before, expect, "yazılan kare");
    assert_eq!(x.ctl.xruns(), 0);
}

#[test]
fn automix_transition_never_makes_the_audio_thread_allocate() {
    // Ses iş parçacığı yolu (çekme) AutoMix geçişinde de ayırmaz: sessizlik atlama,
    // plan, iki yuva, bindirme. (`X::pull` yalnız çekmeyi sayar.)
    let lead = 20 * PACKET;
    let run = natural_auto(
        auto_spec(5_000),
        None,
        Some(hard_tail(2_000)),
        20,
        2,
        lead,
        12,
    );
    assert!(Transition::unpack(ld(&run.x.ctl.transition)).is_some());
    assert_eq!(
        run.x.pull_allocs, 0,
        "ses iş parçacığı {} kez bellek ayırdı (AutoMix geçişi)",
        run.x.pull_allocs
    );
}
