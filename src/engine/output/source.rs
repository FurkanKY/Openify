//! Ses iş parçacığı: halkadan okuyan rodio kaynağı (`RingSource`). Bkz.
//! `mod.rs` başlığı.

use std::{
    sync::{Arc, atomic::Ordering::*},
    time::Duration,
};

use super::{
    BLOCK, BURST_GAP_NS, CHANNELS, DEFAULT_PERIOD_NS, FADE_FRAMES, GAIN_RAMP_FRAMES, OutputCtl,
    RATE, XRUN_GROW_MS, frames_to_ns, ms_to_frames,
};

use super::{eq::Eq, karaoke::Karaoke};

/// `RingSource::fill`in bir blok için verdiği karar (`plan` aşaması).
pub(super) struct Plan {
    /// Geri çağrı anının saati (ns).
    now: u64,
    /// Ses düzeyi hedefi.
    gain_target: f32,
    /// Zarf iner: flush, emekli kuşak, durdurma ya da duck.
    hold: bool,
    /// Halkanın okuma ve yazma uçları (kare).
    r: u64,
    w: u64,
    /// Halkada bu blok için hazır kare (en çok `BLOCK`).
    avail: usize,
    /// Blok tamamen sessiz: `buf` sıfırlandı, çekilecek kare yok.
    silent: bool,
    /// Ekolayzır ve karaoke çalışmıyor: ham kare `pull`da hemen çarpılır (aynı
    /// işlem, ayrı bir geçiş yok); `shape` yalnız kuyruğu sıfırlar.
    direct: bool,
}

/// Bu bloğun kareleri hangi halka yuvasında (bkz. `OutputCtl::reserve`).
const LOW: u8 = 0;
const HIGH: u8 = 1;
const BOTH: u8 = 2;

/// Halkadan okuyan rodio kaynağı. `next` ses kartının geri çağrısında,
/// örnek örnek çağrılır: kilit yok, bellek ayırma yok, bekleme yok; asla
/// `None` dönmez.
pub(in crate::engine) struct RingSource {
    ctl: Arc<OutputCtl>,
    /// Çıkış bloğu (L, R, L, R, …). `pull` ile `shape` arasında ham kareler.
    pub(super) buf: [f32; BLOCK * CHANNELS],
    /// Her karenin çarpanı (ses düzeyi × zarf), `pull` koyar, `shape` uygular.
    coef: [f32; BLOCK],
    /// Ekolayzır (kapalıyken hiç çalışmaz: `Plan::direct`).
    pub(super) eq: Eq,
    /// Karaoke (yalnız açıkken çalışır: `Plan::direct`).
    pub(super) karaoke: Karaoke,
    pos: usize,
    /// İniş / çıkış zarfı (0..1).
    env: f32,
    gain: f32,
    gain_bits: u32,
    gain_step: f32,
    gain_left: u32,
    flush_done: u64,
    held: bool,
    /// Bekletmeden çıktı; ilk gerçek kare çalınınca iz "audible" yazar.
    resumed: bool,
    /// Bu çalma döneminde gerçek ses çalındı (başta veri beklemek xrun değil).
    primed: bool,
    starving: bool,
    starve_frames: u64,
    last_fill: u64,
    burst: u64,
    period_ns: f64,
    next_cb: f64,
    /// Kaynağın kuşağı: yalnız `OutputCtl::source_gen` olan, bir öncekisi
    /// bıraktıktan sonra halkadan okur. Eski kuşak (aygıt değişti) inişle
    /// susup bırakır, bir daha okumaz (`retired`).
    epoch: u64,
    retired: bool,
}

impl RingSource {
    /// Etkin kuşağın kaynağı (testler).
    #[cfg(test)]
    pub fn new(ctl: Arc<OutputCtl>) -> Self {
        let epoch = ctl.source_gen.load(SeqCst);
        Self::with_gen(ctl, epoch)
    }

