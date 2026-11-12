//! AutoMix çözümlemesi (`BeatTap`) testleri: sentetik sinyallerle tempo, sessizlik,
//! zarf, evre ve geçerlilik; `#[ignore]` ölçüm (çözümleme işlemci maliyeti).
//! Yazarın AutoMix davranışı (sessizlik atlama, planlı bindirme) `xfade_tests.rs`'te.

use super::beat::{Analysis, BeatTap, ENV_SECS, ENV_UNKNOWN, HAS_HEAD, HAS_TAIL, HOP_RATE};
use super::RATE;

const FS: f64 = RATE as f64;

/// Deterministik gürültü (xorshift), −1..1.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 52) as f64 * 2.0 - 1.0
    }
}

#[derive(Clone, Copy)]
enum Pattern {
    /// Her vuruşta kısa, sönümlü 1,5 kHz tıkırtı.
    Clicks,
    /// Her vuruşta tekme (alçak sinüs), 2. ve 4. vuruşta kutu (gürültü), sekizlik
    /// aralarda tiz şapka.
    Drums,
}

/// `secs` sn'lik tek kanallı müzik; vuruşlar `first_beat` sn'den başlar.
fn render(pattern: Pattern, bpm: f64, secs: f64, first_beat: f64) -> Vec<f64> {
    let n = (secs * FS) as usize;
    let mut out = vec![0.0f64; n];
    if matches!(pattern, Pattern::Drums) {
        // Sürekli, alçak bir "pad": müzik şarkının sonuna dek sürer (sessizlik ölçümlerinin
        // beklenen değeri bununla kesin).
        for (i, s) in out.iter_mut().enumerate() {
            *s += 0.03 * (2.0 * std::f64::consts::PI * 220.0 * i as f64 / FS).sin();
        }
    }
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let period = 60.0 / bpm;
    let mut beat = 0usize;
    loop {
        let t = first_beat + beat as f64 * period;
        let start = (t * FS).round() as usize;
        if start >= n {
            break;
        }
        match pattern {
            Pattern::Clicks => add_click(&mut out, start),
            Pattern::Drums => {
                add_kick(&mut out, start);
                if beat % 2 == 1 {
                    add_snare(&mut out, start, &mut noise);
                }
                let half = start + (period * FS / 2.0).round() as usize;
                if half < n {
                    add_hat(&mut out, half, &mut noise);
                }
            }
        }
        beat += 1;
    }
    out
}

fn add_click(out: &mut [f64], at: usize) {
    for i in 0..(0.1 * FS) as usize {
        if at + i >= out.len() {
            break;
        }
        let t = i as f64 / FS;
        out[at + i] += 0.6 * (-t / 0.012).exp() * (2.0 * std::f64::consts::PI * 1500.0 * t).sin();
    }
}

fn add_kick(out: &mut [f64], at: usize) {
    for i in 0..(0.25 * FS) as usize {
        if at + i >= out.len() {
            break;
        }
        let t = i as f64 / FS;
        let f = 50.0 + 90.0 * (-t / 0.03).exp();
        out[at + i] += 0.7 * (-t / 0.08).exp() * (2.0 * std::f64::consts::PI * f * t).sin();
    }
}

fn add_snare(out: &mut [f64], at: usize, noise: &mut Noise) {
    for i in 0..(0.15 * FS) as usize {
        if at + i >= out.len() {
            break;
        }
        let t = i as f64 / FS;
        out[at + i] += 0.35 * (-t / 0.05).exp() * noise.next();
    }
}

fn add_hat(out: &mut [f64], at: usize, noise: &mut Noise) {
    let mut prev = 0.0;
    for i in 0..(0.05 * FS) as usize {
        if at + i >= out.len() {
            break;
        }
        let t = i as f64 / FS;
        let x = noise.next();
        // Yüksek geçiren (birinci fark).
        out[at + i] += 0.12 * (-t / 0.015).exp() * (x - prev);
        prev = x;
    }
}

