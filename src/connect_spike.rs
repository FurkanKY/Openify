//! `openify connect-spike`: Spotify Connect (Spirc) canlı denemesi. Yalnız
//! `connect-spike` özelliğiyle derlenir; varsayılan derleme bu dosyayı hiç görmez.
//!
//! GERÇEK hesap ve GERÇEK telefon ister: yalnız geliştirici çalıştırır. Bir onay listesidir: her adım telefonda ne yapılacağını
//! söyler, gördüklerini (olaylar, süreler, kaç istek gittiği) yazar, gerekirse
//! sana "e / h" diye sorar ve her sonucu bir günlük dosyasına yazar
//! (`%TEMP%\openify-connect-spike\connect-spike-<an>.log`).
//!
//! Hesap ayarlarına DOKUNMAZ: Spotify'a yalnız kayıtlı girişle bağlanır (Openify'ın
//! kaydı okunur, silinmez, yazılmaz), hesapta "Openify (deneme)" adlı bir Connect
//! aygıtı kaydeder (deneme bitince Spirc kapanırken silinir) ve senin telefondan
//! yaptığın aktarmayı dinler. Kütüphane, takip, çalma listesi ya da ayarlara yazan
//! hiçbir uç çağrılmaz. Openify'ın gerçek ayar dosyasına (`settings.json`) ve
//! önbelleğine de dokunmaz: aygıt kimliği geçici klasörde tutulur.
//!
//! Senaryolar:
//! - S1: bağlı oturumla `Spirc::with_session` (dealer dinleyicileri bağlı oturuma
//!   eklenebiliyor mu; telefon aygıtı görüyor mu?)
//! - S2: etkin değilken yerel çalmaya KARIŞMAMA (şarkı sonu / önyükleme olayları)
//! - S3: telefondan aktarma (bilgisayardan ses, telefondan denetim)
//! - S4: telefon çalmayı geri alınca
//! - S5: yeniden bağlanmada aynı `device_id` (hayalet aygıt yok)
//! - S6: Spotify'ı yormama (boşta `connect-state` isteği yok)
//! - S7: ayna (isteğe bağlı): yerelde başlayan çalma telefonda görünür mü?
//!
//! S2–S6'dan biri kalırsa Connect v1'den çıkar (plan).

use std::{
    collections::BTreeMap,
    io::Write as _,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use librespot::{
    connect::{
        ClusterView, ConnectConfig, HostView, LoadRequest, LoadRequestOptions, PlayingTrack, Spirc, SpircEvent,
    },
    core::{SpotifyUri, config::DeviceType, session::Session},
    metadata::{Metadata, Track},
    playback::{
        config::{Bitrate, PlayerConfig},
        mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
        player::{Player, PlayerEvent},
    },
};
use tokio::sync::{Notify, mpsc};

use crate::{
    auth,
    engine::{self, AudioLevels, OutputCtl},
    link::parse_track,
};

/// Telefonun cihaz listesinde görünen ad.
const DEVICE_NAME: &str = "Openify (deneme)";
/// Varsayılan deneme şarkısı (uzun, her yerde var): isteğe `--track` ile değiştirilir.
const DEFAULT_TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
/// Deneme boyunca ses düzeyi (0..65535): kulağı korumak için düşük.
const START_VOLUME: u16 = 16_384;

// ── Komut satırı ────────────────────────────────────────────────────────

struct Options {
    track: String,
    idle_min: u64,
    only: Option<Vec<String>>,
}

fn parse_options(args: &[String]) -> Result<Options> {
    let mut o = Options { track: DEFAULT_TRACK.to_string(), idle_min: 30, only: None };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().with_context(|| format!("{name} bir değer ister"));
        match arg.as_str() {
            "--track" => o.track = value("--track")?,
            "--idle-min" => {
                o.idle_min = value("--idle-min")?.trim().parse().context("--idle-min bir sayı olmalı (dakika)")?;
            }
            "--only" => {
                let list = value("--only")?;
                o.only = Some(list.split(',').map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).collect());
            }
            "-h" | "--help" => {
                println!(
                    "openify connect-spike [--only S1,S3,...] [--idle-min DAKİKA] [--track spotify:track:ID]\n\
                     Adımlar çalışırken ekranda yazar."
                );
                std::process::exit(0);
            }
            other => bail!("Bilinmeyen seçenek: {other} (yardım: --help)"),
        }
    }
    Ok(o)
}

// ── Günlük ve istek sayacı ──────────────────────────────────────────────

/// Günlük dosyası + Spotify'a giden isteklerin sayacı. librespot her HTTP isteğini
/// `debug!("Requesting {uri}")` ile bildirir (`librespot_core::http_client`); burada
/// sorgu dizgisi atılıp yalnız yol sayılır (jeton taşıyabilir).
struct SpikeLog {
    start: Instant,
    file: Mutex<std::fs::File>,
    http: Mutex<HttpCounts>,
}

#[derive(Default, Clone)]
struct HttpCounts {
    total: u64,
    /// `/connect-state/` yolları: aygıt durumu yazan (PUT / DELETE) uçlar.
    connect_state: u64,
    by_path: BTreeMap<String, u64>,
    /// Bu zamandan beri görülen uyarı / hata sayısı ve ilk birkaçı.
    problems: Vec<String>,
}

static LOG: OnceLock<&'static SpikeLog> = OnceLock::new();

impl SpikeLog {
    fn stamp(&self) -> String {
        format!("[{:8.1}s]", self.start.elapsed().as_secs_f64())
    }