    /// `epoch`: etkin kuşaktan büyükse kaynak bekleme kipinde başlar (yeni
    /// aygıt; bkz. `Device::switch`).
    pub(super) fn with_gen(ctl: Arc<OutputCtl>, epoch: u64) -> Self {
        let gain_bits = ctl.gain.load(SeqCst);
        // 0: bir `flush` hiç görülmemiş say. Aygıt değişiminde eski kaynağın
        // uygulamadığı (ya da yazarın henüz onaylamadığı) atma burada yapılır.
        let flush_done = 0;
        Self {
            ctl,
            buf: [0.0; BLOCK * CHANNELS],
            coef: [0.0; BLOCK],
            eq: Eq::new(),
            karaoke: Karaoke::new(),
            pos: BLOCK * CHANNELS,
            env: 0.0,
            gain: f32::from_bits(gain_bits),
            gain_bits,
            gain_step: 0.0,
            gain_left: 0,
            flush_done,
            // `OutputCtl::held` ile aynı başlar (çalar `start` demeden sessiz).
            held: true,
            resumed: false,
            primed: false,
            starving: false,
            starve_frames: 0,
            last_fill: 0,
            burst: 0,
            period_ns: DEFAULT_PERIOD_NS,
            next_cb: 0.0,
            epoch,
            retired: false,
        }
    }

    /// Bir blok (`BLOCK` kare) üretir: üç aşama.
    ///
    /// 1. `plan`: saat, ses düzeyi hedefi, kuşak, flush / bekletme kararı.
    ///    Blok tamamen sessizse `buf`ı sıfırlayıp biter.
    /// 2. `pull`: halkadan ham kareleri `buf`a, her karenin çarpanını
    ///    (`coef[i]` = ses düzeyi × zarf) `coef`a koyar; xrun, çapa ve yazarı
    ///    uyandırma burada. Kaç kare çekildiğini döner.
    /// 3. `shape`: (varsa ekolayzır, ham kareler üstünde), sonra `buf = ham ×
    ///    coef`, gerisi sıfır.
    pub(super) fn fill(&mut self) {
        self.pos = 0;
        let now = self.ctl.now_ns();
        let plan = self.plan(now);
        if plan.silent {
            return;
        }
        let n = self.pull(&plan);
        self.shape(&plan, n);
    }

    /// 1. aşama: bu bloğun kararı. `now`: geri çağrı anının saati (ns).
    fn plan(&mut self, now: u64) -> Plan {
        let ctl = &self.ctl;

        // Geri çağrı saati: patlamanın başı, sonraki patlamanın tahmini.
        if self.last_fill == 0 || now.saturating_sub(self.last_fill) > BURST_GAP_NS {
            if self.burst != 0 {
                let dt = now.saturating_sub(self.burst) as f64;
                if (1e6..=1e8).contains(&dt) {
                    self.period_ns += (dt - self.period_ns) * 0.1;
                }
            }
            self.burst = now;
            self.next_cb = now as f64 + self.period_ns;
        }
        self.last_fill = now;

        // Ses düzeyi hedefi değişti: 10 ms'lik doğrusal rampa.
        let gain_bits = ctl.gain.load(Relaxed);
        if gain_bits != self.gain_bits {
            self.gain_bits = gain_bits;
            self.gain_step = (f32::from_bits(gain_bits) - self.gain) / GAIN_RAMP_FRAMES as f32;
            self.gain_left = GAIN_RAMP_FRAMES;
            ctl.t_gain.store(now.max(1), Relaxed);
        }
        let gain_target = f32::from_bits(self.gain_bits);
        // Yeni ekolayzır ayarı yayınlandıysa al (blok başına bir atomik okuma).
        self.eq.poll(&ctl.eq);
        self.karaoke.poll(&ctl.karaoke);
        let silent = |r: u64| Plan {
            now,
            gain_target,
            hold: true,
            r,
            w: r,
            avail: 0,
            silent: true,
            direct: true,
        };

        // Aygıt değişimi: bekleme kipindeki yeni kaynak (önceki henüz
        // bırakmadı) ve emekli eski kaynak halkaya dokunmaz, sessiz çalar.
        let active = ctl.source_gen.load(SeqCst);
        if self.retired
            || self.epoch > active
            || (self.epoch == active && ctl.released_gen.load(SeqCst) + 1 < self.epoch)
        {
            self.buf = [0.0; BLOCK * CHANNELS];
            return silent(0);
        }
        let retiring = self.epoch < active;

        let req = ctl.flush_req.load(SeqCst);
        let flushing = req != self.flush_done;
        let hold = flushing || retiring || !ctl.running.load(SeqCst) || ctl.duck.load(SeqCst);
        let mut r = ctl.read.load(SeqCst);

        if hold && self.env <= 0.0 {
            if retiring {
                // İniş bitti: yeni aygıtın kaynağı bu kareden sürer.
                self.retired = true;
                ctl.held.store(true, SeqCst);
                ctl.released_gen.fetch_max(self.epoch, SeqCst);
                self.buf = [0.0; BLOCK * CHANNELS];
                return silent(r);
            }
            // Tamamen sessiz: kuyruğa dokunma (devamda kaldığı kareden sürer).
            if !self.held {
                self.held = true;
                ctl.held.store(true, SeqCst);
                ctl.t_silent.store(now.max(1), Relaxed);
            }
            self.primed = false;
            self.starving = false;
            self.starve_frames = 0;
            if flushing && ctl.flush_ack.load(SeqCst) == req {
                // Yazar onayladı: eski sesi at.
                let to = ctl.discard_to.load(SeqCst);
                if to > r {
                    r = to;
                    ctl.read.store(r, SeqCst);
                }
                self.flush_done = req;
                // Kuyruk atıldı, akış kesildi: ekolayzırın çınlaması yeni sese sızmasın.
                self.eq.reset();
                self.karaoke.reset();
            }
            self.gain = gain_target;
            self.gain_left = 0;
            self.buf = [0.0; BLOCK * CHANNELS];
            self.publish_anchor(r as f64);
            return silent(r);
        }
        if self.held {
            self.held = false;
            ctl.held.store(false, SeqCst);
            self.resumed = true;
        }

        let w = ctl.written.load(SeqCst);
        let avail = (w - r.min(w)).min(BLOCK as u64) as usize;
        Plan {
            now,
            gain_target,
            hold,
            r,
            w,
            avail,
            silent: false,
            direct: !(self.eq.running() || self.karaoke.running()),
        }
    }