fn stereo(mono: &[f64]) -> Vec<f64> {
    mono.iter().flat_map(|s| [*s, *s]).collect()
}

fn silence(secs: f64) -> Vec<f64> {
    vec![0.0; (secs * FS) as usize * 2]
}

/// Bir şarkıyı baştan sona (1 024 karelik paketlerle) çözümler.
fn analyse(song: &[f64], from_start: bool, ended: bool) -> Option<Analysis> {
    let mut tap = BeatTap::new();
    tap.begin(Some("spotify:track:test".into()), from_start);
    for chunk in song.chunks(2048) {
        tap.feed(chunk);
    }
    tap.finish(ended).map(|(_, a)| a)
}

fn circular_ms(a: u32, b: u32, period_ms: f64) -> f64 {
    let d = (a as f64 - b as f64).rem_euclid(period_ms);
    d.min(period_ms - d)
}

// ── Tempo ───────────────────────────────────────────────────────────────

/// Tempo ±1 BPM: tıkırtı dizileri ve davul örüntüsü, baştan ve sondan.
#[test]
fn tempo_is_within_one_bpm_for_clicks_and_drums() {
    for pattern in [Pattern::Clicks, Pattern::Drums] {
        for bpm in [90.0f64, 120.0, 128.0, 174.0] {
            let song = stereo(&render(pattern, bpm, 70.0, 0.37));
            let a = analyse(&song, true, true).expect("çözümleme yok");
            assert!(a.has_head() && a.has_tail(), "uçlar geçerli olmalı");
            for (name, edge) in [("head", a.head), ("tail", a.tail)] {
                assert!(
                    (edge.bpm as f64 - bpm).abs() <= 1.0,
                    "{} {bpm} BPM: {name} ucu {:.2} BPM okudu (güven {:.2})",
                    if matches!(pattern, Pattern::Clicks) { "tıkırtı" } else { "davul" },
                    edge.bpm,
                    edge.conf
                );
                assert!(edge.conf > 0.5, "{bpm} BPM {name}: güven {:.2}", edge.conf);
            }
        }
    }
}

/// Beklenen vuruş evresi: ilk vuruş müziğin başlangıcından `first_beat` sonra; son
/// vuruş müzik sonundan, vuruş ızgarası boyunca geriye doğru.
#[test]
fn beat_phase_matches_the_grid_within_thirty_ms() {
    for bpm in [100.0f64, 120.0, 128.0] {
        let first = 0.21;
        let secs = 60.0;
        let song = stereo(&render(Pattern::Drums, bpm, secs, first));
        let a = analyse(&song, true, true).unwrap();
        let period_ms = 60_000.0 / bpm;
        // Baş: ilk vuruş `first` sn (müzik 0. sn'de başlıyor, baştaki sessizlik yok).
        assert!(
            circular_ms(a.head.beat_ms, (first * 1000.0) as u32, period_ms) < 30.0,
            "{bpm} BPM baş evresi {} ms, beklenen {} ms",
            a.head.beat_ms,
            first * 1000.0
        );
        // Son: son vuruşun müzik sonuna uzaklığı. Vuruşlar first + k × period; sondaki
        // kuyruk (son vuruşun sönümü) dahil müzik bitişi `secs`.
        let n_beats = ((secs - first) / (60.0 / bpm)).floor();
        let last_beat = first + n_beats * 60.0 / bpm;
        let expect = ((secs - last_beat) * 1000.0).max(0.0);
        // Sessizlik eşiği (−60 dB) son vuruşun sönümünü sona kadar sürdürür: bitiş = secs.
        assert!(
            circular_ms(a.tail.beat_ms, expect as u32, period_ms) < 30.0,
            "{bpm} BPM son evre {} ms, beklenen {:.0} ms",
            a.tail.beat_ms,
            expect
        );
    }
}

