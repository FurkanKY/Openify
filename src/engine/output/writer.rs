//! Yazar: çalar iş parçacığında yaşayan, halkaya yazan taraf (kapı, söz saati
//! tabanı, kuyruk hedefini bekleme, geçişte bindirme). Bkz. `mod.rs` başlığı.

use std::{
    borrow::Cow,
    sync::{Arc, atomic::Ordering::*},
    thread,
    time::{Duration, Instant},
};

use librespot::playback::{
    audio_backend::{SinkError, SinkResult},
    player::{PlayerEvent, PlayerEventChannel},
};

use super::{
    automix::{LeadSkip, TailTrim},
    beat::Analysis,
    CHANNELS, Curve, DEVICE_LOST, FADE_FRAMES, GATE_LIMIT_FRAMES, GUARD_FRAMES, MIN_OVERLAP_FRAMES, OutputCtl, PARK,
    PRODUCER_GAP_NS, RATE, STALL, TAIL_MS, Transition, mark_of, ms_to_frames,
};

/// Bindirmede `read`i yeniden okuma aralığı (kare): yazar her dilimin başında
/// ses iş parçacığının nerede olduğuna yeniden bakar.
const OVERLAP_CHUNK: u64 = 512;

/// Süren bindirme: yeni şarkının `k`. karesi halkada `start + k`'da, orada
/// duran giden sesle karışır.
struct Xf {
    start: u64,
    overlap: u64,
    /// Yeni şarkının şimdiye dek işlenen karesi (0..overlap).
    k: u64,
    curve: Curve,
    /// Geç gelen kareler atlandıysa (`start + k` okuma ucunun payına girdi):
    /// karışımın yeniden girişi için kalan yumuşatma karesi.
    blend: u32,
}

/// Yeni şarkının ilk paketi geldiğinde bilinenler (`Writer::pending_track`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::engine) struct NewTrack {
    /// Çaların `TrackChanged`inden şarkı adresi.
    pub uri: Option<String>,
    /// Şarkı 0. msten başladı.
    pub from_start: bool,
    /// Önceki şarkı kendiliğinden bitti (`EndOfTrack`): sonu çözümlemeye değer.
    pub ended: bool,
}

enum Gate {
    Open,
    /// `flush`tan sonra: önce `marker`, sonra komutun sonucu görülene dek
    /// paketler atılır.
    Closed { marker: u64, marker_seen: bool, dropped: u64 },
}

/// Komutun çalarda işlendiğini gösteren olaylar (`flush`tan sonraki ilk
/// komut bunlardan birini mutlaka gönderir: `load` → `PlayRequestIdChanged`,
/// `seek` → `Seeked`; yükleme sürerken `seek` → `PlayRequestIdChanged`).
fn confirms(event: &PlayerEvent) -> bool {
    matches!(
        event,
        PlayerEvent::PlayRequestIdChanged { .. }
            | PlayerEvent::Seeked { .. }
            | PlayerEvent::Loading { .. }
            | PlayerEvent::Playing { .. }
            | PlayerEvent::Paused { .. }
            | PlayerEvent::Stopped { .. }
            | PlayerEvent::TrackChanged { .. }
            | PlayerEvent::Unavailable { .. }
    )
}

