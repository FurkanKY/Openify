//! Kare aşama ölçümü (geliştirme aracı).
//!
//! `OPENIFY_FRAME_STATS` etkin olduğunda DXGI sunulan/yansıtılan kare sayaçları
//! ile GPUI düzen, boyama ve sunum süreleri saniyede tek bir günlük satırında
//! birleştirilir.

use std::time::Duration;

pub fn enabled() -> bool {
    gpui::openify_frame_stats_enabled()
}

pub fn render_started() {}
pub fn render_built() {}
pub fn text_spent(_: Duration, _: bool) {}

pub fn painted(window: &gpui::Window) {
    if enabled() {
        window.record_openify_frame_phases();
    }
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