/// Vuruşsuz (gürültü / sabit ton / sessizlik) içerikte güven düşük ya da çözümleme yok.
#[test]
fn unrhythmic_audio_has_low_confidence() {
    let mut noise = Noise(7);
    let song: Vec<f64> = (0..(40.0 * FS) as usize).flat_map(|_| {
        let x = 0.1 * noise.next();
        [x, x]
    }).collect();
    let a = analyse(&song, true, true).expect("gürültü de içeriktir");
    assert!(a.head.conf < 0.3 && a.tail.conf < 0.3, "gürültüde güven {:.2} / {:.2}", a.head.conf, a.tail.conf);
    // Sabit ton: vuruş yok.
    let tone: Vec<f64> = (0..(40.0 * FS) as usize)
        .flat_map(|i| {
            let x = 0.2 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / FS).sin();
            [x, x]
        })
        .collect();
    let a = analyse(&tone, true, true).unwrap();
    assert!(a.head.bpm == 0.0 || a.head.conf < 0.3, "sabit tonda tempo: {:?}", a.head);
    // Tam sessizlik: müzik yok, çözümleme yok.
    assert!(analyse(&silence(30.0), true, true).is_none());
}

// ── Sessizlik ───────────────────────────────────────────────────────────

/// Baştaki ve sondaki dijital sessizlik 50 ms içinde.
#[test]
fn lead_and_tail_silence_are_found_within_fifty_ms() {
    for (lead, tail) in [(2.35, 3.10), (0.0, 0.0), (0.73, 8.0), (5.5, 0.4)] {
        let mut song = silence(lead);
        song.extend(stereo(&render(Pattern::Drums, 120.0, 40.0, 0.0)));
        song.extend(silence(tail));
        let a = analyse(&song, true, true).expect("çözümleme yok");
        assert!(
            (a.lead_ms as f64 - lead * 1000.0).abs() <= 50.0,
            "baş sessizlik {lead} sn: {} ms okundu",
            a.lead_ms
        );
        assert!(
            (a.tail_ms as f64 - tail * 1000.0).abs() <= 50.0,
            "son sessizlik {tail} sn: {} ms okundu",
            a.tail_ms
        );
        let total = (lead + 40.0 + tail) * 1000.0;
        assert!((a.total_ms as f64 - total).abs() <= 20.0, "toplam {} ms, beklenen {total}", a.total_ms);
    }
}

/// Sönme: üstel (dB'de doğrusal) sönüş. Müziğin bittiği yer tepe genliğin −60 dBFS'in
/// altına indiği an; sönme uzunluğu zarftan okunur.
#[test]
fn fade_out_end_is_where_the_level_drops_below_minus_sixty_db() {
    let body = 30.0;
    let fade = 14.0;
    let tau = 1.2; // genlik 0,4·e^(−t/τ)
    // Sürekli ton (vuruşlar aralıklı olduğundan sönme bitişi bununla kesin ölçülür).
    let mono: Vec<f64> = (0..((body + fade) * FS) as usize)
        .map(|i| {
            let t = i as f64 / FS;
            let env = if t > body { (-(t - body) / tau).exp() } else { 1.0 };
            0.4 * env * (2.0 * std::f64::consts::PI * 440.0 * t).sin()
        })
        .collect();
    let song = stereo(&mono);
    let a = analyse(&song, true, true).unwrap();
    // Tepe genlik 0,4·e^(−(t−30)/1,2) ≥ 0,001 olduğu en son an.
    let cutoff = body + tau * (0.4f64 / 0.001).ln();
    let expect_tail = (body + fade - cutoff).max(0.0) * 1000.0;
    assert!(
        (a.tail_ms as f64 - expect_tail).abs() <= 50.0,
        "sönme bitişi: son sessizlik {} ms, beklenen {expect_tail:.0} ms",
        a.tail_ms
    );
    // Zarf: son saniye gövdeden çok daha alçak, sönme uzunluğu birkaç saniye.
    let view = a.tail_view().unwrap();
    assert!(view.ramp_s >= 4 && view.ramp_s <= 12, "sönme uzunluğu {} sn", view.ramp_s);
    assert!(a.tail_env[0] < a.tail_env[ENV_SECS - 1] - 12, "son saniye {} dB, 20 sn önce {} dB", a.tail_env[0], a.tail_env[ENV_SECS - 1]);
}