    /// Dosyaya bir satır (konsola yazmaz).
    fn line(&self, tag: &str, text: &str) {
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{} {tag:<6} {text}", self.stamp());
        }
    }

    fn http_snapshot(&self) -> HttpCounts {
        self.http.lock().map(|h| h.clone()).unwrap_or_default()
    }

    fn count_request(&self, uri: &str) {
        let without_query = uri.split('?').next().unwrap_or(uri);
        let path = without_query.split("://").nth(1).unwrap_or(without_query);
        let mut path: String = path.chars().take(90).collect();
        let is_state = path.contains("/connect-state/");
        if path.is_empty() {
            path.push('?');
        }
        if let Ok(mut h) = self.http.lock() {
            h.total += 1;
            if is_state {
                h.connect_state += 1;
            }
            *h.by_path.entry(path.clone()).or_default() += 1;
        }
        self.line("HTTP", &path);
    }
}

impl log::Log for SpikeLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let t = metadata.target();
        t.starts_with("openify") || t.starts_with("librespot")
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let target = record.target();
        let msg = record.args().to_string();
        if target == "librespot_core::http_client" && let Some(uri) = msg.strip_prefix("Requesting ") {
            self.count_request(uri);
            return;
        }
        // Hesap adı günlüğe yazılmasın.
        if msg.contains("Authenticated as") {
            return;
        }
        let level = record.level();
        let interesting = target.starts_with("librespot_connect")
            || target.starts_with("openify")
            || level <= log::Level::Info && target.starts_with("librespot");
        if interesting {
            self.line("LOG", &format!("{level:<5} {target}: {msg}"));
        }
        if level <= log::Level::Warn {
            if let Ok(mut h) = self.http.lock() {
                if h.problems.len() < 50 {
                    h.problems.push(format!("{level} {target}: {msg}"));
                }
            }
            eprintln!("  [librespot {level}] {msg}");
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

fn spike_dir() -> PathBuf {
    std::env::temp_dir().join("openify-connect-spike")
}

/// Takvimsiz UTC damgası (`20261112-153045`): günlük dosya adı için.
fn utc_stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant'ın "civil from days" algoritması.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn init_log() -> Result<(&'static SpikeLog, PathBuf)> {
    let dir = spike_dir();
    std::fs::create_dir_all(&dir).context("Günlük klasörü oluşturulamadı")?;
    let path = dir.join(format!("connect-spike-{}.log", utc_stamp()));
    let file = std::fs::File::create(&path).context("Günlük dosyası oluşturulamadı")?;
    let log: &'static SpikeLog =
        Box::leak(Box::new(SpikeLog { start: Instant::now(), file: Mutex::new(file), http: Mutex::new(HttpCounts::default()) }));
    log::set_logger(log).map_err(|e| anyhow::anyhow!("günlük kurulamadı: {e}"))?;
    log::set_max_level(log::LevelFilter::Debug);
    let _ = LOG.set(log);
    Ok((log, path))
}

// ── Gözlenen olaylar ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Playing,
    Paused,
    Stopped,
    Loading,
    EndOfTrack,
    PreloadDue,
    Unavailable,
    TrackChanged,
    Seeked,
    Volume,
    SessionConnected,
    SessionDisconnected,
    SpircState,
    SpircCluster,
}

#[derive(Default)]
struct Observed {
    /// (başlangıçtan ms, tür, kısa açıklama), sırayla.
    timeline: Vec<(u128, Kind, String)>,
    cluster: Option<ClusterView>,
    clusters: u32,
    host: Option<HostView>,
    states: u32,
}

impl Observed {
    fn count_since(&self, mark: usize, kind: Kind) -> usize {
        self.timeline[mark.min(self.timeline.len())..].iter().filter(|(_, k, _)| *k == kind).count()
    }

    fn active(&self) -> bool {
        self.host.as_ref().is_some_and(|h| h.active)
    }
}

fn short(event: &PlayerEvent) -> Option<(Kind, String)> {
    Some(match event {
        PlayerEvent::Playing { position_ms, .. } => (Kind::Playing, format!("çalıyor {position_ms} ms")),
        PlayerEvent::Paused { position_ms, .. } => (Kind::Paused, format!("duraklatıldı {position_ms} ms")),
        PlayerEvent::Stopped { .. } => (Kind::Stopped, "durdu".into()),
        PlayerEvent::Loading { .. } => (Kind::Loading, "yükleniyor".into()),
        PlayerEvent::EndOfTrack { .. } => (Kind::EndOfTrack, "şarkı bitti".into()),
        PlayerEvent::TimeToPreloadNextTrack { .. } => (Kind::PreloadDue, "önyükleme zamanı".into()),
        PlayerEvent::Unavailable { .. } => (Kind::Unavailable, "şarkı çalınamadı".into()),
        PlayerEvent::TrackChanged { audio_item } => (Kind::TrackChanged, format!("şarkı: {}", audio_item.name)),
        PlayerEvent::Seeked { position_ms, .. } => (Kind::Seeked, format!("sarıldı {position_ms} ms")),
        PlayerEvent::VolumeChanged { volume } => (Kind::Volume, format!("ses düzeyi {volume}")),
        PlayerEvent::SessionConnected { .. } => (Kind::SessionConnected, "oturum bağlandı (Spirc etkin)".into()),
        PlayerEvent::SessionDisconnected { .. } => (Kind::SessionDisconnected, "oturum ayrıldı (Spirc etkin değil)".into()),
        _ => return None,
    })
}

fn push(obs: &Mutex<Observed>, log: &SpikeLog, kind: Kind, text: String) {
    log.line("EVENT", &text);
    if let Ok(mut o) = obs.lock() {
        o.timeline.push((log.start.elapsed().as_millis(), kind, text));
    }
}

fn describe_cluster(c: &ClusterView, own_id: &str) -> String {
    let names: Vec<String> =
        c.devices.iter().map(|d| format!("{}{}", d.name, if d.id == own_id { " (bu)" } else { "" })).collect();
    format!(
        "{} aygıt [{}], etkin: {}, çalıyor: {}",
        c.devices.len(),
        names.join(", "),
        if c.active_device_id.is_empty() { "yok" } else if c.active_device_id == own_id { "bu bilgisayar" } else { "başka aygıt" },
        c.playing
    )
}

// ── Konsol ──────────────────────────────────────────────────────────────

