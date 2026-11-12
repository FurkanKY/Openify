//! Openify'ın kalıcı ayarları.
//!
//! Gizli olmayan tercihler burada, düz bir JSON dosyasında tutulur:
//!   Windows: %APPDATA%\Openify\settings.json
//! Spotify oturum anahtarı burada değil, Credential Manager'da (bkz. auth.rs).

use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Ayar dosyasının biçim sürümü. Dosya biçimi değişince (alan adı, değerin
/// anlamı) artırılır ve `MIGRATIONS`'a o sürüme götüren adım eklenir; yeni bir
/// alan eklemek göç GEREKTİRMEZ (`#[serde(default)]` varsayılanı doldurur).
///   1  sürümün eklendiği ilk biçim (öncesi: dosyada `version` yok, 0 sayılır)
pub const SETTINGS_VERSION: u32 = 1;

/// Göç adımları: (hedef sürüm, adım). Adım ham JSON üstünde çalışır (tipli
/// okumadan ÖNCE: eski alan adları, anlamı değişen değerler buradan geçer).
type Migration = (u32, fn(&mut Value));
const MIGRATIONS: [Migration; 1] = [(1, v1_versioned)];

/// 0 → 1: yalnız sürüm damgası; sürümsüz dosyaların biçimi zaten 1'in biçimi.
fn v1_versioned(_settings: &mut Value) {}

/// `value`yu `steps`in sonuna kadar yükseltir; (dosyadaki sürüm, bir şey
/// değişti mi) döner. Dosyanın sürümü bilinenden yeniyse (daha yeni bir
/// Openify yazmış) dokunulmaz, sürüm bu kodunkine çekilir: bilinen alanlar
/// okunur, bir sonraki kayıt bu biçimde yazılır ve yeni sürüm kendi göçünü
/// yeniden çalıştırır.
fn migrate_value(value: &mut Value, steps: &[Migration]) -> (u32, bool) {
    let latest = steps.last().map_or(0, |s| s.0);
    let from = value.get("version").and_then(Value::as_u64).map_or(0, |v| v.min(u64::from(u32::MAX)) as u32);
    if from > latest {
        log::warn!("Ayar dosyası daha yeni bir sürümden (v{from}, bu uygulama v{latest}): bilinen alanlar okunuyor");
    }
    let mut changed = false;
    for (to, step) in steps.iter().filter(|(to, _)| *to > from) {
        step(value);
        changed = true;
        log::info!("ayarlar: v{from} → v{to} adımı uygulandı");
    }
    if let Some(map) = value.as_object_mut() {
        map.insert("version".into(), Value::from(latest));
    }
    (from, changed)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Dosya biçiminin sürümü (bkz. `SETTINGS_VERSION`). Yazarken hep güncel.
    pub version: u32,
    /// 0.0–1.0. Pencere kapanırken kaydedilir. Sessizdeyken sessize
    /// almadan önceki düzey (açılınca dönülecek yer).
    pub volume: f32,
    /// Sessize alınmış mı? Ayrı tutulur: düzey 0 yazılsaydı bir sonraki
    /// açılışta "Sesi aç" dönecek bir düzey bulamıyordu.
    pub muted: bool,
    /// Son çalınan listelerin anahtarları (`CollectionId::key`), en yenisi
    /// başta. Kapak rafı bu sırayla dizilir. Kullanıcı bir listeyi çalmaya
    /// başlayınca arayüz yazar; en çok `RECENT_MAX` tane.
    pub recent: Vec<String>,
    /// Açılışta sağdaki "Şimdi çalıyor" paneli açık mı (Ayarlar → Görünüm ve
    /// davranış)? Geniş pencerede (bkz. `ui::now_panel::DOCK_MIN_WIDTH`)
    /// yerleşik, darda yüzen panel. Çalışırken panel açılıp kapanabilir; bu
    /// yalnız bir sonraki açılışın başlangıcıdır. (Eskiden son kapanışı
    /// hatırlıyordu; alan adı aynı kaldı, eski dosyalar bozulmasın.)
    pub now_panel: bool,
    /// Kütüphane sekmesi: kartlar ızgarada mı, listede mi (`"grid"` / `"list"`).
    pub library_view: String,
    /// Kütüphane sıralaması (`LIBRARY_SORTS`): son çalınan, eklenme tarihi, ad, yaratıcı.
    pub library_sort: String,
    /// Kütüphane listesinde açık klasörlerin kalıcı kimlikleri
    /// (`engine::folder_id`: `spotify:start-group:<kimlik>`, ad içermez).
    pub library_open_folders: Vec<String>,
    /// Palette yapılan son aramalar, en yenisi başta (kutu boşken gösterilir).
    pub recent_searches: Vec<String>,

    // ── Ayarlar sayfası (Adım 15) ───────────────────────────────────────
    /// Ses kalitesi: `"normal"` (96 kbps), `"high"` (160), `"very_high"` (320).
    /// Sıradaki şarkıdan geçerli olur.
    pub quality: String,
    /// Ses düzeylerini eşitle (şarkılar arası sesi dengeler).
    pub normalise: bool,
    /// Eşitleme düzeyi (`NORMALISE_LEVELS`): `"quiet"`, `"normal"`, `"loud"`.
    /// Eşitleme kapalıysa (`normalise` yanlış) bir şey ifade etmez.
    pub normalise_level: String,
    /// Ses çıkış aygıtının adı; `None`: Windows'un varsayılan aygıtı.
    pub output_device: Option<String>,
    /// Animasyonları azalt: `"system"` (Windows'un ayarı), `"on"`, `"off"`.
    pub reduce_motion: String,
    /// Kapat düğmesi pencereyi tepsiye gizler (`true`) ya da uygulamadan çıkar.
    pub close_to_tray: bool,
    /// Her yerden arama kısayolu (Ctrl+Shift+Boşluk).
    pub global_hotkey: bool,
    /// Açılışta kaldığı şarkıyı ve sırayı geri yükle.
    pub resume: bool,
    /// Arayüz dili: `"system"` (Windows'un görüntü dili), `"en"`, `"tr"` (bkz. `i18n`).
    pub language: String,

    // ── Karıştırma ve sıra kuralları (bkz. `engine::rules`) ─────────────
    /// Karıştırılmış bağlamda aynı şarkının sürümlerinden (Remastered, Live,
    /// Radio Edit…) yalnız biri çalsın.
    pub dedupe_versions: bool,
    /// Açık sözlü (explicit) şarkıları çalma sırasına alma.
    pub skip_explicit: bool,
    /// "Bu sanatçıyı hiç çalma": otomatik çalma, karıştırma ve Sonraki bu
    /// sanatçıların şarkılarını atlar; kullanıcının kendi seçtiği şarkıya
    /// dokunmaz. En çok `engine::rules::BLOCKED_MAX` kayıt.
    pub blocked_artists: Vec<BlockedArtist>,

    // ── Arkadaş etkinliği (bkz. `engine::friends`, `ui::friends`) ───────
    /// Sağdaki "Arkadaş etkinliği" paneli (Spotify masaüstündeki gibi,
    /// arkadaşların son dinlediği şarkı). Varsayılan KAPALI: açılana dek ne
    /// düğme görünür ne de Spotify'a tek istek gider. Gayriresmî bir uç
    /// kullanılır; yalnız dinlediklerini paylaşan arkadaşlar görünür.
    pub friend_activity: bool,

    // ── Söz çevirisi (bkz. `engine::lyrics`, `ui::lyrics_view`) ─────────
    /// Sözlerde her satırın altında Spotify'ın çevirisi (arayüz dilinde)
    /// gösterilsin. Yalnız çevirisi olan şarkıda bir anlamı var; düğme de
    /// yalnız orada görünür. Varsayılan KAPALI. Tercih kalıcı.
    pub lyrics_translation: bool,

    // ── Karaoke / vokal azaltma (bkz. `engine::output::karaoke`) ────────
    /// Karaoke açık mı: orta kanalın konuşma bandı bastırılır (vokal azalır,
    /// bas ve yan kanal kalır). Varsayılan KAPALI: kapalıyken ses yolu hiç
    /// değişmez. Tercih kalıcı; mono kaynakta kendiliğinden etkisiz.
    pub karaoke_enabled: bool,
    /// Karaoke miktarı, 0.0–1.0 (arayüzde % 0–100).
    pub karaoke_amount: f32,

    // ── Geçiş / crossfade (bkz. `engine::playback::crossfade`) ──────────
    /// Şarkılar arası geçişin süresi, ms. 0 = KAPALI (varsayılan: boşluksuz).
    /// Yalnız şarkı kendiliğinden bitince, farklı albümler arasında uygulanır;
    /// aynı albümün ardışık şarkıları, tekrar "tek" ve kullanıcının atlaması
    /// bindirmesiz kalır. En çok `CROSSFADE_MAX_MS`.
    pub crossfade_ms: u32,
    /// AutoMix (akıllı geçiş): açıkken geçişin yeri ve uzunluğu şarkıların
    /// çözümlemesinden seçilir (`crossfade_ms` yok sayılır, değeri korunur);
    /// aynı kurallar (kendiliğinden geçiş, aynı albüm değil). Varsayılan KAPALI.
    pub automix: bool,

    // ── Ekolayzer (bkz. `engine::equalizer`, `ui::equalizer`) ───────────
    /// Ekolayzer açık mı. Varsayılan KAPALI: kapalıyken ses yolu hiç değişmez.
    pub eq_enabled: bool,
    /// 10 bandın kazancı (dB; 31,5 Hz … 16 kHz). Eksik / bozuk dosyada sıfır.
    pub eq_gains: Vec<f32>,
    /// Elle ön kazanç (dB). Motor ayrıca otomatik bir pay çıkarır.
    pub eq_preamp: f32,
    /// Seçili kulaklık profili (OPRA): kazançlar bundan uydurulmuştur; yalnız
    /// hangi profil olduğunu ve atıf bilgisini tutar (veri depoda yok).
    pub eq_profile: Option<EqProfile>,

    // ── Spotify Connect (bkz. PLAN-MOTOR-CONNECT.md §5, 7.1) ────────────
    /// Bu kurulumun KALICI Spotify aygıt kimliği. İlk açılışta bir kez üretilir
    /// (`Settings::load`) ve dosyada kalır: librespot her oturumda rastgele bir
    /// kimlik üretir, Connect'te ise telefonun listesinde aynı aygıt kalmalı
    /// (yeniden bağlanınca "hayalet aygıt" birikmesin). Elle silinirse yenisi
    /// üretilir; bozuk / garip biçimli değer de atılır (`sanitized`).
    pub device_id: String,
    /// Openify, Spotify Connect aygıtı olarak telefonda / öteki uygulamalarda
    /// görünsün mü? Varsayılan AÇIK (13 Kasım, sahibinin denemesinden sonra);
    /// kapatılınca Spotify'a aygıt kaydı gitmez.
    pub connect_enabled: bool,
    /// Connect aygıt listesinde görünen ad. Varsayılan "Openify (<bilgisayar adı>)".
    pub device_name: String,
}

/// Seçili OPRA kulaklık profilinin kaydı (atıf satırı için).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EqProfile {
    /// OPRA `eq` kimliği.
    pub id: String,
    /// "Sennheiser HD 650".
    pub name: String,
    /// Ölçümü / EQ'yu yapan (CC BY-SA atfı).
    pub author: String,
    /// Ek bilgi ("Harman Target • Standard"), boş olabilir.
    pub details: String,
    /// 10 banda yaklaştırmanın karekök ortalama sapması (dB); 0: bilinmiyor.
    pub deviation_db: f32,
}