/// Sert bitiş / sert başlangıç: sönme ve giriş 0.
#[test]
fn hard_edges_have_no_ramp() {
    let song = stereo(&render(Pattern::Drums, 120.0, 40.0, 0.0));
    let a = analyse(&song, true, true).unwrap();
    assert_eq!(a.tail_view().unwrap().ramp_s, 0);
    assert_eq!(a.head_view().unwrap().ramp_s, 0);
}

/// Yavaş giriş: baştaki 8 sn doğrusal (genlikte) yükselir.
#[test]
fn slow_intro_is_measured() {
    let mut mono = render(Pattern::Drums, 120.0, 40.0, 0.0);
    for (i, s) in mono.iter_mut().enumerate() {
        let t = i as f64 / FS;
        if t < 8.0 {
            *s *= (t / 8.0).powi(2) + 0.002;
        }
    }
    let a = analyse(&stereo(&mono), true, true).unwrap();
    let view = a.head_view().unwrap();
    assert!((4..=9).contains(&view.ramp_s), "giriş uzunluğu {} sn", view.ramp_s);
}

// ── Zarf ────────────────────────────────────────────────────────────────

#[test]
fn envelope_reads_decibels_per_second() {
    // Sabit sinüs (L = R): genlik 0,1 → ortalama güç 0,005 → −23 dBFS.
    let tone: Vec<f64> = (0..(30.0 * FS) as usize)
        .flat_map(|i| {
            let x = 0.1 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / FS).sin();
            [x, x]
        })
        .collect();
    let a = analyse(&tone, true, true).unwrap();
    for env in [a.head_env, a.tail_env] {
        for v in env {
            assert!((v as i32 + 23).abs() <= 1, "zarf {v} dB, beklenen −23");
        }
    }
    // Kısa şarkı: pencere dolmayan saniyeler bilinmiyor.
    let short: Vec<f64> = tone[..(10.0 * FS) as usize * 2].to_vec();
    let a = analyse(&short, true, true).expect("10 sn yeterli");
    assert!(a.head_env[..9].iter().all(|v| *v != ENV_UNKNOWN));
    assert!(a.head_env[12..].iter().all(|v| *v == ENV_UNKNOWN), "dolmayan saniyeler bilinmiyor");
}

// ── Geçerlilik ──────────────────────────────────────────────────────────

#[test]
fn only_continuous_listening_gives_valid_edges() {
    let song = stereo(&render(Pattern::Drums, 120.0, 60.0, 0.0));
    // Şarkı ortadan başladı: baş uç geçersiz, son uç geçerli.
    let a = analyse(&song, false, true).unwrap();
    assert_eq!(a.flags, HAS_TAIL);
    // Kendiliğinden bitmedi (atlandı): son uç geçersiz, baş uç geçerli.
    let a = analyse(&song, true, false).unwrap();
    assert_eq!(a.flags, HAS_HEAD);
    // Hiçbiri: çözümleme yok.
    assert!(analyse(&song, false, false).is_none());
    // Çok kısa dinleme (6 sn): hiçbir uç.
    assert!(analyse(&song[..6 * RATE as usize * 2], true, true).is_none());
    // Sarma: baş uç geçersiz olur, sondaki kesintisiz kısım yeter.
    let mut tap = BeatTap::new();
    tap.begin(Some("t".into()), true);
    for chunk in song[..30 * RATE as usize * 2].chunks(2048) {
        tap.feed(chunk);
    }
    tap.discontinuity();
    for chunk in song[10 * RATE as usize * 2..].chunks(2048) {
        tap.feed(chunk);
    }
    let a = tap.finish(true).unwrap().1;
    assert_eq!(a.flags, HAS_TAIL, "sarmadan sonra baş geçersiz, son geçerli");
    // Sarmadan hemen sonra biten (12 sn'den kısa kesintisiz): son uç da yok.
    let mut tap = BeatTap::new();
    tap.begin(Some("t".into()), true);
    for chunk in song[..30 * RATE as usize * 2].chunks(2048) {
        tap.feed(chunk);
    }
    tap.discontinuity();
    for chunk in song[..5 * RATE as usize * 2].chunks(2048) {
        tap.feed(chunk);
    }
    assert!(tap.finish(true).is_none());
}

