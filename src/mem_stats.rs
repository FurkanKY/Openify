//! Bellek hesabı (yalnız `--features mem-stats` ile derlenir).
//!
//! Süreç özel belleğinin ne kadarı Rust yığını, ne kadarı SQLite, ne kadarı
//! "geri kalan" (gpui / DirectWrite / GPU sürücüsü) görmek için:
//! `System` ayırıcısını saran bir `#[global_allocator]` canlı bayt ve canlı
//! ayırma sayısını (gevşek atomiklerle) tutar. `OPENIFY_MEM_STATS=<csv yolu>`
//! verilirse arka plan iş parçacığı her 500 ms'de bir satır ekler:
//!   t_s,rust_live_mb,rust_peak_mb,rust_allocs,sqlite_mb,sqlite_peak_mb
//! Her satır hemen yazılır (flush), süreç öldürülse de veri kalır.
//! SQLite sayıları `sqlite3_memory_used()` / `sqlite3_memory_highwater(0)`'dan
//! gelir (rusqlite "bundled" olduğundan `rusqlite::ffi` içinde var).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

struct Sayac;

static CANLI: AtomicUsize = AtomicUsize::new(0);
static ZIRVE: AtomicUsize = AtomicUsize::new(0);
static ADET: AtomicUsize = AtomicUsize::new(0);

fn ekle(boyut: usize) {
    let simdi = CANLI.fetch_add(boyut, Relaxed) + boyut;
    ADET.fetch_add(1, Relaxed);
    ZIRVE.fetch_max(simdi, Relaxed);
}

fn cikar(boyut: usize) {
    CANLI.fetch_sub(boyut, Relaxed);
    ADET.fetch_sub(1, Relaxed);
}

unsafe impl GlobalAlloc for Sayac {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            ekle(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            ekle(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        cikar(l.size());
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, yeni: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, yeni) };
        if !q.is_null() {
            // Ayırma sayısı değişmez; yalnız bayt farkı.
            if yeni >= l.size() {
                let simdi = CANLI.fetch_add(yeni - l.size(), Relaxed) + (yeni - l.size());
                ZIRVE.fetch_max(simdi, Relaxed);
            } else {
                CANLI.fetch_sub(l.size() - yeni, Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static AYIRICI: Sayac = Sayac;

/// `OPENIFY_MEM_STATS` verilmişse yazıcı iş parçacığını başlatır.
pub fn start() {
    let Some(yol) = std::env::var_os("OPENIFY_MEM_STATS") else {
        return;
    };
    let yol = std::path::PathBuf::from(yol);
    if let Some(dir) = yol.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut dosya) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&yol)
    else {
        return;
    };
    if dosya.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        let _ = writeln!(
            dosya,
            "t_s,rust_live_mb,rust_peak_mb,rust_allocs,sqlite_mb,sqlite_peak_mb"
        );
    }
    let _ = std::thread::Builder::new()
        .name("mem-stats".into())
        .spawn(move || {
            const MB: f64 = 1024.0 * 1024.0;
            let t0 = Instant::now();
            loop {
                // SAFETY: parametresiz/bayraklı salt okuma çağrıları; SQLite
                // başlatılmamış olsa da güvenli (sayaçlar 0 döner).
                let (sq, sq_zirve) = unsafe {
                    (
                        rusqlite::ffi::sqlite3_memory_used(),
                        rusqlite::ffi::sqlite3_memory_highwater(0),
                    )
                };
                let _ = writeln!(
                    dosya,
                    "{:.1},{:.2},{:.2},{},{:.2},{:.2}",
                    t0.elapsed().as_secs_f64(),
                    CANLI.load(Relaxed) as f64 / MB,
                    ZIRVE.load(Relaxed) as f64 / MB,
                    ADET.load(Relaxed),
                    sq as f64 / MB,
                    sq_zirve as f64 / MB,
                );
                let _ = dosya.flush();
                std::thread::sleep(Duration::from_millis(500));
            }
        });
}