/// Ses kalitesi seçenekleri: kayıt anahtarı, kbps.
pub const QUALITIES: [(&str, u16); 3] = [("normal", 96), ("high", 160), ("very_high", 320)];
/// Animasyon tercihi seçenekleri.
pub const MOTION_CHOICES: [&str; 3] = ["system", "on", "off"];
/// Kütüphane sıralaması seçenekleri (kayıt anahtarı; bkz. `ui::shelf::Sort`).
pub const LIBRARY_SORTS: [&str; 4] = ["recent", "added", "name", "creator"];
/// Açık klasör olarak hatırlanan en çok kimlik (silinmiş klasörlerin kimlikleri birikmesin).
pub const LIBRARY_OPEN_FOLDERS_MAX: usize = 200;

/// Ses düzeyi eşitleme düzeyleri: anahtar, hedef gürlük (LUFS). Spotify
/// verisi "Normal"e (−14 LUFS) göre ölçülü gelir; düzey, çaların ön kazancı
/// (`normalisation_pregain_db`) olarak Normal'e göre farkıyla uygulanır:
/// Sessiz −9 dB, Normal 0, Yüksek +3 dB.
pub const NORMALISE_LEVELS: [(&str, f64); 3] = [("quiet", -23.0), ("normal", -14.0), ("loud", -11.0)];
/// Eşitlemenin ölçü aldığı gürlük (Spotify "Normal"i).
pub const NORMALISE_REFERENCE_LUFS: f64 = -14.0;

/// Kara listedeki bir sanatçı. Eşleştirme ADLA yapılır (şarkı satırları
/// sanatçı adlarını taşır, adresleri değil); adres, varsa yalnız kayıt için.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedArtist {
    pub name: String,
    #[serde(default)]
    pub uri: String,
}

/// Arayüzün sunduğu geçiş süreleri (ms; 0 kapalı). Dosyada başka bir değer
/// (elle yazılmış) olabilir: motor onu da uygular, arayüz en yakınını gösterir.
pub const CROSSFADE_CHOICES_MS: [u32; 5] = [0, 2_000, 5_000, 8_000, 12_000];
/// Geçişin üst sınırı (motorun bindirme sınırı).
pub const CROSSFADE_MAX_MS: u32 = crate::engine::MAX_OVERLAP_MS;

/// Saklanan son arama sayısı.
pub const RECENT_SEARCHES_MAX: usize = 8;
/// Hatırlanan son çalınan liste sayısı.
pub const RECENT_MAX: usize = 50;

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            volume: 0.8,
            muted: false,
            recent: Vec::new(),
            now_panel: true,
            library_view: "grid".into(),
            library_sort: "recent".into(),
            library_open_folders: Vec::new(),
            recent_searches: Vec::new(),
            quality: "very_high".into(),
            normalise: true,
            normalise_level: "normal".into(),
            output_device: None,
            reduce_motion: "system".into(),
            close_to_tray: true,
            global_hotkey: true,
            resume: true,
            language: "system".into(),
            dedupe_versions: true,
            skip_explicit: false,
            blocked_artists: Vec::new(),
            friend_activity: false,
            lyrics_translation: false,
            karaoke_enabled: false,
            karaoke_amount: 1.0,
            crossfade_ms: 0,
            automix: false,
            eq_enabled: false,
            eq_gains: vec![0.0; crate::engine::equalizer::BANDS],
            eq_preamp: 0.0,
            eq_profile: None,
            // Boş kalır: kimliği `Settings::load` üretir (varsayılanın içinde
            // rastgelelik olsa "{}" ile `default()` birbirine eşit olmazdı).
            device_id: String::new(),
            connect_enabled: true,
            device_name: default_device_name(),
        }
    }
}