/// Çıkışın yazan tarafı; yalnız çalar iş parçacığında yaşar.
pub(in crate::engine) struct Writer {
    ctl: Arc<OutputCtl>,
    flush_seen: u64,
    duck_seen: u64,
    duck_marker_seen: bool,
    gate: Gate,
    events: Option<PlayerEventChannel>,
    xruns_seen: u64,
    /// `TrackChanged` görüldü: sıradaki `Playing` şarkının başlangıç
    /// konumunu verir (söz saati tabanı, bkz. `OutputCtl::audible_track_ms`).
    track_fresh: bool,
    /// Yeni şarkının ilk paketi henüz yazılmadı (iz: `first_write`).
    pub(super) first_write_pending: bool,
    /// `begin_packet` bu paketi yeni şarkının ilki olarak işledi (iz ve üretici
    /// boşluğu için; `push` alır).
    first_now: bool,
    /// Şarkı kendiliğinden bitti (`EndOfTrack`): şimdiye dek yazılanın hepsi
    /// biten şarkının, yani sıradaki şarkı tam bu kareden başlar (çalar
    /// `EndOfTrack` ile sıradaki `load` arasında hiç yazmaz). `flush` siler.
    pub(super) eot: Option<u64>,
    /// Yeni şarkının süresi (kare; `TrackChanged`ten) ve gelen `Playing`in
    /// başlangıcı: bindirme şarkıların yarısından uzun olmasın.
    next_len: Option<u64>,
    /// Yeni şarkı `eot`tan hemen sonra, baştan başladı: bindirme olabilir.
    pub(super) xf_candidate: bool,
    /// Yeni şarkı tabanı konmadan önceki (giden) şarkının tabanı.
    prev_base: i64,
    xf: Option<Xf>,
    /// Yeni şarkının ilk karesi ve o an açık olan şarkı sonu payının
    /// kuşağı: yeni şarkı payı kendi sesiyle doldurunca pay kapanır.
    pub(super) refill: Option<(u64, u64)>,
    /// Son yazmanın bittiği an (kuyruğu bekleme dahil); çalarken bir
    /// sonraki paket bundan `PRODUCER_GAP_NS` sonra gelirse iz satırı.
    last_free: Option<u64>,
    /// Geliştirme kancası `OPENIFY_TEST_TRANSITION_STALL_MS`: yeni şarkının
    /// ilk paketinden önce bu kadar uyu (diskte takılan çözücü gibi).
    stall: Option<Duration>,
    /// AutoMix: son `TrackChanged`in şarkı adresi, son `Playing`in başlangıç konumu.
    next_uri: Option<String>,
    start_ms: u32,
    /// Biten şarkının canlı çözümlemesi (`tap.rs` koyar; geçiş kurulurken tüketilir).
    prev_analysis: Option<Analysis>,
    /// Sondaki sessizliği yazmama; yeni şarkıda `tail_gen` bu değerden farklı olunca
    /// (yani motor SIRADAKİ geçiş için payı yeniden açınca) devreye girer.
    trim: TailTrim,
    trim_gen: u64,
    /// Gelen şarkının baştaki sessizliğini atma ve atılan kareyle düzeltilecek şarkı
    /// tabanının dayanağı (yeni şarkının sessizlik atılmamış 0. karesinin halkadaki yeri).
    lead: Option<LeadSkip>,
    lead_anchor: i64,
}