    /// 2. aşama: halkadan ham kareleri `buf`a, her karenin çarpanını `coef`a
    /// koyar (`coef[i] = ses düzeyi_i × zarf_i`, f32 işlem sırası eskisiyle
    /// aynı). Çekilen kare sayısı döner; ilk `n` kare gerçek, gerisi
    /// `shape`te sıfırlanır. Ekolayzır yokken (`plan.direct`) kare başına
    /// dal olmasın diye döngü iki kez derlenir: ham kare orada hemen çarpılır.
    fn pull(&mut self, plan: &Plan) -> usize {
        // Halka tek yuvadayken (her zamanki durum) blok başına tek atomik okuma.
        // Yazar ikinci yuvaya geçtiyse (`big_from`) blok ya tümüyle birinde ya
        // da (geçişin tam üstünde, bir kez) ikisinde: kare başına dal yalnız o
        // bloğun döngüsünde.
        let split = self.ctl.big_from.load(SeqCst);
        let mode = if split == u64::MAX || plan.r + plan.avail as u64 <= split {
            LOW
        } else if plan.r >= split {
            HIGH
        } else {
            BOTH
        };
        match (plan.direct, mode) {
            (true, LOW) => self.pull_with::<true, LOW>(plan, split),
            (true, HIGH) => self.pull_with::<true, HIGH>(plan, split),
            (true, _) => self.pull_with::<true, BOTH>(plan, split),
            (false, LOW) => self.pull_with::<false, LOW>(plan, split),
            (false, HIGH) => self.pull_with::<false, HIGH>(plan, split),
            (false, _) => self.pull_with::<false, BOTH>(plan, split),
        }
    }