static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);
static INTERRUPT_NOTIFY: OnceLock<Notify> = OnceLock::new();

fn interrupted() -> bool {
    INTERRUPTS.load(Ordering::SeqCst) > 0
}

struct Console {
    lines: mpsc::UnboundedReceiver<String>,
}

impl Console {
    fn start() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = std::thread::Builder::new().name("spike-stdin".into()).spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line.trim().to_string()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self { lines: rx }
    }

    /// Bir satır; stdin kapandıysa ya da Ctrl+C gelirse `None`.
    async fn line(&mut self) -> Option<String> {
        if interrupted() {
            return None;
        }
        let notify = INTERRUPT_NOTIFY.get_or_init(Notify::new);
        tokio::select! {
            line = self.lines.recv() => line,
            _ = notify.notified() => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail,
    Skipped,
}

impl Verdict {
    fn word(self) -> &'static str {
        match self {
            Self::Pass => "GEÇTİ",
            Self::Fail => "KALDI",
            Self::Skipped => "ATLANDI",
        }
    }
}

// ── Deneme ──────────────────────────────────────────────────────────────

struct SpircHandle {
    spirc: Spirc,
    task: tokio::task::JoinHandle<()>,
}

struct Spike {
    log: &'static SpikeLog,
    log_path: PathBuf,
    console: Console,
    session: Session,
    player: Arc<Player>,
    out: Arc<OutputCtl>,
    mixer: Arc<dyn Mixer>,
    obs: Arc<Mutex<Observed>>,
    spirc: Option<SpircHandle>,
    device_id: String,
    track_uri: String,
    track: SpotifyUri,
    results: Vec<(&'static str, Verdict, String)>,
}

impl Spike {
    fn say(&self, text: &str) {
        println!("{text}");
        self.log.line("SAY", text);
    }

    fn mark(&self) -> usize {
        self.obs.lock().map(|o| o.timeline.len()).unwrap_or(0)
    }

    fn with_obs<R>(&self, f: impl FnOnce(&Observed) -> R) -> Option<R> {
        self.obs.lock().ok().map(|o| f(&o))
    }