impl Writer {
    pub fn new(ctl: Arc<OutputCtl>) -> Self {
        let stall = std::env::var("OPENIFY_TEST_TRANSITION_STALL_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis);
        Self::with_stall(ctl, stall)
    }

    pub(super) fn with_stall(ctl: Arc<OutputCtl>, stall: Option<Duration>) -> Self {
        if let Some(d) = stall {
            log::warn!("OPENIFY_TEST_TRANSITION_STALL_MS: her yeni şarkının ilk paketinden önce {} ms uyunacak", d.as_millis());
        }
        Self {
            ctl,
            flush_seen: 0,
            duck_seen: 0,
            duck_marker_seen: false,
            gate: Gate::Open,
            events: None,
            xruns_seen: 0,
            track_fresh: false,
            first_write_pending: false,
            first_now: false,
            eot: None,
            next_len: None,
            xf_candidate: false,
            prev_base: i64::MIN,
            xf: None,
            refill: None,
            last_free: None,
            stall,
            next_uri: None,
            start_ms: 0,
            prev_analysis: None,
            trim: TailTrim::default(),
            trim_gen: 0,
            lead: None,
            lead_anchor: 0,
        }
    }

    pub fn ctl(&self) -> &Arc<OutputCtl> {
        &self.ctl
    }

    pub fn now_ns(&self) -> u64 {
        self.ctl.now_ns()
    }

    /// Şimdiye dek yazılan kare: sıradaki paketin ilk karesinin numarası.
    pub fn written(&self) -> u64 {
        self.ctl.written.load(SeqCst)
    }

    /// Sıradaki çalınacak kare (atılacak eski ses atlanmış).
    pub fn read_frame(&self) -> u64 {
        self.ctl.read.load(SeqCst).max(self.ctl.discard_to.load(SeqCst))
    }

    /// Motor komutlarını ve çalar olaylarını işler. Bir `flush` bu çağrıda
    /// onaylandıysa `true` (akış kesildi: seviye çözümleyici sıfırlanmalı).
    pub fn sync(&mut self) -> bool {
        let ctl = Arc::clone(&self.ctl);
        ctl.writer.get_or_init(thread::current);
        if self.events.is_none()
            && let Ok(mut slot) = ctl.events.lock()
        {
            self.events = slot.take();
        }

        let mut flushed = false;
        let req = ctl.flush_req.load(SeqCst);
        if req != self.flush_seen {
            // Şimdiye dek yazılanların hepsi eski: atılsın. Ses iş parçacığı
            // inişi bitirince `read`i buraya atlatır.
            self.flush_seen = req;
            ctl.discard_to.store(ctl.written.load(SeqCst), SeqCst);
            ctl.last_packet.store(0, SeqCst);
            // Yeni konum çaların olayıyla gelene dek söz saati bilinmiyor.
            ctl.track_base.store(i64::MIN, SeqCst);
            self.track_fresh = false;
            // Sarma / yükleme gecikmesi üretici boşluğu sayılmaz.
            self.last_free = None;
            // Atılan sesle birlikte süren bindirme ve biten şarkının sınırı da
            // gider; yazma ucu yeniden yazma ucuna eşit (`head`).
            self.eot = None;
            self.xf = None;
            self.xf_candidate = false;
            self.prev_analysis = None;
            self.lead = None;
            self.trim.reset();
            ctl.end_transition();
            ctl.head.store(ctl.written.load(SeqCst), SeqCst);
            ctl.flush_ack.store(req, SeqCst);
            self.gate = if self.events.is_some() {
                Gate::Closed { marker: req, marker_seen: false, dropped: 0 }
            } else {
                // Olay kanalı yok (terminal çalar): kapı yok, yarış kabul.
                Gate::Open
            };
            ctl.trace("flush_ack");
            flushed = true;
        }

        let duck_mark = ctl.duck_mark.load(SeqCst);
        if duck_mark != self.duck_seen {
            self.duck_seen = duck_mark;
            self.duck_marker_seen = false;
            // Duraklatılan süre üretici boşluğu değil.
            self.last_free = None;
        }

        if let Some(events) = &mut self.events {
            while let Ok(event) = events.try_recv() {
                if let Some(mark) = mark_of(&event) {
                    if let Gate::Closed { marker, marker_seen, .. } = &mut self.gate
                        && mark == *marker
                    {
                        *marker_seen = true;
                    }
                    if self.duck_seen != 0 && mark == self.duck_seen {
                        self.duck_marker_seen = true;
                    }
                    continue;
                }
                if matches!(self.gate, Gate::Closed { marker_seen: true, .. }) && confirms(&event) {
                    self.gate = Gate::Open;
                    ctl.trace("gate_open");
                }
                // Söz saati: konum bildiren olaylar yazmadan ÖNCE gelir, yani
                // sıradaki yazılacak kare o konumdur. İşaretten önceki (atılacak
                // eski sese ait) olaylar sayılmaz. Devamdaki `Playing` (çözülmüş
                // son paketin başı) taban değiştirmez: kuyruk atılmadı, aynı.
                if !matches!(self.gate, Gate::Closed { marker_seen: false, .. }) {
                    match &event {
                        PlayerEvent::EndOfTrack { .. } => self.eot = Some(ctl.written.load(SeqCst)),
                        PlayerEvent::TrackChanged { audio_item } => {
                            self.track_fresh = true;
                            self.next_uri = Some(audio_item.uri.clone());
                            self.next_len = Some(ms_to_frames(audio_item.duration_ms as u64));
                        }
                        PlayerEvent::Playing { position_ms, .. } if self.track_fresh => {
                            self.track_fresh = false;
                            self.start_ms = *position_ms;
                            self.prev_base = ctl.track_base.load(SeqCst);
                            ctl.set_track_base(*position_ms);
                            self.first_write_pending = true;
                            self.xf_candidate = self.eot.is_some() && *position_ms == 0;
                            self.refill = ctl
                                .tail
                                .load(SeqCst)
                                .then(|| (ctl.written.load(SeqCst), ctl.tail_gen.load(SeqCst)));
                        }
                        PlayerEvent::Seeked { position_ms, .. }
                        | PlayerEvent::PositionCorrection { position_ms, .. } => {
                            ctl.set_track_base(*position_ms);
                        }
                        _ => {}
                    }
                }
                // Duraklatma istendi ama çalar sonra yine de çalmaya geçti
                // (örn. duraklatma şarkı sonuna denk geldi, sıradaki başladı):
                // susturma kalksın, yoksa "çalıyor" görünüp sessiz kalırdı.
                if self.duck_marker_seen && matches!(event, PlayerEvent::Playing { .. }) {
                    self.duck_marker_seen = false;
                    ctl.unduck();
                }
            }
        }

        let xruns = ctl.xruns.load(Relaxed);
        if xruns != self.xruns_seen {
            self.xruns_seen = xruns;
            log::warn!(
                "Ses çıkışı boşaldı (xrun #{xruns}, toplam {:.0} ms sessizlik); kuyruk hedefi {:.0} ms",
                ctl.underrun_frames.load(Relaxed) as f64 * 1000.0 / RATE as f64,
                ctl.target_frames() as f64 * 1000.0 / RATE as f64,
            );
        }
        flushed
    }

    /// Bu paket çalınsın mı (kapı)? `false`: `flush`tan önce çözülmüş eski
    /// ses, atılır.
    pub fn accept(&mut self, frames: u64) -> bool {
        match &mut self.gate {
            Gate::Open => true,
            Gate::Closed { dropped, .. } => {
                *dropped += frames;
                if *dropped > GATE_LIMIT_FRAMES {
                    log::warn!("Ses çıkışı: komutun sonucu gelmedi, kapı kendiliğinden açıldı");
                    self.gate = Gate::Open;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Yeni şarkının ilk paketinin işi (paket başına en çok bir kez; `push` da
    /// çağırır, `tap::LevelSink::write` seviye çözümlemesinden ÖNCE çağırır ki
    /// bindirilecek kare sayısı bilinsin): geliştirme kancasının uykusu ve
    /// bindirmeli geçişin kurulması. Geçişin süresi, uykudan SONRA, okuma ucuna
    /// göre belirlenir (takılma payı yer; bkz. `start_transition`).
    pub fn begin_packet(&mut self) {
        if !std::mem::take(&mut self.first_write_pending) {
            return;
        }
        self.first_now = true;
        self.next_uri = None;
        self.trim.reset();
        self.trim_gen = self.ctl.tail_gen.load(SeqCst);
        self.lead = None;
        if let Some(stall) = self.stall {
            thread::sleep(stall);
        }
        self.start_transition();
    }

    /// Yeni şarkının ilk paketi geldi ama `begin_packet` henüz işlemedi: o şarkı
    /// hakkında bilinenler (AutoMix çözümleyicisi önceki şarkıyı kapatıp yenisini
    /// başlatır; bkz. `tap.rs`).
    pub(in crate::engine) fn pending_track(&self) -> Option<NewTrack> {
        self.first_write_pending.then(|| NewTrack {
            uri: self.next_uri.clone(),
            from_start: self.start_ms == 0,
            ended: self.eot.is_some(),
        })
    }

    /// Biten şarkının canlı çözümlemesi: sıradaki `begin_packet` geçişi bununla kurar.
    pub(in crate::engine) fn set_prev_analysis(&mut self, analysis: Option<Analysis>) {
        self.prev_analysis = analysis;
    }

    /// AutoMix sessizlik atlama: paketi halkaya girecek biçime getirir. Gelen şarkının
    /// baştaki sessizliği atılır; giden şarkının sonundaki uzun sessizlik (pay açıkken,
    /// bir sonraki geçiş AutoMix ise) yazılmaz. Değişiklik yoksa paketin kendisi döner,
    /// ayırma yok. Boş dönerse (hepsi atıldı) yazılacak bir şey yok.
    pub fn prepare<'a>(&mut self, samples: &'a [f64]) -> Cow<'a, [f64]> {
        let mut rest = samples;
        if let Some(lead) = self.lead.as_mut() {
            let before = lead.dropped;
            rest = lead.skip(samples);
            if lead.dropped != before {
                // Yeni şarkının 0. karesi atılan kadar geride kalır.
                self.ctl.track_base.store(self.lead_anchor - lead.dropped as i64, SeqCst);
            }
            if lead.is_done() {
                self.lead = None;
            }
        }
        if self.trim_active() { self.trim.apply(rest) } else { Cow::Borrowed(rest) }
    }

    fn trim_active(&self) -> bool {
        let ctl = &self.ctl;
        ctl.automix_on()
            && self.lead.is_none()
            && self.eot.is_none()
            && ctl.tail_gen.load(SeqCst) != self.trim_gen
            && ctl.armed_spec().auto
    }

    /// Bu paketin baştaki kaç karesi giden sesin üstüne bindirilecek (halkaya
    /// EKLENMEZ, karışır; yazma ucu ilerlemez). Seviye çözümleyici bunları
    /// atlar (bkz. `tap.rs`).
    pub fn overlap_in(&self, frames: u64) -> u64 {
        self.xf.as_ref().map_or(0, |xf| (xf.overlap - xf.k).min(frames))
    }

    /// Örnekleri halkaya koyar (yer yoksa yer açılana dek bekler). Bu arada
    /// bir `flush` onaylandıysa paketin kalanı atılır ve `true` döner.
    ///
    /// Bindirme sürüyorsa paketin başı (`overlap_in` kare) halkada zaten duran
    /// giden sesin üstüne karışır, kalanı her zamanki gibi sona eklenir.
    pub fn push(&mut self, samples: &[f64]) -> SinkResult<bool> {
        let frames = (samples.len() / CHANNELS) as u64;
        self.begin_packet();
        let first = std::mem::take(&mut self.first_now);
        let now = self.ctl.now_ns();
        if let Some(free) = self.last_free.take()
            && self.is_live()
            && now.saturating_sub(free) > PRODUCER_GAP_NS
        {
            self.ctl.trace_arg("producer_gap", (now - free) as f64 / 1e6);
        }
        if first {
            self.ctl.trace("first_write");
        }
        self.grow();
        let mut done = self.mix_overlap(samples, frames);
        while done < frames {
            let ctl = Arc::clone(&self.ctl);
            let w = ctl.written.load(SeqCst);
            let used = w - ctl.read.load(SeqCst);
            let cap = ctl.write_cap();
            let free = cap - used.min(cap);
            if free == 0 {
                if self.block_until(|c| c.written.load(SeqCst) - c.read.load(SeqCst) < c.write_cap())? {
                    return Ok(true);
                }
                continue;
            }
            let n = free.min(frames - done);
            // `grow` yalnız paket başında, yazma ucunda yuva değiştirir: bu
            // dilimin hepsi aynı yuvada.
            let ring = ctl.ring_for(w);
            for i in 0..n {
                let src = ((done + i) as usize) * CHANNELS;
                ring.store(w + i, samples[src] as f32, samples[src + 1] as f32);
            }
            ctl.head.store(w + n, SeqCst);
            ctl.written.store(w + n, SeqCst);
            done += n;
        }
        let ctl = &self.ctl;
        ctl.last_packet.store(frames, SeqCst);
        // Boşluksuz geçişten sonra yeni şarkı payı kendi sesiyle doldurdu:
        // hedef 120 ms'ye döner (kuyruk kendiliğinden erir). Bu arada pay
        // yeniden açıldıysa (çok kısa şarkı) dokunma.
        if let Some((start, tail_gen)) = self.refill
            && ctl.written.load(SeqCst).saturating_sub(start) >= ms_to_frames(TAIL_MS)
        {
            self.refill = None;
            if ctl.tail_gen.load(SeqCst) == tail_gen {
                ctl.set_tail(false);
            }
        }
        Ok(false)
    }

    /// Halka büyütüldüyse (`OutputCtl::reserve`) ikinci yuvaya geç: yazma
    /// ucundan itibaren yeni kareler orada. Eski yuvadaki kuyruk yerinde kalır
    /// (ses iş parçacığı sırayla ikisini de okur); bindirme sürerken geçilmez.
    fn grow(&mut self) {
        let ctl = &self.ctl;
        if self.xf.is_none() && ctl.big.get().is_some() && ctl.big_from.load(SeqCst) == u64::MAX {
            ctl.big_from.store(ctl.written.load(SeqCst), SeqCst);
            ctl.trace("ring_grow");
        }
    }

    /// Yeni şarkının ilk paketi geldi. Şarkı kendiliğinden bittiyse (`eot`) ve
    /// motor bindirme istediyse (`arm_transition`), geçişi kur: yeni şarkının
    /// 0. karesi `eot − overlap`'a, giden sesin son `overlap` karesinin üstüne.
    ///
    /// `overlap` istenenden kısa olabilir:
    /// - giden ses `read + GUARD_FRAMES`'ten öncesi değiştirilemez (ses iş
    ///   parçacığı orayı çalıyor): yeni şarkının ilk paketi geç geldiyse
    ///   (takılma) bindirme o kadar kısalır, hiç yer yoksa geçiş boşluksuz;
    /// - her iki şarkının yarısından uzun olmaz.
    ///
    /// Şarkı tabanı ve arayüzün şarkı değişim anı yeni şarkının GERÇEK
    /// başlangıcına (`start`) çekilir.
    fn start_transition(&mut self) {
        let ctl = Arc::clone(&self.ctl);
        let eot = self.eot.take();
        let candidate = std::mem::take(&mut self.xf_candidate);
        let next_len = self.next_len.take();
        let prev_live = self.prev_analysis.take();
        // İstek yalnız biten bir şarkıdan sonraki ilk pakette okunur (şarkının
        // KENDİ ilk paketi, kısa şarkıda `PreloadDue` hemen gelir, isteği
        // görmez) ve SİLİNMEZ: şarkı sonu payı bindirme bitene dek derin kalmalı
        // (yoksa yazar hedefin altına inene dek bekler, bindirme geç kalır);
        // pay kapanınca (`OutputCtl::set_tail(false)`) istek de silinir.
        let Some(eot) = eot.filter(|_| candidate) else { return };
        let spec = ctl.armed_spec();
        if spec.is_none() {
            return;
        }
        // Arada başka şey yazıldıysa (sınır artık `eot` değil) bindirme yok.
        if ctl.written.load(SeqCst) != eot {
            return;
        }
        // AutoMix: gelen şarkının baştaki sessizliği, bindirme kurulsun kurulmasın, atılır.
        if spec.auto && ctl.automix_on() {
            self.lead = Some(LeadSkip::default());
            self.lead_anchor = eot as i64;
        }
        let floor = self.read_frame() + GUARD_FRAMES;
        let prev_len = (self.prev_base != i64::MIN && (self.prev_base as u64) < eot).then(|| eot - self.prev_base as u64);
        // Planlanan bindirme: AutoMix'te biten şarkının canlı çözümlemesinden
        // (giden son ucu) ve önbellekteki gelen baş ucundan; çözümleme yoksa
        // istekteki sabit süre.
        let planned = match prev_live.as_ref().and_then(|a| a.plan(ctl.armed_incoming().as_ref())) {
            Some(plan) if spec.auto => {
                ctl.trace_arg(
                    if plan.aligned { "automix_aligned" } else { "automix" },
                    plan.overlap as f64 * 1000.0 / RATE as f64,
                );
                plan.overlap as u64
            }
            _ => spec.overlap as u64,
        };
        let want = planned.min(prev_len.map_or(u64::MAX, |l| l / 2)).min(next_len.map_or(u64::MAX, |l| l / 2));
        let room = eot.saturating_sub(floor);
        let overlap = want.min(room);
        if overlap < MIN_OVERLAP_FRAMES {
            return;
        }
        let start = eot - overlap;
        self.xf = Some(Xf { start, overlap, k: 0, curve: spec.curve, blend: 0 });
        ctl.begin_transition(Transition { start, overlap: overlap as u32 });
        // Yeni şarkının 0. karesi `start`ta (`set_track_base` `written`a göre koymuştu).
        ctl.track_base.store(start as i64, SeqCst);
        self.lead_anchor = start as i64;
        ctl.move_handover_boundary(start);
        ctl.head.store(start, SeqCst);
    }

    /// Paketin baştaki (en çok `overlap − k`) karesini giden sesle karıştırır:
    /// `halka[start + k] = halka × giden(t) + yeni × gelen(t)`. Yalnız
    /// `read + GUARD_FRAMES`'ten SONRAKİ kareler değiştirilir (ses iş parçacığı
    /// dilim başına `read`i yeniden okunarak izlenir); bu payın içine düşen
    /// geç kareler atlanır ve karışım 8 ms'lik yumuşatmayla yeniden girer.
    /// Bütün işlem atomik hücrelerde: ses iş parçacığı kilitsiz okumayı sürer.
    /// İşlenen kare sayısını döner.
    fn mix_overlap(&mut self, samples: &[f64], frames: u64) -> u64 {
        let Some(mut xf) = self.xf.take() else { return 0 };
        let ctl = Arc::clone(&self.ctl);
        let n = frames.min(xf.overlap - xf.k);
        let mut i = 0u64;
        while i < n {
            let chunk = (n - i).min(OVERLAP_CHUNK);
            let floor = self.read_frame() + GUARD_FRAMES;
            for j in i..i + chunk {
                let k = xf.k + j;
                let pos = xf.start + k;
                if pos < floor {
                    xf.blend = FADE_FRAMES;
                    continue;
                }
                let (mut out, mut inn) = xf.curve.gains((k as f64 + 0.5) / xf.overlap as f64);
                if xf.blend > 0 {
                    let w = 1.0 - xf.blend as f32 / FADE_FRAMES as f32;
                    out = out * w + (1.0 - w);
                    inn *= w;
                    xf.blend -= 1;
                }
                let ring = ctl.ring_for(pos);
                let (old_l, old_r) = ring.load(pos);
                let src = (j as usize) * CHANNELS;
                #[cfg(test)]
                ctl.min_lead.fetch_min(pos.saturating_sub(self.read_frame()), Relaxed);
                ring.store(pos, old_l * out + samples[src] as f32 * inn, old_r * out + samples[src + 1] as f32 * inn);
            }
            i += chunk;
        }
        xf.k += n;
        if xf.k < xf.overlap {
            ctl.head.store(xf.start + xf.k, SeqCst);
            self.xf = Some(xf);
        } else {
            ctl.head.store(ctl.written.load(SeqCst), SeqCst);
        }
        n
    }

    /// Kuyruk hedefe inene dek bekler. Çıkış bekletiliyorsa (duraklatma
    /// istendi, durdu) beklemez: çalar komutu işleyebilsin. Varsayılan aygıt
    /// değiştiyse de beklemez: sıradaki yazma yeni aygıta geçer. Bu arada
    /// bir `flush` onaylandıysa `true`.
    pub fn wait(&mut self) -> SinkResult<bool> {
        let flushed = self.block_until(|c| {
            !c.running.load(SeqCst) || c.duck.load(SeqCst) || c.queued_raw() <= c.target_frames() || c.reroute_due()
        })?;
        // Çözücü bundan sonra serbest: bir sonraki pakete dek geçen süre
        // üreticinin (çözme, disk, ağ).
        self.last_free = (!flushed && self.is_live()).then(|| self.ctl.now_ns());
        Ok(flushed)
    }

    /// Aygıt gittiyse hata (çalar duraklar; bkz. `OutputCtl::signal_device_lost`).
    pub fn check_device(&self) -> SinkResult<()> {
        if self.ctl.device_lost.load(SeqCst) {
            self.ctl.device_notice.store(DEVICE_LOST, SeqCst);
            return Err(SinkError::OnWrite("ses aygıtı ayrıldı".into()));
        }
        Ok(())
    }

    /// Çıkış çalıyor (durdurulmamış, susturulmamış).
    pub fn is_live(&self) -> bool {
        self.ctl.running.load(SeqCst) && !self.ctl.duck.load(SeqCst)
    }

    pub fn start(&mut self) {
        self.ctl.duck.store(false, SeqCst);
        self.ctl.running.store(true, SeqCst);
        self.last_free = None;
        self.ctl.trace("start");
    }

    pub fn stop(&mut self) {
        self.ctl.running.store(false, SeqCst);
        self.last_free = None;
        self.ctl.cut_levels();
        self.ctl.trace("stop");
    }

    /// `ready` doğru olana ya da bir `flush` onaylanana (`true`) dek uyur.
    fn block_until(&mut self, ready: impl Fn(&OutputCtl) -> bool) -> SinkResult<bool> {
        let mut last_read = self.ctl.read.load(SeqCst);
        let mut since = Instant::now();
        loop {
            if self.sync() {
                return Ok(true);
            }
            // Aygıt gitti: 2 sn beklemeden dur (7 Kasım: Bluetooth kulaklık).
            self.check_device()?;
            let ctl = Arc::clone(&self.ctl);
            if ready(&ctl) {
                return Ok(false);
            }
            ctl.writer_waiting.store(true, SeqCst);
            if ready(&ctl) || ctl.flush_req.load(SeqCst) != self.flush_seen || ctl.device_lost.load(SeqCst) {
                ctl.writer_waiting.store(false, SeqCst);
                continue;
            }
            thread::park_timeout(PARK);
            ctl.writer_waiting.store(false, SeqCst);
            let r = ctl.read.load(SeqCst);
            if r != last_read {
                last_read = r;
                since = Instant::now();
            } else if since.elapsed() > STALL {
                // Haber vermeden ölen aygıt: gitmiş say, Çal yeniden açsın.
                log::error!("Ses çıkışı {} sn'dir ilerlemiyor (aygıt gitti mi?)", STALL.as_secs());
                ctl.signal_device_lost("çıkış ilerlemiyor");
                ctl.device_notice.store(DEVICE_LOST, SeqCst);
                return Err(SinkError::OnWrite("ses çıkışı ilerlemiyor".into()));
            }
        }
    }
}