#[test]
fn bars_and_hops_use_fixed_constants() {
    assert!((HOP_RATE - 86.1328125).abs() < 1e-6);
    let mut a = Analysis::EMPTY;
    a.flags = HAS_HEAD;
    a.head.bpm = 120.0;
    let mut b = Analysis::EMPTY;
    b.flags = HAS_TAIL;
    b.tail.bpm = 100.0;
    let m = a.merged(&b);
    assert_eq!((m.flags, m.head.bpm, m.tail.bpm), (HAS_HEAD | HAS_TAIL, 120.0, 100.0));
}

// ── Ölçüm ───────────────────────────────────────────────────────────────

/// `cargo test --release beat_cost -- --ignored --nocapture`: çözümlemenin işlemci
/// maliyeti. `feed` (paket başına, çalar iş parçacığında) ve şarkı başına ağır işler
/// (baş penceresi dolunca, şarkı bitince) ayrı ölçülür; dakikaya düşen toplam yazdırılır.
#[test]
#[ignore = "ölçüm: elle çalıştır"]
fn beat_cost() {
    let song = stereo(&render(Pattern::Drums, 124.0, 240.0, 0.1));
    let minutes = 4.0;
    let reps = 5;
    let mut feed_only = f64::MAX;
    let mut total = f64::MAX;
    for _ in 0..reps {
        let mut tap = BeatTap::new();
        tap.begin(Some("t".into()), true);
        let t = std::time::Instant::now();
        for chunk in song.chunks(2048) {
            tap.feed(chunk);
        }
        let fed = t.elapsed().as_secs_f64();
        let f = std::time::Instant::now();
        let a = tap.finish(true);
        let fin = f.elapsed().as_secs_f64();
        std::hint::black_box(a);
        feed_only = feed_only.min(fed);
        total = total.min(fed + fin);
        eprintln!("  koşu: feed {:.1} ms (içinde baş penceresi dahil), finish {:.1} ms", fed * 1e3, fin * 1e3);
    }
    let audio_s = minutes * 60.0;
    eprintln!(
        "BeatTap: {:.2} ms / dakika ses (feed + baş penceresi), şarkı sonu işi dahil toplam {:.2} ms / dakika; gerçek zamana oranı %{:.4}",
        feed_only / minutes * 1e3,
        total / minutes * 1e3,
        total / audio_s * 100.0
    );
}

#[test]
#[ignore = "tanı"]
fn debug_tempo() {
    for pattern in [Pattern::Clicks, Pattern::Drums] {
        for bpm in [90.0f64, 100.0, 120.0, 128.0, 174.0] {
            let song = stereo(&render(pattern, bpm, 70.0, 0.21));
            let a = analyse(&song, true, true).unwrap();
            eprintln!(
                "{} {bpm}: lead {} ms baş {:.2} bpm conf {:.2} faz {} ms | son {:.2} conf {:.2} faz {} ms",
                if matches!(pattern, Pattern::Clicks) { "tık " } else { "davul" },
                a.lead_ms, a.head.bpm, a.head.conf, a.head.beat_ms, a.tail.bpm, a.tail.conf, a.tail.beat_ms
            );
        }
    }
}