    /// `cond` doğru olana dek (en çok `timeout`) bekler; doğruysa `true`.
    async fn wait_for(&self, timeout: Duration, cond: impl Fn(&Observed) -> bool) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if self.with_obs(&cond).unwrap_or(false) {
                return true;
            }
            if Instant::now() >= end || interrupted() {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    async fn ask(&mut self, question: &str) -> Answer {
        println!("\n  ? {question}");
        println!("    (e = evet, h = hayır, Enter = atla)");
        self.log.line("ASK", question);
        let answer = match self.console.line().await.as_deref().map(|s| s.to_lowercase()) {
            Some(s) if matches!(s.as_str(), "e" | "evet" | "y" | "yes") => Answer::Yes,
            Some(s) if matches!(s.as_str(), "h" | "hayır" | "hayir" | "n" | "no") => Answer::No,
            _ => Answer::Skip,
        };
        self.log.line("ANSWER", &format!("{answer:?}"));
        answer
    }

    /// Enter'a (ya da Ctrl+C'ye) kadar bekler.
    async fn press_enter(&mut self, text: &str) {
        println!("\n  >> {text}");
        let _ = self.console.line().await;
    }

    fn record(&mut self, id: &'static str, verdict: Verdict, note: impl Into<String>) {
        let note = note.into();
        self.say(&format!("\n  => {id}: {} {note}", verdict.word()));
        self.log.line("RESULT", &format!("{id} {} {note}", verdict.word()));
        self.results.push((id, verdict, note));
    }

    // ── Spirc ──

    async fn start_spirc(&mut self) -> Result<()> {
        let config = ConnectConfig {
            name: DEVICE_NAME.to_string(),
            device_type: DeviceType::Computer,
            initial_volume: START_VOLUME,
            ..ConnectConfig::default()
        };
        let (spirc, task, events) =
            Spirc::with_session(config, self.session.clone(), self.player.clone(), self.mixer.clone())
                .await
                .map_err(|e| anyhow::anyhow!("Spirc::with_session başarısız: {e}"))?;
        let task = tokio::spawn(task);
        let (obs, log, own_id) = (self.obs.clone(), self.log, self.device_id.clone());
        tokio::spawn(pump_spirc(events, obs, log, own_id));
        self.spirc = Some(SpircHandle { spirc, task });
        Ok(())
    }

    /// Spirc'i kapatır (cihaz kaydını Spotify'dan siler) ve görevin bitmesini bekler.
    async fn stop_spirc(&mut self) {
        if let Some(SpircHandle { spirc, task }) = self.spirc.take() {
            let _ = spirc.shutdown();
            match tokio::time::timeout(Duration::from_secs(10), task).await {
                Ok(_) => self.log.line("SPIRC", "kapandı"),
                Err(_) => self.log.line("SPIRC", "10 sn'de kapanmadı"),
            }
        }
    }

    // ── Senaryolar ──

    async fn s1(&mut self) {
        self.say("\n=== S1: bağlı oturumla Spirc (dealer dinleyicileri bağlı oturuma eklenebiliyor mu?) ===");
        let got = self.wait_for(Duration::from_secs(25), |o| o.clusters > 0).await;
        let (own, summary) = self
            .with_obs(|o| {
                let c = o.cluster.clone();
                (
                    c.as_ref().is_some_and(|c| c.devices.iter().any(|d| d.id == self.device_id)),
                    c.map(|c| describe_cluster(&c, &self.device_id)).unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        if got {
            self.say(&format!("  Spotify'dan küme bilgisi geldi: {summary}"));
        } else {
            self.say("  25 sn içinde Spotify'dan küme bilgisi GELMEDİ (dealer / NEW_DEVICE kaydı yürümedi).");
        }
        self.say(&format!("  Bu bilgisayar aygıt listesinde: {}", if own { "evet" } else { "hayır" }));
        self.say(&format!("\n  TELEFONDA: Spotify'ı aç, çalan şarkı çubuğundan / 'Cihazlara bağlan'dan cihaz listesine bak. '{DEVICE_NAME}' görünmeli."));
        let seen = self.ask(&format!("Telefonun cihaz listesinde '{DEVICE_NAME}' görünüyor mu?")).await;
        let verdict = match (got && own, seen) {
            (true, Answer::Yes) => Verdict::Pass,
            (_, Answer::Skip) if got && own => Verdict::Skipped,
            _ => Verdict::Fail,
        };
        self.record("S1", verdict, format!("(küme geldi: {got}, listede: {own}, telefon: {seen:?})"));
    }

    async fn s2(&mut self) {
        self.say("\n=== S2: Spirc etkin değilken yerel çalmaya KARIŞMAZ ===");
        if self.with_obs(|o| o.active()).unwrap_or(false) {
            self.record("S2", Verdict::Skipped, "(Spirc etkin: önce S4 gibi telefonu geri al)");
            return;
        }
        // Şarkının son ~12 sn'sinden çal: şarkı sonu ve önyükleme olayları hemen gelsin.
        let duration_ms = match Track::get(&self.session, &self.track).await {
            Ok(t) => t.duration.max(0) as u32,
            Err(e) => {
                self.record("S2", Verdict::Skipped, format!("(şarkı bilgisi alınamadı: {e})"));
                return;
            }
        };
        let start = duration_ms.saturating_sub(12_000);
        self.say(&format!("  Şarkı bu bilgisayarda yerel çalınıyor: son ~12 sn ({start} ms'den). Sesi duyacaksın."));
        let mark = self.mark();
        self.player.load(self.track.clone(), true, start);
        let playing = self.wait_for(Duration::from_secs(20), |o| o.count_since(mark, Kind::Playing) > 0).await;
        if !playing {
            self.record("S2", Verdict::Fail, "(yerel şarkı 20 sn'de çalmaya başlamadı)");
            return;
        }
        let ended = self.wait_for(Duration::from_secs(40), |o| o.count_since(mark, Kind::EndOfTrack) > 0).await;
        if !ended {
            self.player.stop();
            self.record("S2", Verdict::Fail, "(şarkı sonu olayı 40 sn'de gelmedi)");
            return;
        }
        let after = self.mark();
        // Spirc yamasız olsaydı: EndOfTrack → handle_next → player.stop() → Stopped.
        tokio::time::sleep(Duration::from_secs(8)).await;
        let (stopped, loading, playing_again, preload) = self
            .with_obs(|o| {
                (
                    o.count_since(after, Kind::Stopped),
                    o.count_since(after, Kind::Loading),
                    o.count_since(after, Kind::Playing),
                    o.count_since(mark, Kind::PreloadDue),
                )
            })
            .unwrap_or_default();
        self.say(&format!(
            "  Şarkı sonundan sonra 8 sn: durdu={stopped}, yükleme={loading}, yeniden çalma={playing_again}; önyükleme olayı={preload}"
        ));
        let clean = stopped == 0 && loading == 0 && playing_again == 0;
        let still_inactive = !self.with_obs(|o| o.active()).unwrap_or(false);
        let verdict = if clean && still_inactive { Verdict::Pass } else { Verdict::Fail };
        self.record(
            "S2",
            verdict,
            format!("(Spirc karışmadı: {clean}, Spirc etkin değil: {still_inactive}; karışsaydı 'durdu' olayı gelirdi)"),
        );
    }

    async fn s3(&mut self) {
        self.say("\n=== S3: telefondan aktarma ===");
        self.say(&format!(
            "  TELEFONDA: bir şarkı çal, cihaz listesinden '{DEVICE_NAME}' seç (çalma bilgisayara geçmeli).\n  60 sn bekliyorum..."
        ));
        let mark = self.mark();
        let activated = self.wait_for(Duration::from_secs(60), |o| o.active()).await;
        if !activated {
            self.record("S3", Verdict::Fail, "(60 sn içinde aktarma gelmedi: Spirc etkinleşmedi)");
            return;
        }
        let t0 = Instant::now();
        let playing = self.wait_for(Duration::from_secs(15), |o| o.count_since(mark, Kind::Playing) > 0).await;
        let to_sound = t0.elapsed();
        let (track, ctx, volume) = self
            .with_obs(|o| {
                let h = o.host.clone().unwrap_or_default();
                (h.track_uri, h.context_uri, o.count_since(mark, Kind::Volume))
            })
            .unwrap_or_default();
        self.say(&format!(
            "  Etkinleşti. Çalma başladı: {playing} (etkinleşmeden {} ms sonra). Şarkı: {}, bağlam: {}, ses düzeyi olayı: {volume}",
            to_sound.as_millis(),
            if track.is_empty() { "?" } else { &track },
            if ctx.is_empty() { "yok" } else { &ctx }
        ));
        let heard = self.ask("Ses BU BİLGİSAYARDAN geliyor mu?").await;

        self.say("\n  TELEFONDAN dene (sırayla): duraklat, sürdür, şarkıyı ileri sar, sonraki şarkıya geç, ses düzeyini değiştir.");
        let control_mark = self.mark();
        self.press_enter("Hepsini denedikten sonra Enter'a bas.").await;
        let (paused, resumed, seeked, tracks, volumes) = self
            .with_obs(|o| {
                (
                    o.count_since(control_mark, Kind::Paused),
                    o.count_since(control_mark, Kind::Playing),
                    o.count_since(control_mark, Kind::Seeked),
                    o.count_since(control_mark, Kind::TrackChanged),
                    o.count_since(control_mark, Kind::Volume),
                )
            })
            .unwrap_or_default();
        self.say(&format!(
            "  Bilgisayarın gördüğü: duraklatma={paused}, çalma={resumed}, sarma={seeked}, şarkı değişimi={tracks}, ses düzeyi={volumes}"
        ));
        let control = self.ask("Telefondan denetim (duraklat / sürdür / sar / sonraki / ses) doğru çalıştı mı?").await;
        let verdict = match (playing, heard, control) {
            (true, Answer::Yes, Answer::Yes) => Verdict::Pass,
            (true, _, _) if heard == Answer::Skip || control == Answer::Skip => Verdict::Skipped,
            _ => Verdict::Fail,
        };
        self.record(
            "S3",
            verdict,
            format!("(sese kadar {} ms, ses bilgisayardan: {heard:?}, telefon denetimi: {control:?})", to_sound.as_millis()),
        );
    }

    async fn s4(&mut self) {
        self.say("\n=== S4: telefon çalmayı geri alınca ===");
        if !self.with_obs(|o| o.active()).unwrap_or(false) {
            self.record("S4", Verdict::Skipped, "(Spirc etkin değil: S3 geçmedi ya da atlandı)");
            return;
        }
        self.say("  TELEFONDA: cihaz listesinden telefonu (kendisini) seç; çalma telefona dönmeli. 45 sn bekliyorum...");
        let mark = self.mark();
        let inactive = self.wait_for(Duration::from_secs(45), |o| !o.active()).await;
        if !inactive {
            self.record("S4", Verdict::Fail, "(45 sn içinde Spirc etkinlikten çıkmadı)");
            return;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        let (stopped, playing) =
            self.with_obs(|o| (o.count_since(mark, Kind::Stopped) + o.count_since(mark, Kind::Paused), o.count_since(mark, Kind::Playing))).unwrap_or_default();
        let queued = self.out.queued_frames();
        self.say(&format!("  Etkinlikten çıktı. Bilgisayar: durdu/duraklatıldı olayı={stopped}, yeniden çalma={playing}, çıkışta bekleyen kare={queued}"));
        let silent = self.ask("Bilgisayardan ses KESİLDİ mi (telefon çalıyor)?").await;
        let verdict = match silent {
            Answer::Yes if playing == 0 => Verdict::Pass,
            Answer::Skip => Verdict::Skipped,
            _ => Verdict::Fail,
        };
        self.record("S4", verdict, format!("(durdu olayı: {stopped}, sonradan çalma: {playing}, sessiz: {silent:?})"));
    }

    async fn s5(&mut self) {
        self.say("\n=== S5: yeniden bağlanmada aynı device_id (hayalet aygıt yok) ===");
        let before = self.device_id.clone();
        self.say(&format!("  device_id (ilk 8): {}…; Spirc kapatılıyor, oturum yenileniyor.", &before[..before.len().min(8)]));
        self.stop_spirc().await;
        self.session.shutdown();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let session = match auth::connect_saved_with(None, Some(self.device_id.as_str()), false).await {
            Ok(s) => s,
            Err(e) => {
                self.record("S5", Verdict::Fail, format!("(yeni oturum açılamadı: {e})"));
                return;
            }
        };
        let same = session.device_id() == before;
        self.player.set_session(session.clone());
        self.session = session;
        if let Ok(mut o) = self.obs.lock() {
            o.cluster = None;
            o.clusters = 0;
        }
        let t0 = Instant::now();
        if let Err(e) = self.start_spirc().await {
            self.record("S5", Verdict::Fail, format!("(yeni Spirc kurulamadı: {e})"));
            return;
        }
        let got = self.wait_for(Duration::from_secs(25), |o| o.clusters > 0).await;
        let (own_count, same_name, summary) = self
            .with_obs(|o| {
                let c = o.cluster.clone().unwrap_or_default();
                (
                    c.devices.iter().filter(|d| d.id == self.device_id).count(),
                    c.devices.iter().filter(|d| d.name == DEVICE_NAME).count(),
                    describe_cluster(&c, &self.device_id),
                )
            })
            .unwrap_or_default();
        self.say(&format!("  Yeni oturum {} ms sonra kaydoldu: {summary}", t0.elapsed().as_millis()));
        self.say(&format!("  Oturumun device_id'si öncekiyle aynı: {same}; listede bu kimlik {own_count} kez, bu adla {same_name} aygıt."));
        let one = self.ask(&format!("Telefonun cihaz listesinde '{DEVICE_NAME}' TAM 1 kez mi (çift / hayalet yok)?")).await;
        let auto_ok = got && same && own_count == 1 && same_name == 1;
        let verdict = match (auto_ok, one) {
            (true, Answer::Yes) => Verdict::Pass,
            (true, Answer::Skip) => Verdict::Skipped,
            _ => Verdict::Fail,
        };
        self.record("S5", verdict, format!("(aynı kimlik: {same}, listede: {own_count}/{same_name}, telefon: {one:?})"));
    }

    async fn s6(&mut self, idle_min: u64) {
        self.say(&format!("\n=== S6: Spotify'ı yormama: {idle_min} dk boşta (Ctrl+C: erken bitir) ==="));
        self.say("  Spirc kayıtlı, hiçbir şey çalmıyor. Telefonda bu cihaza DOKUNMA. Spotify'a giden connect-state isteklerini sayıyorum.");
        let base = self.log.http_snapshot();
        let states_before = self.with_obs(|o| o.states).unwrap_or(0);
        let clusters_before = self.with_obs(|o| o.clusters).unwrap_or(0);
        let started = Instant::now();
        let total = Duration::from_secs(idle_min * 60);
        let mut next_report = Duration::from_secs(300);
        while started.elapsed() < total && !interrupted() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if started.elapsed() >= next_report {
                let now = self.log.http_snapshot();
                self.say(&format!(
                    "  {:>3} dk: connect-state istekleri +{}, tüm istekler +{}",
                    started.elapsed().as_secs() / 60,
                    now.connect_state - base.connect_state,
                    now.total - base.total
                ));
                next_report += Duration::from_secs(300);
            }
        }
        let ran = started.elapsed();
        let now = self.log.http_snapshot();
        let state_reqs = now.connect_state - base.connect_state;
        let all_reqs = now.total - base.total;
        let states = self.with_obs(|o| o.states).unwrap_or(0) - states_before;
        let clusters = self.with_obs(|o| o.clusters).unwrap_or(0) - clusters_before;
        self.say(&format!(
            "  {} dk {} sn boşta kaldı: connect-state istekleri={state_reqs}, tüm istekler={all_reqs}, Spirc durum bildirimi={states}, gelen küme güncellemesi={clusters}",
            ran.as_secs() / 60,
            ran.as_secs() % 60
        ));
        for (path, n) in now.by_path.iter().filter(|(p, n)| **n > base.by_path.get(*p).copied().unwrap_or(0)) {
            let delta = n - base.by_path.get(path).copied().unwrap_or(0);
            self.say(&format!("    +{delta}  {path}"));
        }
        let problems = now.problems.len().saturating_sub(base.problems.len());
        if problems > 0 {
            self.say(&format!("  Boşta {problems} uyarı / hata günlüğe düştü (günlük dosyasına bak)."));
        }
        let full = ran >= total;
        let verdict = if state_reqs > 0 {
            Verdict::Fail
        } else if full {
            Verdict::Pass
        } else {
            Verdict::Skipped
        };
        self.record(
            "S6",
            verdict,
            format!("({} sn boşta, connect-state isteği: {state_reqs}{})", ran.as_secs(), if full { "" } else { "; süre dolmadan kesildi" }),
        );
    }

    async fn s7(&mut self) {
        self.say("\n=== S7 (isteğe bağlı): ayna: yerelde başlayan çalma telefonda görünür mü? ===");
        if self.with_obs(|o| o.active()).unwrap_or(false) {
            self.record("S7", Verdict::Skipped, "(Spirc etkin: önce telefonu geri al)");
            return;
        }
        let Some(handle) = self.spirc.as_ref() else {
            self.record("S7", Verdict::Skipped, "(Spirc yok)");
            return;
        };
        // Ana uygulama sürüyor: Spirc çalar olaylarına karışmaz (P2).
        handle.spirc.set_host_drives(true);
        let mark = self.mark();
        self.say("  Şarkı bu bilgisayarda yerel (Spirc'siz) çalınıyor, ~8 sn sonra Spirc'e aynı şarkıyı aynı konumdan yüklettireceğim.");
        self.player.load(self.track.clone(), true, 0);
        let playing = self.wait_for(Duration::from_secs(20), |o| o.count_since(mark, Kind::Playing) > 0).await;
        if !playing {
            self.record("S7", Verdict::Fail, "(yerel şarkı çalmadı)");
            return;
        }
        let t_play = Instant::now();
        tokio::time::sleep(Duration::from_secs(8)).await;
        let position = t_play.elapsed().as_millis() as u32;
        let mark2 = self.mark();
        let uri = self.track_uri.clone();
        let t0 = Instant::now();
        if let Some(handle) = self.spirc.as_ref() {
            let _ = handle.spirc.activate();
            let _ = handle.spirc.load(LoadRequest::from_tracks(
                vec![uri],
                LoadRequestOptions {
                    start_playing: true,
                    seek_to: position,
                    playing_track: Some(PlayingTrack::Index(0)),
                    context_options: None,
                },
            ));
        }
        let sound = self.wait_for(Duration::from_secs(15), |o| o.count_since(mark2, Kind::Playing) > 0).await;
        let took = t0.elapsed();
        let shown = self.wait_for(Duration::from_secs(5), |o| o.host.as_ref().is_some_and(|h| h.active && !h.track_uri.is_empty())).await;
        self.say(&format!(
            "  Spirc etkinleşti ve şarkıyı yükledi: {shown}; ses yeniden başladı: {sound} (yüklemeden ~{} ms sonra)",
            took.as_millis()
        ));
        self.say(&format!("  TELEFONDA: '{DEVICE_NAME}' çalan cihaz olarak görünmeli; şarkı adı, ilerleme ve duraklat / sar çalışmalı."));
        let seen = self.ask("Telefon bu bilgisayarı ÇALAN cihaz olarak, doğru şarkı ve ilerlemeyle gösteriyor mu?").await;
        let glitch = self.ask("Aynayı kurarken ses kesintisi ya da şarkının başa dönmesi DUYDUN mu? (e = duydum)").await;
        let ctl = self.ask("Telefondan duraklat / ileri sar bu bilgisayarı yönetiyor mu?").await;
        // Temizlik: Spirc etkinliğini bırak, çalmayı durdur.
        if let Some(handle) = self.spirc.as_ref() {
            let _ = handle.spirc.disconnect(true);
            handle.spirc.set_host_drives(false);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.player.stop();
        let verdict = match (shown && sound, seen, ctl) {
            (true, Answer::Yes, Answer::Yes) => Verdict::Pass,
            (_, Answer::Skip, _) | (_, _, Answer::Skip) => Verdict::Skipped,
            _ => Verdict::Fail,
        };
        self.record("S7", verdict, format!("(telefonda görünür: {seen:?}, kesinti duyuldu: {glitch:?}, telefondan denetim: {ctl:?})"));
    }
}

/// Spirc'in durum bildirimlerini gözlem listesine aktarır.
async fn pump_spirc(mut events: mpsc::UnboundedReceiver<SpircEvent>, obs: Arc<Mutex<Observed>>, log: &'static SpikeLog, own_id: String) {
    while let Some(event) = events.recv().await {
        match event {
            SpircEvent::State(h) => {
                push(
                    &obs,
                    log,
                    Kind::SpircState,
                    format!(
                        "Spirc durumu: etkin={} çalıyor={} duraklatıldı={} şarkı={} bağlam={} konum={}ms ses={}",
                        h.active, h.playing, h.paused, h.track_uri, h.context_uri, h.position_ms, h.volume
                    ),
                );
                if let Ok(mut o) = obs.lock() {
                    o.states += 1;
                    o.host = Some(h);
                }
            }
            SpircEvent::Cluster(c) => {
                push(&obs, log, Kind::SpircCluster, format!("Küme: {}", describe_cluster(&c, &own_id)));
                if let Ok(mut o) = obs.lock() {
                    o.clusters += 1;
                    o.cluster = Some(c);
                }
            }
        }
    }
}

/// Çaların olaylarını gözlem listesine aktarır; ses düzeyi olaylarını çıkışa uygular
/// (telefondan ses düzeyi değişince bilgisayar da değişsin).
async fn pump_player(
    mut events: librespot::playback::player::PlayerEventChannel,
    obs: Arc<Mutex<Observed>>,
    log: &'static SpikeLog,
    mixer: Arc<dyn Mixer>,
    out: Arc<OutputCtl>,
) {
    let getter = mixer.get_soft_volume();
    while let Some(event) = events.recv().await {
        if let PlayerEvent::VolumeChanged { .. } = event {
            out.set_gain(getter.attenuation_factor() as f32);
        }
        if let Some((kind, text)) = short(&event) {
            push(&obs, log, kind, text);
        }
    }
}

// ── Giriş noktası ───────────────────────────────────────────────────────

/// `openify connect-spike [...]`
pub fn run(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    tokio::runtime::Builder::new_multi_thread().worker_threads(3).enable_all().build()?.block_on(run_async(options))
}

fn wants(options: &Options, id: &str) -> bool {
    options.only.as_ref().is_none_or(|only| only.iter().any(|s| s == id))
}

/// Kayıtlı giriş yoksa ya da bağlanamıyorsa açıklayıcı hata.
async fn connect(device_id: &str) -> Result<Session> {
    match auth::connect_saved_with(None, Some(device_id), false).await {
        Ok(s) => Ok(s),
        Err(auth::ConnectError::NoCredentials) => bail!("Kayıtlı Spotify girişi yok: önce Openify'ı normal aç ve Spotify'a giriş yap, sonra bu denemeyi çalıştır."),
        Err(auth::ConnectError::Rejected) => bail!("Spotify kayıtlı girişi reddetti (kayıt SİLİNMEDİ): Openify'ı normal aç, gerekirse yeniden giriş yap."),
        Err(auth::ConnectError::NeedsPremium) => bail!("Bu hesap Premium değil; Connect denemesi yapılamaz."),
        Err(auth::ConnectError::Offline(e)) => bail!("Spotify'a bağlanılamadı: {e:#}"),
    }
}

/// Ses aygıtı kimliği: bu denemeye özel, geçici klasörde (Openify'ın gerçek ayarına yazılmaz).
fn load_device_id() -> Result<String> {
    let path = spike_dir().join("device-id.txt");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = text.trim().to_string();
        if id.len() >= 16 {
            return Ok(id);
        }
    }
    let id = uuid::Uuid::new_v4().as_hyphenated().to_string();
    std::fs::create_dir_all(spike_dir())?;
    std::fs::write(&path, &id)?;
    Ok(id)
}

async fn run_async(options: Options) -> Result<()> {
    let (log, log_path) = init_log()?;
    INTERRUPT_NOTIFY.get_or_init(Notify::new);
    tokio::spawn(async {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            let n = INTERRUPTS.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= 2 {
                eprintln!("\nİkinci Ctrl+C: hemen çıkılıyor (cihaz Spotify listesinde bir süre kalabilir).");
                std::process::exit(130);
            }
            eprintln!("\nCtrl+C: bu adım bitirilip özetle çıkılacak (tekrar basarsan hemen çıkar).");
            INTERRUPT_NOTIFY.get_or_init(Notify::new).notify_waiters();
        }
    });

    println!("Openify Spotify Connect denemesi\n");
    println!("Bu deneme GERÇEK hesabına bağlanır ve hesapta '{DEVICE_NAME}' adlı bir Connect aygıtı kaydeder");
    println!("(deneme bitince silinir). Hesap ayarlarını, kütüphaneni, listelerini DEĞİŞTİRMEZ; Openify'ın ayar");
    println!("dosyasına ve önbelleğine dokunmaz. Bu bilgisayardan ses çıkacak; telefonun elinin altında olsun.");
    println!("Günlük: {}\n", log_path.display());
    if !engine::has_output_device() {
        bail!("Ses çıkışı (hoparlör / kulaklık) bulunamadı.");
    }
    let track = parse_track(&options.track).context("--track geçersiz")?;
    let track_uri = track.to_uri().map_err(|e| anyhow::anyhow!("şarkı adresi çözülemedi: {e}"))?;

    let mut console = Console::start();
    println!("Devam edilsin mi? (e = evet, başka tuş = çık)");
    if !matches!(console.line().await.as_deref().map(str::to_lowercase).as_deref(), Some("e" | "evet" | "y" | "yes")) {
        println!("İptal edildi; hiçbir şey yapılmadı.");
        return Ok(());
    }

    let device_id = load_device_id()?;
    log.line("SETUP", &format!("device_id {}…, şarkı {track_uri}", &device_id[..8]));
    println!("\nSpotify'a bağlanılıyor (kayıtlı girişle)...");
    let session = connect(&device_id).await?;
    println!("Bağlandı.");

    let mixer: Arc<dyn Mixer> = Arc::new(SoftMixer::open(MixerConfig::default()).map_err(|e| anyhow::anyhow!("ses düzeyi açılamadı: {e}"))?);
    mixer.set_volume(START_VOLUME);
    let gain = mixer.get_soft_volume().attenuation_factor() as f32;
    let config = PlayerConfig { bitrate: Bitrate::Bitrate320, normalisation: true, ..PlayerConfig::default() };
    let (player, out) = engine::new_player(config, session.clone(), AudioLevels::new(), gain, 0);
    let obs = Arc::new(Mutex::new(Observed::default()));
    tokio::spawn(pump_player(player.get_player_event_channel(), obs.clone(), log, mixer.clone(), out.clone()));

    let mut spike = Spike {
        log,
        log_path: log_path.clone(),
        console,
        session,
        player,
        out,
        mixer,
        obs,
        spirc: None,
        device_id,
        track_uri,
        track,
        results: Vec::new(),
    };

    spike.say("Spirc (Spotify Connect aygıtı) kuruluyor...");
    if let Err(e) = spike.start_spirc().await {
        spike.say(&format!("Spirc kurulamadı: {e:#}"));
        spike.record("S1", Verdict::Fail, format!("({e:#})"));
        return finish(spike).await;
    }

    if wants(&options, "S1") {
        spike.s1().await;
    }
    if wants(&options, "S2") && !interrupted() {
        spike.s2().await;
    }
    if wants(&options, "S3") && !interrupted() {
        spike.s3().await;
    }
    if wants(&options, "S4") && !interrupted() {
        spike.s4().await;
    }
    if wants(&options, "S5") && !interrupted() {
        spike.s5().await;
    }
    if wants(&options, "S6") && !interrupted() {
        spike.s6(options.idle_min).await;
    }
    if wants(&options, "S7") && options.only.is_some() && !interrupted() {
        spike.s7().await;
    } else if options.only.is_none() && !interrupted() {
        let ans = spike.ask("S7 (ayna, isteğe bağlı) denensin mi?").await;
        if ans == Answer::Yes {
            spike.s7().await;
        }
    }
    finish(spike).await
}

/// Temizlik ve özet.
async fn finish(mut spike: Spike) -> Result<()> {
    spike.say("\nTemizleniyor: çalma durduruluyor, Spirc kapatılıyor (cihaz kaydı Spotify'dan silinir)...");
    spike.player.stop();
    spike.stop_spirc().await;
    spike.session.shutdown();

    let counts = spike.log.http_snapshot();
    spike.say("\n===== ÖZET =====");
    let results = std::mem::take(&mut spike.results);
    for (id, verdict, note) in &results {
        spike.say(&format!("{id}  {:<8} {note}", verdict.word()));
    }
    spike.say(&format!("\nToplam Spotify HTTP isteği: {} (connect-state: {})", counts.total, counts.connect_state));
    let failed: Vec<&str> = results.iter().filter(|(_, v, _)| *v == Verdict::Fail).map(|(id, ..)| *id).collect();
    if failed.is_empty() {
        spike.say("Kalan senaryo yok (atlananlar hariç).");
    } else {
        spike.say(&format!("KALAN: {}. S2–S6'dan biri kalırsa Connect v1'den çıkar (plan).", failed.join(", ")));
    }
    spike.say(&format!("\nGünlük dosyası: {}\nBu dosyayı (ve yukarıdaki özeti) geliştiriciye ilet.", spike.log_path.display()));
    spike.log.flush_all();
    Ok(())
}

impl SpikeLog {
    fn flush_all(&self) {
        log::Log::flush(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log() -> SpikeLog {
        let path = std::env::temp_dir().join(format!("openify-spike-test-{}.log", std::process::id()));
        SpikeLog { start: Instant::now(), file: Mutex::new(std::fs::File::create(path).unwrap()), http: Mutex::new(HttpCounts::default()) }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_have_defaults_and_parse() {
        let o = parse_options(&[]).unwrap();
        assert_eq!((o.idle_min, o.only.is_none(), o.track.as_str()), (30, true, DEFAULT_TRACK));
        let o = parse_options(&args(&["--only", "s1, S3,,", "--idle-min", "2", "--track", "spotify:track:abc"])).unwrap();
        assert_eq!(o.only, Some(vec!["S1".to_string(), "S3".to_string()]));
        assert_eq!((o.idle_min, o.track.as_str()), (2, "spotify:track:abc"));
        assert!(wants(&o, "S3") && !wants(&o, "S2"));
        assert!(wants(&parse_options(&[]).unwrap(), "S6"), "--only yoksa hepsi");
        assert!(parse_options(&args(&["--idle-min", "x"])).is_err());
        assert!(parse_options(&args(&["--track"])).is_err());
        assert!(parse_options(&args(&["--bilinmeyen"])).is_err());
    }

    #[test]
    fn requests_are_counted_by_path_without_the_query_and_connect_state_is_told_apart() {
        let log = temp_log();
        log.count_request("https://spclient.wg.spotify.com/connect-state/v1/devices/abc?jeton=GIZLI");
        log.count_request("https://spclient.wg.spotify.com/connect-state/v1/devices/abc");
        log.count_request("https://spclient.wg.spotify.com/metadata/4/track/xyz?market=from_token");
        let c = log.http_snapshot();
        assert_eq!((c.total, c.connect_state), (3, 2));
        assert_eq!(c.by_path["spclient.wg.spotify.com/connect-state/v1/devices/abc"], 2);
        assert!(c.by_path.keys().all(|k| !k.contains("GIZLI") && !k.contains('?')), "sorgu dizgisi (jeton) sayılmaz: {:?}", c.by_path);
    }

    #[test]
    fn utc_stamp_is_a_sortable_timestamp() {
        let s = utc_stamp();
        assert_eq!((s.len(), s.as_bytes()[8]), (15, b'-'), "{s}");
        assert!(s.chars().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit()), "{s}");
        assert!(s.starts_with("20"), "{s}");
    }

    #[test]
    fn observations_are_counted_after_a_mark() {
        let mut o = Observed::default();
        o.timeline.push((10, Kind::Playing, "a".into()));
        o.timeline.push((20, Kind::Stopped, "b".into()));
        o.timeline.push((30, Kind::Playing, "c".into()));
        assert_eq!(o.count_since(0, Kind::Playing), 2);
        assert_eq!(o.count_since(1, Kind::Playing), 1);
        assert_eq!(o.count_since(99, Kind::Playing), 0, "işaret sonu aşarsa boş");
        assert!(!o.active());
        o.host = Some(HostView { active: true, ..HostView::default() });
        assert!(o.active());
    }
}
