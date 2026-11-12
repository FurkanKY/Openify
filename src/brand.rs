//! Uygulamanın görünen adı: TEK yer.
//!
//! Arayüz yazılarında (`src/i18n/table.rs`), pencere başlığında, tepsi ipucunda
//! ve görev çubuğu / medya denetiminde görünen ad buradan gelir. Adı değiştirmek
//! için yalnız aşağıdaki üç makroyu düzenle (düz, belirtme "-ı", yönelme "-a") (görünen yazı için tek satırlık iş).
//!
//! Türkçede ek ad bozulmasın diye belirtme hâli ayrı bir makro: "Openify'ı yeniden
//! aç". Ek, adın okunuşuna göre değişir (ünlü uyumu), bu yüzden otomatik
//! türetilmiyor, elle yazılır. Yeni adın ünlü uyumuna göre `app_name_acc!`'yi düzelt.
//!
//! Makro olmalarının nedeni `concat!`: çeviri tablosundaki yazılar derleme
//! zamanında birleşir, `&'static str` kalır.
//!
//! DİKKAT: Aşağıdakiler GÖRÜNEN ad değil, KİMLİK; adı değiştirirken bilerek ayrı
//! karar verilir (kurulu kullanıcıda ayar / önbellek / açılış kaydı taşınır):
//! `auth::KEYRING_SERVICE`, `autostart::VALUE_NAME` (Run anahtarı),
//! `settings::path` (%APPDATA%\Openify), `engine::cache` (%LOCALAPPDATA%\Openify),
//! `main::init_logging` günlük klasörü, `system::instance::BASE` + `Openify.Ipc`
//! sınıf adı, `system::set_app_id` (AUMID), `ui` pencere `app_id`, Velopack paket
//! kimliği (`Cargo.toml` / `scripts/surum-hazirla.ps1`), `update.rs` yayın adresi,
//! kullanıcı ajanı (`User-Agent`) dizgeleri.

/// Görünen ad (düz, ek yok).
macro_rules! app_name {
    () => {
        "Openify"
    };
}

/// Görünen ad, Türkçe belirtme hâli ("… -ı yeniden aç").
macro_rules! app_name_acc {
    () => {
        "Openify'ı"
    };
}

/// Görünen ad, Türkçe yönelme hâli ("… -a giriş yapıldı").
macro_rules! app_name_dat {
    () => {
        "Openify'a"
    };
}

pub(crate) use {app_name, app_name_acc, app_name_dat};

/// Görünen ad: pencere başlığı, tepsi ipucu, medya denetimi.
pub const APP_NAME: &str = app_name!();