/// Connect aygıtının varsayılan adı: "Openify (<bilgisayar adı>)"; bilgisayar
/// adı bilinmiyorsa yalnız uygulama adı.
fn default_device_name() -> String {
    device_name_for(std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().as_deref())
}

fn device_name_for(computer: Option<&str>) -> String {
    match computer.map(str::trim).filter(|c| !c.is_empty()) {
        Some(pc) => format!("{} ({pc})", crate::brand::APP_NAME),
        None => crate::brand::APP_NAME.to_string(),
    }
}

/// Aygıt adının en uzun hâli (karakter): Spotify listesinde taşmasın.
pub const DEVICE_NAME_MAX: usize = 64;

/// Yeni bir aygıt kimliği (rastgele UUID, tireli: librespot'un kendi biçimi).
fn new_device_id() -> String {
    uuid::Uuid::new_v4().as_hyphenated().to_string()
}

/// Dosyadan gelen bir kimlik kullanılabilir mi? (Elle düzenlenmiş dosyada boş /
/// bozuk olabilir.) ASCII harf, rakam ve tire; 16–64 karakter.
fn valid_device_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `key` şimdi çalındı: listenin başına alınır (önceki yeri silinir), liste
/// `RECENT_MAX`'ta kesilir.
pub fn note_played(recent: &mut Vec<String>, key: &str) {
    recent.retain(|k| k != key);
    recent.insert(0, key.to_string());
    recent.truncate(RECENT_MAX);
}

impl Settings {
    /// Ayarları okur. Dosya yoksa varsayılanları döndürür. Dosya bozuksa
    /// (yarım yazılmış, elle bozulmuş) son sağlam yedekten (`settings.json.bak`)
    /// geri yükler; yedek de yoksa varsayılanlara döner. Her durumda
    /// kullanıcıyı kaybettiren sessiz bir sıfırlama olmasın diye günlüğe yazılır;
    /// ayarlar yüzünden uygulama açılmamazlık da etmez.
    pub fn load() -> Self {
        path().map(|p| Self::load_ensuring_id(&p)).unwrap_or_else(|| {
            let mut settings = Self::default();
            settings.ensure_device_id();
            settings
        })
    }

    /// `load_from` + aygıt kimliği yoksa üretip hemen yazar: kimlik "bir kez
    /// üretilir"in tek yeri. Yazılamazsa (salt okunur klasör) kimlik bu oturumda
    /// bellekte kalır, bir sonraki açılışta yeniden denenir.
    fn load_ensuring_id(path: &std::path::Path) -> Self {
        let mut settings = Self::load_from(path);
        if settings.ensure_device_id() {
            match settings.save_to(path) {
                Ok(()) => log::info!("Spotify aygıt kimliği üretildi ve kaydedildi"),
                Err(e) => log::warn!("Aygıt kimliği yazılamadı (bir sonraki açılışta yeniden denenir): {e:#}"),
            }
        }
        settings
    }

    /// Kimlik boşsa üretir; üretildiyse `true` (dosyaya yazılmalı).
    fn ensure_device_id(&mut self) -> bool {
        if self.device_id.is_empty() {
            self.device_id = new_device_id();
            true
        } else {
            false
        }
    }