    fn pull_with<const DIRECT: bool, const SLOT: u8>(&mut self, plan: &Plan, split: u64) -> usize {
        let ctl = &self.ctl;
        let low = &ctl.ring;
        // `HIGH` / `BOTH` yalnız `big_from` konduktan sonra (yazar `big`i koyduktan
        // sonra) seçilir: yuva vardır.
        let high = ctl.big.get().unwrap_or(low);
        let Plan {
            now,
            gain_target,
            hold,
            r,
            w,
            avail,
            ..
        } = *plan;
        let mut r = r;
        let env_step = 1.0 / FADE_FRAMES as f32;
        let was_starving = self.starving;
        let mut produced = 0usize;
        let mut stopped = false;
        for i in 0..BLOCK {
            if self.gain_left > 0 {
                self.gain_left -= 1;
                self.gain = if self.gain_left == 0 {
                    gain_target
                } else {
                    self.gain + self.gain_step
                };
            }
            if stopped {
                continue;
            }
            self.env = if hold {
                (self.env - env_step).max(0.0)
            } else {
                (self.env + env_step).min(1.0)
            };
            if hold && self.env <= 0.0 {
                // İniş bitti: bu kare çalınmadı, devamda bundan sürer.
                stopped = true;
            } else if i >= avail {
                // Veri yok: sıfır. Akış ortasındaysa kesinti (xrun).
                if !hold && self.primed {
                    self.starving = true;
                    self.starve_frames += (BLOCK - i) as u64;
                }
                // Veri gelince yumuşak giriş.
                self.env = 0.0;
                stopped = true;
            } else {
                let frame = r + i as u64;
                let c = self.gain * self.env;
                let (left, right) = match SLOT {
                    LOW => low.load(frame),
                    HIGH => high.load(frame),
                    _ if frame >= split => high.load(frame),
                    _ => low.load(frame),
                };
                if DIRECT {
                    self.buf[i * CHANNELS] = left * c;
                    self.buf[i * CHANNELS + 1] = right * c;
                } else {
                    self.coef[i] = c;
                    self.buf[i * CHANNELS] = left;
                    self.buf[i * CHANNELS + 1] = right;
                }
                produced += 1;
                self.primed = true;
            }
        }

        if produced > 0 {
            let start = r;
            r += produced as u64;
            ctl.read.store(r, SeqCst);
            ctl.played.fetch_add(produced as u64, Relaxed);
            if was_starving {
                // Kesinti bitti: say, hedefi büyüt (yazar uyarı yazar).
                self.starving = self.starving && produced < BLOCK;
                ctl.xruns.fetch_add(1, Relaxed);
                ctl.underrun_frames.fetch_add(self.starve_frames, Relaxed);
                self.starve_frames = 0;
                ctl.t_xrun.store(now.max(1), Relaxed);
                let target = ctl.target.load(Relaxed);
                if target < ctl.target_limit {
                    ctl.target.store(
                        (target + ms_to_frames(XRUN_GROW_MS)).min(ctl.target_limit),
                        Relaxed,
                    );
                }
            }
            if self.resumed {
                self.resumed = false;
                ctl.t_audible.store(now.max(1), Relaxed);
            }
            if ctl.writer_waiting.load(SeqCst)
                && w.saturating_sub(r) <= ctl.target_frames()
                && let Some(t) = ctl.writer.get()
            {
                t.unpark();
            }
            // Bu bloğun kareleri bu ya da sonraki geri çağrıda verilir: orta.
            self.publish_anchor(start as f64 + produced as f64 / 2.0);
        } else {
            self.publish_anchor(r as f64);
        }
        produced
    }

    /// 3. aşama: `buf`taki ilk `n` ham kareyi `coef` ile çarpar (varsa önce
    /// ekolayzır, sonra karaoke, ham kareler üstünde), gerisini sıfırlar.
    /// İkisi de yokken (`plan.direct`) çarpma `pull`da yapılmıştır, burada
    /// yalnız kuyruk sıfırlanır.
    fn shape(&mut self, plan: &Plan, n: usize) {
        if !plan.direct {
            if n > 0 {
                if self.eq.running() {
                    self.eq.process(&mut self.buf, n);
                }
                if self.karaoke.running() {
                    self.karaoke.process(&mut self.buf, n);
                }
            }
            for (frame, c) in self.buf.chunks_exact_mut(CHANNELS).zip(&self.coef).take(n) {
                frame[0] *= *c;
                frame[1] *= *c;
            }
        }
        if n < BLOCK {
            self.buf[n * CHANNELS..].fill(0.0);
        }
    }

    /// `next_cb` anında `boundary` karesine gelinmiş olacak.
    fn publish_anchor(&self, boundary: f64) {
        let anchor = self.next_cb - frames_to_ns(boundary);
        self.ctl.anchor.store(anchor as i64, SeqCst);
    }
}

