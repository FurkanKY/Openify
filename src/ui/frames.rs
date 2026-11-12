//! Kare ölçümü (geliştirme aracı).
//!
//! `OPENIFY_FRAME_STATS=<dosya>` verilirse her karede iki süre kaydedilir:
//! - çizim: pencerenin `render`'ının başından son öğenin boyanmasına kadar
//!   geçen süre (ana iş parçacığında kurma, yerleşim, boyama; ekran kartına
//!   gönderme hariç),
//! - aralık: bir önceki karenin boyanmasından bu yana geçen süre (sürekli
//!   hareket varken kare hızını gösterir).
//!
//! Satırlar `zaman_sn,çizim_ms,aralık_ms` olarak saniyede bir dosyaya
//! bir yazıcı iş parçacığınca her 200 ms'de (değişmişse) dosyaya yazılır;
//! kareler dursa da son satırlar kalır, süreç öldürülse de kayıt eksik
//! olmaz. Değişken yoksa hiçbir şey yapmaz.

use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Recorder {
    origin: Instant,
    render_start: Option<Instant>,
    render_built: Option<Instant>,
    last_paint: Option<Instant>,
    /// Satırlar; yazıcı iş parçacığıyla paylaşılır.
    rows: Arc<Mutex<String>>,
    /// Bu karede `Line` öğelerinde geçen süre ve öğe sayısı.
    text: Duration,
    text_count: u32,
}

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = RefCell::new(
        std::env::var_os("OPENIFY_FRAME_STATS").map(|path| {
            let path = PathBuf::from(path);
            let rows = Arc::new(Mutex::new(String::from(
                "t_s,cpu_ms,interval_ms,build_ms,text_ms,text_n\n",
            )));
            spawn_writer(path, Arc::clone(&rows));
            Recorder {
                origin: Instant::now(),
                render_start: None,
                render_built: None,
                last_paint: None,
                rows,
                text: Duration::ZERO,
                text_count: 0,
            }
        })
    );
}

/// Dosyayı kareler dursa da güncel tutar: kare başına G/Ç yok, ama son
/// satırlar en geç `FLUSH_EVERY` sonra diskte (süreç öldürülse de kalır).
fn spawn_writer(path: PathBuf, rows: Arc<Mutex<String>>) {
    const FLUSH_EVERY: Duration = Duration::from_millis(200);
    let _ = std::thread::Builder::new().name("frame-stats".into()).spawn(move || {
        let mut written = 0;
        loop {
            std::thread::sleep(FLUSH_EVERY);
            let snapshot = {
                let rows = rows.lock().unwrap_or_else(|e| e.into_inner());
                (rows.len() != written).then(|| (rows.len(), rows.clone()))
            };
            if let Some((len, text)) = snapshot {
                if std::fs::write(&path, text).is_ok() {
                    written = len;
                }
            }
        }
    });
}

/// Pencerenin `render`'ı başladı.
pub fn render_started() {
    RECORDER.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut() {
            r.render_start = Some(Instant::now());
        }
    });
}

/// Öğe ağacı kuruldu (`render` bitti; yerleşim ve boyama başlıyor).
pub fn render_built() {
    RECORDER.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut() {
            r.render_built = Some(Instant::now());
        }
    });
}

/// Yazı öğesinde geçen süreyi bu kareye ekle.
pub fn text_spent(d: Duration, new_element: bool) {
    RECORDER.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut() {
            r.text += d;
            r.text_count += new_element as u32;
        }
    });
}

/// Son öğe boyandı: kareyi kaydet.
pub fn painted() {
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        let Some(r) = r.as_mut() else { return };
        let now = Instant::now();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let build = match (r.render_start, r.render_built.take()) {
            (Some(s), Some(b)) => format!("{:.3}", ms(b - s)),
            _ => String::new(),
        };
        let cpu = r.render_start.take().map(|s| format!("{:.3}", ms(now - s)));
        let interval = r.last_paint.map(|l| format!("{:.3}", ms(now - l)));
        r.last_paint = Some(now);
        let row = format!(
            "{:.4},{},{},{},{:.3},{}\n",
            (now - r.origin).as_secs_f64(),
            cpu.unwrap_or_default(),
            interval.unwrap_or_default(),
            build,
            ms(std::mem::take(&mut r.text)),
            std::mem::take(&mut r.text_count)
        );
        r.rows.lock().unwrap_or_else(|e| e.into_inner()).push_str(&row);
    });
}

/// Ölçüm açık mı?
pub fn enabled() -> bool {
    RECORDER.with(|r| r.borrow().is_some())
}

/// Ölçüm kancası: `OPENIFY_PROF_HIDE="shelf,list,..."` içinde `part` var mı?
/// (Değişken bir kez okunur.)
pub fn hidden(part: &str) -> bool {
    static HIDE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    HIDE.get_or_init(|| {
        std::env::var("OPENIFY_PROF_HIDE")
            .map(|v| v.split(',').map(str::to_owned).collect())
            .unwrap_or_default()
    })
    .iter()
    .any(|p| p == part)
}
