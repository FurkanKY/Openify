//! Kare aşama ölçümü (geliştirme aracı).
//!
//! `OPENIFY_FRAME_STATS` etkin olduğunda GPUI kare aşamaları, metin şekli ve
//! görsel sorguları, kök render süresi ve UI iş parçacığı CPU'su saniyede bir
//! özetlenir. Kapalıyken bu ölçümlerin hiçbir işi yapılmaz.

use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub fn enabled() -> bool {
    gpui::openify_frame_stats_enabled()
}

#[derive(Clone, Copy)]
pub enum TextPhase {
    Layout,
    Prepaint,
    Paint,
}

#[derive(Clone, Copy)]
pub enum ImageResult {
    Ready,
    Missing,
    Empty,
}

#[derive(Default)]
struct Details {
    started: Option<Instant>,
    frames: u64,
    render: Duration,
    layout: Duration,
    paint: Duration,
    present: Duration,
    text_layout: Duration,
    text_prepaint: Duration,
    text_paint: Duration,
    text_counts: [u64; 3],
    image_lookup: Duration,
    image_results: [u64; 3],
    components: BTreeMap<&'static str, u64>,
    cpu_samples_ms: Vec<f64>,
}

static DETAILS: OnceLock<Mutex<Details>> = OnceLock::new();

thread_local! {
    static FRAME_START: Cell<Option<(Instant, Option<u64>)>> = const { Cell::new(None) };
}

pub fn render_started() {
    if enabled() {
        FRAME_START.with(|start| start.set(Some((Instant::now(), current_thread_cpu_100ns()))));
    }
}

pub fn render_built() {
    if !enabled() {
        return;
    }
    FRAME_START.with(|start| {
        if let Some((started, _)) = start.get() {
            details().render += started.elapsed();
        }
    });
}

pub fn text_spent(duration: Duration, phase: TextPhase) {
    if !enabled() {
        return;
    }
    let mut stats = details();
    match phase {
        TextPhase::Layout => {
            stats.text_layout += duration;
            stats.text_counts[0] += 1;
        }
        TextPhase::Prepaint => {
            stats.text_prepaint += duration;
            stats.text_counts[1] += 1;
        }
        TextPhase::Paint => {
            stats.text_paint += duration;
            stats.text_counts[2] += 1;
        }
    }
}

pub fn image_lookup(duration: Duration, result: ImageResult) {
    if !enabled() {
        return;
    }
    let mut stats = details();
    stats.image_lookup += duration;
    stats.image_results[match result {
        ImageResult::Ready => 0,
        ImageResult::Missing => 1,
        ImageResult::Empty => 2,
    }] += 1;
}

pub fn component_built(name: &'static str) {
    if enabled() {
        *details().components.entry(name).or_default() += 1;
    }
}

pub fn painted(window: &gpui::Window) {
    if !enabled() {
        return;
    }

    let (layout, paint, present) = window.frame_phase_timings();
    window.record_openify_frame_phases();
    let cpu_end = current_thread_cpu_100ns();
    let cpu_sample = FRAME_START.with(|start| {
        start
            .take()
            .and_then(|(_, cpu_start)| Some((cpu_end? - cpu_start?) as f64 / 10_000.0))
    });

    let mut stats = details();
    stats.frames += 1;
    stats.layout += layout;
    stats.paint += paint;
    stats.present += present;
    if let Some(cpu_ms) = cpu_sample {
        stats.cpu_samples_ms.push(cpu_ms);
    }
    let elapsed = stats.started.get_or_insert_with(Instant::now).elapsed();
    if elapsed < Duration::from_secs(1) {
        return;
    }

    let frames = stats.frames.max(1) as f64;
    let per_frame_ms = |duration: Duration| duration.as_secs_f64() * 1000.0 / frames;
    let mut cpu = std::mem::take(&mut stats.cpu_samples_ms);
    cpu.sort_by(f64::total_cmp);
    let percentile = |p: f64| -> f64 {
        cpu.get(((cpu.len() as f64 * p).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0.0)
    };
    let counts = stats.text_counts;
    let images = stats.image_results;
    let components = std::mem::take(&mut stats.components);
    log::info!(target: "openify",
        "OPENIFY_FRAME_DETAIL {:.2}s kare={} ui_cpu_ms_p50={:.3} ui_cpu_ms_p95={:.3} render_ms={:.3} duzen_ms={:.3} boyama_ms={:.3} sunum_ms={:.3} text_layout_ms={:.3}/{} text_prepaint_ms={:.3}/{} text_paint_ms={:.3}/{} image_lookup_ms={:.3} image_ready={} image_missing={} image_empty={} components={:?}",
        elapsed.as_secs_f64(),
        stats.frames,
        percentile(0.50),
        percentile(0.95),
        per_frame_ms(stats.render),
        per_frame_ms(stats.layout),
        per_frame_ms(stats.paint),
        per_frame_ms(stats.present),
        per_frame_ms(stats.text_layout), counts[0],
        per_frame_ms(stats.text_prepaint), counts[1],
        per_frame_ms(stats.text_paint), counts[2],
        per_frame_ms(stats.image_lookup),
        images[0], images[1], images[2], components,
    );
    *stats = Details {
        started: Some(Instant::now()),
        ..Details::default()
    };
}

fn details() -> std::sync::MutexGuard<'static, Details> {
    DETAILS
        .get_or_init(|| Mutex::new(Details::default()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(windows)]
fn current_thread_cpu_100ns() -> Option<u64> {
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn GetThreadTimes(
            thread: *mut std::ffi::c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    // GetThreadTimes tüm FILETIME çıktı işaretçilerini zorunlu tutar; ilk iki
    // sonuç kullanılmasa bile NULL geçirilemez (Windows KERNELBASE NULL'a yazar).
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    // FILETIME is in 100 ns units; only kernel and user time accumulate here.
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    (ok != 0).then(|| {
        let value = |time: FileTime| (u64::from(time.high) << 32) | u64::from(time.low);
        value(kernel) + value(user)
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::current_thread_cpu_100ns;

    #[test]
    fn current_thread_cpu_sample_uses_valid_output_pointers() {
        assert!(current_thread_cpu_100ns().is_some());
    }
}

#[cfg(not(windows))]
fn current_thread_cpu_100ns() -> Option<u64> {
    None
}

/// Ölçüm kancası: `OPENIFY_PROF_HIDE` içinde `part` var mı?
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