/// Ölçüm (çıta: EQ kapalı `fill` ≤ +%5): yuva seçimi (`pull`) eklenmeden
/// ÖNCEKİ `fill`in aynısı (halka tek yuva, sabit `MASK`). `xfade_tests.rs`
/// ikisini aynı süreçte, dönüşümlü çalıştırıp oranlar.
#[cfg(test)]
impl RingSource {
    pub(super) fn fill_baseline(&mut self) {
        self.pos = 0;
        let now = self.ctl.now_ns();
        let plan = self.plan(now);
        if plan.silent {
            return;
        }
        let n = if plan.direct {
            self.pull_baseline::<true>(&plan)
        } else {
            self.pull_baseline::<false>(&plan)
        };
        self.shape(&plan, n);
    }

    fn pull_baseline<const DIRECT: bool>(&mut self, plan: &Plan) -> usize {
        const MASK: u64 = super::CAP_FRAMES - 1;
        let ctl = &self.ctl;
        let cells = ctl.ring.raw();
        let Plan {
            now,
            gain_target,
            hold,
            r,
            w,
            avail,
            ..
        } = *plan;
        let mut r = r;
        let env_step = 1.0 / FADE_FRAMES as f32;
        let was_starving = self.starving;
        let mut produced = 0usize;
        let mut stopped = false;
        for i in 0..BLOCK {
            if self.gain_left > 0 {
                self.gain_left -= 1;
                self.gain = if self.gain_left == 0 {
                    gain_target
                } else {
                    self.gain + self.gain_step
                };
            }
            if stopped {
                continue;
            }
            self.env = if hold {
                (self.env - env_step).max(0.0)
            } else {
                (self.env + env_step).min(1.0)
            };
            if hold && self.env <= 0.0 {
                stopped = true;
            } else if i >= avail {
                if !hold && self.primed {
                    self.starving = true;
                    self.starve_frames += (BLOCK - i) as u64;
                }
                self.env = 0.0;
                stopped = true;
            } else {
                let cell = (((r + i as u64) & MASK) as usize) * CHANNELS;
                let c = self.gain * self.env;
                let left = f32::from_bits(cells[cell].load(Relaxed));
                let right = f32::from_bits(cells[cell + 1].load(Relaxed));
                if DIRECT {
                    self.buf[i * CHANNELS] = left * c;
                    self.buf[i * CHANNELS + 1] = right * c;
                } else {
                    self.coef[i] = c;
                    self.buf[i * CHANNELS] = left;
                    self.buf[i * CHANNELS + 1] = right;
                }
                produced += 1;
                self.primed = true;
            }
        }
        if produced > 0 {
            let start = r;
            r += produced as u64;
            ctl.read.store(r, SeqCst);
            ctl.played.fetch_add(produced as u64, Relaxed);
            if was_starving {
                self.starving = self.starving && produced < BLOCK;
                ctl.xruns.fetch_add(1, Relaxed);
                ctl.underrun_frames.fetch_add(self.starve_frames, Relaxed);
                self.starve_frames = 0;
                ctl.t_xrun.store(now.max(1), Relaxed);
                let target = ctl.target.load(Relaxed);
                if target < ctl.target_limit {
                    ctl.target.store(
                        (target + ms_to_frames(XRUN_GROW_MS)).min(ctl.target_limit),
                        Relaxed,
                    );
                }
            }
            if self.resumed {
                self.resumed = false;
                ctl.t_audible.store(now.max(1), Relaxed);
            }
            if ctl.writer_waiting.load(SeqCst)
                && w.saturating_sub(r) <= ctl.target_frames()
                && let Some(t) = ctl.writer.get()
            {
                t.unpark();
            }
            self.publish_anchor(start as f64 + produced as f64 / 2.0);
        } else {
            self.publish_anchor(r as f64);
        }
        produced
    }
}

impl Iterator for RingSource {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.pos == BLOCK * CHANNELS {
            self.fill();
        }
        let sample = self.buf[self.pos];
        self.pos += 1;
        Some(sample)
    }
}

impl rodio::Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        CHANNELS as rodio::ChannelCount
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