    /// `load`'ın yol alan hâli (testler gerçek ayar dosyasına dokunmasın).
    fn load_from(path: &std::path::Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            // Dosya yok: ilk açılış (ya da kullanıcı ayarları silmiş); yedekten
            // geri getirmek istenmeyen bir diriltme olurdu.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                log::warn!("Ayar dosyası okunamadı ({}): {e}", path.display());
                return Self::restore_from_backup(path).unwrap_or_default();
            }
        };
        match Self::parse(&text) {
            Ok((settings, migrated)) => {
                // Sağlam: yedek de bu olsun (bir sonraki bozulmaya karşı). Göç
                // varsa yedek, göçten ÖNCEKİ metindir: geri dönüş yolu.
                refresh_backup(path, &text);
                let settings = settings.sanitized();
                if migrated {
                    // Göç edilmiş hâl hemen yazılır (yedek yukarıda eski metni tuttu).
                    match settings.save_to(path) {
                        Ok(()) => log::info!("Ayar dosyası v{SETTINGS_VERSION} biçimine geçirildi ({})", path.display()),
                        Err(e) => log::warn!("Göç edilen ayarlar yazılamadı (bir sonraki kayıtta yazılır): {e:#}"),
                    }
                }
                settings
            }
            Err(e) => {
                log::warn!("Ayar dosyası bozuk ({}): {e}", path.display());
                // Bozuk dosya incelenebilsin diye yanına alınır (en son biri kalır).
                let corrupt = path.with_extension("json.corrupt");
                if let Err(e) = fs::rename(path, &corrupt) {
                    log::warn!("Bozuk ayar dosyası kenara alınamadı: {e}");
                }
                match Self::restore_from_backup(path) {
                    Some(settings) => settings,
                    None => {
                        log::warn!("Ayar yedeği de yok ya da bozuk: varsayılan ayarlarla açılıyor");
                        Self::default()
                    }
                }
            }
        }
    }

    /// `settings.json.bak` sağlamsa onu asıl dosyanın yerine koyar ve döndürür.
    fn restore_from_backup(path: &std::path::Path) -> Option<Self> {
        let backup = backup_path(path);
        let text = fs::read_to_string(&backup).ok()?;
        let (settings, _) = Self::parse(&text).ok()?;
        match fs::write(path, &text) {
            Ok(()) => log::warn!("Ayarlar son sağlam yedekten geri yüklendi ({})", backup.display()),
            Err(e) => log::warn!("Yedekten okundu ama asıl ayar dosyasına yazılamadı: {e}"),
        }
        Some(settings.sanitized())
    }

    pub fn save(&self) -> Result<()> {
        let path = path().context("Ayar klasörü bulunamadı")?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let path = path.to_path_buf();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).context("Ayar klasörü oluşturulamadı")?;
        }
        // Üzerine yazmadan önce mevcut dosya sağlamsa yedeği o olur (bozuk
        // bir dosya yedeği ezmez).
        if let Ok(previous) = fs::read_to_string(&path) {
            if Self::parse(&previous).is_ok() {
                refresh_backup(&path, &previous);
            }
        }
        // Önce geçici dosyaya yaz, sonra yerine taşı: yazma yarıda kesilirse
        // eski ayarlar bozulmasın.
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(&tmp, &path).context("Ayarlar kaydedilemedi")?;
        Ok(())
    }

    /// Dosya metnini okur: ham JSON önce göç adımlarından geçer, sonra tipli
    /// okunur. İkinci değer: dosya eski bir sürümdendi (yeniden yazılmalı).
    fn parse(text: &str) -> Result<(Self, bool), serde_json::Error> {
        let mut value: Value = serde_json::from_str(text)?;
        let (from, _) = migrate_value(&mut value, &MIGRATIONS);
        let settings: Self = serde_json::from_value(value)?;
        Ok((settings, from < SETTINGS_VERSION))
    }

    fn sanitized(mut self) -> Self {
        self.crossfade_ms = self.crossfade_ms.min(CROSSFADE_MAX_MS);
        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            Self::default().volume
        };
        // Elle düzenlenmiş dosyada boş ya da tekrarlanan anahtarlar olabilir.
        let mut seen = std::collections::HashSet::new();
        self.recent.retain(|k| !k.is_empty() && seen.insert(k.clone()));
        self.recent.truncate(RECENT_MAX);
        let mut seen = std::collections::HashSet::new();
        self.recent_searches.retain(|q| !q.trim().is_empty() && seen.insert(q.clone()));
        self.recent_searches.truncate(RECENT_SEARCHES_MAX);
        if !matches!(self.library_view.as_str(), "grid" | "list") {
            self.library_view = Self::default().library_view;
        }
        if !LIBRARY_SORTS.contains(&self.library_sort.as_str()) {
            self.library_sort = Self::default().library_sort;
        }
        let mut seen = std::collections::HashSet::new();
        self.library_open_folders.retain(|k| k.starts_with("spotify:start-group:") && seen.insert(k.clone()));
        self.library_open_folders.truncate(LIBRARY_OPEN_FOLDERS_MAX);
        if !QUALITIES.iter().any(|(key, _)| *key == self.quality) {
            self.quality = Self::default().quality;
        }
        if !MOTION_CHOICES.contains(&self.reduce_motion.as_str()) {
            self.reduce_motion = Self::default().reduce_motion;
        }
        if !crate::i18n::PREFS.contains(&self.language.as_str()) {
            self.language = Self::default().language;
        }
        // Boş aygıt adı "varsayılan" demektir.
        if self.output_device.as_ref().is_some_and(|d| d.trim().is_empty()) {
            self.output_device = None;
        }
        if !NORMALISE_LEVELS.iter().any(|(key, _)| *key == self.normalise_level) {
            self.normalise_level = Self::default().normalise_level;
        }
        // Kara liste: boş adlar, aynı ad (büyük / küçük harf farkı yok) iki kez
        // ve fazlası atılır.
        let mut seen = std::collections::HashSet::new();
        self.blocked_artists.retain_mut(|a| {
            a.name = a.name.trim().to_string();
            !a.name.is_empty() && seen.insert(crate::engine::fold_name(&a.name))
        });
        self.blocked_artists.truncate(crate::engine::BLOCKED_MAX);
        // Karaoke miktarı: 0..1 (elle düzenlenmiş dosya; sonlu olmayan varsayılana).
        self.karaoke_amount = if self.karaoke_amount.is_finite() { self.karaoke_amount.clamp(0.0, 1.0) } else { 1.0 };
        // Ekolayzer: tam 10 bant, sınırlar içinde (elle düzenlenmiş dosya).
        let eq = self.eq_setting();
        self.eq_gains = eq.gains_db.to_vec();
        self.eq_preamp = eq.preamp_db;
        if self.eq_profile.as_ref().is_some_and(|p| p.id.is_empty()) {
            self.eq_profile = None;
        }
        // Connect: bozuk kimlik atılır (`load` yenisini üretir); boş / uzun ad düzelir.
        self.device_id = self.device_id.trim().to_string();
        if !self.device_id.is_empty() && !valid_device_id(&self.device_id) {
            log::warn!("Ayar dosyasındaki aygıt kimliği geçersiz, yenisi üretilecek");
            self.device_id.clear();
        }
        let name = self.device_name.trim();
        self.device_name = if name.is_empty() {
            default_device_name()
        } else {
            name.chars().take(DEVICE_NAME_MAX).collect()
        };
        self
    }

    /// Ekolayzer ayarı (motora `Command::SetEq` olarak gider). Eksik ya da
    /// fazla bant sayısı sıfırla tamamlanır / kesilir; değerler sınırlanır.
    pub fn eq_setting(&self) -> crate::engine::equalizer::EqSetting {
        let mut gains = [0.0f32; crate::engine::equalizer::BANDS];
        for (slot, v) in gains.iter_mut().zip(&self.eq_gains) {
            *slot = *v;
        }
        crate::engine::equalizer::EqSetting { enabled: self.eq_enabled, gains_db: gains, preamp_db: self.eq_preamp }.sanitized()
    }

    /// Seçili ses kalitesinin kbps karşılığı.
    pub fn kbps(&self) -> u16 {
        QUALITIES.iter().find(|(key, _)| *key == self.quality).map_or(320, |(_, kbps)| *kbps)
    }

    /// Açılışta çalınacak düzey.
    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    /// Eşitleme düzeyinin çaldaki ön kazancı (dB), Normal'e göre. Eşitleme
    /// kapalıyken anlamsız (çalar eşitlemeyi hiç uygulamaz).
    pub fn normalise_pregain_db(&self) -> f64 {
        NORMALISE_LEVELS
            .iter()
            .find(|(key, _)| *key == self.normalise_level)
            .map_or(0.0, |(_, lufs)| lufs - NORMALISE_REFERENCE_LUFS)
    }

    /// Karıştırma ve sıra kuralları (motora gider, bkz. `engine::rules`).
    pub fn queue_rules(&self) -> crate::engine::Rules {
        crate::engine::Rules::new(self.dedupe_versions, self.skip_explicit, self.blocked_artists.iter().map(|a| a.name.as_str()))
    }

    /// Bu sanatçı kara listede mi (ad, büyük / küçük harf farkı yok)?
    pub fn is_blocked(&self, name: &str) -> bool {
        let folded = crate::engine::fold_name(name);
        self.blocked_artists.iter().any(|a| crate::engine::fold_name(&a.name) == folded)
    }

    /// Kara listeye ekler / çıkarır. Değişti mi?
    pub fn set_blocked(&mut self, name: &str, uri: &str, blocked: bool) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let folded = crate::engine::fold_name(name);
        let at = self.blocked_artists.iter().position(|a| crate::engine::fold_name(&a.name) == folded);
        match (at, blocked) {
            (None, true) if self.blocked_artists.len() < crate::engine::BLOCKED_MAX => {
                self.blocked_artists.push(BlockedArtist { name: name.to_string(), uri: uri.to_string() });
                true
            }
            (Some(i), false) => {
                self.blocked_artists.remove(i);
                true
            }
            _ => false,
        }
    }
}

/// Son sağlam ayarların yedeği: `settings.json.bak`.
fn backup_path(path: &std::path::Path) -> PathBuf {
    path.with_extension("json.bak")
}

/// Yedek zaten bu metinse dokunmaz; değilse yazar (yazamazsa günlüğe).
fn refresh_backup(path: &std::path::Path, good_text: &str) {
    let backup = backup_path(path);
    if fs::read_to_string(&backup).is_ok_and(|b| b == good_text) {
        return;
    }
    if let Err(e) = fs::write(&backup, good_text) {
        log::warn!("Ayar yedeği yazılamadı ({}): {e}", backup.display());
    }
}

fn path() -> Option<PathBuf> {
    // Sınama kancası: OPENIFY_SETTINGS_PATH verilirse ayarlar oradan okunur
    // ve oraya yazılır. Sınama betikleri kullanıcının gerçek ayar dosyasına
    // hiç dokunmaz (aynı anda çalışan betikler birbirinin geçici "ses 0"
    // değerini asıl değer sanıp geri yazıyordu).
    if let Some(p) = std::env::var_os("OPENIFY_SETTINGS_PATH") {
        return Some(PathBuf::from(p));
    }
    Some(dirs::config_dir()?.join("Openify").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn language_defaults_to_system_and_is_validated() {
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert_eq!(old.language, "system");
        let bad: Settings = serde_json::from_str(r#"{"language": "fr"}"#).unwrap();
        assert_eq!(bad.sanitized().language, "system");
        for ok in ["system", "en", "tr"] {
            let s: Settings = serde_json::from_str(&format!(r#"{{"language": "{ok}"}}"#)).unwrap();
            assert_eq!(s.sanitized().language, ok);
        }
    }

    #[test]
    fn out_of_range_volume_is_clamped() {
        let s: Settings = serde_json::from_str(r#"{"volume": 3.5}"#).unwrap();
        assert_eq!(s.sanitized().volume, 1.0);
    }

    #[test]
    fn muted_keeps_the_level_to_return_to() {
        let s: Settings = serde_json::from_str(r#"{"volume": 0.6, "muted": true}"#).unwrap();
        assert_eq!(s.effective_volume(), 0.0);
        assert_eq!(s.volume, 0.6);
        // Eski dosyalar (muted alanı yok) sessiz açılmaz.
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert_eq!(old.effective_volume(), 0.5);
        assert!(old.recent.is_empty());
    }

    #[test]
    fn now_panel_defaults_open_and_remembers_closing() {
        // Eski dosyalarda alan yok: panel açık gelir.
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert!(old.now_panel);
        let closed: Settings = serde_json::from_str(r#"{"now_panel": false}"#).unwrap();
        assert!(!closed.now_panel);
        let json = serde_json::to_string(&closed).unwrap();
        assert!(!serde_json::from_str::<Settings>(&json).unwrap().now_panel);
    }

    #[test]
    fn library_view_and_sort_default_and_are_validated() {
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert_eq!((old.library_view.as_str(), old.library_sort.as_str()), ("grid", "recent"));
        let bad: Settings = serde_json::from_str(r#"{"library_view": "x", "library_sort": "y"}"#).unwrap();
        let bad = bad.sanitized();
        assert_eq!((bad.library_view.as_str(), bad.library_sort.as_str()), ("grid", "recent"));
        let ok: Settings = serde_json::from_str(r#"{"library_view": "list", "library_sort": "name"}"#).unwrap();
        let ok = ok.sanitized();
        assert_eq!((ok.library_view.as_str(), ok.library_sort.as_str()), ("list", "name"));
        // Dört sıralamanın hepsi kalıcı; eski dosyada klasör durumu yok.
        for sort in LIBRARY_SORTS {
            let json = format!(r#"{{"library_sort": "{sort}"}}"#);
            let s: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(s.sanitized().library_sort, sort);
        }
        assert!(old.library_open_folders.is_empty());
    }

    #[test]
    fn open_folders_are_kept_deduplicated_and_validated() {
        let json = r#"{"library_open_folders": ["spotify:start-group:a", "x", "spotify:start-group:a", "spotify:start-group:b"]}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.sanitized().library_open_folders, ["spotify:start-group:a", "spotify:start-group:b"]);
        let many = Settings {
            library_open_folders: (0..LIBRARY_OPEN_FOLDERS_MAX + 20).map(|i| format!("spotify:start-group:{i}")).collect(),
            ..Settings::default()
        };
        assert_eq!(many.sanitized().library_open_folders.len(), LIBRARY_OPEN_FOLDERS_MAX);
    }

    #[test]
    fn recent_searches_default_empty_and_are_cleaned() {
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert!(old.recent_searches.is_empty());
        let s: Settings = serde_json::from_str(r#"{"recent_searches": ["tarkan", " ", "tarkan", "sezen"]}"#).unwrap();
        assert_eq!(s.sanitized().recent_searches, ["tarkan", "sezen"]);
    }

    #[test]
    fn recently_played_moves_to_front_and_is_capped() {
        let mut recent = Vec::new();
        note_played(&mut recent, "liked");
        note_played(&mut recent, "spotify:album:a");
        note_played(&mut recent, "liked");
        assert_eq!(recent, ["liked", "spotify:album:a"]);
        for i in 0..(RECENT_MAX + 10) {
            note_played(&mut recent, &format!("spotify:playlist:{i}"));
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], format!("spotify:playlist:{}", RECENT_MAX + 9));
    }

    /// Eski bir dosya (Ayarlar sayfasından önce yazılmış): yeni alanların
    /// hepsi varsayılanla gelir, mevcut davranış değişmez.
    #[test]
    fn old_settings_file_loads_with_the_new_defaults() {
        let old = r#"{"volume": 0.4, "muted": false, "recent": ["liked"], "now_panel": false,
                      "library_view": "list", "library_sort": "name", "recent_searches": ["a"]}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        let s = s.sanitized();
        assert_eq!((s.volume, s.now_panel, s.library_view.as_str()), (0.4, false, "list"));
        assert_eq!(s.quality, "very_high");
        assert_eq!(s.kbps(), 320, "eskiden de 320 kbps çalıyordu");
        assert!(s.normalise, "eskiden eşitleme açıktı");
        assert_eq!(s.output_device, None);
        assert_eq!(s.reduce_motion, "system");
        assert!(s.close_to_tray && s.global_hotkey && s.resume);
    }

    #[test]
    fn new_settings_round_trip() {
        let s = Settings {
            quality: "normal".into(),
            normalise: false,
            output_device: Some("Hoparlör (Realtek)".into()),
            reduce_motion: "on".into(),
            close_to_tray: false,
            global_hotkey: false,
            resume: false,
            ..Settings::default()
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.sanitized(), s);
    }

    #[test]
    fn unknown_choices_fall_back_to_defaults() {
        let s: Settings =
            serde_json::from_str(r#"{"quality": "ultra", "reduce_motion": "belki", "output_device": "  "}"#).unwrap();
        let s = s.sanitized();
        assert_eq!((s.quality.as_str(), s.reduce_motion.as_str()), ("very_high", "system"));
        assert_eq!(s.output_device, None);
    }

    #[test]
    fn quality_maps_to_librespot_bitrates() {
        let kbps = |q: &str| Settings { quality: q.into(), ..Settings::default() }.kbps();
        assert_eq!((kbps("normal"), kbps("high"), kbps("very_high")), (96, 160, 320));
    }

    // ── Karıştırma ve sıra kuralları, eşitleme düzeyi ──

    #[test]
    fn queue_rule_fields_default_for_old_files_and_round_trip() {
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5, "normalise": true}"#).unwrap();
        assert!(old.dedupe_versions, "sürüm eleme varsayılan açık");
        assert!(!old.skip_explicit);
        assert!(old.blocked_artists.is_empty());
        assert_eq!(old.normalise_level, "normal");
        assert_eq!(old.version, SETTINGS_VERSION, "yeni alanlar göç gerektirmez");

        let mut s = Settings { skip_explicit: true, dedupe_versions: false, normalise_level: "loud".into(), ..Settings::default() };
        s.set_blocked("Drake", "spotify:artist:3TVXtAsR1Inumwj472S9r4", true);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sanitized(), s);
        // Adres alanı olmayan eski bir kayıt da okunur.
        let bare: Settings = serde_json::from_str(r#"{"blocked_artists": [{"name": "Drake"}]}"#).unwrap();
        assert_eq!(bare.blocked_artists[0].uri, "");
    }

    #[test]
    fn friend_activity_is_off_by_default_and_needs_no_migration() {
        assert!(!Settings::default().friend_activity);
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5, "now_panel": false}"#).unwrap();
        assert!(!old.friend_activity, "eski dosyada alan yok: kapalı");
        assert_eq!(old.version, SETTINGS_VERSION, "yeni alan göç gerektirmez, sürüm artmaz");
        let on = Settings { friend_activity: true, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert!(back.sanitized().friend_activity);
    }

    #[test]
    fn karaoke_is_off_by_default_persists_and_needs_no_migration() {
        let d = Settings::default();
        assert!(!d.karaoke_enabled && d.karaoke_amount == 1.0);
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert!(!old.karaoke_enabled && old.karaoke_amount == 1.0, "eski dosyada alan yok: kapalı");
        let on = Settings { karaoke_enabled: true, karaoke_amount: 0.4, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        let back = back.sanitized();
        assert!(back.karaoke_enabled && back.karaoke_amount == 0.4);
        let wild: Settings = serde_json::from_str(r#"{"karaoke_amount": 7.5}"#).unwrap();
        assert_eq!(wild.sanitized().karaoke_amount, 1.0);
        let neg: Settings = serde_json::from_str(r#"{"karaoke_amount": -3}"#).unwrap();
        assert_eq!(neg.sanitized().karaoke_amount, 0.0);
    }

    #[test]
    fn lyrics_translation_is_off_by_default_persists_and_needs_no_migration() {
        assert!(!Settings::default().lyrics_translation);
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert!(!old.lyrics_translation, "eski dosyada alan yok: kapalı");
        assert_eq!(old.version, SETTINGS_VERSION, "yeni alan göç gerektirmez, sürüm artmaz");
        let on = Settings { lyrics_translation: true, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert!(back.sanitized().lyrics_translation);
    }

    #[test]
    fn crossfade_is_off_by_default_persists_is_capped_and_needs_no_migration() {
        assert_eq!(Settings::default().crossfade_ms, 0);
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert_eq!(old.crossfade_ms, 0, "eski dosyada alan yok: kapalı");
        assert_eq!(old.version, SETTINGS_VERSION, "yeni alan göç gerektirmez, sürüm artmaz");
        let on = Settings { crossfade_ms: 5_000, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert_eq!(back.sanitized().crossfade_ms, 5_000);
        let wild: Settings = serde_json::from_str(r#"{"crossfade_ms": 90000}"#).unwrap();
        assert_eq!(wild.sanitized().crossfade_ms, CROSSFADE_MAX_MS);
        assert_eq!(CROSSFADE_MAX_MS, 12_000);
        // AutoMix: varsayılan kapalı, eski dosyada alan yok = kapalı, açıkken sabit süre korunur.
        assert!(!Settings::default().automix && !old.automix);
        let auto = Settings { automix: true, crossfade_ms: 8_000, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&auto).unwrap()).unwrap();
        assert!(back.automix && back.sanitized().crossfade_ms == 8_000);
        assert!(CROSSFADE_CHOICES_MS.iter().all(|ms| *ms <= CROSSFADE_MAX_MS));
    }

    // ── Spotify Connect alanları (7.1) ──

    #[test]
    fn connect_fields_default_on_and_need_no_migration() {
        let d = Settings::default();
        assert!(d.connect_enabled, "Connect varsayılan AÇIK");
        assert_eq!(d.device_id, "", "kimliği default değil load üretir");
        assert!(d.device_name.starts_with("Openify"), "{}", d.device_name);
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert!(old.connect_enabled && old.device_id.is_empty());
        assert_eq!(old.device_name, d.device_name);
        assert_eq!(old.version, SETTINGS_VERSION, "yeni alanlar göç gerektirmez, sürüm artmaz");
    }

    #[test]
    fn default_device_name_carries_the_computer_name() {
        assert_eq!(device_name_for(Some("FURKAN-PC")), "Openify (FURKAN-PC)");
        assert_eq!(device_name_for(Some("  Masaüstü ")), "Openify (Masaüstü)");
        assert_eq!(device_name_for(Some("   ")), "Openify");
        assert_eq!(device_name_for(None), "Openify");
    }

    #[test]
    fn device_name_is_trimmed_defaulted_and_capped() {
        let blank: Settings = serde_json::from_str(r#"{"device_name": "   "}"#).unwrap();
        assert_eq!(blank.sanitized().device_name, default_device_name());
        let padded: Settings = serde_json::from_str(r#"{"device_name": "  Salon  "}"#).unwrap();
        assert_eq!(padded.sanitized().device_name, "Salon");
        let long = "ş".repeat(DEVICE_NAME_MAX + 30);
        let capped = Settings { device_name: long, ..Settings::default() }.sanitized();
        assert_eq!(capped.device_name.chars().count(), DEVICE_NAME_MAX, "karakter sayısıyla kesilir, bayt değil");
    }

    #[test]
    fn device_id_is_a_uuid_and_every_new_one_differs() {
        let (a, b) = (new_device_id(), new_device_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert!(valid_device_id(&a), "{a}");
    }

    #[test]
    fn device_id_validation_rejects_odd_values() {
        assert!(valid_device_id("0123456789abcdef"));
        assert!(valid_device_id("3f9c1b0e-44aa-4c52-9d6e-0a1b2c3d4e5f"));
        for bad in ["", "kısa", "0123456789abcde", "has space 0123456789", "ünlü-0123456789abcdef", &"a".repeat(65)] {
            assert!(!valid_device_id(bad), "{bad:?}");
        }
        let s: Settings = serde_json::from_str(r#"{"device_id": "bozuk id!"}"#).unwrap();
        assert_eq!(s.sanitized().device_id, "", "geçersiz kimlik atılır (load yenisini üretir)");
        let ok: Settings = serde_json::from_str(r#"{"device_id": " 0123456789abcdef "}"#).unwrap();
        assert_eq!(ok.sanitized().device_id, "0123456789abcdef", "kenar boşluğu kırpılır, geçerli kimlik korunur");
    }

    /// Kimlik İLK açılışta bir kez üretilir ve dosyada kalır; sonraki açılışlar,
    /// ayar kayıtları ve yeniden yüklemeler aynı kimliği verir.
    #[test]
    fn device_id_is_generated_once_and_persists() {
        let dir = temp_dir("device-id");
        let path = dir.join("settings.json");
        let first = Settings::load_ensuring_id(&path);
        assert!(valid_device_id(&first.device_id), "{}", first.device_id);
        assert!(path.exists(), "ilk açılışta hemen yazılır");
        let on_disk: Settings = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk.device_id, first.device_id);
        for _ in 0..3 {
            assert_eq!(Settings::load_ensuring_id(&path).device_id, first.device_id, "her açılışta aynı");
        }
        // Kullanıcı bir ayarı değiştirip kaydedince (arayüz `..prefs.clone()` ile yazar) kimlik gitmez.
        let changed = Settings { quality: "high".into(), connect_enabled: true, ..first.clone() };
        changed.save_to(&path).unwrap();
        let again = Settings::load_ensuring_id(&path);
        assert_eq!((again.device_id.as_str(), again.connect_enabled), (first.device_id.as_str(), true));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Elle silinen ya da bozulan kimlik yenilenir ve YENİSİ kalıcı olur;
    /// geçerli, elle yazılmış bir kimliğe dokunulmaz.
    #[test]
    fn a_missing_or_broken_id_is_regenerated_a_valid_custom_one_is_kept() {
        let dir = temp_dir("device-id-repair");
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, r#"{"volume": 0.3, "device_id": "bozuk id!"}"#).unwrap();
        let fixed = Settings::load_ensuring_id(&path);
        assert!(valid_device_id(&fixed.device_id));
        assert_eq!(fixed.volume, 0.3, "öteki ayarlar korunur");
        assert_eq!(Settings::load_ensuring_id(&path).device_id, fixed.device_id, "yenisi kalıcı");

        fs::write(&path, r#"{"device_id": "my-own-device-0001"}"#).unwrap();
        assert_eq!(Settings::load_ensuring_id(&path).device_id, "my-own-device-0001");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn connect_fields_round_trip_through_the_file() {
        let dir = temp_dir("connect-round-trip");
        let path = dir.join("settings.json");
        let s = Settings {
            device_id: "3f9c1b0e-44aa-4c52-9d6e-0a1b2c3d4e5f".into(),
            connect_enabled: true,
            device_name: "Salon bilgisayarı".into(),
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Ekolayzer ayarı: kapalı varsayılan, eski dosyada alan yok (göç yok),
    /// dosyaya yazılıp okununca aynen gelir, bozuk / eksik değerler düzelir.
    #[test]
    fn equalizer_is_off_by_default_persists_and_is_sanitized() {
        let d = Settings::default();
        assert!(!d.eq_enabled && d.eq_profile.is_none() && d.eq_preamp == 0.0);
        assert_eq!(d.eq_gains, vec![0.0; 10]);
        let old: Settings = serde_json::from_str(r#"{"volume": 0.5}"#).unwrap();
        assert!(!old.eq_enabled, "eski dosyada alan yok: kapalı");
        assert_eq!(old.eq_gains.len(), 10);
        assert_eq!(old.version, SETTINGS_VERSION, "yeni alan göç gerektirmez, sürüm artmaz");
        assert!(!old.eq_setting().enabled);

        // Dosya turu: açık, kazançlar, ön kazanç ve profil kaydı kalıcı.
        let dir = temp_dir("equalizer");
        let path = dir.join("settings.json");
        let gains = vec![6.0, 5.5, 4.5, 2.5, 0.5, 0.0, 0.0, 0.0, -1.5, 3.0];
        let profile = EqProfile { id: "a::b".into(), name: "Acme Alpha".into(), author: "Lab".into(), details: "x".into(), deviation_db: 0.8 };
        let s = Settings {
            eq_enabled: true,
            eq_gains: gains.clone(),
            eq_preamp: -1.5,
            eq_profile: Some(profile.clone()),
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        let back = Settings::load_from(&path);
        assert!(back.eq_enabled);
        assert_eq!(back.eq_gains, gains);
        assert_eq!(back.eq_preamp, -1.5);
        assert_eq!(back.eq_profile, Some(profile));
        let set = back.eq_setting();
        assert!(set.enabled && set.gains_db[0] == 6.0 && set.gains_db[8] == -1.5 && set.preamp_db == -1.5);
        let _ = fs::remove_dir_all(&dir);

        // Elle düzenlenmiş dosya: kısa liste sıfırla tamamlanır, uzun kesilir,
        // sınır dışı çekilir; kimliksiz profil atılır.
        let bad: Settings = serde_json::from_str(
            r#"{"eq_enabled": true, "eq_gains": [99, -99, 3], "eq_preamp": 40, "eq_profile": {"name": "x"}}"#,
        )
        .unwrap();
        let bad = bad.sanitized();
        assert_eq!(bad.eq_gains.len(), 10);
        assert_eq!(&bad.eq_gains[..4], &[12.0, -12.0, 3.0, 0.0]);
        assert_eq!(bad.eq_preamp, 12.0);
        assert!(bad.eq_profile.is_none());
        let long: Settings = serde_json::from_str(r#"{"eq_gains": [1,1,1,1,1,1,1,1,1,1,1,1,1]}"#).unwrap();
        assert_eq!(long.sanitized().eq_gains.len(), 10);
    }

    #[test]
    fn blocked_artists_are_trimmed_deduplicated_and_capped() {
        let json = r#"{"blocked_artists": [{"name": "  Drake "}, {"name": "drake"}, {"name": " "}, {"name": ""}, {"name": "Future"}]}"#;
        let s = serde_json::from_str::<Settings>(json).unwrap().sanitized();
        let names: Vec<&str> = s.blocked_artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Drake", "Future"]);
        let many = (0..crate::engine::BLOCKED_MAX + 50).map(|i| format!(r#"{{"name": "A{i}"}}"#)).collect::<Vec<_>>().join(",");
        let s = serde_json::from_str::<Settings>(&format!(r#"{{"blocked_artists": [{many}]}}"#)).unwrap().sanitized();
        assert_eq!(s.blocked_artists.len(), crate::engine::BLOCKED_MAX);
    }

    #[test]
    fn blocking_is_case_insensitive_and_reversible() {
        let mut s = Settings::default();
        assert!(s.set_blocked("Ezhel", "", true));
        assert!(!s.set_blocked("EZHEL", "", true), "ikinci kez eklenmez");
        assert!(s.is_blocked("ezhel") && s.is_blocked(" Ezhel "));
        assert!(!s.is_blocked("Ezhel Bell"));
        assert!(s.queue_rules().blocks("Sagopa Kajmer, Ezhel"));
        assert!(!s.set_blocked("", "", true), "boş ad eklenmez");
        assert!(s.set_blocked("ezhel", "", false));
        assert!(!s.set_blocked("ezhel", "", false), "olmayan çıkarılmaz");
        assert!(s.blocked_artists.is_empty());
        assert!(s.queue_rules().is_empty() || s.queue_rules().filters_nothing());
    }

    #[test]
    fn queue_rules_follow_the_settings() {
        let s = Settings { dedupe_versions: true, skip_explicit: true, ..Settings::default() };
        let r = s.queue_rules();
        assert!(r.dedupe_versions && r.skip_explicit && !r.is_empty());
        let off = Settings { dedupe_versions: false, skip_explicit: false, ..Settings::default() };
        assert!(off.queue_rules().is_empty());
    }

    #[test]
    fn normalise_levels_map_to_pregain_relative_to_normal() {
        let pregain = |l: &str| Settings { normalise_level: l.into(), ..Settings::default() }.normalise_pregain_db();
        assert_eq!(pregain("normal"), 0.0);
        assert_eq!(pregain("quiet"), -9.0, "-23 LUFS, Normal'e göre");
        assert_eq!(pregain("loud"), 3.0, "-11 LUFS, Normal'e göre");
        let bad: Settings = serde_json::from_str(r#"{"normalise_level": "ultra"}"#).unwrap();
        assert_eq!(bad.sanitized().normalise_level, "normal");
        // Her düzey anahtarı Ayarlar sayfasındaki seçeneklerle aynı.
        let keys: Vec<&str> = NORMALISE_LEVELS.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["quiet", "normal", "loud"]);
    }

    /// Dosyaya yazıp okuma, geçici bir yolda (gerçek `%APPDATA%` dosyasına
    /// hiç dokunulmaz); eksik dosya ve (yedeksiz) bozuk dosya varsayılanlara döner.
    #[test]
    fn file_round_trip_in_a_temp_path() {
        let dir = temp_dir("round-trip");
        let path = dir.join("settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default(), "dosya yok");
        let s = Settings { quality: "high".into(), now_panel: false, ..Settings::default() };
        s.save_to(&path).unwrap();
        // Yedek yok (ilk kayıt, öncesi yoktu) ve henüz okunmadı.
        assert!(!backup_path(&path).exists());
        fs::write(&path, "{ bozuk").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default(), "bozuk dosya, yedek yok");
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openify-ayar-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn good(volume: f32) -> Settings {
        Settings { volume, quality: "high".into(), ..Settings::default() }
    }

    #[test]
    fn loading_a_good_file_keeps_a_backup_of_it() {
        let dir = temp_dir("backup-on-load");
        let path = dir.join("settings.json");
        good(0.3).save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), good(0.3));
        let bak: Settings = serde_json::from_str(&fs::read_to_string(backup_path(&path)).unwrap()).unwrap();
        assert_eq!(bak, good(0.3));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_keeps_the_previous_good_file_as_backup() {
        let dir = temp_dir("backup-on-save");
        let path = dir.join("settings.json");
        good(0.3).save_to(&path).unwrap();
        good(0.6).save_to(&path).unwrap();
        let bak: Settings = serde_json::from_str(&fs::read_to_string(backup_path(&path)).unwrap()).unwrap();
        assert_eq!(bak.volume, 0.3, "yedek bir önceki sağlam hâl");
        assert_eq!(Settings::load_from(&path).volume, 0.6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_restored_from_the_backup_and_logged_aside() {
        let dir = temp_dir("restore");
        let path = dir.join("settings.json");
        good(0.4).save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), good(0.4)); // yedek yazılır
        fs::write(&path, "{\"volume\": 0.9, \"quality\"").unwrap(); // yarım yazılmış
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, good(0.4), "son sağlam yedekten");
        // Asıl dosya da onarıldı: bir sonraki açılış sorunsuz.
        assert_eq!(Settings::load_from(&path), good(0.4));
        // Bozuk dosya silinmeden yanına alındı.
        assert_eq!(fs::read_to_string(path.with_extension("json.corrupt")).unwrap(), "{\"volume\": 0.9, \"quality\"");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_does_not_overwrite_the_good_backup_on_save() {
        let dir = temp_dir("bad-save");
        let path = dir.join("settings.json");
        good(0.4).save_to(&path).unwrap();
        Settings::load_from(&path); // yedek: 0.4
        fs::write(&path, "kırık").unwrap();
        good(0.7).save_to(&path).unwrap(); // bozuk dosya yedeği ezmemeli
        let bak: Settings = serde_json::from_str(&fs::read_to_string(backup_path(&path)).unwrap()).unwrap();
        assert_eq!(bak.volume, 0.4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_and_corrupt_backup_fall_back_to_defaults() {
        let dir = temp_dir("both-bad");
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "{").unwrap();
        fs::write(backup_path(&path), "[1, 2").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        // Boş dosya (yarım kalmış yazma) da bozuk sayılır.
        fs::write(&path, "").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_not_resurrected_from_the_backup() {
        let dir = temp_dir("missing");
        let path = dir.join("settings.json");
        good(0.4).save_to(&path).unwrap();
        Settings::load_from(&path);
        fs::remove_file(&path).unwrap(); // kullanıcı ayarları sildi
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn values_restored_from_the_backup_are_still_sanitized() {
        let dir = temp_dir("sanitize");
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "yok").unwrap();
        fs::write(backup_path(&path), r#"{"volume": 7.0, "quality": "ultra"}"#).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!((s.volume, s.quality.as_str()), (1.0, "very_high"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hand_edited_recent_list_is_cleaned() {
        let s: Settings =
            serde_json::from_str(r#"{"recent": ["liked", "", "liked", "spotify:album:a"]}"#).unwrap();
        assert_eq!(s.sanitized().recent, ["liked", "spotify:album:a"]);
    }

    // ── Sürüm ve göç ────────────────────────────────────────────────────

    #[test]
    fn files_without_a_version_are_migrated_and_stamped() {
        let (s, migrated) = Settings::parse(r#"{"volume": 0.4, "quality": "high"}"#).unwrap();
        assert!(migrated, "sürümsüz dosya 0 sayılır, yeniden yazılmalı");
        assert_eq!((s.version, s.volume, s.quality.as_str()), (SETTINGS_VERSION, 0.4, "high"));
        // Güncel dosya göç gerektirmez.
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(&format!("\"version\":{SETTINGS_VERSION}")), "{json}");
        let (back, migrated) = Settings::parse(&json).unwrap();
        assert!(!migrated);
        assert_eq!(back, Settings::default());
    }

    #[test]
    fn every_version_from_zero_loads_to_the_same_settings() {
        let want = Settings { volume: 0.35, quality: "high".into(), recent: vec!["liked".into()], ..Settings::default() };
        for v in 0..=SETTINGS_VERSION {
            let mut value = serde_json::to_value(&want).unwrap();
            if v == 0 {
                value.as_object_mut().unwrap().remove("version");
            } else {
                value["version"] = v.into();
            }
            let (s, migrated) = Settings::parse(&value.to_string()).unwrap();
            assert_eq!(s, want, "sürüm {v}");
            assert_eq!(migrated, v < SETTINGS_VERSION, "sürüm {v}");
        }
    }

    #[test]
    fn migration_steps_run_in_order_on_raw_json() {
        fn rename(v: &mut Value) {
            let old = v.as_object_mut().unwrap().remove("ses");
            if let Some(old) = old {
                v["volume"] = old;
            }
        }
        fn halve(v: &mut Value) {
            let half = v["volume"].as_f64().unwrap_or(0.0) / 2.0;
            v["volume"] = half.into();
        }
        let steps: [Migration; 3] = [(1, v1_versioned), (2, rename), (3, halve)];
        // v0: eski alan adı, ikisi de uygulanır, sırayla.
        let mut v: Value = serde_json::from_str(r#"{"ses": 0.8}"#).unwrap();
        assert_eq!(migrate_value(&mut v, &steps), (0, true));
        assert_eq!((v["volume"].as_f64(), v["version"].as_u64()), (Some(0.4), Some(3)));
        // v2: yalnız üçüncü adım.
        let mut v: Value = serde_json::from_str(r#"{"version": 2, "volume": 0.8}"#).unwrap();
        assert_eq!(migrate_value(&mut v, &steps), (2, true));
        assert_eq!(v["volume"].as_f64(), Some(0.4));
        // Güncel: dokunulmaz.
        let mut v: Value = serde_json::from_str(r#"{"version": 3, "volume": 0.8}"#).unwrap();
        assert_eq!(migrate_value(&mut v, &steps), (3, false));
        assert_eq!(v["volume"].as_f64(), Some(0.8));
        // Daha yeni: adım çalışmaz, sürüm bu kodunkine çekilir.
        let mut v: Value = serde_json::from_str(r#"{"version": 9, "volume": 0.8}"#).unwrap();
        assert_eq!(migrate_value(&mut v, &steps), (9, false));
        assert_eq!((v["volume"].as_f64(), v["version"].as_u64()), (Some(0.8), Some(3)));
        // Sürüm alanı bozuksa (sayı değil) 0 sayılır.
        let mut v: Value = serde_json::from_str(r#"{"version": "x", "ses": 1.0}"#).unwrap();
        assert_eq!(migrate_value(&mut v, &steps).0, 0);
    }

    #[test]
    fn loading_an_unversioned_file_rewrites_it_and_keeps_the_old_text_as_backup() {
        let dir = temp_dir("migrate-on-load");
        let path = dir.join("settings.json");
        fs::create_dir_all(&dir).unwrap();
        let old = r#"{ "volume": 0.35, "quality": "high" }"#;
        fs::write(&path, old).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!((s.version, s.volume, s.quality.as_str()), (SETTINGS_VERSION, 0.35, "high"));
        // Dosya artık sürümlü, değerler aynı.
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\""), "{text}");
        // Göçten önceki metin yedekte: geri dönüş yolu (bir sonraki sağlam
        // açılış yedeği her zamanki gibi yeniler).
        assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), old);
        assert_eq!(Settings::load_from(&path), s);
        // Bozuk dosya kurtarma yolu göçten sonra da çalışır.
        fs::write(&path, "{ kırık").unwrap();
        assert_eq!(Settings::load_from(&path).quality, "high", "yedekten");
        assert!(fs::read_to_string(path.with_extension("json.corrupt")).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
